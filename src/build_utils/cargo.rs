use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::Result;

pub fn warn(msg: impl AsRef<str>) {
    println!("cargo::warning={}", msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    println!("cargo::error={}", msg.as_ref());
}

pub fn build_out_dir_path(rel: impl AsRef<str>) -> PathBuf {
    let out_dir = env::var("OUT_DIR").unwrap();
    let path = Path::new(&out_dir).join(rel.as_ref());

    warn(format!("build_out_dir_path {}", path.display()));

    path
}

pub async fn workspace_root_dir_path(rel: impl AsRef<str>) -> Result<PathBuf> {
    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&cargo_manifest_dir).join(rel.as_ref());

    let Some(dir) = path.parent() else {
        return Err(anyhow::anyhow!(
            "Failed to get parent directory of {}",
            path.display()
        ));
    };

    tokio::fs::create_dir_all(dir).await?;

    warn(format!("workspace_root_dir_path {}", path.display()));
    Ok(path)
}
