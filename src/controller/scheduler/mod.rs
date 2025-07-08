pub mod queue;

use std::sync::Arc;

use anyhow::Result;
use async_channel::Receiver;
use tracing::{error, info};

use crate::{
    controller::{
        Controller, ReconcileNext,
        context::{ControllerContext, ControllerEvent, ControllerKey},
        scheduler::queue::WorkQueue,
    },
    machinery::store::Store,
    repository::Repository,
};

pub struct SchedulerConfig {
    pub worker_count: usize,
}

pub struct Scheduler {
    pub store: Arc<Store>,
    pub repository: Arc<Repository>,
    config: SchedulerConfig,
    queue: WorkQueue,
    rx: Receiver<ControllerKey>,
    ctrl: Arc<Vec<Box<dyn Controller>>>,
}

impl Scheduler {
    pub fn new(
        store: Arc<Store>,
        repository: Arc<Repository>,
        config: SchedulerConfig,
        ctrls: Vec<Box<dyn Controller>>,
    ) -> Self {
        let (queue, rx) = WorkQueue::new();

        Self {
            store,
            repository,
            config,
            queue,
            rx,
            ctrl: Arc::new(ctrls),
        }
    }

    pub fn start_workers(&self) {
        info!("starting {} workers", self.config.worker_count);
        for _ in 0..self.config.worker_count {
            let queue = self.queue.clone();
            let store = self.store.clone();
            let repository = self.repository.clone();
            let ctrl = self.ctrl.clone();
            let rx = self.rx.clone();

            tokio::spawn(async move {
                while let Ok(key) = rx.recv().await {
                    for ctrl in ctrl.iter() {
                        let ctx = ControllerContext::new(
                            key.tenant.clone(),
                            store.clone(),
                            repository.clone(),
                        );

                        if !ctrl.should_reconcile(ctx.clone(), key.clone()).await {
                            continue;
                        }

                        let reconcile = ctrl.reconcile(ctx.clone(), key.clone()).await;

                        let next = match reconcile {
                            Ok(next) => next,
                            Err(e) => ctrl.handle_error(ctx, key.clone(), e).await,
                        };

                        match next {
                            ReconcileNext::Done => {}
                            ReconcileNext::Immediate => {
                                queue.push(&key).await;
                            }
                            ReconcileNext::After(duration) => {
                                queue.push_after(&key, duration);
                            }
                        }
                    }

                    queue.done(&key).await;
                }
            });
        }
    }

    pub async fn push(&self, tenant: impl AsRef<str>, ev: ControllerEvent) -> Result<()> {
        for ctrl in self.ctrl.iter() {
            let ctx = ControllerContext::new(
                tenant.as_ref(),
                self.store.clone(),
                self.repository.clone(),
            );

            let result = ctrl.schedule(ctx, ev.clone()).await;
            match result {
                Ok(Some(key)) => {
                    self.queue.push(&key).await;
                }
                Ok(None) => {}
                Err(e) => {
                    error!("failed to schedule event for controller: {}", e);
                }
            }
        }

        Ok(())
    }
}
