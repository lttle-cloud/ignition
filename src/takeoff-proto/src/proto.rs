use std::collections::HashMap;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TakeoffInitArgs {
    #[serde(rename = "e")]
    pub envs: HashMap<String, String>,
    #[serde(rename = "c")]
    pub cmd: Option<Vec<String>>,
    #[serde(rename = "m")]
    pub mount_points: Vec<MountPoint>,
    #[serde(rename = "l")]
    pub logs_telemetry_config: LogsTelemetryConfig,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MountPoint {
    #[serde(rename = "s")]
    pub source: String,
    #[serde(rename = "t")]
    pub target: String,
    #[serde(rename = "r")]
    pub read_only: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct LogsTelemetryConfig {
    #[serde(rename = "e")]
    pub endpoint: String,
    #[serde(rename = "s")]
    pub service_name: String,
    #[serde(rename = "t")]
    pub tenant_id: String,
    #[serde(rename = "n")]
    pub service_namespace: String,
    #[serde(rename = "g")]
    pub service_group: String,
}

impl TakeoffInitArgs {
    pub fn encode(&self) -> Result<String> {
        let bytes = serde_json::to_string(&self)?;
        let data = hex::encode(bytes);
        Ok(data)
    }

    pub fn decode(data: &str) -> Result<Self> {
        let bytes = hex::decode(data)?;
        let args = serde_json::from_slice(&bytes)?;
        Ok(args)
    }

    pub fn try_parse_from_kernel_cmdline(cmdline: &str) -> Result<Self> {
        // extract takeoff=... from cmdline
        let takeoff_str = cmdline.split("takeoff=").nth(1).unwrap_or("");
        let takeoff_str = takeoff_str.split(" ").nth(0).unwrap_or("");
        let takeoff_str = takeoff_str.trim();

        if takeoff_str.is_empty() {
            bail!("No takeoff args found in cmdline");
        }

        let args = Self::decode(takeoff_str)?;
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode() {
        let args = TakeoffInitArgs {
            envs: HashMap::from([("TEST".to_string(), "test".to_string())]),
            cmd: Some(vec!["echo".to_string(), "test".to_string()]),
            mount_points: vec![MountPoint {
                source: "/dev/vdb".to_string(),
                target: "/mnt/data".to_string(),
                read_only: true,
            }],
            logs_telemetry_config: LogsTelemetryConfig {
                endpoint: "http://localhost:3100/otlp/v1/logs".to_string(),
                service_name: "test".to_string(),
                tenant_id: "test".to_string(),
                service_namespace: "test".to_string(),
                service_group: "test".to_string(),
            },
        };
        let encoded = args.encode().unwrap();
        let decoded = TakeoffInitArgs::decode(&encoded).unwrap();
        assert_eq!(args, decoded);
    }
}
