use std::net::Ipv4Addr;

use sds::{Collection, Store};
use util::{
    async_runtime::sync::Mutex,
    encoding::codec,
    rand::{self, Rng},
    result::{Context, Result, bail},
};

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
}

#[codec]
#[derive(Debug, Clone)]
struct ReservedIp {
    ip: String,
}

pub struct IpPoolConfig {
    pub name: String,
    pub cidr: String, // e.g. "10.0.0.0/16"
}

pub struct IpPool {
    range: IpPoolRange,
    name: String,
    store: Store,
    reserved_ips_collection: Collection<ReservedIp>,
    reserved_ips: Mutex<Vec<ReservedIp>>,
}

impl IpPool {
    pub async fn new(config: IpPoolConfig, store: Store) -> Result<Self> {
        let collection =
            store.collection::<ReservedIp>(&format!("reserved_ips_{}", config.name))?;

        let pool = Self {
            range: IpPoolRange::from_cidr(&config.cidr)?,
            name: config.name,
            store,
            reserved_ips_collection: collection,
            reserved_ips: Mutex::new(vec![]),
        };

        pool.load_reserved_ips().await?;

        Ok(pool)
    }

    async fn load_reserved_ips(&self) -> Result<()> {
        let tx = self.store.read_txn()?;

        let stored_reserved_ips = tx
            .iter(&self.reserved_ips_collection)?
            .collect::<Result<Vec<_>, sds::Error>>()
            .context("failed to collect reserved ips")?
            .into_iter()
            .map(|(_, ip)| ip)
            .collect::<Vec<_>>();

        {
            let mut reserved_ips = self.reserved_ips.lock().await;
            *reserved_ips = stored_reserved_ips;
        }

        Ok(())
    }

    pub async fn reserve(&self) -> Result<Ipv4Addr> {
        let mut reserved_ips = self.reserved_ips.lock().await;

        let mut ip = self.range.random();
        while reserved_ips
            .iter()
            .any(|reserved_ip| reserved_ip.ip == ip.to_string())
        {
            ip = self.range.random();
        }

        let reserved_ip = ReservedIp { ip: ip.to_string() };

        let mut tx = self.store.write_txn()?;
        tx.put(&self.reserved_ips_collection, &reserved_ip.ip, &reserved_ip)
            .context("failed to reserve ip")?;
        tx.commit()?;

        reserved_ips.push(reserved_ip);

        Ok(ip)
    }

    pub async fn release(&self, ip: Ipv4Addr) -> Result<()> {
        let mut reserved_ips = self.reserved_ips.lock().await;

        let ip = ip.to_string();
        let mut tx = self.store.write_txn()?;
        tx.del(&self.reserved_ips_collection, &ip)
            .context("failed to release ip")?;
        tx.commit()?;

        reserved_ips.retain(|reserved_ip| reserved_ip.ip != ip);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use util::async_runtime;

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
    fn test_reserve() {
        let rt = async_runtime::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            let store = Store::new(sds::StoreConfig {
                dir_path: "reserve_ips_test".into(),
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
            .await
            .unwrap();

            let ip = pool.reserve().await.unwrap();
            assert!(ip.is_private());
            {
                let reserved_ips = pool.reserved_ips.lock().await;
                assert!(reserved_ips.iter().any(|ip| ip.ip == ip.ip.to_string()));
            }

            let read_tx = store.read_txn().unwrap();
            let reserved_ips = read_tx.iter(&pool.reserved_ips_collection).unwrap();
            assert_eq!(reserved_ips.count(), 1);
            drop(read_tx);

            pool.release(ip).await.unwrap();
            {
                let reserved_ips = pool.reserved_ips.lock().await;
                assert_eq!(reserved_ips.len(), 0);
            }

            let read_tx = store.read_txn().unwrap();
            let reserved_ips = read_tx.iter(&pool.reserved_ips_collection).unwrap();
            assert_eq!(reserved_ips.count(), 0);
            drop(read_tx);

            // delete the test_store dir
            std::fs::remove_dir_all("test_store").unwrap();
        });
    }
}
