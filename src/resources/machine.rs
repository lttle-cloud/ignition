use anyhow::Result;
use meta::resource;
use std::collections::BTreeMap;

use crate::resources::FromResource;

#[resource(name = "Machine", tag = "machine")]
mod machine {

    #[version(stored + served + latest)]
    struct V1 {
        image: String,
        resources: MachineResources,
        mode: Option<MachineMode>,
        env: Option<BTreeMap<String, String>>,
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
        Flash(MachineSnapshotStrategy),
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
    }

    #[status]
    struct Status {
        hash: u64,
        phase: MachinePhase,
    }

    #[schema]
    enum MachinePhase {
        #[serde(rename = "idle")]
        Idle,
        #[serde(rename = "pulling-image")]
        PullingImage,
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
        })
    }
}
