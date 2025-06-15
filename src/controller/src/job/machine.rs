use std::sync::Arc;

use oci_client::Reference;
use util::{
    async_runtime::spawn,
    id::short_id_with_prefix,
    result::{Result, bail},
    tracing::warn,
    uuid,
};
use vmm::config::SnapshotPolicy;

use crate::{
    image::ImagePool,
    logs::LogsPool,
    machine::{Machine, MachineConfig, MachinePool, MachineStatus, MachineStopReason},
    model::machine::{StoredMachine, StoredMachineState},
    net::{ip::IpPool, tap::TapPool},
};

use super::Job;

pub struct DeployMachineJobInput {
    pub name: String,
    pub image_name: String,
    pub vcpu_count: u8,
    pub memory_size_mib: usize,
    pub envs: Vec<(String, String)>,
    pub snapshot_policy: Option<SnapshotPolicy>,
}

pub struct DeployMachineJob {
    work_state: DeployMachineJobWorkState,

    image_pool: Arc<ImagePool>,
    tap_pool: Arc<TapPool>,
    ip_pool: Arc<IpPool>,
    logs_pool: Arc<LogsPool>,
    machine_pool: Arc<MachinePool>,
    input: DeployMachineJobInput,
}

#[derive(Clone, Default)]
pub struct DeployMachineJobWorkState {
    tap_name: Option<String>,
    ip_addr: Option<String>,
    image_volume_id: Option<String>,
    machine: Option<Machine>,
    stored_machine: Option<StoredMachine>,
}

impl DeployMachineJob {
    pub fn new(
        image_pool: Arc<ImagePool>,
        tap_pool: Arc<TapPool>,
        ip_pool: Arc<IpPool>,
        logs_pool: Arc<LogsPool>,
        machine_pool: Arc<MachinePool>,
        input: DeployMachineJobInput,
    ) -> Result<Self> {
        let job = Self {
            work_state: Default::default(),
            image_pool,
            tap_pool,
            ip_pool,
            logs_pool,
            machine_pool,
            input,
        };

        Ok(job)
    }
}

impl Job for DeployMachineJob {
    type Output = (String, StoredMachine, Machine);

    async fn run(&mut self) -> Result<Self::Output> {
        let id = short_id_with_prefix(&self.input.name);

        let image_reference = self.input.image_name.parse::<Reference>()?;
        let Some(image) = self
            .image_pool
            .get_latest_by_reference(&image_reference)
            .await?
        else {
            bail!("Image not found: {}", self.input.image_name);
        };

        let envs_strs = self
            .input
            .envs
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>();

        let image_volume = self.image_pool.get_clone_image_volume(&image, &id).await?;
        self.work_state.image_volume_id = Some(image_volume.id.clone());

        let tap_name = self.tap_pool.create_tap().await?;
        self.work_state.tap_name = Some(tap_name.clone());

        let ip_addr = self.ip_pool.reserve_tagged(format!("machine-{}", id))?;
        self.work_state.ip_addr = Some(ip_addr.addr.clone());

        let mut stored_machine = StoredMachine {
            id: id.clone(),
            name: self.input.name.clone(),
            state: StoredMachineState::Creating,
            memory_size_mib: self.input.memory_size_mib,
            vcpu_count: self.input.vcpu_count,
            image_id: image.id.clone(),
            image_reference: image.reference.to_string(),
            image_volume_id: image_volume.id.clone(),
            envs: self.input.envs.clone(),
            snapshot_policy: self.input.snapshot_policy.clone(),
        };

        self.machine_pool
            .set_partial_machine(stored_machine.clone())
            .await?;
        self.work_state.stored_machine = Some(stored_machine.clone());

        let machine_config = MachineConfig {
            name: self.input.name.clone(),
            id: id.clone(),
            memory_size_mib: self.input.memory_size_mib,
            vcpu_count: self.input.vcpu_count,
            rootfs_path: image_volume.path,
            tap_name,
            ip_addr: ip_addr.addr,
            gateway: self.ip_pool.gateway().to_string(),
            netmask: self.ip_pool.netmask().to_string(),
            envs: envs_strs,
            log_file_path: self.logs_pool.get_machine_log_path(&id),
            snapshot_policy: self.input.snapshot_policy.clone(),
        };

        let machine = Machine::new(machine_config)?;
        machine.start().await?;

        let status_to_wait_for = match self.input.snapshot_policy {
            Some(_) => MachineStatus::Suspended,
            None => MachineStatus::Running,
        };

        if let Err(e) = machine.wait_for_status(status_to_wait_for).await {
            bail!("Failed to start machine: {}", e);
        };

        stored_machine.state = StoredMachineState::Created;
        self.machine_pool
            .set_machine(stored_machine.clone(), machine.clone())
            .await?;
        self.work_state.stored_machine = Some(stored_machine.clone());

        println!("Machine started");

        let other_machines = self
            .machine_pool
            .get_stored_machines_for_name(&self.input.name)?;

        println!("Other machines: {:?}", other_machines);

        for machine in other_machines {
            if machine.id == id {
                println!("Skipping self");
                continue;
            }

            println!("deleting machine: {:?}", machine.id);

            self.machine_pool.delete_machine(&machine.id, false).await?;
        }

        Ok((id, stored_machine, machine))
    }

