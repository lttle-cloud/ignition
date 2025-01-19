mod print;

use std::{
    ffi::c_void,
    fs,
    net::{IpAddr, Ipv4Addr},
    os::fd::FromRawFd,
    time,
};

use axum::{routing::get, Router};
use nix::{
    fcntl::{open, OFlag},
    sys::{
        mman::{mmap, MapFlags, ProtFlags},
        stat::Mode,
    },
};
use print::init_print;
use util::{async_runtime, result::Result};

const PAGE_SIZE: usize = 4096;
const MAGIC_MMIO_ADDR: i64 = 0xd0000000;

struct GuestManager {
    map_base: ::core::ptr::NonNull<c_void>,
}

impl GuestManager {
    pub fn new() -> Result<GuestManager> {
        let fd = open(
            "/dev/mem",
            OFlag::O_RDWR | OFlag::O_SYNC | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;

        let fd = unsafe { fs::File::from_raw_fd(fd) };

        let map_base = unsafe {
            mmap(
                None,
                PAGE_SIZE.try_into()?,
                ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED,
                fd,
                MAGIC_MMIO_ADDR,
            )?
        };

        Ok(GuestManager { map_base })
    }

    #[allow(unused)]
    pub fn trigger_snapshot(&self) {
        unsafe {
            let ptr: *mut u64 = self.map_base.as_ptr() as *mut u64;
            ptr.write_volatile(0x00_00_00_00_00_00_00_01);
        }
    }

    pub fn mark_boot_ready(&self) {
        unsafe {
            let ptr = self.map_base.as_ptr() as *mut u64;
            ptr.write_volatile(0x00_00_00_00_00_00_00_0a);
        }
    }
}

fn check_internet() {
    let Err(res) = ping::ping(
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
        // IpAddr::V4(Ipv4Addr::new(192, 168, 1, 16)),
        Some(time::Duration::from_secs(10)),
        None,
        None,
        None,
        None,
    ) else {
        rprintln!("internet check ok");
        return;
    };

    rprintln!("internet check failed: {:?}", res);
}

async fn start_server() {
    let app = Router::new().route("/", get(|| async { "Hello, World!" }));
    let listener = async_runtime::net::TcpListener::bind("0.0.0.0:3000")
        .await // the guest will be suspended here
        .unwrap();

    rprintln!("listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.unwrap();
}

async fn takeoff() {
    init_print();

    let guest_manager = GuestManager::new().unwrap();
    guest_manager.mark_boot_ready();

    rprintln!("takeoff");

    check_internet();

    start_server().await;

    // guest_manager.trigger_snapshot(); // here the guest is suspended. will comtinue from here.

    // guest_manager.mark_boot_ready();
    // rprintln!("takeoff is back");
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(takeoff());

    Ok(())
}
