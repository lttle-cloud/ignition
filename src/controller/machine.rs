use std::{str::FromStr, sync::Arc, time::Duration};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use chrono::Utc;
use oci_client::Reference;
use takeoff_proto::proto::LogsTelemetryConfig;
use tokio::{runtime, task::spawn_blocking};
use tracing::{error, info, warn};

use crate::{
    agent::{
        Agent,
        machine::machine::{
            MachineConfig, MachineMode, MachineResources, MachineState, MachineStateRetentionMode,
            NetworkConfig, SnapshotStrategy, VolumeMountConfig,
        },
        net::{IpReservationKind, compute_mac_for_ip},
    },
    constants::{DEFAULT_NAMESPACE, DEFAULT_SUSPEND_TIMEOUT_SECS},
    controller::{
        AdmissionCheckBeforeSet, Controller, ReconcileNext,
        context::{AsyncWork, ControllerContext, ControllerEvent, ControllerKey},
    },
    repository::Repository,
    resource_index::ResourceKind,
    resources::{
        self, Convert,
        machine::{Machine, MachinePhase, MachineStatus},
        metadata::{Metadata, Namespace},
        volume::VolumeMode,
    },
};

// Restart policy constants
const MAX_RESTART_COUNT: u64 = 3;
const BASE_RESTART_BACKOFF_SECS: u64 = 2;

pub struct MachineController;

impl MachineController {
    pub fn new_boxed() -> Box<Self> {
        Box::new(Self)
    }
}

fn pull_image_job_key(reference: &Reference) -> String {
    format!("pull-image-{}", reference)
}

fn image_is_latest_available_job_key(reference: &Reference) -> String {
    format!("image-is-latest-available-{}", reference)
}

pub fn machine_name_from_key(key: &ControllerKey) -> String {
    format!("{}-{}", key.tenant, key.metadata().to_string())
}

