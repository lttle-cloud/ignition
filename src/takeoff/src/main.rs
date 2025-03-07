mod serial;

use std::{
    collections::HashMap,
    ffi::c_void,
    net::{IpAddr, Ipv4Addr},
    os::fd::FromRawFd,
    path::PathBuf,
    sync::Arc,
    time,
};

use mime_guess::Mime;
use tracing::info;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
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
    async_runtime::{self, fs, sync::Mutex},
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

#[derive(Clone)]
struct AssetCacheEntry {
    path: String,
    mime_type: Mime,
    content: Vec<u8>,
}

struct AssetCache {
    entries: Mutex<HashMap<String, AssetCacheEntry>>,
}

impl AssetCache {
    pub fn new() -> AssetCache {
        AssetCache {
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get(&self, path: &str) -> Option<AssetCacheEntry> {
        let mut cache = self.entries.lock().await;

        let existing = cache.get(path);
        if let Some(entry) = existing {
            return Some(entry.clone());
        }

        let Ok(content) = fs::read(format!("/mnt/{}", path)).await else {
            return None;
        };

        let mime_type = mime_guess::from_path(&path).first_or_octet_stream();
        let entry = AssetCacheEntry {
            path: path.to_string(),
            mime_type,
            content,
        };

        cache.insert(path.to_string(), entry.clone());

        Some(entry)
    }
}

struct AppState {
    guest_manager: Arc<GuestManager>,
    asset_cache: Arc<AssetCache>,
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

async fn handle_get(State(app_state): State<Arc<AppState>>) -> String {
    let Ok(page) = fs::read_to_string("/mnt/index.html").await else {
        return "<h1>failed to load index.html</h1>".to_string();
    };

    let us = app_state.guest_manager.get_boot_ready_time_us();
    let ms_int = us / 1000;
    let ms_frac = us % 1000;

    let ms = format!("{ms_int}<span class=\"frac\">.{ms_frac}</span>ms");

    page.replace("{ms}", &ms)
}

async fn handle_static_assets(
    Path(path): Path<String>,
    State(app_state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Some(asset) = app_state.asset_cache.get(&path).await else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    (
        StatusCode::OK,
        [("Content-Type", asset.mime_type.to_string().as_str())],
        asset.content,
    )
        .into_response()
}

async fn start_server(app_state: Arc<AppState>) {
    let app = Router::new()
        .route("/", get(handle_get))
        .route("/{*path}", get(handle_static_assets))
        .with_state(app_state);

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
    check_internet();

    let asset_cache = Arc::new(AssetCache::new());

    let app_state = AppState {
        guest_manager,
        asset_cache,
    };

    start_server(Arc::new(app_state)).await;

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