    fn cancel(&mut self) {
        warn!("Cancelling create machine job");

        let work_state = self.work_state.clone();
        let tap_pool = self.tap_pool.clone();
        let ip_pool = self.ip_pool.clone();
        let image_pool = self.image_pool.clone();
        let machine_pool = self.machine_pool.clone();

        spawn(async move {
            if let Some(machine) = work_state.machine {
                if let Err(e) = machine.stop(MachineStopReason::Shutdown).await {
                    warn!("Failed to stop machine: {}", e);
                }
            }

            if let Some(stored_machine) = work_state.stored_machine {
                if let Err(e) = machine_pool.delete_machine(&stored_machine.id, false).await {
                    warn!("Failed to delete machine: {}", e);
                }
            }

            if let Some(tap_name) = work_state.tap_name {
                if let Err(e) = tap_pool.delete_tap(&tap_name).await {
                    warn!("Failed to delete tap: {}", e);
                }
            }

            if let Some(ip_addr) = work_state.ip_addr {
                if let Err(e) = ip_pool.release(&ip_addr) {
                    warn!("Failed to release IP: {}", e);
                }
            }

            if let Some(image_volume_id) = work_state.image_volume_id {
                if let Err(e) = image_pool
                    .get_volume_pool()
                    .delete_volume(&image_volume_id)
                    .await
                {
                    warn!("Failed to delete image volume: {}", e);
                }
            }

            warn!("Create machine job cancelled");
        });
    }
}

pub struct BringUpMachineJobInput {
    pub stored_machine: StoredMachine,
}

pub struct BringUpMachineJob {
    work_state: BringUpMachineJobWorkState,

    image_pool: Arc<ImagePool>,
    tap_pool: Arc<TapPool>,
    ip_pool: Arc<IpPool>,
    logs_pool: Arc<LogsPool>,
    machine_pool: Arc<MachinePool>,
    input: BringUpMachineJobInput,
}

#[derive(Clone, Default)]
pub struct BringUpMachineJobWorkState {
    tap_name: Option<String>,
    ip_addr: Option<String>,
    image_volume_id: Option<String>,
    machine: Option<Machine>,
    stored_machine: Option<StoredMachine>,
}

impl BringUpMachineJob {
    pub fn new(
        image_pool: Arc<ImagePool>,
        tap_pool: Arc<TapPool>,
        ip_pool: Arc<IpPool>,
        logs_pool: Arc<LogsPool>,
        machine_pool: Arc<MachinePool>,
        input: BringUpMachineJobInput,
    ) -> Result<Self> {
        let job = Self {
            work_state: Default::default(),
            image_pool,
            tap_pool,
            ip_pool,
            logs_pool,
            machine_pool,
            input,
        };

        Ok(job)
    }
}

impl Job for BringUpMachineJob {
    type Output = ();

