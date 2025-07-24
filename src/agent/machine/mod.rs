pub mod machine;
pub mod vm;

use anyhow::Result;
use papaya::HashMap;
use std::sync::Arc;

use crate::agent::machine::machine::{Machine, MachineConfig, MachineRef};

#[derive(Debug, Clone)]
pub struct MachineAgentConfig {
    pub kernel_path: String,
    pub initrd_path: String,
    pub kernel_cmd_init: String,
}

pub struct MachineAgent {
    config: MachineAgentConfig,
    machines: Arc<HashMap<String, MachineRef>>,
}

impl MachineAgent {
    pub fn new(config: MachineAgentConfig) -> Self {
        Self {
            config,
            machines: Arc::new(HashMap::new()),
        }
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
        let machine = Machine::new(&self.config, config).await?;

        let machines = self.machines.pin();
        machines.insert(machine.config.name.clone(), machine.clone());

        Ok(machine)
    }
}
