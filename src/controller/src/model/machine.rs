use util::encoding::codec;
use vmm::config::SnapshotPolicy;

#[codec]
#[derive(Clone, Debug, PartialEq)]
pub enum StoredMachineState {
    Creating,
    Created,
    Stopped,
}

#[codec]
#[derive(Clone, Debug)]
pub struct StoredMachine {
    pub id: String,
    pub name: String,
    pub state: StoredMachineState,
    pub image_id: String,
    pub image_reference: String,
    pub image_volume_id: String,
    pub memory_size_mib: usize,
    pub vcpu_count: u8,
    pub envs: Vec<(String, String)>,
    pub snapshot_policy: Option<SnapshotPolicy>,
}
