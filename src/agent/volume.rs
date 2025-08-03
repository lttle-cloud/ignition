pub mod fs;

use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    agent::data::Collections,
    constants::DEFAULT_AGENT_TENANT,
    machinery::store::{Key, PartialKey, Store},
};

#[derive(Debug, Clone)]
pub struct VolumeAgentConfig {
    pub base_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub id: String,
    pub sparse_size: u64,
    pub path: String,
    pub ov_path: String,
    pub cloned_from: Option<String>,
}

pub struct VolumeAgent {
    base_path: PathBuf,
    store: Arc<Store>,
}

impl VolumeAgent {
    pub async fn new(config: VolumeAgentConfig, store: Arc<Store>) -> Result<Self> {
        let base_path = PathBuf::from(&config.base_path);

        if !base_path.exists() {
            tokio::fs::create_dir_all(&base_path).await?;
        }

        Ok(Self { base_path, store })
    }

    pub fn volume(&self, id: &str) -> Result<Option<Volume>> {
        let key = Key::<Volume>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Volume)
            .key(id);

        let volume = self.store.get(&key)?;
        Ok(volume)
    }

    pub fn volume_list(&self) -> Result<Vec<Volume>> {
        let key = PartialKey::<Volume>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Volume);

        let volumes = self.store.list(&key)?;
        Ok(volumes)
    }

    pub async fn volume_create_empty_sparse(&self, sparse_size: u64) -> Result<Volume> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = self.base_path.join(&id).to_string_lossy().to_string();
        fs::create_sparse_file(&path, sparse_size).await?;

        let ov_path = self
            .base_path
            .join(format!("{}.ov", id))
            .to_string_lossy()
            .to_string();
        fs::create_sparse_file(&ov_path, sparse_size).await?;

        let volume = Volume {
            id: id.clone(),
            sparse_size,
            path,
            ov_path,
            cloned_from: None,
        };

        let key = Key::<Volume>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Volume)
            .key(&id);

        self.store.put(&key, &volume)?;

        Ok(volume)
    }

    pub async fn volume_create_empty_ext4_sparse(&self, sparse_size: u64) -> Result<Volume> {
        let volume = self.volume_create_empty_sparse(sparse_size).await?;

        if let Err(e) = fs::format_file_as_ext4_volume_empty(&volume.path).await {
            self.volume_delete(&volume.id).await?;
            return Err(e);
        }

        Ok(volume)
    }

    pub async fn volume_create_ext4_sparse(
        &self,
        dir: &str,
        sparse_size: Option<u64>,
    ) -> Result<Volume> {
        let sparse_size = match sparse_size {
            Some(sparse_size) => sparse_size,
            None => fs::dir_size_in_bytes_recursive(dir)?,
        };

        let volume = self.volume_create_empty_ext4_sparse(sparse_size).await?;

        if let Err(e) = fs::format_file_as_ext4_volume_from_dir(&volume.path, dir).await {
            self.volume_delete(&volume.id).await?;
            return Err(e);
        }

        Ok(volume)
    }

    pub async fn volume_delete(&self, id: &str) -> Result<()> {
        let Some(volume) = self.volume(id)? else {
            return Err(anyhow::anyhow!("Volume not found"));
        };

        tokio::fs::remove_file(&volume.path).await?;

        let key = Key::<Volume>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Volume)
            .key(&volume.id);

        self.store.delete(&key)?;

        Ok(())
    }

    pub async fn volume_clone_with_overlay(&self, source_id: &str) -> Result<Volume> {
        let Some(source_volume) = self.volume(source_id)? else {
            return Err(anyhow::anyhow!("Source volume not found"));
        };

        let id = uuid::Uuid::new_v4().to_string();
        let ov_path = self
            .base_path
            .join(format!("{}.ov", id))
            .to_string_lossy()
            .to_string();
        fs::create_sparse_file(&ov_path, source_volume.sparse_size).await?;

        let new_volume = Volume {
            id,
            sparse_size: source_volume.sparse_size,
            path: source_volume.path.clone(),
            ov_path,
            cloned_from: Some(source_id.to_string()),
        };

        let key = Key::<Volume>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Volume)
            .key(&new_volume.id);

        self.store.put(&key, &new_volume)?;

        Ok(new_volume)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_agent(store_dir: &str, volumes_dir: &str) -> VolumeAgent {
        let agent = VolumeAgent::new(
            VolumeAgentConfig {
                base_path: volumes_dir.to_string(),
            },
            Arc::new(Store::new(store_dir.to_string()).await.unwrap()),
        )
        .await
        .unwrap();

        agent
    }

    #[tokio::test]
    async fn test_volume_create_and_delete() {
        let store_temp_dir = tempfile::tempdir().unwrap();
        let volumes_temp_dir = tempfile::tempdir().unwrap();
        let agent = create_test_agent(
            store_temp_dir.path().to_str().unwrap(),
            volumes_temp_dir.path().to_str().unwrap(),
        )
        .await;

        let volume = agent.volume_create_empty_sparse(1024).await.unwrap();
        let path = PathBuf::from(volume.path);
        assert!(path.exists());

        agent.volume_delete(&volume.id).await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_volume_list() {
        let store_temp_dir = tempfile::tempdir().unwrap();
        let volumes_temp_dir = tempfile::tempdir().unwrap();
        let agent = create_test_agent(
            store_temp_dir.path().to_str().unwrap(),
            volumes_temp_dir.path().to_str().unwrap(),
        )
        .await;

        let volume = agent.volume_create_empty_sparse(1024).await.unwrap();
        let volume2 = agent.volume_create_empty_sparse(2048).await.unwrap();

        let volumes = agent.volume_list().unwrap();
        assert_eq!(volumes.len(), 2);
        assert!(volumes.iter().any(|v| v.id == volume.id));
        assert!(volumes.iter().any(|v| v.id == volume2.id));
    }

    #[tokio::test]
    async fn test_volume_clone() {
        let store_temp_dir = tempfile::tempdir().unwrap();
        let volumes_temp_dir = tempfile::tempdir().unwrap();
        let agent = create_test_agent(
            store_temp_dir.path().to_str().unwrap(),
            volumes_temp_dir.path().to_str().unwrap(),
        )
        .await;

        let volume = agent.volume_create_empty_sparse(1024).await.unwrap();
        let volume2 = agent.volume_clone_with_overlay(&volume.id).await.unwrap();

        assert_eq!(volume2.cloned_from, Some(volume.id.clone()));

        let volumes = agent.volume_list().unwrap();
        assert_eq!(volumes.len(), 2);
        assert!(volumes.iter().any(|v| v.id == volume.id));
        assert!(volumes.iter().any(|v| v.id == volume2.id));
    }
}
