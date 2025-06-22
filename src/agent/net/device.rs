use anyhow::{Result, bail};
use futures_util::TryStreamExt;
use nix::libc;
use rtnetlink::{new_connection, packet_route::link::LinkAttribute};
use tokio::spawn;

const SIOCBRADDIF: libc::Ioctl = 0x89a2;

fn str_to_const_ifname(name: &str) -> [libc::c_char; libc::IFNAMSIZ] {
    let mut ifname = [0i8; libc::IFNAMSIZ];
    for (i, c) in name.as_bytes().iter().enumerate() {
        ifname[i] = *c as libc::c_char;
    }
    ifname
}

pub async fn nl_device_exists(name: &str) -> Result<bool> {
    let (connection, handle, _) = new_connection()?;
    spawn(connection);

    let mut link = handle.link().get().match_name(name.to_string()).execute();

    match link.try_next().await {
        Ok(Some(_)) => Ok(true),
        _ => Ok(false),
    }
}

pub async fn nl_device_list_with_prefix(prefix: &str) -> Result<Vec<String>> {
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

pub async fn nl_device_delete(name: &str) -> Result<()> {
    let (connection, handle, _) = new_connection()?;
    spawn(connection);

    let mut link = handle.link().get().match_name(name.to_string()).execute();

    let Some(link) = link.try_next().await? else {
        bail!("device {name} not found");
    };

    handle.link().del(link.header.index).execute().await?;

    Ok(())
}

pub async fn nl_device_index(name: &str) -> Result<u32> {
    let (connection, handle, _) = new_connection()?;
    spawn(connection);

    let mut link = handle.link().get().match_name(name.to_string()).execute();

    let Some(link) = link.try_next().await? else {
        bail!("device {name} not found");
    };

    Ok(link.header.index)
}

pub async fn device_create(name: &str, bridge_name: &str) -> Result<()> {
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
