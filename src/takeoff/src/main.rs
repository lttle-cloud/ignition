mod serial;

use std::{
    ffi::c_void,
    net::{IpAddr, Ipv4Addr},
    os::fd::FromRawFd,
    path::PathBuf,
    sync::Arc,
    time,
};

use tracing::info;

use axum::{extract::State, routing::get, Router};
use nix::{
    fcntl::{open, OFlag},
    mount::{self, MsFlags},
    sys::{
        mman::{mmap, MapFlags, ProtFlags},
        stat::Mode,
    },
};
use serial::SerialWriter;
use util::{
    async_runtime::{self, fs},
    result::Result,
};

const PAGE_SIZE: usize = 4096;
const MAGIC_MMIO_ADDR: i64 = 0xd0000000;

struct GuestManager {
    map_base: ::core::ptr::NonNull<c_void>,
}
unsafe impl Send for GuestManager {}
unsafe impl Sync for GuestManager {}

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

    pub fn get_boot_ready_time_us(&self) -> u64 {
        unsafe {
            let ptr = self.map_base.as_ptr() as *mut u64;
            let time_us = ptr.read_volatile();

            time_us
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
        info!("internet check ok");
        return;
    };

    info!("internet check failed: {:?}", res);
}

fn mount_block() {
    if let Err(e) = mount::mount(
        Some(&PathBuf::from("devtmpfs")),
        "/dev",
        Some("devtmpfs"),
        MsFlags::empty(),
        Some(&PathBuf::from("")),
    ) {
        info!("mount /dev failed: {:?}", e);
    };

    if let Err(e) = mount::mount(
        Some(&PathBuf::from("/dev/vda")),
        "/mnt",
        Some("ext4"),
        MsFlags::empty(),
        Some(&PathBuf::from("")),
    ) {
        info!("mount /mnt failed: {:?}", e);
    };
}

async fn handle_get(State(guest_manager): State<Arc<GuestManager>>) -> String {
    let Ok(page) = fs::read_to_string("/mnt/index.html").await else {
        return "<h1>failed to load index.html</h1>".to_string();
    };

    let us = guest_manager.get_boot_ready_time_us();
    let ms_int = us / 1000;
    let ms_frac = us % 1000;

    let ms = format!("{ms_int}<span class=\"frac\">.{ms_frac}</span>ms");

    page.replace("{ms}", &ms)
}

async fn start_server(guest_manager: Arc<GuestManager>) {
    let app = Router::new()
        .route("/", get(handle_get))
        .with_state(guest_manager);

    let listener = async_runtime::net::TcpListener::bind("0.0.0.0:3000")
        .await // the guest will be suspended here
        .unwrap();

    info!("listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app).await.unwrap();
}

async fn takeoff() {
    let guest_manager = Arc::new(GuestManager::new().unwrap());
    guest_manager.mark_boot_ready();

    info!("takeoff is ready");

    mount_block();
    // check_internet();

    start_server(guest_manager.clone()).await;

    // guest_manager.trigger_snapshot(); // here the guest is suspended. will comtinue from here.

    // guest_manager.mark_boot_ready();
    // info!("takeoff is back");
}

fn main() -> Result<()> {
    SerialWriter::initialize_serial();

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .with_writer(SerialWriter)
        .init();

    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(takeoff());

    Ok(())
}
