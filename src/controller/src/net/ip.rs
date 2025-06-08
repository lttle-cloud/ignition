use std::net::Ipv4Addr;

use sds::{Collection, Store};
use util::{
    encoding::codec,
    rand::{self, Rng},
    result::{Context, Result, bail},
};

#[derive(Clone)]
struct IpPoolRange {
    pub net: u32,
    pub mask: u32,
}

impl IpPoolRange {
    pub fn from_cidr(cidr: &str) -> Result<Self> {
        let parts = cidr.split('/').collect::<Vec<&str>>();
        if parts.len() != 2 {
            bail!("Invalid CIDR: {}", cidr);
        }

        let net_parts = parts[0].split('.').collect::<Vec<&str>>();
        if net_parts.len() != 4 {
            bail!("Invalid CIDR: {}", cidr);
        }

        let mut net = 0u32;
        for part in net_parts {
            if part.len() > 3 {
                bail!("Invalid CIDR: {}", cidr);
            }

            let part = part
                .parse::<u8>()
                .context(format!("Invalid CIDR: {}", cidr))?;
            net = (net << 8) | part as u32;
        }

        let mask = parts[1]
            .parse::<u32>()
            .context(format!("Invalid CIDR: {}", cidr))?;
        if mask > 32 {
            bail!("Invalid CIDR: {}", cidr);
        }

        let mask = 0xffffffff << (32 - mask);

        Ok(IpPoolRange { net, mask })
    }

    pub fn random(&self) -> Ipv4Addr {
        let mut rng = rand::rng();
        let mut ip = self.net;
        ip = (ip & self.mask) | (rng.random_range(0..=u32::MAX) & !self.mask);
        Ipv4Addr::new(
            ((ip >> 24) & 0xff) as u8,
            ((ip >> 16) & 0xff) as u8,
            ((ip >> 8) & 0xff) as u8,
            (ip & 0xff) as u8,
        )
    }

    pub fn gateway(&self) -> Ipv4Addr {
        Ipv4Addr::new(
            ((self.net >> 24) & 0xff) as u8,
            ((self.net >> 16) & 0xff) as u8,
            ((self.net >> 8) & 0xff) as u8,
            ((self.net & 0xff) + 1) as u8,
        )
    }

    pub fn netmask(&self) -> Ipv4Addr {
        Ipv4Addr::new(
            ((self.mask >> 24) & 0xff) as u8,
            ((self.mask >> 16) & 0xff) as u8,
            ((self.mask >> 8) & 0xff) as u8,
            (self.mask & 0xff) as u8,
        )
    }
}

#[codec]
#[derive(Debug, Clone)]
pub struct ReservedIp {
    pub addr: String,
    pub tag: Option<String>,
}

pub struct IpPoolConfig {
    pub name: String,
    pub cidr: String, // e.g. "10.0.0.0/16"
}

#[derive(Clone)]
pub struct IpPool {
    range: IpPoolRange,
    store: Store,
    reserved_ips_collection: Collection<ReservedIp>,
}

impl IpPool {
    pub fn new(config: IpPoolConfig, store: Store) -> Result<Self> {
        let collection =
            store.collection::<ReservedIp>(&format!("ip_pool:{}:reserved_ips", config.name))?;

        let pool = Self {
            range: IpPoolRange::from_cidr(&config.cidr)?,
            store,
            reserved_ips_collection: collection,
        };

        Ok(pool)
    }

    pub fn gateway(&self) -> Ipv4Addr {
        self.range.gateway()
    }

    pub fn netmask(&self) -> Ipv4Addr {
        self.range.netmask()
    }

    fn get_reserved_ips(&self) -> Result<Vec<ReservedIp>> {
        let tx = self.store.read_txn()?;
        let reserved_ips = tx.get_all_values(&self.reserved_ips_collection)?;
        Ok(reserved_ips)
    }

    pub fn reserve_tagged(&self, tag: impl AsRef<str>) -> Result<ReservedIp> {
        self.reserve(Some(tag.as_ref().to_string()))
    }

    pub fn reserve_untagged(&self) -> Result<ReservedIp> {
        self.reserve(None)
    }

