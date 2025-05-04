use nix::libc;
use rtnetlink::{Handle, new_connection, packet_route::link::LinkAttribute};
use util::{
    async_runtime::{spawn, task::JoinHandle},
    futures_util::TryStreamExt,
    rand::{self, Rng},
    result::{Result, bail},
};

const SIOCBRADDIF: libc::Ioctl = 0x89a2;

const DEV_NET_TUN: *const libc::c_char = b"/dev/net/tun\0".as_ptr() as *const libc::c_char;
const TAP_PREFIX: &str = "tap_lt_";

pub struct TapPoolConfig {
    pub bridge_name: String,
}

pub struct TapPool {
    bridge_name: String,
    nl_connection_task: JoinHandle<()>,
    nl_handle: Handle,
}

fn str_to_const_ifname(name: &str) -> [libc::c_char; libc::IFNAMSIZ] {
    let mut ifname = [0i8; libc::IFNAMSIZ];
    for (i, c) in name.as_bytes().iter().enumerate() {
        ifname[i] = *c as libc::c_char;
    }
    ifname
}

impl TapPool {
    pub async fn new(config: TapPoolConfig) -> Result<Self> {
        let (connection, handle, _) = new_connection()?;
        let nl_connection_task = spawn(connection);

        Ok(Self {
            bridge_name: config.bridge_name,
            nl_connection_task,
            nl_handle: handle,
        })
    }

    async fn generate_tap_name(&self) -> Result<String> {
        let mut tap_links = self.nl_handle.link().get().execute();

        let mut tap_names = Vec::new();
        while let Some(link) = tap_links.try_next().await? {
            let link_name = link.attributes.iter().find(|attr| match attr {
                LinkAttribute::IfName(_) => true,
                _ => false,
            });

            let Some(LinkAttribute::IfName(name)) = link_name else {
                continue;
            };

            tap_names.push(name.clone());
        }

        tap_names.sort();
        tap_names.dedup();

        let new_name = loop {
            let random_str = rand::rng()
                .sample_iter(&rand::distr::Alphanumeric)
                .take(6)
                .map(char::from)
                .collect::<String>();

            let new_name = format!("{}{}", TAP_PREFIX, random_str);

            if !tap_names.contains(&new_name) {
                break new_name;
            }
        };

        Ok(new_name)
    }

    pub async fn exists(&self, name: &str) -> Result<bool> {
        let mut link = self
            .nl_handle
            .link()
            .get()
            .match_name(name.to_string())
            .execute();

        match link.try_next().await {
            Ok(Some(_)) => Ok(true),
            _ => Ok(false),
        }
    }

    pub async fn list_taps(&self) -> Result<Vec<String>> {
        let mut link = self.nl_handle.link().get().execute();

        let mut taps = Vec::new();
        while let Some(link) = link.try_next().await? {
            let link_name = link.attributes.iter().find(|attr| match attr {
                LinkAttribute::IfName(name) => name.starts_with(TAP_PREFIX),
                _ => false,
            });

            let Some(LinkAttribute::IfName(name)) = link_name else {
                continue;
            };

            taps.push(name.clone());
        }

        Ok(taps)
    }

    pub async fn create_tap(&self) -> Result<String> {
        let mut bridge_link = self
            .nl_handle
            .link()
            .get()
            .match_name(self.bridge_name.clone())
            .execute();

        let Some(_) = bridge_link.try_next().await? else {
            bail!("bridge {} not found", self.bridge_name);
        };

        let new_tap_name = self.generate_tap_name().await?;

        let mut req = libc::ifreq {
            ifr_name: str_to_const_ifname(&new_tap_name),
            ifr_ifru: libc::__c_anonymous_ifr_ifru {
                ifru_flags: (libc::IFF_TAP | libc::IFF_NO_PI) as i16,
            },
        };

        let fd = unsafe { libc::open(DEV_NET_TUN, libc::O_RDWR | libc::O_CLOEXEC) };
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

        let mut link = self
            .nl_handle
            .link()
            .get()
            .match_name(new_tap_name.clone())
            .execute();

        let Some(link) = link.try_next().await? else {
            unsafe { libc::close(ctrl_fd) };
            bail!("link not found after creation");
        };

        let mut req: libc::ifreq = unsafe { std::mem::zeroed() };
        req.ifr_name = str_to_const_ifname(&self.bridge_name);
        req.ifr_ifru.ifru_ifindex = link.header.index as i32;

        if unsafe { libc::ioctl(ctrl_fd, SIOCBRADDIF, std::ptr::addr_of_mut!(req)) } != 0 {
            unsafe { libc::close(ctrl_fd) };
            bail!("failed to set master: {}", std::io::Error::last_os_error());
        }

        unsafe { libc::close(ctrl_fd) };

        Ok(new_tap_name)
    }

    // consumes the tap
    pub async fn delete_tap(&self, name: &str) -> Result<()> {
        let mut link = self
            .nl_handle
            .link()
            .get()
            .match_name(name.to_string())
            .execute();

        let Some(link) = link.try_next().await? else {
            bail!("link not found");
        };

        self.nl_handle
            .link()
            .del(link.header.index)
            .execute()
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use util::async_runtime;

    use super::*;

    #[test]
    fn test_create_tap() {
        let rt = async_runtime::runtime::Runtime::new().unwrap();

        rt.block_on(async {
            let pool = TapPool::new(TapPoolConfig {
                bridge_name: "ltbr0".to_string(),
            })
            .await
            .unwrap();

            let taps = pool.list_taps().await.unwrap();
            assert!(taps.is_empty());

            let tap = pool.create_tap().await.unwrap();

            let taps = pool.list_taps().await.unwrap();
            assert!(taps.contains(&tap));

            assert!(pool.exists(&tap).await.unwrap());

            pool.delete_tap(&tap).await.unwrap();

            let taps = pool.list_taps().await.unwrap();
            assert!(!taps.contains(&tap));

            assert!(!pool.exists(&tap).await.unwrap());
        });
    }
}
