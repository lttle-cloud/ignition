use std::path::PathBuf;

use nix::mount::{self, MsFlags};
use tokio::fs;
use tracing::warn;

pub async fn mount(device: &str, mount_point: &str, fs_type: Option<&str>) {
    // make sure mount point exists
    fs::create_dir_all(mount_point)
        .await
        .expect("create mount point");

    if let Err(e) = mount::mount(
        Some(&PathBuf::from(device)),
        mount_point,
        fs_type,
        MsFlags::empty(),
        Some(&PathBuf::from("")),
    ) {
        warn!("mount {} failed: {:?}", mount_point, e);
    }
}
