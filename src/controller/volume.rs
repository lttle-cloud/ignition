use std::sync::Arc;

use anyhow::{Result, bail};
use async_trait::async_trait;
use tracing::{error, info};

use crate::{
    controller::{
        BeforeDelete, Controller, ReconcileNext,
        context::{ControllerContext, ControllerEvent, ControllerKey},
    },
    repository::Repository,
    resource_index::ResourceKind,
    resources::{
        Convert,
        metadata::{Metadata, Namespace},
        volume::Volume,
    },
};

pub struct VolumeController;

impl VolumeController {
    pub fn new_boxed() -> Box<Self> {
        Box::new(Self)
    }
}

#[async_trait]
impl Controller for VolumeController {
    async fn schedule(
        &self,
        ctx: ControllerContext,
        event: ControllerEvent,
    ) -> Result<Option<ControllerKey>> {
        info!("scheduling volume controller for event: {:?}", event);
        let key = match event {
            ControllerEvent::BringUp(ResourceKind::Volume, metadata) => Some(ControllerKey::new(
                ctx.tenant.clone(),
                ResourceKind::Service,
                metadata.namespace,
                metadata.name,
            )),
            ControllerEvent::ResourceChange(ResourceKind::Volume, metadata) => {
                Some(ControllerKey::new(
                    ctx.tenant.clone(),
                    ResourceKind::Volume,
                    metadata.namespace,
                    metadata.name,
                ))
            }
            _ => None,
        };
        Ok(key)
    }

    async fn should_reconcile(&self, _ctx: ControllerContext, key: ControllerKey) -> bool {
        info!(
            "should reconcile volume controller for key: {}",
            key.to_string()
        );

        return key.kind == ResourceKind::Volume;
    }

    async fn reconcile(&self, ctx: ControllerContext, key: ControllerKey) -> Result<ReconcileNext> {
        info!("reconciling volume controller for key: {}", key.to_string());

        let Some((volume, status)) = ctx
            .repository
            .volume(ctx.tenant.clone())
            .get_with_status(key.metadata().clone())?
        else {
            // the volume was deleted.
            let Some(status) = ctx
                .repository
                .volume(ctx.tenant.clone())
                .get_status(key.metadata().clone())?
            else {
                return Ok(ReconcileNext::done());
            };

            if let Some(volume_id) = status.volume_id {
                ctx.agent.volume().volume_delete(&volume_id).await.ok();
            }

            ctx.repository
                .volume(ctx.tenant.clone())
                .delete_status(key.metadata().clone())
                .await?;

            return Ok(ReconcileNext::done());
        };

        let volume_id = if let Some(volume_id) = status.volume_id {
            volume_id
        } else {
            ctx.agent
                .volume()
                .volume_create_empty_ext4_sparse(status.size_bytes)
                .await?
                .id
        };

        let hash = volume.hash_with_updated_metadata();
        ctx.repository
            .volume(ctx.tenant.clone())
            .patch_status(key.metadata().clone(), |status| {
                status.hash = hash;
                status.volume_id = Some(volume_id.clone());
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
            "handling error for volume controller for key: {} error: {}",
            key.to_string(),
            err
        );

        ReconcileNext::done()
    }
}

#[async_trait]
impl BeforeDelete for Volume {
    async fn before_delete(
        &self,
        tenant: String,
        repo: Arc<Repository>,
        metadata: Metadata,
    ) -> Result<()> {
        let Some(volume) = repo.volume(tenant.clone()).get(
            Namespace::from_value_or_default(metadata.namespace.clone()),
            metadata.name.clone(),
        )?
        else {
            bail!("volume not found");
        };
        let volume = volume.latest();

        let machines = repo.machine(tenant.clone()).list(Namespace::Unspecified)?;

        let mut usage_count = 0;
        for machine in machines {
            let machine = machine.latest();
            let Some(volumes) = machine.volumes else {
                continue;
            };

            volumes.iter().for_each(|volume_bind| {
                if volume_bind.name == volume.name {
                    let namespace = Namespace::from_value_or_default(
                        volume_bind
                            .namespace
                            .clone()
                            .or_else(|| machine.namespace.clone()),
                    )
                    .as_value();

                    if namespace == metadata.namespace {
                        usage_count += 1;
                    }
                }
            });
        }

        if usage_count > 0 {
            bail!("volume is still in use by {} machines", usage_count);
        }

        Ok(())
    }
}
