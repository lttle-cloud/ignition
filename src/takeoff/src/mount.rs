use std::path::PathBuf;

use nix::mount::{self, MsFlags};
use tokio::fs;
use tracing::warn;

pub async fn mount(device: &str, mount_point: &str, fs_type: Option<&str>) {
    mount_with_options(device, mount_point, fs_type, MsFlags::empty(), None).await;
}

pub async fn mount_with_options(
    device: &str,
    mount_point: &str,
    fs_type: Option<&str>,
    flags: MsFlags,
    options: Option<&str>,
) {
    // make sure mount point exists
    fs::create_dir_all(mount_point)
        .await
        .expect("create mount point");

    let empty_path = PathBuf::from("");
    let opts_path = options.map(PathBuf::from);
    let data = if let Some(ref opts) = opts_path {
        Some(opts)
    } else {
        Some(&empty_path)
    };

    let device_path = PathBuf::from(device);
    if let Err(e) = mount::mount(Some(&device_path), mount_point, fs_type, flags, data) {
        warn!("mount {} failed: {:?}", mount_point, e);
    }
}
