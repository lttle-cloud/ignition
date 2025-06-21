pub mod ip_range;

use std::sync::Arc;

use anyhow::{Result, bail};
use futures_util::TryStreamExt;
use nix::libc;
use rtnetlink::{new_connection, packet_route::link::LinkAttribute};
use serde::{Deserialize, Serialize};
use tokio::spawn;

use crate::{
    agent::{
        data::{Collections, DEFAULT_AGENT_TENANT},
        net::ip_range::IpRange,
    },
    machinery::store::{Key, PartialKey, Store},
    utils::id::short_id,
};

const NET_DEVICE_PREFIX: &str = "tap_lt_";
const SIOCBRADDIF: libc::Ioctl = 0x89a2;

fn str_to_const_ifname(name: &str) -> [libc::c_char; libc::IFNAMSIZ] {
    let mut ifname = [0i8; libc::IFNAMSIZ];
    for (i, c) in name.as_bytes().iter().enumerate() {
        ifname[i] = *c as libc::c_char;
    }
    ifname
}

async fn nl_device_exists(name: &str) -> Result<bool> {
    let (connection, handle, _) = new_connection()?;
    spawn(connection);

    let mut link = handle.link().get().match_name(name.to_string()).execute();

    match link.try_next().await {
        Ok(Some(_)) => Ok(true),
        _ => Ok(false),
    }
}

async fn nl_device_list_with_prefix(prefix: &str) -> Result<Vec<String>> {
    let (connection, handle, _) = new_connection()?;
    spawn(connection);

    let mut link = handle.link().get().execute();

    let mut taps = Vec::new();
    while let Some(link) = link.try_next().await? {
        let link_name = link.attributes.iter().find(|attr| match attr {
            LinkAttribute::IfName(name) => name.starts_with(prefix),
            _ => false,
        });

        let Some(LinkAttribute::IfName(name)) = link_name else {
            continue;
        };

        taps.push(name.clone());
    }

    Ok(taps)
}

async fn nl_device_delete(name: &str) -> Result<()> {
    let (connection, handle, _) = new_connection()?;
    spawn(connection);

    let mut link = handle.link().get().match_name(name.to_string()).execute();

    let Some(link) = link.try_next().await? else {
        bail!("device {name} not found");
    };

    handle.link().del(link.header.index).execute().await?;

    Ok(())
}

async fn nl_device_index(name: &str) -> Result<u32> {
    let (connection, handle, _) = new_connection()?;
    spawn(connection);

    let mut link = handle.link().get().match_name(name.to_string()).execute();

    let Some(link) = link.try_next().await? else {
        bail!("device {name} not found");
    };

    Ok(link.header.index)
}

