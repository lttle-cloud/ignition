pub mod device;
pub mod ip_range;

use std::{net::Ipv4Addr, sync::Arc};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    agent::{
        data::Collections,
        net::{
            device::{
                device_create, nl_device_delete, nl_device_exists, nl_device_list_with_prefix,
            },
            ip_range::IpRange,
        },
    },
    constants::DEFAULT_AGENT_TENANT,
    machinery::store::{Key, PartialKey, Store},
    utils::id::short_id,
};

const NET_DEVICE_PREFIX: &str = "tap_lt_";

#[derive(Debug, Clone)]
pub struct NetDevice {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct NetAgentConfig {
    pub bridge_name: String,
    pub vm_ip_cidr: String,
    pub service_ip_cidr: String,
}

pub struct NetAgent {
    pub config: NetAgentConfig,
    store: Arc<Store>,

    vm_ip_range: IpRange,
    service_ip_range: IpRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IpReservationKind {
    VM,
    Service,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpReservation {
    pub kind: IpReservationKind,
    pub ip: String,
    pub tag: Option<String>,
}

pub fn compute_mac_for_ip(ip: &str) -> Result<String> {
    let mut mac = [0u8; 6];
    let ip: Ipv4Addr = ip.parse()?;

    mac[0] = 0x02; // Local Admin bit set
    mac[1] = 0x42; // Arbitrary value
    mac[2] = (ip.octets()[0] ^ 0x42) & 0x3f; // Mask to ensure unicast
    mac[3] = ip.octets()[1];
    mac[4] = ip.octets()[2];
    mac[5] = ip.octets()[3];

    let mac_str = mac
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(":");

    Ok(mac_str)
}

impl NetDevice {
    pub async fn delete(self) -> Result<()> {
        nl_device_delete(&self.name).await
    }
}

impl NetAgent {
    pub async fn new(config: NetAgentConfig, store: Arc<Store>) -> Result<Self> {
        let vm_ip_range = IpRange::from_cidr(&config.vm_ip_cidr)?;
        let service_ip_range = IpRange::from_cidr(&config.service_ip_cidr)?;

        if !nl_device_exists(&config.bridge_name).await? {
            bail!("bridge {} not found", config.bridge_name);
        }

        Ok(Self {
            config,
            store,
            vm_ip_range,
            service_ip_range,
        })
    }

    pub fn vm_gateway(&self) -> Ipv4Addr {
        self.vm_ip_range.gateway()
    }

    pub fn service_gateway(&self) -> Ipv4Addr {
        self.service_ip_range.gateway()
    }

    pub fn vm_netmask(&self) -> Ipv4Addr {
        self.vm_ip_range.netmask()
    }

    pub fn service_netmask(&self) -> Ipv4Addr {
        self.service_ip_range.netmask()
    }

    pub fn device_unchecked(&self, name: &str) -> Result<NetDevice> {
        Ok(NetDevice {
            name: name.to_string(),
        })
    }

    pub async fn device_create(&self) -> Result<NetDevice> {
        let mut name = format!("{}{}", NET_DEVICE_PREFIX, short_id());
        while nl_device_exists(&name).await? {
            name = format!("{}{}", NET_DEVICE_PREFIX, short_id());
        }

        device_create(&name, &self.config.bridge_name).await?;

        self.device_unchecked(&name)
    }

    pub async fn device_delete(&self, name: &str) -> Result<()> {
        nl_device_delete(name).await
    }

    pub async fn device(&self, name: &str) -> Result<NetDevice> {
        if !nl_device_exists(name).await? {
            device_create(name, &self.config.bridge_name).await?;
        }

        self.device_unchecked(name)
    }

    pub async fn device_list(&self) -> Result<Vec<NetDevice>> {
        let devices = nl_device_list_with_prefix(NET_DEVICE_PREFIX).await?;

        Ok(devices
            .into_iter()
            .map(|name| self.device_unchecked(&name))
            .collect::<Result<Vec<_>>>()?)
    }

    pub fn ip_reservation_create(
        &self,
        kind: IpReservationKind,
        tag: Option<String>,
    ) -> Result<IpReservation> {
        let ip_range = match kind {
            IpReservationKind::VM => &self.vm_ip_range,
            IpReservationKind::Service => &self.service_ip_range,
        };

        loop {
            let ip = ip_range.random();

            let collection = match kind {
                IpReservationKind::VM => Collections::VmIpReservation,
                IpReservationKind::Service => Collections::ServiceIpReservation,
            };

            let key = Key::<IpReservation>::not_namespaced()
                .tenant(DEFAULT_AGENT_TENANT)
                .collection(collection)
                .key(ip.to_string());

            if self.store.get::<IpReservation>(&key)?.is_some() {
                continue;
            }

            let reservation = IpReservation {
                kind,
                ip: ip.to_string(),
                tag,
            };

            self.store.put(&key, &reservation)?;
            return Ok(reservation);
        }
    }

    pub fn ip_reservation_list(&self, kind: IpReservationKind) -> Result<Vec<IpReservation>> {
        let collection = match kind {
            IpReservationKind::VM => Collections::VmIpReservation,
            IpReservationKind::Service => Collections::ServiceIpReservation,
        };

        let key = PartialKey::<IpReservation>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(collection);

        let reservations = self.store.list(&key)?;

        Ok(reservations)
    }

    pub fn ip_reservation_delete(
        &self,
        kind: IpReservationKind,
        ip: impl AsRef<str>,
    ) -> Result<()> {
        let collection = match kind {
            IpReservationKind::VM => Collections::VmIpReservation,
            IpReservationKind::Service => Collections::ServiceIpReservation,
        };

        let key = Key::<IpReservation>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(collection)
            .key(ip.as_ref().to_string());

        self.store.delete(&key)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    async fn create_test_agent(dir: &Path) -> NetAgent {
        let store = Store::new(dir).await.unwrap();

        let config = NetAgentConfig {
            bridge_name: "ltbr0".to_string(),
            vm_ip_cidr: "10.0.0.0/24".to_string(),
            service_ip_cidr: "10.0.1.0/24".to_string(),
        };

        let agent = NetAgent::new(config, Arc::new(store)).await.unwrap();

        agent
    }

    #[tokio::test]
    async fn test_ip_reservation_create_and_delete() {
        let store_dir = tempfile::tempdir().unwrap();
        let agent = create_test_agent(store_dir.path()).await;

        let ip = agent
            .ip_reservation_create(IpReservationKind::VM, None)
            .unwrap();

        assert_eq!(ip.kind, IpReservationKind::VM);
        assert!(ip.ip.starts_with("10.0.0."));
        assert_eq!(ip.tag, None);

        agent
            .ip_reservation_delete(IpReservationKind::VM, ip.ip)
            .unwrap();
    }

    #[tokio::test]
    async fn test_ip_reservation_list() {
        let store_dir = tempfile::tempdir().unwrap();
        let agent = create_test_agent(store_dir.path()).await;

        let ip = agent
            .ip_reservation_create(IpReservationKind::VM, None)
            .unwrap();
        let ip2 = agent
            .ip_reservation_create(IpReservationKind::VM, None)
            .unwrap();

        let ips = agent.ip_reservation_list(IpReservationKind::VM).unwrap();

        assert_eq!(ips.len(), 2);
        assert!(ips.iter().any(|i| i.ip == ip.ip));
        assert!(ips.iter().any(|i| i.ip == ip2.ip));
    }

    #[tokio::test]
    async fn test_device_create_and_delete() {
        let store_dir = tempfile::tempdir().unwrap();
        let agent = create_test_agent(store_dir.path()).await;

        if let Ok(device) = agent.device_create().await {
            let device_name = device.name.clone();
            device.delete().await.unwrap();

            assert!(
                device_name.starts_with(NET_DEVICE_PREFIX),
                "device name: {device_name}"
            );
        }
    }

    #[tokio::test]
    async fn test_device_list() {
        let store_dir = tempfile::tempdir().unwrap();
        let agent = create_test_agent(store_dir.path()).await;

        let device = agent.device_create().await.unwrap();
        let device_name = device.name.clone();

        let Some(device) = agent
            .device_list()
            .await
            .unwrap()
            .into_iter()
            .find(|d| d.name == device_name)
        else {
            device.delete().await.unwrap();
            panic!("device not found: {device_name}");
        };

        assert_eq!(device.name, device_name);
        device.delete().await.unwrap();
    }
}
