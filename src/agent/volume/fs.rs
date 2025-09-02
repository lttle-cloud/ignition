use std::path::Path;

use anyhow::{Result, bail};
use caps::{CapSet, Capability};
use tokio::{fs::OpenOptions, process::Command};

pub fn dir_size_in_bytes_recursive(dir_path: impl AsRef<Path>) -> Result<u64> {
    let dir_path = dir_path.as_ref();
    let mut size = 0;
    for entry in std::fs::read_dir(dir_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            size += std::fs::metadata(path)?.len();
        } else if path.is_dir() {
            size += dir_size_in_bytes_recursive(&path)?;
        }
    }
    Ok(size)
}

pub async fn create_sparse_file(path: impl AsRef<Path>, size: u64) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .await?;

    file.set_len(size).await?;
    Ok(())
}

pub async fn format_file_as_ext4_volume_empty(file: impl AsRef<Path>) -> Result<()> {
    let file_path = file.as_ref();

    let output = Command::new("mkfs.ext4")
        .arg("-F")
        .arg(file_path)
        .output()
        .await?;

    if !output.status.success() {
        bail!(
            "failed to format volume as ext4: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

pub async fn format_file_as_ext4_volume_from_dir(
    file: impl AsRef<Path>,
    source_dir: impl AsRef<Path>,
) -> Result<()> {
    let file_path = file.as_ref();
    let source_dir_path = source_dir.as_ref();

    let needed_caps = [
        Capability::CAP_DAC_READ_SEARCH,
        Capability::CAP_DAC_OVERRIDE,
    ];

    for cap in &needed_caps {
        caps::raise(None, CapSet::Inheritable, *cap)?;
        caps::raise(None, CapSet::Ambient, *cap)?;
    }

    let output = Command::new("mkfs.ext4")
        .arg("-F")
        .arg("-d")
        .arg(source_dir_path)
        .arg(file_path)
        .output()
        .await?;

    if !output.status.success() {
        bail!(
            "failed to format volume as ext4: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}