async fn device_create(name: &str, bridge_name: &str) -> Result<()> {
    let mut req = libc::ifreq {
        ifr_name: str_to_const_ifname(name),
        ifr_ifru: libc::__c_anonymous_ifr_ifru {
            ifru_flags: (libc::IFF_TAP | libc::IFF_NO_PI) as i16,
        },
    };

    let fd = unsafe {
        libc::open(
            b"/dev/net/tun\0".as_ptr() as *const libc::c_char,
            libc::O_RDWR | libc::O_CLOEXEC,
        )
    };
    if fd == -1 {
        bail!(
            "failed to open /dev/net/tun: {}",
            std::io::Error::last_os_error()
        );
    }

    if unsafe { libc::ioctl(fd, libc::TUNSETIFF, std::ptr::addr_of_mut!(req)) } != 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        bail!("failed to set interface name: {}", err);
    };

    if unsafe { libc::ioctl(fd, libc::TUNSETPERSIST, 1) } != 0 {
        bail!("failed to set persist: {}", std::io::Error::last_os_error());
    }

    let mut req = libc::ifreq {
        ifr_name: [0; 16],
        ifr_ifru: libc::__c_anonymous_ifr_ifru { ifru_flags: 0 },
    };

    if unsafe { libc::ioctl(fd, libc::TUNGETIFF, std::ptr::addr_of_mut!(req)) } != 0 {
        bail!(
            "failed to get interface name: {}",
            std::io::Error::last_os_error()
        );
    }

    unsafe { libc::close(fd) };

    let ctrl_fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if ctrl_fd < 0 {
        bail!(
            "failed to create socket: {}",
            std::io::Error::last_os_error()
        );
    };

    unsafe { req.ifr_ifru.ifru_flags |= libc::IFF_UP as i16 };

    if unsafe { libc::ioctl(ctrl_fd, libc::SIOCSIFFLAGS, std::ptr::addr_of_mut!(req)) } != 0 {
        unsafe { libc::close(ctrl_fd) };
        bail!(
            "failed to set interface up: {}",
            std::io::Error::last_os_error()
        );
    }

    let Ok(index) = nl_device_index(name).await else {
        bail!("failed to get device index after creation {name}");
    };

    let mut req: libc::ifreq = unsafe { std::mem::zeroed() };
    req.ifr_name = str_to_const_ifname(bridge_name);
    req.ifr_ifru.ifru_ifindex = index as i32;

    if unsafe { libc::ioctl(ctrl_fd, SIOCBRADDIF, std::ptr::addr_of_mut!(req)) } != 0 {
        unsafe { libc::close(ctrl_fd) };
        bail!("failed to set master: {}", std::io::Error::last_os_error());
    }

    unsafe { libc::close(ctrl_fd) };

    Ok(())
}

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

    pub async fn ip_reservation_create(
        &self,
        kind: IpReservationKind,
        tag: Option<String>,
    ) -> Result<IpReservation> {
        let ip = match kind {
            IpReservationKind::VM => self.vm_ip_range.random(),
            IpReservationKind::Service => self.service_ip_range.random(),
        };

        let collection = match kind {
            IpReservationKind::VM => Collections::VmIpReservation,
            IpReservationKind::Service => Collections::ServiceIpReservation,
        };

        let key = Key::<IpReservation>::not_namespaced()
            .collection(collection)
            .tenant(DEFAULT_AGENT_TENANT)
            .key(ip.to_string());

        let reservation = IpReservation {
            kind,
            ip: ip.to_string(),
            tag,
        };

        self.store.put(&key, &reservation).await?;

        Ok(reservation)
    }

    pub async fn ip_reservation_list(&self, kind: IpReservationKind) -> Result<Vec<IpReservation>> {
        let collection = match kind {
            IpReservationKind::VM => Collections::VmIpReservation,
            IpReservationKind::Service => Collections::ServiceIpReservation,
        };

        let key = PartialKey::<IpReservation>::not_namespaced()
            .collection(collection)
            .tenant(DEFAULT_AGENT_TENANT);

        let reservations = self.store.list(&key).await?;

        Ok(reservations)
    }

    pub async fn ip_reservation_delete(
        &self,
        kind: IpReservationKind,
        ip: impl AsRef<str>,
    ) -> Result<()> {
        let collection = match kind {
            IpReservationKind::VM => Collections::VmIpReservation,
            IpReservationKind::Service => Collections::ServiceIpReservation,
        };

        let key = Key::<IpReservation>::not_namespaced()
            .collection(collection)
            .tenant(DEFAULT_AGENT_TENANT)
            .key(ip.as_ref().to_string());

        self.store.delete(&key).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::machinery::store;

    use super::*;

    async fn create_test_agent(dir: &Path) -> NetAgent {
        println!("dir: {:?}", dir);
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
            .await
            .unwrap();

        assert_eq!(ip.kind, IpReservationKind::VM);
        assert!(ip.ip.starts_with("10.0.0."));
        assert_eq!(ip.tag, None);

        agent
            .ip_reservation_delete(IpReservationKind::VM, ip.ip)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_ip_reservation_list() {
        let store_dir = tempfile::tempdir().unwrap();
        let agent = create_test_agent(store_dir.path()).await;

        let ip = agent
            .ip_reservation_create(IpReservationKind::VM, None)
            .await
            .unwrap();
        let ip2 = agent
            .ip_reservation_create(IpReservationKind::VM, None)
            .await
            .unwrap();

        let ips = agent
            .ip_reservation_list(IpReservationKind::VM)
            .await
            .unwrap();

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