    fn reserve(&self, tag: Option<String>) -> Result<ReservedIp> {
        let mut tx = self.store.write_txn()?;

        let reserved_ips = tx.get_all_values(&self.reserved_ips_collection)?;

        let mut ip = self.range.random();
        while reserved_ips
            .iter()
            .any(|reserved_ip| reserved_ip.addr == ip.to_string())
        {
            ip = self.range.random();
        }

        let reserved_ip = ReservedIp {
            addr: ip.to_string(),
            tag,
        };

        tx.put(
            &self.reserved_ips_collection,
            &reserved_ip.addr,
            &reserved_ip,
        )
        .context("failed to reserve ip")?;

        tx.commit()?;

        Ok(reserved_ip)
    }

    pub fn get_by_tag(&self, tag: impl AsRef<str>) -> Option<ReservedIp> {
        let Some(reserved_ip) = self.get_reserved_ips().ok().and_then(|ips| {
            ips.iter()
                .find(|reserved_ip| reserved_ip.tag == Some(tag.as_ref().to_string()))
                .cloned()
        }) else {
            return None;
        };

        Some(reserved_ip)
    }

    pub fn release_tag(&self, tag: impl AsRef<str>) -> Result<()> {
        let Some(reserved_ip) = self.get_by_tag(&tag) else {
            bail!("ip with tag {} not found", tag.as_ref());
        };

        let mut tx = self.store.write_txn()?;
        tx.del(&self.reserved_ips_collection, &reserved_ip.addr)
            .context("failed to release ip")?;
        tx.commit()?;

        Ok(())
    }

    pub fn release(&self, addr: impl AsRef<str>) -> Result<()> {
        let ip: String = addr.as_ref().to_string();
        let mut tx = self.store.write_txn()?;
        tx.del(&self.reserved_ips_collection, &ip)
            .context("failed to release ip")?;
        tx.commit()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cidr() {
        let cidr = "10.0.0.0/16";
        let range = IpPoolRange::from_cidr(cidr).unwrap();
        assert_eq!(range.net, 0x0a000000);
        assert_eq!(range.mask, 0xffff0000);
    }

    #[test]
    fn test_random() {
        let cidr = "10.0.0.0/16";
        let range = IpPoolRange::from_cidr(cidr).unwrap();
        let ip = range.random();
        assert!(ip.is_private());
    }

    #[test]
    fn test_reserve_untagged() {
        let db_dir = tempfile::tempdir().unwrap();
        let store = Store::new(sds::StoreConfig {
            dir_path: db_dir.path().to_path_buf(),
            size_mib: 1024,
        })
        .unwrap();

        let pool = IpPool::new(
            IpPoolConfig {
                name: "test".to_string(),
                cidr: "10.0.0.0/16".to_string(),
            },
            store.clone(),
        )
        .unwrap();

        let ip = pool.reserve_untagged().unwrap();
        let ip_addr: Ipv4Addr = ip.addr.parse().unwrap();
        assert!(ip_addr.is_private());
        {
            let reserved_ips = pool.get_reserved_ips().unwrap();
            assert!(reserved_ips.iter().any(|ip| ip.addr == ip.addr.to_string()));
        }

        pool.release(&ip.addr).unwrap();
        {
            let reserved_ips = pool.get_reserved_ips().unwrap();
            assert_eq!(reserved_ips.len(), 0);
        }
    }

    #[test]
    fn test_reserve_tagged() {
        let db_dir = tempfile::tempdir().unwrap();
        let store = Store::new(sds::StoreConfig {
            dir_path: db_dir.path().to_path_buf(),
            size_mib: 1024,
        })
        .unwrap();

        let pool = IpPool::new(
            IpPoolConfig {
                name: "test".to_string(),
                cidr: "10.0.0.0/16".to_string(),
            },
            store.clone(),
        )
        .unwrap();

        let ip = pool.reserve_tagged("test").unwrap();
        let ip_addr: Ipv4Addr = ip.addr.parse().unwrap();
        assert!(ip_addr.is_private());
        assert_eq!(ip.tag, Some("test".to_string()));

        pool.release_tag("test").unwrap();
        {
            let reserved_ips = pool.get_reserved_ips().unwrap();
            assert_eq!(reserved_ips.len(), 0);
        }

        // delete the test_store dir
        std::fs::remove_dir_all("/tmp/test-dbs/reserve_tagged_ips_test").unwrap();
    }
}
