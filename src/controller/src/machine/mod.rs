mod machine;

pub use machine::*;
pub use vmm::config::SnapshotPolicy;

use papaya::HashMap;
use sds::{Collection, Store};
use util::{
    result::{Result, bail},
    tracing::info,
};

use crate::model::machine::StoredMachine;

#[derive(Debug, Clone)]
pub struct MachineInfo {
    pub id: String,
    pub name: String,
    pub status: MachineStatus,
    pub image_reference: String,
    pub snapshot_policy: Option<SnapshotPolicy>,
    pub ip_addr: Option<String>,
}

pub struct MachinePool {
    store: Store,
    collection: Collection<StoredMachine>,

    machines: HashMap<String, Machine>,
}

impl MachinePool {
    pub fn new(store: Store) -> Result<Self> {
        let collection = store.collection::<StoredMachine>("machines")?;

        Ok(Self {
            store,
            collection,
            machines: HashMap::new(),
        })
    }

    pub async fn get_stored_machines(&self) -> Result<Vec<StoredMachine>> {
        let tx = self.store.read_txn()?;
        let stored_machines = tx.get_all_values(&self.collection)?;
        Ok(stored_machines)
    }

    pub async fn get_machine_info(&self, id: &str) -> Result<Option<MachineInfo>> {
        let stored_machines = self.get_stored_machines().await?;
        let Some(stored_machine) = stored_machines.iter().find(|m| m.id == id).cloned() else {
            return Ok(None);
        };

        let machine = self.get_machine(&id);

        let info = match machine {
            Some(machine) => {
                let status = machine.status().await?;

                MachineInfo {
                    id: stored_machine.id,
                    name: stored_machine.name,
                    status,
                    image_reference: stored_machine.image_reference,
                    snapshot_policy: stored_machine.snapshot_policy,
                    ip_addr: machine.config.ip_addr.into(),
                }
            }
            None => MachineInfo {
                id: stored_machine.id,
                name: stored_machine.name,
                status: MachineStatus::New,
                image_reference: stored_machine.image_reference,
                snapshot_policy: stored_machine.snapshot_policy,
                ip_addr: None,
            },
        };

        Ok(Some(info))
    }

    pub async fn get_machines_info(&self) -> Result<Vec<MachineInfo>> {
        let stored_machines = self.get_stored_machines().await?;

        let mut machines_info = Vec::new();

        for stored_machine in stored_machines {
            let machine = self.get_machine(&stored_machine.id);

            let info = match machine {
                Some(machine) => {
                    let status = machine.status().await?;

                    MachineInfo {
                        id: stored_machine.id,
                        name: stored_machine.name,
                        status,
                        image_reference: stored_machine.image_reference,
                        snapshot_policy: stored_machine.snapshot_policy,
                        ip_addr: machine.config.ip_addr.into(),
                    }
                }
                None => MachineInfo {
                    id: stored_machine.id,
                    name: stored_machine.name,
                    status: MachineStatus::New,
                    image_reference: stored_machine.image_reference,
                    snapshot_policy: stored_machine.snapshot_policy,
                    ip_addr: None,
                },
            };

            machines_info.push(info);
        }

        Ok(machines_info)
    }

    pub fn get_stored_machines_for_name(&self, name: &str) -> Result<Vec<StoredMachine>> {
        let tx = self.store.read_txn()?;
        let stored_machines = tx.get_all_values_prefix(&self.collection, name)?;
        Ok(stored_machines)
    }

    pub fn get_machine(&self, id: &str) -> Option<Machine> {
        let machines = self.machines.pin();
        let Some(machine) = machines.get(id) else {
            return None;
        };

        Some(machine.clone())
    }

    pub async fn stop_machine(&self, id: &str) -> Result<()> {
        let Some(machine) = self.get_machine(id) else {
            bail!("Machine not found");
        };

        machine.stop(MachineStopReason::Shutdown).await?;
        machine.wait_for_status(MachineStatus::Stopped).await?;

        Ok(())
    }

    pub async fn start_machine(&self, id: &str) -> Result<()> {
        let Some(machine) = self.get_machine(id) else {
            bail!("Machine not found");
        };
        machine.start().await?;
        Ok(())
    }

    pub async fn set_partial_machine(&self, stored_machine: StoredMachine) -> Result<()> {
        let key = format!("{}:{}", stored_machine.name, stored_machine.id);

        {
            let mut tx = self.store.write_txn()?;
            tx.put(&self.collection, &key, &stored_machine)?;
            tx.commit()?;
        }

        Ok(())
    }

    pub async fn set_machine(&self, stored_machine: StoredMachine, machine: Machine) -> Result<()> {
        let key = format!("{}:{}", stored_machine.name, stored_machine.id);

        {
            let mut tx = self.store.write_txn()?;
            tx.put(&self.collection, &key, &stored_machine)?;
            tx.commit()?;
        }

        let machines = self.machines.pin();
        machines.insert(stored_machine.id, machine);

        Ok(())
    }

    pub async fn resolve_existing_machine_from_name(&self, name: &str) -> Result<Machine> {
        // todo: we should cache resolutions
        let stored_machines = self.get_stored_machines_for_name(name)?;
        for stored_machine in stored_machines {
            let machine = self.get_machine(&stored_machine.id);
            if let Some(machine) = machine {
                return Ok(machine);
            }
        }

        bail!("Machine not found");
    }

    pub async fn delete_machine(&self, id: &str, wait_for_stopped: bool) -> Result<()> {
        info!("Deleting machine: {}", id);
        let Some(machine) = self.get_machine(id) else {
            bail!("Machine not found");
        };

        info!("Stopping machine: {}", id);
        machine.stop(MachineStopReason::Shutdown).await?;
        if wait_for_stopped {
            info!("Waiting for machine to stop: {}", id);
            machine.wait_for_status(MachineStatus::Stopped).await?;
        }
        info!("Machine stopped: {}", id);

        let key = format!("{}:{}", machine.config.name, machine.config.id);

        {
            let mut tx = self.store.write_txn()?;
            tx.del(&self.collection, &key)?;
            tx.commit()?;
        }

        let machines = self.machines.pin();
        machines.remove(id);

        Ok(())
    }

    pub async fn get_used_tap_names(&self) -> Result<Vec<String>> {
        let machines = self.machines.pin();

        let tap_names = machines
            .iter()
            .map(|(_, m)| m.config.tap_name.clone())
            .collect();
        Ok(tap_names)
    }

    pub async fn get_used_ip_addrs(&self) -> Result<Vec<String>> {
        let machines = self.machines.pin();
        let ip_addrs = machines
            .iter()
            .map(|(_, m)| m.config.ip_addr.clone())
            .collect();
        Ok(ip_addrs)
    }

    pub async fn get_used_image_ids(&self) -> Result<Vec<String>> {
        let stored_machines = self.get_stored_machines().await?;

        let image_ids = stored_machines.iter().map(|m| m.image_id.clone()).collect();

        Ok(image_ids)
    }
    pub async fn get_used_image_volume_ids(&self) -> Result<Vec<String>> {
        let stored_machines = self.get_stored_machines().await?;

        let image_volume_ids = stored_machines
            .iter()
            .map(|m| m.image_volume_id.clone())
            .collect();
        Ok(image_volume_ids)
    }
}