fn calculate_restart_backoff(restart_count: u64) -> Duration {
    // Exponential backoff: 2^restart_count * BASE_RESTART_BACKOFF_SECS seconds
    // restart_count=0: 2s, restart_count=1: 4s, restart_count=2: 8s, restart_count=3: 16s
    let backoff_multiplier = 2u64.saturating_pow(restart_count as u32);
    Duration::from_secs(BASE_RESTART_BACKOFF_SECS * backoff_multiplier)
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
                let Ok(Some(_)) = ctx
                    .repository
                    .machine(ctx.tenant.clone())
                    .get_with_status(metadata.clone())
                else {
                    return Ok(None);
                };

                ctx.repository
                    .machine(ctx.tenant.clone())
                    .patch_status(metadata.clone(), move |status| {
                        status.phase = MachinePhase::Idle;
                        status.last_exit_code = None;
                    })
                    .await?;

                Some(ControllerKey::new(
                    ctx.tenant.clone(),
                    ResourceKind::Machine,
                    metadata.namespace,
                    metadata.name,
                ))
            }
            ControllerEvent::ResourceChange(ResourceKind::Machine, metadata) => {
                let key = ControllerKey::new(
                    ctx.tenant.clone(),
                    ResourceKind::Machine,
                    metadata.namespace.clone(),
                    metadata.name.clone(),
                );

                if let Some((machine, status)) = ctx
                    .repository
                    .machine(ctx.tenant.clone())
                    .get_with_status(metadata.clone())?
                {
                    'check_machine: {
                        let machine = machine.latest();
                        let Some(image) = machine.image.clone() else {
                            bail!("image is not set for machine: {}", metadata.name);
                        };

                        let tags = machine.tags.clone().unwrap_or_default();
                        // we will restart the machine anyways
                        if tags.contains(&"ignitiond.restart".to_string()) {
                            break 'check_machine;
                        }

                        if status.hash == 0 {
                            break 'check_machine;
                        }

                        let tenant = ctx.tenant.clone();
                        let image_agent = ctx.agent.image();
                        let image_status = status.clone();

                        let reference = Reference::from_str(&image)
                            .map_err(|_| anyhow!("invalid image reference: {}", image))?;

                        ctx.agent
                            .job()
                            .run_with_notify(
                                key.clone(),
                                image_is_latest_available_job_key(&reference),
                                async move {
                                    let latest_available_image = image_agent
                                        .image_latest_available(tenant, reference.clone())
                                        .await
                                        .map_err(|e| {
                                            warn!(
                                                "failed to check if image is latest available: {}",
                                                e
                                            );
                                            format!(
                                                "failed to check if image is latest available: {:?}",
                                                &reference
                                            )
                                        })?;

                                    let is_latest_available = if let Some(latest_available_image) = latest_available_image {
                                        image_status.image_id == Some(latest_available_image.id)
                                    } else {
                                        false
                                    };

                                    Ok(is_latest_available)
                                },
                                |result: std::result::Result<bool, String>, key| match result {
                                    Ok(is_latest_available) if !is_latest_available => {
                                        Some(ControllerEvent::AsyncWorkChange(
                                            key,
                                            AsyncWork::ImageNeedsPull,
                                        ))
                                    }
                                    _ => None,
                                },
                            )
                            .await?;
                    }
                }

                Some(key)
            }
            ControllerEvent::AsyncWorkChange(
                key,
                AsyncWork::ImagePullComplete { .. }
                | AsyncWork::ImageNeedsPull
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
                (Some(running_machine), Some((stored_machine, status)))
                    if status.phase == MachinePhase::Restarting =>
                {
                    let original_status = status.clone();

                    info!(
                        "cleaning up machine {} with status: {:?}",
                        machine_name, status
                    );

                    // we have a running machine but no stored machine. time to clean up
                    if let Err(e) = running_machine.stop().await {
                        // might be stuck in stopping state or error state
                        warn!("failed to stop machine: {}", e);
                    }
                    ctx.agent.machine().delete_machine(&machine_name).await?;

                    // delete tap device
                    if let Some(tap_name) = status.machine_tap {
                        ctx.agent.net().device_delete(&tap_name).await?;
                    }

                    // delete associated ip reservation
                    if let Some(ip) = status.machine_ip {
                        ctx.agent
                            .net()
                            .ip_reservation_delete(IpReservationKind::VM, &ip)?;
                    }

                    // delete image volume
                    if let Some(volume_id) = status.machine_image_volume_id {
                        ctx.agent.volume().volume_delete(&volume_id).await?;
                    }

                    ctx.repository
                        .machine(ctx.tenant.clone())
                        .patch_status(key.metadata(), |status| {
                            status.machine_ip = None;
                            status.machine_tap = None;
                            status.machine_image_volume_id = None;
                        })
                        .await?;

                    Some((stored_machine, original_status))
                }
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
                        .and_then(|duration| Some(duration.as_micros() as u64));

                    let first_boot_duration_us = running_machine
                        .get_first_boot_duration()
                        .await
                        .and_then(|duration| Some(duration.as_micros() as u64));

                    let last_exit_code = running_machine.get_last_exit_code().await;

                    if let Some(new_phase) = new_phase {
                        if new_phase != status.phase {
                            let new_status = ctx
                                .repository
                                .machine(ctx.tenant.clone())
                                .patch_status(key.metadata(), move |status| {
                                    status.phase = new_phase.clone();
                                    status.last_boot_time_us = last_boot_duration_us;
                                    status.first_boot_time_us = first_boot_duration_us;
                                    if let Some(last_exit_code) = last_exit_code {
                                        status.last_exit_code = Some(last_exit_code);
                                    }
                                    // Reset restart counter when machine successfully reaches Ready state
                                    if matches!(new_phase, MachinePhase::Ready) {
                                        status.restart_count = Some(0);
                                    }
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
                    let Some(status) = ctx
                        .repository
                        .machine(ctx.tenant.clone())
                        .get_status(key.metadata())?
                    else {
                        break 'actual_vs_desired None;
                    };

                    info!(
                        "cleaning up machine {} with status: {:?}",
                        machine_name, status
                    );

                    // we have a running machine but no stored machine. time to clean up
                    if let Err(e) = running_machine.stop().await {
                        // might be stuck in stopping state or error state
                        warn!("failed to stop machine: {}", e);
                    }
                    ctx.agent.machine().delete_machine(&machine_name).await?;

                    // delete tap device
                    if let Some(tap_name) = status.machine_tap {
                        ctx.agent.net().device_delete(&tap_name).await?;
                    }

                    // delete associated ip reservation
                    if let Some(ip) = status.machine_ip {
                        ctx.agent
                            .net()
                            .ip_reservation_delete(IpReservationKind::VM, &ip)?;
                    }

                    // delete image volume
                    if let Some(volume_id) = status.machine_image_volume_id {
                        ctx.agent.volume().volume_delete(&volume_id).await?;
                    }

                    ctx.repository
                        .machine(ctx.tenant.clone())
                        .delete_status(key.metadata())
                        .await?;

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

        let hash = machine.hash_with_updated_metadata();

        let mut machine = machine.latest();

        let Some(image) = machine.image.clone() else {
            bail!("image is not set for machine: {}", machine_name);
        };

        let reference = Reference::from_str(&image)
            .map_err(|_| anyhow!("invalid image reference: {}", image))?;
        let resolved_reference_str = reference.to_string();

        let tags = machine.tags.clone().unwrap_or_default();

        // remove the tag and restart the machine if it's set
        if tags.contains(&"ignitiond.restart".to_string()) {
            machine.tags = Some(
                tags.into_iter()
                    .filter(|tag| tag != "ignitiond.restart")
                    .collect(),
            );

            ctx.repository
                .machine(key.tenant.clone())
                .set(machine.into())
                .await?;

            ctx.repository
                .machine(key.tenant.clone())
                .patch_status(key.metadata(), |status| {
                    status.phase = MachinePhase::Restarting;
                    status.last_restarting_time_us = Some(Utc::now().timestamp_millis() as u64);
                    // Reset restart counter for manual restarts
                    status.restart_count = Some(0);
                })
                .await?;

            return Ok(ReconcileNext::immediate());
        }

        if hash != status.hash && status.hash != 0 {
            // the resource has changed, let's recreate the machine
            ctx.repository
                .machine(key.tenant.clone())
                .patch_status(key.metadata(), |status| {
                    status.hash = hash;
                    status.phase = MachinePhase::Restarting;
                    status.last_restarting_time_us = Some(Utc::now().timestamp_millis() as u64);
                    // Reset restart counter for spec changes
                    status.restart_count = Some(0);
                })
                .await?;

            return Ok(ReconcileNext::immediate());
        }

        if let Some(_event) = ctx
            .agent
            .job()
            .get_result(image_is_latest_available_job_key(&reference), key.clone())
            .await?
        {
            info!("image digest changed, restarting machine");

            // we need to restart the machine to pull the latest image
            ctx.repository
                .machine(key.tenant.clone())
                .patch_status(key.metadata(), |status| {
                    status.phase = MachinePhase::Restarting;
                    status.last_restarting_time_us = Some(Utc::now().timestamp_millis() as u64);
                    // Reset restart counter for image updates
                    status.restart_count = Some(0);
                })
                .await?;

            ctx.agent
                .job()
                .consume_result(image_is_latest_available_job_key(&reference), key.clone())
                .await?;

            return Ok(ReconcileNext::immediate());
        };

        ctx.repository
            .machine(key.tenant.clone())
            .patch_status(key.metadata(), |status| {
                status.hash = hash;
            })
            .await?;

        'phase_match: {
            match status.phase {
                MachinePhase::Idle => {
                    let image_agent = ctx.agent.image();
                    let tenant = ctx.tenant.clone();
                    ctx.agent
                        .job()
                        .run_with_notify(
                            key.clone(),
                            pull_image_job_key(&reference),
                            async move {
                                let image = image_agent
                                    .image_pull(tenant.clone(), reference)
                                    .await
                                    .map_err(|e| {
                                        warn!("failed to pull image: {}", e);
                                        format!("failed to pull image: {}", &image)
                                    })?;

                                let reference = format!("{}@{}", image.reference, image.digest);

                                Ok((image.id, reference))
                            },
                            |result, key| match result {
                                Ok((id, reference)) => Some(ControllerEvent::AsyncWorkChange(
                                    key,
                                    AsyncWork::ImagePullComplete { id, reference },
                                )),
                                Err(e) => {
                                    Some(ControllerEvent::AsyncWorkChange(key, AsyncWork::Error(e)))
                                }
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

                    ctx.agent
                        .job()
                        .consume_result(pull_image_job_key(&reference), key.clone())
                        .await?;

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
                                    status.phase = MachinePhase::Waiting;
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
                MachinePhase::Waiting => {
                    // check if all volumes are ready
                    let volumes = machine.volumes.clone().unwrap_or_default();
                    for volume in volumes {
                        let volume_namespace = Namespace::from_value_or_default(
                            volume.namespace.or_else(|| machine.namespace.clone()),
                        );
                        let volume_metadata = Metadata::new(&volume.name, volume_namespace);

                        let Ok(Some(_volume_status)) = ctx
                            .repository
                            .volume(ctx.tenant.clone())
                            .get_status(volume_metadata)
                        else {
                            info!("waiting for volume {} to be ready", volume.name);
                            return Ok(ReconcileNext::after(Duration::from_secs(2)));
                        };
                    }

                    // check if all dependencies are ready
                    let dependencies = machine.depends_on.clone().unwrap_or_default();
                    for dependency in dependencies {
                        let dependency_namespace = Namespace::from_value_or_default(
                            dependency.namespace.or_else(|| machine.namespace.clone()),
                        );
                        let dependency_metadata =
                            Metadata::new(&dependency.name, dependency_namespace);
                        let dependency_status = ctx
                            .repository
                            .machine(ctx.tenant.clone())
                            .get_status(dependency_metadata)?;

                        match dependency_status {
                            Some(MachineStatus {
                                phase:
                                    MachinePhase::Ready
                                    | MachinePhase::Suspended
                                    | MachinePhase::Suspending,
                                ..
                            }) => {
                                continue;
                            }
                            _ => {
                                info!("waiting for dependency {} to be ready", dependency.name);
                                return Ok(ReconcileNext::after(Duration::from_secs(2)));
                            }
                        }
                    }

                    ctx.repository
                        .machine(ctx.tenant.clone())
                        .patch_status(key.metadata(), |status| {
                            status.phase = MachinePhase::Creating;
                        })
                        .await?;

                    info!("all dependencies are ready, creating machine");

                    return Ok(ReconcileNext::immediate());
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

                    let image_volume_id = root_volume.id.clone();
                    let mut machine_volume_mounts = vec![VolumeMountConfig {
                        volume: root_volume,
                        mount_at: "/".to_string(),
                        read_only: false,
                        root: true,
                    }];

                    let volume_bindings = machine.volumes.unwrap_or_default();
                    for volume_bind in volume_bindings {
                        let volume_resource_namespace = Namespace::from_value_or_default(
                            volume_bind.namespace.or_else(|| machine.namespace.clone()),
                        );
                        let volume_resource_metadata =
                            Metadata::new(&volume_bind.name, volume_resource_namespace);

                        let Ok(Some((volume_resource, volume_status))) = ctx
                            .repository
                            .volume(ctx.tenant.clone())
                            .get_with_status(volume_resource_metadata)
                        else {
                            bail!(
                                "volume resource {} not found for machine: {}",
                                volume_bind.name,
                                name
                            );
                        };
                        let volume_resource = volume_resource.latest();

                        let Some(Ok(Some(volume))) = volume_status
                            .volume_id
                            .map(|id| ctx.agent.volume().volume(&id))
                        else {
                            bail!(
                                "volume resource {} not found for machine: {}",
                                volume_bind.name,
                                name
                            );
                        };

                        machine_volume_mounts.push(VolumeMountConfig {
                            volume,
                            mount_at: volume_bind.path,
                            read_only: volume_resource.mode == VolumeMode::ReadOnly,
                            root: false,
                        });
                    }

                    // alloc ip for machine
                    let ip = match status.machine_ip {
                        Some(ip) => ip.clone(),
                        None => {
                            ctx.agent
                                .net()
                                .ip_reservation_create(
                                    IpReservationKind::VM,
                                    Some(name.clone()),
                                    ctx.tenant.clone(),
                                )
                                .map_err(|_| {
                                    anyhow!("failed to allocate IP for machine: {}", name)
                                })?
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
                    .map_err(|e| {
                        anyhow!("failed to create tap device for machine: {}: {}", name, e)
                    })?;

                    let mode = match machine.mode {
                    None | Some(resources::machine::MachineMode::Regular) => MachineMode::Regular,
                    Some(resources::machine::MachineMode::Flash { strategy, timeout }) => {
                        match strategy {
                            resources::machine::MachineSnapshotStrategy::WaitForUserSpaceReady => {
                                MachineMode::Flash {
                                    snapshot_strategy: SnapshotStrategy::WaitForUserSpaceReady,
                                    suspend_timeout: Duration::from_secs(
                                        timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS),
                                    ),
                                }
                            }
                            resources::machine::MachineSnapshotStrategy::WaitForFirstListen => {
                                MachineMode::Flash {
                                    snapshot_strategy: SnapshotStrategy::WaitForFirstListen,
                                    suspend_timeout: Duration::from_secs(
                                        timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS),
                                    ),
                                }
                            }
                            resources::machine::MachineSnapshotStrategy::WaitForNthListen(n) => {
                                MachineMode::Flash {
                                    snapshot_strategy: SnapshotStrategy::WaitForNthListen(n),
                                    suspend_timeout: Duration::from_secs(
                                        timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS),
                                    ),
                                }
                            }
                            resources::machine::MachineSnapshotStrategy::WaitForListenOnPort(n) => {
                                MachineMode::Flash {
                                    snapshot_strategy: SnapshotStrategy::WaitForListenOnPort(n),
                                    suspend_timeout: Duration::from_secs(
                                        timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS),
                                    ),
                                }
                            }
                            resources::machine::MachineSnapshotStrategy::Manual => {
                                MachineMode::Flash {
                                    snapshot_strategy: SnapshotStrategy::Manual,
                                    suspend_timeout: Duration::from_secs(
                                        timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS),
                                    ),
                                }
                            }
                        }
                    }
                };

                    let mac = compute_mac_for_ip(&ip)
                        .map_err(|_| anyhow!("failed to compute MAC address for IP: {}", ip))?;

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
                            cmd: machine.command.clone(),
                            envs: machine
                                .environment
                                .unwrap_or_default()
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),
                            state_retention_mode: MachineStateRetentionMode::OnDisk {
                                path: ctx.agent.machine().transient_dir(&name),
                            },
                            volume_mounts: machine_volume_mounts,
                            network: NetworkConfig {
                                tap_device: tap.name,
                                ip_address: ip,
                                mac_address: mac,
                                gateway: ctx.agent.net().vm_gateway().to_string(),
                                netmask: ctx.agent.net().vm_netmask().to_string(),
                                dns_servers: vec![ctx.agent.net().service_gateway().to_string()],
                            },
                            logs_telemetry_config: LogsTelemetryConfig {
                                endpoint: ctx.agent.logs().get_otel_ingest_endpoint().clone(),
                                service_name: machine.name.clone(),
                                tenant_id: ctx.tenant.clone().to_string(),
                                service_namespace: machine
                                    .namespace
                                    .clone()
                                    .unwrap_or(DEFAULT_NAMESPACE.to_string()),
                                service_group: machine.name.clone(),
                            },
                        })
                        .await
                        .map_err(|e| {
                            anyhow!("failed to create machine: {} with error: {}", name, e)
                        })?;

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
                MachinePhase::Stopped => {
                    let should_restart = match machine
                        .restart_policy
                        .unwrap_or(resources::machine::MachineRestartPolicy::Always)
                    {
                        resources::machine::MachineRestartPolicy::Always => true,
                        resources::machine::MachineRestartPolicy::OnFailure => {
                            // last status code exists and is non zero
                            !(matches!(status.last_exit_code, Some(0)))
                        }
                        resources::machine::MachineRestartPolicy::Never => false,
                        resources::machine::MachineRestartPolicy::Remove => {
                            let metadata = key.metadata();
                            let namespace = Namespace::from_value_or_default(metadata.namespace);
                            if let Err(e) = ctx
                                .repository
                                .machine(ctx.tenant.clone())
                                .delete(namespace, metadata.name)
                                .await
                            {
                                warn!("restart-policy remove failed to delete machine {}", e);
                            };

                            false
                        }
                    };

                    if !should_restart {
                        break 'phase_match;
                    }

                    // Check if we've exceeded max restart count
                    let restart_count = status.restart_count.unwrap_or(0);
                    if restart_count >= MAX_RESTART_COUNT {
                        warn!(
                            "Machine {} exceeded max restart count ({}/{}), entering error state",
                            machine_name, restart_count, MAX_RESTART_COUNT
                        );
                        ctx.repository
                            .machine(ctx.tenant.clone())
                            .patch_status(key.metadata(), |status| {
                                status.phase = MachinePhase::Error {
                                    message: format!(
                                        "Max restart count exceeded ({}/{})",
                                        restart_count, MAX_RESTART_COUNT
                                    ),
                                };
                            })
                            .await?;
                        break 'phase_match;
                    }

                    ctx.repository
                        .machine(ctx.tenant.clone())
                        .patch_status(key.metadata(), |status| {
                            status.phase = MachinePhase::Restarting;
                            status.last_restarting_time_us =
                                Some(Utc::now().timestamp_millis() as u64);
                        })
                        .await?;

                    return Ok(ReconcileNext::immediate());
                }
                MachinePhase::Restarting => {
                    if let Some(last_restarting_time_us) = status.last_restarting_time_us {
                        let now = Utc::now().timestamp_millis() as u64;
                        let duration =
                            Duration::from_millis((now - last_restarting_time_us) as u64);

                        // Calculate exponential backoff based on restart count
                        let restart_count = status.restart_count.unwrap_or(0);
                        let required_backoff = calculate_restart_backoff(restart_count);

                        if duration < required_backoff {
                            let remaining = required_backoff - duration;
                            info!(
                                "Machine {} waiting for restart backoff: {:?} remaining (restart attempt {})",
                                machine_name,
                                remaining,
                                restart_count + 1
                            );
                            return Ok(ReconcileNext::after(remaining));
                        }
                    }

                    let restart_count = status.restart_count.unwrap_or(0);
                    ctx.repository
                        .machine(ctx.tenant.clone())
                        .patch_status(key.metadata(), |status| {
                            status.phase = MachinePhase::Idle;
                            status.restart_count = Some(restart_count + 1);
                        })
                        .await?;

                    return Ok(ReconcileNext::immediate());
                }

                MachinePhase::Error { message } => {
                    // Check if this is a VCPU timeout error requiring immediate cleanup
                    if message.contains("VCPU timeout") || message.contains("timed out") {
                        let restart_count = status.restart_count.unwrap_or(0);
                        // Check if we've exceeded max restart count
                        if restart_count >= MAX_RESTART_COUNT {
                            warn!(
                                "Machine {} has VCPU timeout error but exceeded max restart count ({}/{}), staying in error state: {}",
                                machine_name, restart_count, MAX_RESTART_COUNT, message
                            );
                            // Update the error message to indicate max restarts exceeded
                            ctx.repository
                                .machine(ctx.tenant.clone())
                                .patch_status(key.metadata(), |status| {
                                    status.phase = MachinePhase::Error {
                                        message: format!(
                                            "VCPU timeout - Max restart count exceeded ({}/{}). Original error: {}",
                                            restart_count, MAX_RESTART_COUNT, message
                                        ),
                                    };
                                })
                                .await?;
                            break 'phase_match;
                        }

                        warn!(
                            "Machine {} has VCPU timeout error, transitioning to restarting for cleanup: {}",
                            machine_name, message
                        );

                        // Transition to Restarting state - this will trigger the existing cleanup logic
                        ctx.repository
                            .machine(ctx.tenant.clone())
                            .patch_status(key.metadata(), |status| {
                                status.phase = MachinePhase::Restarting;
                                status.last_restarting_time_us =
                                    Some(Utc::now().timestamp_millis() as u64);
                            })
                            .await?;

                        return Ok(ReconcileNext::immediate());
                    }
                }
                _ => {}
            }
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

#[async_trait]
impl AdmissionCheckBeforeSet for Machine {
    async fn before_set(
        &self,
        _before: Option<&Self>,
        tenant: String,
        repo: Arc<Repository>,
        _agent: Arc<Agent>,
        _metadata: Metadata,
    ) -> Result<()> {
        let resource = self.latest();
        let resource_namespace = Namespace::from_value_or_default(resource.namespace.clone());

        if resource.build.is_some() {
            bail!("machine builds must be resolved by client");
        }

        if resource.image.is_none() {
            bail!("image is not set for machine: {}", resource.name);
        }

        // see if the volumes are being used by other machines
        let volumes = resource.volumes.unwrap_or_default();
        if volumes.is_empty() {
            return Ok(());
        }

        let machines = repo.machine(tenant.clone()).list(Namespace::Unspecified)?;
        for machine in machines {
            let machine = machine.latest();
            let machine_namespace = Namespace::from_value_or_default(machine.namespace.clone());
            if machine.name == resource.name && machine_namespace == resource_namespace {
                continue;
            }

            let machine_volumes = machine.volumes.unwrap_or_default();

            for volume in volumes.iter() {
                let volume_namespace = Namespace::from_value_or_default(
                    volume
                        .namespace
                        .clone()
                        .or_else(|| resource.namespace.clone()),
                );

                for machine_volume in machine_volumes.iter() {
                    if machine_volume.name != volume.name {
                        continue;
                    }

                    let machine_volume_namespace = Namespace::from_value_or_default(
                        machine_volume
                            .namespace
                            .clone()
                            .or_else(|| machine.namespace.clone()),
                    );

                    if machine_volume_namespace != volume_namespace {
                        continue;
                    }

                    bail!(
                        "Volume {} is being used by machine {} in namespace {}",
                        volume.name,
                        machine.name,
                        machine_volume_namespace
                            .as_value()
                            .unwrap_or(DEFAULT_NAMESPACE.to_string())
                    );
                }
            }
        }

        Ok(())
    }
}
