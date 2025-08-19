use anyhow::{Result, bail};
use meta::resource;
use std::collections::BTreeMap;

use crate::resources::{AdmissionCheckStatus, Convert, FromResource, ProvideMetadata};

#[resource(name = "Machine", tag = "machine")]
mod machine {

    #[version(stored + served + latest)]
    struct V1 {
        image: String,
        resources: MachineResources,
        mode: Option<MachineMode>,
        volumes: Option<Vec<MachineVolumeBinding>>,
        command: Option<Vec<String>>,
        environment: Option<BTreeMap<String, String>>,
        #[serde(rename = "depends-on")]
        depends_on: Option<Vec<MachineDependency>>,
    }

    #[schema]
    struct MachineResources {
        cpu: u8,
        memory: u64,
    }

    #[schema]
    enum MachineMode {
        #[serde(rename = "regular")]
        Regular,
        #[serde(rename = "flash")]
        Flash {
            strategy: MachineSnapshotStrategy,
            timeout: Option<u64>,
        },
    }

    #[schema]
    enum MachineSnapshotStrategy {
        #[serde(rename = "first-listen")]
        WaitForFirstListen,
        #[serde(rename = "nth-listen")]
        WaitForNthListen(u32),
        #[serde(rename = "listen-on-port")]
        WaitForListenOnPort(u16),
        #[serde(rename = "user-space-ready")]
        WaitForUserSpaceReady,
        #[serde(rename = "manual")]
        Manual,
    }

    #[schema]
    struct MachineVolumeBinding {
        name: String,
        namespace: Option<String>,
        path: String,
    }

    #[schema]
    struct MachineDependency {
        name: String,
        namespace: Option<String>,
    }

    #[status]
    struct Status {
        hash: u64,
        phase: MachinePhase,
        image_id: Option<String>,
        image_resolved_reference: Option<String>,
        machine_id: Option<String>,
        machine_ip: Option<String>,
        machine_tap: Option<String>,
        machine_image_volume_id: Option<String>,
        last_boot_time_us: Option<u128>,
        first_boot_time_us: Option<u128>,
    }

    #[schema]
    enum MachinePhase {
        #[serde(rename = "idle")]
        Idle,
        #[serde(rename = "pulling-image")]
        PullingImage,
        #[serde(rename = "waiting")]
        Waiting,
        #[serde(rename = "creating")]
        Creating,
        #[serde(rename = "booting")]
        Booting,
        #[serde(rename = "ready")]
        Ready,
        #[serde(rename = "suspending")]
        Suspending,
        #[serde(rename = "suspended")]
        Suspended,
        #[serde(rename = "stopping")]
        Stopping,
        #[serde(rename = "stopped")]
        Stopped,
        #[serde(rename = "error")]
        Error { message: String },
    }
}

impl ToString for MachinePhase {
    fn to_string(&self) -> String {
        match self {
            MachinePhase::Idle => "idle".to_string(),
            MachinePhase::PullingImage => "pulling-image".to_string(),
            MachinePhase::Creating => "creating".to_string(),
            MachinePhase::Waiting => "waiting".to_string(),
            MachinePhase::Booting => "booting".to_string(),
            MachinePhase::Ready => "ready".to_string(),
            MachinePhase::Suspending => "suspending".to_string(),
            MachinePhase::Suspended => "suspended".to_string(),
            MachinePhase::Stopping => "stopping".to_string(),
            MachinePhase::Stopped => "stopped".to_string(),
            MachinePhase::Error { message } => format!("error ({})", message),
        }
    }
}

impl FromResource<Machine> for MachineStatus {
    fn from_resource(_resource: Machine) -> Result<Self> {
        Ok(MachineStatus {
            hash: 0,
            phase: MachinePhase::Idle,
            image_id: None,
            image_resolved_reference: None,
            machine_id: None,
            machine_ip: None,
            machine_tap: None,
            machine_image_volume_id: None,
            last_boot_time_us: None,
            first_boot_time_us: None,
        })
    }
}

impl Machine {
    pub fn hash_with_updated_metadata(&self) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let metadata = self.metadata();
        let mut machine = self.stored();
        machine.namespace = metadata.namespace;
        let machine: Machine = machine.into();

        let mut hasher = DefaultHasher::new();
        machine.hash(&mut hasher);
        hasher.finish()
    }
}

impl AdmissionCheckStatus<MachineStatus> for Machine {
    fn admission_check_status(&self, status: &MachineStatus) -> Result<()> {
        let hash = self.hash_with_updated_metadata();

        if hash != status.hash {
            bail!("Machines are not allowed to change their configuration after creation");
        }

        Ok(())
    }
}
