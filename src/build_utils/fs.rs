use std::path::Path;

use anyhow::Result;

use crate::build_utils::cargo;

pub async fn write_if_changed(path: impl AsRef<Path>, content: impl AsRef<str>) -> Result<()> {
    let path = path.as_ref();
    let content = content.as_ref().as_bytes();

    if path.exists() {
        let existing_content = tokio::fs::read(path).await?;
        if existing_content == content {
            cargo::warn(format!("file did not change {}", path.display()));
            return Ok(());
        }
    }

    tokio::fs::write(path, content).await?;
    Ok(())
}
