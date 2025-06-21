use std::net::Ipv4Addr;

use anyhow::{Context, Result, bail};
use rand::Rng;

#[derive(Clone)]
pub struct IpRange {
    pub cidr: String,
    pub net: u32,
    pub mask: u32,
}

impl IpRange {
    pub fn from_cidr(cidr: &str) -> Result<Self> {
        let cidr = cidr.to_string();

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

        Ok(IpRange { cidr, net, mask })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cidr() {
        let cidr = "10.0.0.0/16";
        let range = IpRange::from_cidr(cidr).unwrap();
        assert_eq!(range.net, 0x0a000000);
        assert_eq!(range.mask, 0xffff0000);
    }

    #[test]
    fn test_random() {
        let cidr = "10.0.0.0/16";
        let range = IpRange::from_cidr(cidr).unwrap();
        let ip = range.random();
        assert!(ip.is_private());
    }

    #[test]
    fn test_gateway() {
        let cidr = "10.0.0.0/16";
        let range = IpRange::from_cidr(cidr).unwrap();
        let gateway = range.gateway();
        assert_eq!(gateway, Ipv4Addr::new(10, 0, 0, 1));
    }

    #[test]
    fn test_netmask() {
        let cidr = "10.0.0.0/16";
        let range = IpRange::from_cidr(cidr).unwrap();
        let netmask = range.netmask();
        assert_eq!(netmask, Ipv4Addr::new(255, 255, 0, 0));
    }
}
