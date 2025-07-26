use std::{
    hash::{DefaultHasher, Hash, Hasher},
    str::FromStr,
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use oci_client::Reference;
use tracing::{error, info};
use tracing_subscriber::fmt::format;

use crate::{
    controller::{
        Controller, ReconcileNext,
        context::{AsyncWork, ControllerContext, ControllerEvent, ControllerKey},
    },
    resource_index::ResourceKind,
    resources::{Convert, machine::MachinePhase},
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
            ControllerEvent::AsyncWorkChange(
                key,
                AsyncWork::ImagePullComplete { .. } | AsyncWork::Error(_),
            ) => Some(key),
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

        let machine = machine.latest();

        let mut hasher = DefaultHasher::new();
        machine.hash(&mut hasher);
        let machine_hash = hasher.finish();

        match status.phase {
            MachinePhase::Idle => {
                // TODO: start pulling image
                let image_agent = ctx.agent.image();
                let image = machine.image.clone();
                ctx.agent
                    .job()
                    .run_with_notify(
                        key.clone(),
                        format!("pull-image-{}", image),
                        async move {
                            let reference = Reference::from_str(&image)
                                .map_err(|_| format!("invalid image: {}", &image))?;
                            let image = image_agent
                                .image_pull(reference)
                                .await
                                .map_err(|_| format!("failed to pull image: {}", &image))?;
                            Ok(image.reference)
                        },
                        |result, key| match result {
                            Ok(reference) => ControllerEvent::AsyncWorkChange(
                                key,
                                AsyncWork::ImagePullComplete { reference },
                            ),
                            Err(e) => ControllerEvent::AsyncWorkChange(key, AsyncWork::Error(e)),
                        },
                    )
                    .await?;

                ctx.repository
                    .machine(ctx.tenant.clone())
                    .patch_status(key.metadata(), |status| {
                        status.hash = machine_hash;
                        status.phase = MachinePhase::PullingImage;
                    })
                    .await?;

                return Ok(ReconcileNext::done());
            }
            MachinePhase::PullingImage => {
                let Some(event) = ctx
                    .agent
                    .job()
                    .get_result(format!("pull-image-{}", machine.image), key.clone())
                    .await?
                else {
                    return Ok(ReconcileNext::done());
                };

                match event {
                    ControllerEvent::AsyncWorkChange(_, AsyncWork::ImagePullComplete { .. }) => {
                        ctx.repository
                            .machine(ctx.tenant.clone())
                            .patch_status(key.metadata(), |status| {
                                status.hash = machine_hash;
                                status.phase = MachinePhase::Creating;
                            })
                            .await?;
                        return Ok(ReconcileNext::Immediate);
                    }
                    ControllerEvent::AsyncWorkChange(_, AsyncWork::Error(err)) => {
                        // the job is done, so we can continue
                        ctx.repository
                            .machine(ctx.tenant.clone())
                            .patch_status(key.metadata(), |status| {
                                status.hash = machine_hash;
                                status.phase = MachinePhase::Error {
                                    message: err.clone(),
                                };
                            })
                            .await?;
                    }
                    _ => {}
                }
                // the job is done, so we can continue
            }
            _ => {}
        };

        if status.hash == machine_hash {
            info!("resource hasn't changed");
            return Ok(ReconcileNext::done());
        }

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
