use std::collections::HashMap;

use util::encoding::{codec, schemars};

#[codec(schema = true)]
pub enum MachineSnapshotPolicy {
    #[serde(rename = "on-nth-listen-syscall")]
    OnNthListenSyscall(u32),
    #[serde(rename = "on-listen-on-port")]
    OnListenOnPort(u16),
    #[serde(rename = "on-userspace-ready")]
    OnUserspaceReady,
    #[serde(rename = "manual")]
    Manual,
}

#[codec(schema = true)]
pub struct MachineEnvironmentVariable {
    pub name: String,
    pub value: String,
}

#[codec(schema = true)]
pub enum ServiceProtocol {
    #[serde(rename = "tcp")]
    Tcp { port: u16 },
    #[serde(rename = "tls")]
    Tls { port: u16 },
    #[serde(rename = "http")]
    Http,
}

#[codec(schema = true)]
pub enum ServiceMode {
    #[serde(rename = "internal")]
    Internal,
    #[serde(rename = "external")]
    External { host: String },
}

#[codec(schema = true)]
pub struct ServiceTarget {
    pub name: String,
    pub port: u16,
}

#[codec(schema = true)]
pub struct Service {
    pub name: String,
    pub target: ServiceTarget,
    pub protocol: ServiceProtocol,
    pub mode: ServiceMode,
}

#[codec(schema = true)]
pub struct Machine {
    pub name: String,
    pub image: String,
    pub memory: u64,
    pub vcpus: u8,
    pub environment: Option<Vec<MachineEnvironmentVariable>>,
    #[serde(rename = "snapshot-policy")]
    pub snapshot_policy: Option<MachineSnapshotPolicy>,
}

#[codec(schema = true)]
#[schemars(title = "Ignition Resources")]
pub enum Resource {
    #[serde(rename = "machine")]
    Machine(Machine),
    #[serde(rename = "service")]
    Service(Service),
}