    async fn run(&mut self) -> Result<Self::Output> {
        let id = self.input.stored_machine.id.clone();
        let envs_strs = self
            .input
            .stored_machine
            .envs
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>();

        let Some(image_volume) = self
            .image_pool
            .get_volume_pool()
            .get(&self.input.stored_machine.image_volume_id)
            .await?
        else {
            bail!(
                "Image volume not found: {}",
                self.input.stored_machine.image_volume_id
            );
        };

        self.work_state.image_volume_id = Some(image_volume.id.clone());

        let tap_name = self.tap_pool.create_tap().await?;
        self.work_state.tap_name = Some(tap_name.clone());

        let ip_addr = self.ip_pool.reserve_tagged(format!("machine-{}", id))?;
        self.work_state.ip_addr = Some(ip_addr.addr.clone());

        let mut stored_machine = self.input.stored_machine.clone();
        stored_machine.state = StoredMachineState::Creating;

        self.machine_pool
            .set_partial_machine(stored_machine.clone())
            .await?;
        self.work_state.stored_machine = Some(stored_machine.clone());

        let machine_config = MachineConfig {
            name: self.input.stored_machine.name.clone(),
            id: id.clone(),
            memory_size_mib: self.input.stored_machine.memory_size_mib,
            vcpu_count: self.input.stored_machine.vcpu_count,
            rootfs_path: image_volume.path,
            tap_name,
            ip_addr: ip_addr.addr,
            gateway: self.ip_pool.gateway().to_string(),
            netmask: self.ip_pool.netmask().to_string(),
            envs: envs_strs,
            log_file_path: self.logs_pool.get_machine_log_path(&id),
            snapshot_policy: self.input.stored_machine.snapshot_policy.clone(),
        };

        let machine = Machine::new(machine_config)?;
        machine.start().await?;

        let status_to_wait_for = match self.input.stored_machine.snapshot_policy {
            Some(_) => MachineStatus::Suspended,
            None => MachineStatus::Running,
        };

        if let Err(e) = machine.wait_for_status(status_to_wait_for).await {
            bail!("Failed to start machine: {}", e);
        };

        stored_machine.state = StoredMachineState::Created;
        self.machine_pool
            .set_machine(stored_machine.clone(), machine.clone())
            .await?;
        self.work_state.stored_machine = Some(stored_machine.clone());

        println!("Machine started");

        let other_machines = self
            .machine_pool
            .get_stored_machines_for_name(&self.input.stored_machine.name)?;

        println!("Other machines: {:?}", other_machines);

        for machine in other_machines {
            if machine.id == id {
                println!("Skipping self");
                continue;
            }

            println!("deleting machine: {:?}", machine.id);

            self.machine_pool.delete_machine(&machine.id, false).await?;
        }

        Ok(())
    }

    fn cancel(&mut self) {
        warn!("Cancelling create machine job");

        let work_state = self.work_state.clone();
        let tap_pool = self.tap_pool.clone();
        let ip_pool = self.ip_pool.clone();
        let image_pool = self.image_pool.clone();
        let machine_pool = self.machine_pool.clone();

        spawn(async move {
            if let Some(machine) = work_state.machine {
                if let Err(e) = machine.stop(MachineStopReason::Shutdown).await {
                    warn!("Failed to stop machine: {}", e);
                }
            }

            if let Some(stored_machine) = work_state.stored_machine {
                if let Err(e) = machine_pool.delete_machine(&stored_machine.id, false).await {
                    warn!("Failed to delete machine: {}", e);
                }
            }

            if let Some(tap_name) = work_state.tap_name {
                if let Err(e) = tap_pool.delete_tap(&tap_name).await {
                    warn!("Failed to delete tap: {}", e);
                }
            }

            if let Some(ip_addr) = work_state.ip_addr {
                if let Err(e) = ip_pool.release(&ip_addr) {
                    warn!("Failed to release IP: {}", e);
                }
            }

            if let Some(image_volume_id) = work_state.image_volume_id {
                if let Err(e) = image_pool
                    .get_volume_pool()
                    .delete_volume(&image_volume_id)
                    .await
                {
                    warn!("Failed to delete image volume: {}", e);
                }
            }

            warn!("Create machine job cancelled");
        });
    }
}
