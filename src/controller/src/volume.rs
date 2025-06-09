use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use sds::{Collection, Store};
use util::{
    async_runtime::{
        fs::{OpenOptions, create_dir_all, remove_file},
        process::Command,
    },
    encoding::codec,
    result::{Result, bail},
    tracing::{debug, warn},
    uuid,
};

#[codec]
pub struct Volume {
    pub id: String,
    pub name: Option<String>,
    pub size_mib: u64,
    pub path: String,
    pub read_only: bool,
    pub created_at: u128,
}

pub struct VolumeConfig {
    pub name: Option<String>,
    pub size_mib: u64,
    pub read_only: bool,
}

#[derive(Clone)]
pub struct VolumePoolConfig {
    pub name: String,
    pub root_dir: String,
}

#[derive(Clone)]
pub struct VolumePool {
    store: Store,
    volumes_collection: Arc<Collection<Volume>>,
    config: VolumePoolConfig,
}

impl VolumePool {
    pub fn new(store: Store, config: VolumePoolConfig) -> Result<Self> {
        let volumes_collection =
            store.collection(format!("volume_pool:{}:volumes", config.name))?;

        Ok(Self {
            store,
            volumes_collection: Arc::new(volumes_collection),
            config,
        })
    }

    pub async fn get(&self, id: &str) -> Result<Option<Volume>> {
        let txn = self.store.read_txn()?;
        let volume = txn.get(&self.volumes_collection, id);
        Ok(volume)
    }

    pub async fn create_volume(&self, config: VolumeConfig) -> Result<Volume> {
        let id = uuid::Uuid::new_v4().to_string();

        let volume = Volume {
            id: id.clone(),
            name: config.name,
            size_mib: config.size_mib,
            path: format!("{}/{}", self.config.root_dir, id),
            read_only: config.read_only,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        };

        let path = self.get_path_for_volume(&id).await?;
        self.create_sparse_file(&path, config.size_mib).await?;

        let mut txn = self.store.write_txn()?;
        txn.put(&self.volumes_collection, &id, &volume)?;
        txn.commit()?;

        Ok(volume)
    }

    async fn get_path_for_volume(&self, volume_id: &str) -> Result<PathBuf> {
        // check if base dir exists
        let root_dir = Path::new(&self.config.root_dir);
        if !root_dir.exists() {
            create_dir_all(root_dir).await?;
        }

        let path = root_dir.join(volume_id);
        Ok(PathBuf::from(path))
    }

    async fn create_sparse_file(&self, path: impl AsRef<Path>, size_mib: u64) -> Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .await?;

        file.set_len(size_mib * 1024 * 1024).await?;
        Ok(())
    }

    pub async fn create_copy_of_volume(
        &self,
        volume_id: &str,
        copy_suffix: &str,
    ) -> Result<Volume> {
        debug!(
            "Creating sparse copy of volume: {} -> {}",
            volume_id, copy_suffix
        );

        let Some(volume) = self.get(volume_id).await? else {
            bail!("Volume not found: {}", volume_id);
        };

        // Create a new volume with the same size
        let copy_volume = Volume {
            id: format!("{}-{}", volume_id, copy_suffix),
            name: Some(format!(
                "{}-{}",
                volume.name.as_ref().unwrap_or(&"unnamed".to_string()),
                copy_suffix
            )),
            size_mib: volume.size_mib,
            path: format!("{}-copy-{}", volume.path, copy_suffix),
            read_only: false, // Instance volumes should be writable
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        };

        let mut txn = self.store.write_txn()?;
        txn.put(&self.volumes_collection, &copy_volume.id, &copy_volume)?;
        txn.commit()?;

        debug!(
            "Creating sparse overlay from {} to {}",
            volume.path, copy_volume.path
        );

        // Create a sparse file that starts as the same size but takes no space
        // We'll use a simple approach: create the file and then use a script to
        // make it behave like an overlay

        // First, create a sparse file of the same size
        let sparse_result = Command::new("truncate")
            .arg("-s")
            .arg(format!("{}M", volume.size_mib))
            .arg(&copy_volume.path)
            .output()
            .await;

        match sparse_result {
            Ok(output) if output.status.success() => {
                debug!(
                    "Sparse overlay created: {} ({}MB)",
                    copy_volume.path, volume.size_mib
                );

                // Copy the base image content to the sparse file
                // This is more efficient than cp for large sparse files
                let dd_result = Command::new("dd")
                    .arg(format!("if={}", volume.path))
                    .arg(format!("of={}", copy_volume.path))
                    .arg("bs=1M")
                    .arg("conv=sparse")
                    .output()
                    .await;

                match dd_result {
                    Ok(dd_output) if dd_output.status.success() => {
                        debug!("Sparse copy completed successfully");
                    }
                    _ => {
                        warn!("dd sparse copy failed, falling back to regular copy");
                        // Fallback to regular copy
                        let fallback_result = Command::new("cp")
                            .arg(&volume.path)
                            .arg(&copy_volume.path)
                            .output()
                            .await?;

                        if !fallback_result.status.success() {
                            let stderr = String::from_utf8_lossy(&fallback_result.stderr);
                            bail!("Failed to copy volume: {}", stderr);
                        }
                    }
                }
            }
            _ => {
                // Fallback to regular copy if truncate fails
                warn!("Sparse file creation failed, falling back to regular copy");
                let fallback_result = Command::new("cp")
                    .arg(&volume.path)
                    .arg(&copy_volume.path)
                    .output()
                    .await?;

                if !fallback_result.status.success() {
                    let stderr = String::from_utf8_lossy(&fallback_result.stderr);
                    bail!("Failed to copy volume: {}", stderr);
                }
                debug!("Volume copy created successfully: {}", copy_volume.path);
            }
        }

        Ok(copy_volume)
    }

    pub async fn delete_volume(&self, volume_id: &str) -> Result<()> {
        debug!("Deleting volume: {}", volume_id);

        let volume = self.get(volume_id).await?;
        let Some(volume) = volume else {
            debug!("Volume not found: {}", volume_id);
            bail!("Volume not found: {}", volume_id);
        };

        if let Err(e) = remove_file(&volume.path).await {
            if e.kind() == std::io::ErrorKind::NotFound {
                debug!("Volume file already deleted: {}", volume.path);
            } else {
                warn!("Failed to delete volume file {}: {}", volume.path, e);
                return Err(e.into());
            }
        } else {
            debug!("Volume file deleted successfully: {}", volume.path);
        }

        let mut txn = self.store.write_txn()?;
        txn.del(&self.volumes_collection, volume_id)?;
        txn.commit()?;

        Ok(())
    }
}

impl Volume {
    pub async fn format_as_ext4_volume_empty(&self) -> Result<()> {
        let image_path = Path::new(&self.path);

        let output = Command::new("mkfs.ext4")
            .arg("-F")
            .arg(image_path)
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

    pub async fn format_as_ext4_volume_from_dir(&self, source_dir: &Path) -> Result<()> {
        let image_path = Path::new(&self.path);

        let output = Command::new("mkfs.ext4")
            .arg("-F")
            .arg("-d")
            .arg(source_dir)
            .arg(image_path)
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
}
