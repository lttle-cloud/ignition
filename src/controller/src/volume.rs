use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use sds::{Collection, Store};
use util::{
    async_runtime::{
        fs::{OpenOptions, create_dir_all},
        process::Command,
    },
    encoding::codec,
    result::{Result, bail},
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
        self.create_sparse_volume(&path, config.size_mib).await?;

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

    async fn create_sparse_volume(&self, path: impl AsRef<Path>, size_mib: u64) -> Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .await?;

        file.set_len(size_mib * 1024 * 1024).await?;
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

    pub async fn format_as_ext4_volume_from_dir(&self, dir: impl AsRef<Path>) -> Result<()> {
        let dir_path = dir.as_ref();
        let image_path = Path::new(&self.path);

        let output = Command::new("mkfs.ext4")
            .arg("-F")
            .arg("-d")
            .arg(dir_path)
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
