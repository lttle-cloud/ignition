pub mod machine;
pub mod state_machine;
pub mod vm;

use anyhow::{Result, bail};
use papaya::HashMap;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use crate::{
    agent::machine::machine::{Machine, MachineConfig, MachineRef},
    controller::scheduler::Scheduler,
};

#[derive(Debug, Clone)]
pub struct MachineAgentConfig {
    pub kernel_path: String,
    pub initrd_path: String,
    pub kernel_cmd_init: String,
    pub transient_state_path: PathBuf,
}

pub struct MachineAgent {
    config: MachineAgentConfig,
    scheduler: Weak<Scheduler>,
    machines: Arc<HashMap<String, MachineRef>>,
}

impl MachineAgent {
    pub async fn new(config: MachineAgentConfig, scheduler: Weak<Scheduler>) -> Result<Self> {
        if !config.transient_state_path.exists() {
            tokio::fs::create_dir_all(&config.transient_state_path).await?;
        }

        Ok(Self {
            config,
            scheduler,
            machines: Arc::new(HashMap::new()),
        })
    }

    pub fn transient_dir(&self, rel: impl AsRef<Path>) -> String {
        let path = self.config.transient_state_path.clone().join(rel);
        path.to_string_lossy().to_string()
    }

    pub fn get_machine(&self, name: &str) -> Option<MachineRef> {
        let machines = self.machines.pin();
        machines.get(name).cloned()
    }

    pub fn list_machines(&self) -> Vec<MachineRef> {
        let machines = self.machines.pin();
        machines.values().cloned().collect()
    }

    pub async fn create_machine(&self, config: MachineConfig) -> Result<MachineRef> {
        let machine = Machine::new(&self.config, config, self.scheduler.clone()).await?;

        let machines = self.machines.pin();
        machines.insert(machine.config.name.clone(), machine.clone());

        Ok(machine)
    }

    pub async fn delete_machine(&self, name: &str) -> Result<()> {
        let machines = self.machines.pin();
        if let Some(_) = machines.remove(name) {
            return Ok(());
        };
        bail!("Machine '{}' not found", name)
    }

    pub async fn get_machine_by_network_tag(&self, network_tag: &str) -> Option<MachineRef> {
        let machines = self.machines.pin();

        machines
            .values()
            .find(|m| m.config.network_tag == network_tag)
            .cloned()
    }
}
