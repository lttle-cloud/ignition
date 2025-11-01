use std::path::PathBuf;

use anyhow::{Result, bail};
use ignition::agent::certificate::config::CertProvider;
use ignition::agent::logs::LogsStoreConfig;
use ignition::agent::port_allocator::TcpPortRange;
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(skip_serializing, skip_deserializing)]
    pub config_path: PathBuf,
    #[serde(skip_serializing, skip_deserializing)]
    pub config_dir: PathBuf,

    #[serde(rename = "data-dir")]
    pub data_dir: PathBuf,

    #[serde(rename = "net")]
    pub net_config: NetConfig,

    #[serde(rename = "proxy")]
    pub proxy_config: ProxyConfig,

    #[serde(rename = "machine")]
    pub machine_config: MachineConfig,

    #[serde(rename = "api")]
    pub api_server_config: ApiServerConfig,

    #[serde(rename = "registry")]
    pub registry_config: RegistryConfig,

    #[serde(rename = "dns")]
    pub dns_config: DnsConfig,

    #[serde(rename = "cert-provider", default)]
    pub cert_providers: Vec<CertProvider>,

    #[serde(rename = "logs")]
    pub logs_config: LogsConfig,

    #[serde(rename = "openai")]
    pub openai_config: Option<OpenAIConfig>,

    #[serde(rename = "build")]
    pub build_config: Option<BuildConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetConfig {
    #[serde(rename = "bridge-name")]
    pub bridge_name: String,
    #[serde(rename = "vm-ip-cidr")]
    pub vm_ip_cidr: String,
    #[serde(rename = "service-ip-cidr")]
    pub service_ip_cidr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyConfig {
    #[serde(rename = "external-bind-address")]
    pub external_bind_address: String,
    #[serde(rename = "default-tls-cert-path")]
    pub default_tls_cert_path: String,
    #[serde(rename = "default-tls-key-path")]
    pub default_tls_key_path: String,
    #[serde(rename = "tcp-port-range")]
    pub tcp_port_range: Option<TcpPortRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MachineConfig {
    #[serde(rename = "kernel-path")]
    pub kernel_path: PathBuf,
    #[serde(rename = "initrd-path")]
    pub initrd_path: PathBuf,
    #[serde(rename = "append-cmd-line")]
    pub append_cmd_line: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiServerConfig {
    #[serde(rename = "host")]
    pub host: String,
    #[serde(rename = "port")]
    pub port: u16,
    #[serde(rename = "jwt-secret")]
    pub jwt_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryConfig {
    #[serde(rename = "service")]
    pub service: String,
    #[serde(rename = "registry-robot-hmac-secret")]
    pub registry_robot_hmac_secret: String,
    #[serde(rename = "registry-token-key-path")]
    pub registry_token_key_path: String,
    #[serde(rename = "registry-token-cert-path")]
    pub registry_token_cert_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DnsConfig {
    #[serde(rename = "zone-suffix")]
    pub zone_suffix: String,
    #[serde(rename = "default-ttl")]
    pub default_ttl: u32,
    #[serde(rename = "upstream-dns-servers", default)]
    pub upstream_dns_servers: Vec<String>,
    #[serde(rename = "region-root-domain")]
    pub region_root_domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogsConfig {
    #[serde(rename = "otel-ingest-endpoint")]
    pub otel_ingest_endpoint: String,
    pub store: LogsStoreConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAIConfig {
    #[serde(rename = "api-key")]
    pub api_key: String,

    #[serde(rename = "default-model")]
    pub default_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuildConfig {
    #[serde(rename = "ca-cert-path")]
    pub ca_cert_path: String,
    #[serde(rename = "ca-key-path")]
    pub ca_key_path: String,
    pub pool: Vec<String>,
}

async fn resolve_config_path(path_override: Option<PathBuf>) -> Result<PathBuf> {
    let config_path =
        path_override.or_else(|| std::env::var("IGNITION_CONFIG").ok().map(PathBuf::from));

    if let Some(path) = config_path {
        return Ok(path);
    } else {
        warn!("No config path override found, looking for config in default locations");

        let cwd = std::env::current_dir()?;

        // try to lead from $CWD/ignition.toml
        let path = cwd.join("ignition.toml");
        if path.exists() {
            return Ok(path);
        }

        warn!("No config found in current directory ({})", path.display());
        // try to lead from $HOME/.config/ignition/config.toml
        let Some(project_dirs) = directories::ProjectDirs::from("cloud", "lttle", "ignition")
        else {
            bail!("Failed to get config dir");
        };

        let path = project_dirs.config_dir().join("config.toml");
        if path.exists() {
            return Ok(path);
        }
        warn!("No config found in home config dir ({})", path.display());

        // check /etc/ignition/config.toml
        let path = PathBuf::from("/etc/ignition/config.toml");
        if path.exists() {
            return Ok(path);
        }
        warn!("No config found in global config dir ({})", path.display());
    }

    bail!("Couldn't load config file.");
}

impl Config {
    pub async fn load(path_override: Option<PathBuf>) -> Result<Self> {
        let config_path = resolve_config_path(path_override).await?;

        let config_str = read_to_string(&config_path).await?;
        let mut config: Self = toml::from_str(&config_str)?;
        config.config_path = config_path.clone();

        let Some(config_dir) = config_path.parent().map(|p| p.to_path_buf()) else {
            bail!("Couldn't determine config dir");
        };
        config.config_dir = config_dir;

        Ok(config)
    }

    pub fn absolute_data_dir(&self) -> PathBuf {
        self.config_dir.join(self.data_dir.clone())
    }
}
