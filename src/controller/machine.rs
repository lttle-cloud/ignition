use std::{
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use tracing::{error, info};

use crate::{
    controller::{
        Controller, ReconcileNext,
        context::{ControllerContext, ControllerEvent, ControllerKey},
    },
    resource_index::ResourceKind,
};

pub struct MachineController;

impl MachineController {
    pub fn new_boxed() -> Box<Self> {
        Box::new(Self)
    }
}

#[async_trait]
impl Controller for MachineController {
    async fn schedule(
        &self,
        ctx: ControllerContext,
        event: ControllerEvent,
    ) -> Result<Option<ControllerKey>> {
        info!("scheduling machine controller for event: {:?}", event);
        let key = match event {
            ControllerEvent::ResourceChange(kind, metadata) => Some(ControllerKey::new(
                ctx.tenant.clone(),
                kind,
                metadata.namespace,
                metadata.name,
            )),
            _ => None,
        };
        Ok(key)
    }

    async fn should_reconcile(&self, _ctx: ControllerContext, key: ControllerKey) -> bool {
        info!(
            "should reconcile machine controller for key: {}",
            key.to_string()
        );

        return key.kind == ResourceKind::Machine;
    }

    async fn reconcile(&self, ctx: ControllerContext, key: ControllerKey) -> Result<ReconcileNext> {
        info!(
            "reconciling machine controller for key: {}",
            key.to_string()
        );

        let Some((machine, status)) = ctx
            .repository
            .machine(ctx.tenant.clone())
            .get_with_status(key.metadata())?
        else {
            return Ok(ReconcileNext::done());
        };

        let mut hasher = DefaultHasher::new();
        machine.hash(&mut hasher);
        let machine_hash = hasher.finish();

        if status.hash == machine_hash {
            info!("resource hasn't changed");
            return Ok(ReconcileNext::done());
        }

        tokio::time::sleep(Duration::from_secs(10)).await;

        info!("reconciled machine controller for key: {}", key.to_string());

        ctx.repository
            .machine(ctx.tenant.clone())
            .patch_status(key.metadata(), |status| {
                status.test += 1;
                status.hash = machine_hash;
            })
            .await?;

        Ok(ReconcileNext::done())
    }

    async fn handle_error(
        &self,
        _ctx: ControllerContext,
        key: ControllerKey,
        err: anyhow::Error,
    ) -> ReconcileNext {
        error!(
            "handling error for machine controller for key: {} error: {}",
            key.to_string(),
            err
        );
        ReconcileNext::done()
    }
}
