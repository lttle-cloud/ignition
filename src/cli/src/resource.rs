use util::encoding::{codec, schemars};

#[codec(schema = true)]
#[derive(Default)]
pub enum SnapshotStrategy {
    #[default]
    #[serde(rename = "none")]
    None,
    #[serde(rename = "boot")]
    Boot,
    #[serde(rename = "net")]
    Net,
}

#[codec(schema = true)]
pub struct OnDemandSnapshot {
    strategy: SnapshotStrategy,
    stateful: Option<bool>,
}

#[codec(schema = true)]
pub enum DeploymentMode {
    #[serde(rename = "always-on")]
    AlwaysOn,
    #[serde(rename = "on-demand")]
    OnDemand {
        snapshot: OnDemandSnapshot,
        allow_idle_connection: Option<bool>,
    },
}

impl Default for DeploymentMode {
    fn default() -> Self {
        DeploymentMode::AlwaysOn
    }
}

#[codec(schema = true)]
#[serde(untagged)]
pub enum DeploymentScaling {
    Fixed { replicas: u32 },
    Auto { min: u32, max: u32 },
}

impl Default for DeploymentScaling {
    fn default() -> Self {
        DeploymentScaling::Fixed { replicas: 1 }
    }
}

#[codec(schema = true)]
pub struct DeploymentEnvironmentVariable {
    name: String,
    value: String,
}

#[codec(schema = true)]
pub enum DeploymentInternalServiceProtocol {
    #[serde(rename = "http")]
    Http,

    #[serde(rename = "tcp")]
    Tcp,
}

#[codec(schema = true)]
pub enum DeploymentExternalServiceProtocol {
    #[serde(rename = "http")]
    Http,

    #[serde(rename = "tcp/tls")]
    TcpTls,
}

#[codec(schema = true)]
#[derive(Default)]
pub enum DeploymentExternalSErviceTlsTerminationMode {
    #[serde(rename = "passthrough")]
    Passthrough,

    #[serde(rename = "reencrypt")]
    #[default]
    Reencrypt,
}

#[codec(schema = true)]
#[derive(Default)]
#[serde(untagged)]
pub enum IngressCertificate {
    #[default]
    #[serde(rename = "auto")]
    Auto,

    #[serde(rename = "manual")]
    Manual { name: String },
}

#[codec(schema = true)]
pub struct DeploymentExternalServiceIngress {
    host: String,
    cert: Option<IngressCertificate>,
}

#[codec(schema = true)]
pub enum DeploymentService {
    #[serde(rename = "internal")]
    Internal {
        name: String,
        port: u16,
        protocol: Option<DeploymentInternalServiceProtocol>,
    },
    #[serde(rename = "external")]
    External {
        name: String,
        port: u16,
        protocol: Option<DeploymentExternalServiceProtocol>,
        tls_termination: Option<DeploymentExternalSErviceTlsTerminationMode>,
        ingress: Option<DeploymentExternalServiceIngress>,
    },
}

#[codec(schema = true)]
pub struct Deployment {
    pub name: String,
    pub image: String,
    pub memory: u64,
    pub vcpus: u8,
    pub mode: Option<DeploymentMode>,
    pub scaling: Option<DeploymentScaling>,
    pub environment: Option<Vec<DeploymentEnvironmentVariable>>,
    pub services: Option<Vec<DeploymentService>>,
}

#[codec(schema = true)]
#[schemars(title = "Ignition Resources")]
pub enum Resource {
    #[serde(rename = "deployment")]
    Deployment(Deployment),
}
