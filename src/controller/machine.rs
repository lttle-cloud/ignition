use std::{str::FromStr, time::Duration};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use oci_client::Reference;
use tokio::{runtime, task::spawn_blocking};
use tracing::{error, info, warn};

use crate::{
    agent::{
        machine::machine::{
            MachineConfig, MachineMode, MachineResources, MachineState, MachineStateRetentionMode,
            NetworkConfig, SnapshotStrategy, VolumeMountConfig,
        },
        net::{IpReservationKind, compute_mac_for_ip},
    },
    controller::{
        Controller, ReconcileNext,
        context::{AsyncWork, ControllerContext, ControllerEvent, ControllerKey},
    },
    resource_index::ResourceKind,
    resources::{self, Convert, machine::MachinePhase},
};

pub struct MachineController;

impl MachineController {
    pub fn new_boxed() -> Box<Self> {
        Box::new(Self)
    }
}

fn pull_image_job_key(reference: &Reference) -> String {
    format!("pull-image-{}", reference)
}

fn machine_name_from_key(key: &ControllerKey) -> String {
    format!("{}-{}", key.tenant, key.metadata().to_string())
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
            ControllerEvent::BringUp(ResourceKind::Machine, metadata) => {
                let Ok(Some((_, status))) = ctx
                    .repository
                    .machine(ctx.tenant.clone())
                    .get_with_status(metadata.clone())
                else {
                    return Ok(None);
                };

                let new_phase = match status.phase {
                    MachinePhase::Idle | MachinePhase::PullingImage => Some(MachinePhase::Idle),
                    _ => Some(MachinePhase::Creating),
                };

                if let Some(new_phase) = new_phase {
                    ctx.repository
                        .machine(ctx.tenant.clone())
                        .patch_status(metadata.clone(), move |status| {
                            status.phase = new_phase.clone();
                        })
                        .await?;
                }

                Some(ControllerKey::new(
                    ctx.tenant.clone(),
                    ResourceKind::Machine,
                    metadata.namespace,
                    metadata.name,
                ))
            }
            ControllerEvent::ResourceChange(ResourceKind::Machine, metadata) => {
                Some(ControllerKey::new(
                    ctx.tenant.clone(),
                    ResourceKind::Machine,
                    metadata.namespace,
                    metadata.name,
                ))
            }
            ControllerEvent::AsyncWorkChange(
                key,
                AsyncWork::ImagePullComplete { .. }
                | AsyncWork::MachineStateChange { .. }
                | AsyncWork::Error(_),
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

        let machine_name = machine_name_from_key(&key);
        let running_machine = ctx.agent.machine().get_machine(&machine_name);
        let stored_machine = ctx
            .repository
            .machine(ctx.tenant.clone())
            .get_with_status(key.metadata())?;

        let Some((machine, status)) = ('actual_vs_desired: {
            match (running_machine, stored_machine) {
                (Some(running_machine), Some((stored_machine, status))) => {
                    // we have a running machine and a stored machine
                    let current_state = running_machine.get_state().await;
                    let new_phase = match current_state {
                        MachineState::Booting => Some(MachinePhase::Booting),
                        MachineState::Ready => Some(MachinePhase::Ready),
                        MachineState::Suspending => Some(MachinePhase::Suspending),
                        MachineState::Suspended => Some(MachinePhase::Suspended),
                        MachineState::Stopping => Some(MachinePhase::Stopping),
                        MachineState::Stopped => Some(MachinePhase::Stopped),
                        MachineState::Error(message) => Some(MachinePhase::Error {
                            message: message.clone(),
                        }),
                        _ => None,
                    };

                    let last_boot_duration_us = running_machine
                        .get_last_boot_duration()
                        .await
                        .and_then(|duration| Some(duration.as_micros()));

                    let first_boot_duration_us = running_machine
                        .get_first_boot_duration()
                        .await
                        .and_then(|duration| Some(duration.as_micros()));

                    if let Some(new_phase) = new_phase {
                        if new_phase != status.phase {
                            let new_status = ctx
                                .repository
                                .machine(ctx.tenant.clone())
                                .patch_status(key.metadata(), move |status| {
                                    status.phase = new_phase.clone();
                                    status.last_boot_time_us = last_boot_duration_us;
                                    status.first_boot_time_us = first_boot_duration_us;
                                })
                                .await?;
                            Some((stored_machine, new_status))
                        } else {
                            Some((stored_machine, status))
                        }
                    } else {
                        Some((stored_machine, status))
                    }
                }
                (None, Some((stored_machine, status))) => {
                    // we have a stored machine but no running machine
                    Some((stored_machine, status))
                }
                (Some(running_machine), None) => {
                    let Some(_status) = ctx
                        .repository
                        .machine(ctx.tenant.clone())
                        .get_status(key.metadata())?
                    else {
                        break 'actual_vs_desired None;
                    };

                    info!(
                        "cleaning up machine {} with status: {:?}",
                        machine_name, _status
                    );

                    // we have a running machine but no stored machine. time to clean up
                    running_machine.stop().await?;
                    ctx.agent.machine().delete_machine(&machine_name).await?;

                    ctx.repository
                        .machine(ctx.tenant.clone())
                        .delete_status(key.metadata())
                        .await?;
                    // we don't have
                    None
                }
                (None, None) => {
                    let status = ctx
                        .repository
                        .machine(ctx.tenant.clone())
                        .get_status(key.metadata())?;

                    if status.is_some() {
                        warn!("cleaning up machine status for key: {}", key.to_string());

                        ctx.repository
                            .machine(ctx.tenant.clone())
                            .delete_status(key.metadata())
                            .await?;
                    }

                    None
                }
            }
        }) else {
            return Ok(ReconcileNext::done());
        };

        let machine = machine.latest();
        let reference = Reference::from_str(&machine.image)
            .map_err(|_| anyhow!("invalid image reference: {}", machine.image))?;
        let resolved_reference_str = reference.to_string();

        match status.phase {
            MachinePhase::Idle => {
                // TODO: start pulling image
                let image_agent = ctx.agent.image();
                ctx.agent
                    .job()
                    .run_with_notify(
                        key.clone(),
                        pull_image_job_key(&reference),
                        async move {
                            let image = image_agent
                                .image_pull(reference)
                                .await
                                .map_err(|_| format!("failed to pull image: {}", &machine.image))?;

                            let reference = format!("{}@{}", image.reference, image.digest);

                            Ok((image.id, reference))
                        },
                        |result, key| match result {
                            Ok((id, reference)) => ControllerEvent::AsyncWorkChange(
                                key,
                                AsyncWork::ImagePullComplete { id, reference },
                            ),
                            Err(e) => ControllerEvent::AsyncWorkChange(key, AsyncWork::Error(e)),
                        },
                    )
                    .await?;

                ctx.repository
                    .machine(ctx.tenant.clone())
                    .patch_status(key.metadata(), |status| {
                        status.phase = MachinePhase::PullingImage;
                        status.image_resolved_reference = Some(resolved_reference_str.clone());
                    })
                    .await?;

                return Ok(ReconcileNext::done());
            }
            MachinePhase::PullingImage => {
                let Some(event) = ctx
                    .agent
                    .job()
                    .get_result(pull_image_job_key(&reference), key.clone())
                    .await?
                else {
                    return Ok(ReconcileNext::done());
                };

                match event {
                    ControllerEvent::AsyncWorkChange(
                        _,
                        AsyncWork::ImagePullComplete { id, reference },
                    ) => {
                        ctx.repository
                            .machine(ctx.tenant.clone())
                            .patch_status(key.metadata(), |status| {
                                status.image_id = Some(id.clone());
                                status.image_resolved_reference = Some(reference.clone());
                                status.phase = MachinePhase::Creating;
                            })
                            .await?;
                        return Ok(ReconcileNext::Immediate);
                    }
                    ControllerEvent::AsyncWorkChange(_, AsyncWork::Error(err)) => {
                        ctx.repository
                            .machine(ctx.tenant.clone())
                            .patch_status(key.metadata(), |status| {
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
            MachinePhase::Creating => {
                // alloc name
                let name = machine_name_from_key(&key);

                let image = match status.image_id {
                    Some(ref id) => id.clone(),
                    None => {
                        bail!("image ID is not set for machine: {}", name);
                    }
                };
                let Ok(Some(image)) = ctx.agent.image().image(&image) else {
                    bail!("image not found for machine: {}", name);
                };

                let root_volume = match status.machine_image_volume_id {
                    Some(ref volume_id) => ctx.agent.volume().volume(volume_id)?,
                    None => ctx
                        .agent
                        .volume()
                        .volume_clone_with_overlay(&image.volume_id)
                        .await
                        .ok(),
                };

                let Some(root_volume) = root_volume else {
                    bail!("failed to get or create root volume for machine: {}", name);
                };

                // alloc ip for machine
                let ip = match status.machine_ip {
                    Some(ip) => ip.clone(),
                    None => {
                        ctx.agent
                            .net()
                            .ip_reservation_create(IpReservationKind::VM, Some(name.clone()))
                            .map_err(|_| anyhow!("failed to allocate IP for machine: {}", name))?
                            .ip
                    }
                };

                let tap_device_name = status.machine_tap.clone();

                // alloc tap device for machine
                let net_agent = ctx.agent.net();
                // TODO: this is a bit dirty
                let tap = spawn_blocking(move || {
                    runtime::Handle::current().block_on(async {
                        match tap_device_name {
                            Some(tap_name) => net_agent.device(&tap_name).await,
                            None => net_agent.device_create().await,
                        }
                    })
                })
                .await?
                .map_err(|e| anyhow!("failed to create tap device for machine: {}: {}", name, e))?;

                let mode = match machine.mode {
                    None | Some(resources::machine::MachineMode::Regular) => MachineMode::Regular,
                    Some(resources::machine::MachineMode::Flash(strategy)) => match strategy {
                        resources::machine::MachineSnapshotStrategy::WaitForUserSpaceReady => {
                            MachineMode::Flash {
                                snapshot_strategy: SnapshotStrategy::WaitForUserSpaceReady,
                                suspend_timeout: Duration::from_secs(10),
                            }
                        }
                        resources::machine::MachineSnapshotStrategy::WaitForFirstListen => {
                            MachineMode::Flash {
                                snapshot_strategy: SnapshotStrategy::WaitForFirstListen,
                                suspend_timeout: Duration::from_secs(10),
                            }
                        }
                        resources::machine::MachineSnapshotStrategy::WaitForNthListen(n) => {
                            MachineMode::Flash {
                                snapshot_strategy: SnapshotStrategy::WaitForNthListen(n),
                                suspend_timeout: Duration::from_secs(10),
                            }
                        }
                        resources::machine::MachineSnapshotStrategy::WaitForListenOnPort(n) => {
                            MachineMode::Flash {
                                snapshot_strategy: SnapshotStrategy::WaitForListenOnPort(n),
                                suspend_timeout: Duration::from_secs(10),
                            }
                        }
                        resources::machine::MachineSnapshotStrategy::Manual => MachineMode::Flash {
                            snapshot_strategy: SnapshotStrategy::Manual,
                            suspend_timeout: Duration::from_secs(10),
                        },
                    },
                };

                let mac = compute_mac_for_ip(&ip)
                    .map_err(|_| anyhow!("failed to compute MAC address for IP: {}", ip))?;

                let image_volume_id = root_volume.id.clone();
                let tap_name = tap.name.clone();
                let ip_addr = ip.clone();
                let machine_id = name.clone();

                // create the machine
                let machine = ctx
                    .agent
                    .machine()
                    .create_machine(MachineConfig {
                        name: name.clone(),
                        // TODO: network tag should be something similar to the app name (for scaling issues)
                        network_tag: name.clone(),
                        controller_key: key.clone(),
                        image,
                        mode,
                        resources: MachineResources {
                            cpu: machine.resources.cpu,
                            memory: machine.resources.memory,
                        },
                        envs: machine
                            .env
                            .unwrap_or_default()
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                        state_retention_mode: MachineStateRetentionMode::OnDisk {
                            path: ctx.agent.machine().transient_dir(&name),
                        },
                        volume_mounts: vec![VolumeMountConfig {
                            volume: root_volume,
                            mount_at: "/".to_string(),
                            read_only: false,
                            root: true,
                        }],
                        network: NetworkConfig {
                            tap_device: tap.name,
                            ip_address: ip,
                            mac_address: mac,
                            gateway: ctx.agent.net().vm_gateway().to_string(),
                            netmask: ctx.agent.net().vm_netmask().to_string(),
                        },
                    })
                    .await
                    .map_err(|e| anyhow!("failed to create machine: {} with error: {}", name, e))?;

                machine.start().await.map_err(|e| {
                    anyhow!("failed to start machine: {} with error: {}", machine_id, e)
                })?;

                ctx.repository
                    .machine(ctx.tenant.clone())
                    .patch_status(key.metadata(), move |status| {
                        status.phase = MachinePhase::Booting;
                        status.machine_id = Some(machine_id.clone());
                        status.machine_ip = Some(ip_addr.clone());
                        status.machine_tap = Some(tap_name.clone());
                        status.machine_image_volume_id = Some(image_volume_id.clone());
                    })
                    .await?;
            }
            _ => {}
        };

        Ok(ReconcileNext::done())
    }

    async fn handle_error(
        &self,
        ctx: ControllerContext,
        key: ControllerKey,
        err: anyhow::Error,
    ) -> ReconcileNext {
        error!(
            "handling error for machine controller for key: {} error: {}",
            key.to_string(),
            err
        );

        ctx.repository
            .machine(ctx.tenant.clone())
            .patch_status(key.metadata(), |status| {
                status.phase = MachinePhase::Error {
                    message: err.to_string(),
                };
            })
            .await
            .ok();

        ReconcileNext::done()
    }
}
