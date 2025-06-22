pub mod credentials;
pub mod oci;

use std::{path::PathBuf, str::FromStr, sync::Arc};

use anyhow::{Result, bail};
use oci_client::Reference;
use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;

use crate::{
    agent::{
        data::{Collections, DEFAULT_AGENT_TENANT},
        volume::{VolumeAgent, fs},
    },
    machinery::store::{Key, PartialKey, Store},
    utils::time::now_millis,
};

#[derive(Debug, Clone)]
pub struct ImageAgentConfig {
    pub base_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageLayer {
    pub timestamp: u128,
    pub digest: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub id: String,
    pub reference: String,
    pub digest: String,
    pub timestamp: u128,
    pub volume_id: String,
    pub layer_ids: Vec<String>,
}

pub struct ImageAgent {
    config: ImageAgentConfig,
    store: Arc<Store>,
    volume_agent: Arc<VolumeAgent>,
    base_path: PathBuf,
    base_layers_path: PathBuf,
}

impl ImageAgent {
    pub async fn new(
        config: ImageAgentConfig,
        store: Arc<Store>,
        volume_agent: Arc<VolumeAgent>,
    ) -> Result<Self> {
        let base_path = PathBuf::from(&config.base_path);
        if !base_path.exists() {
            tokio::fs::create_dir_all(&base_path).await?;
        }

        let base_layers_path = base_path.join("layers");
        if !base_layers_path.exists() {
            tokio::fs::create_dir_all(&base_layers_path).await?;
        }

        Ok(Self {
            config,
            store,
            volume_agent,
            base_path,
            base_layers_path,
        })
    }

    pub fn image(&self, id: &str) -> Result<Option<Image>> {
        let key = Key::<Image>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Image)
            .key(id);

        let image = self.store.get(&key)?;
        Ok(image)
    }

    pub fn layer(&self, digest: &str) -> Result<Option<ImageLayer>> {
        let key = Key::<ImageLayer>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::ImageLayer)
            .key(digest);

        let layer = self.store.get(&key)?;
        Ok(layer)
    }

    pub fn image_by_reference(&self, reference: &str) -> Result<Option<Image>> {
        let key = PartialKey::<Image>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Image);

        let mut images = self.store.list(&key)?;
        images.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        let Some(image) = images.first().cloned() else {
            return Ok(None);
        };

        if image.reference != reference {
            return Ok(None);
        }

        Ok(Some(image))
    }

    pub fn image_list(&self) -> Result<Vec<Image>> {
        let key = PartialKey::<Image>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Image);

        let images = self.store.list(&key)?;
        Ok(images)
    }

    pub async fn image_pull(&self, reference: &str) -> Result<Image> {
        let reference = Reference::from_str(reference)?;
        let (manigest, digest, config) = oci::fetch_manifest(&reference).await?;

        if let Some(existing_image) = self.image_by_reference(&reference.to_string())? {
            if existing_image.digest == digest {
                return Ok(existing_image);
            }
        };

        // we are noew ready to pull the image
        // 1. see what layers we already have, and what we need to pull
        let mut layers_to_pull = Vec::new();
        for layer in manigest.layers.iter() {
            if let Some(_) = self.layer(&layer.digest)? {
                continue;
            }

            layers_to_pull.push(layer);
        }

        // 2. pull the needed layers and create entry for each layer
        for layer in layers_to_pull {
            let layer_path = self.base_layers_path.join(&layer.digest);
            let layer_path = layer_path.to_str().unwrap();

            oci::pull_layer(&reference, &layer, layer_path).await?;

            let layer_entry = ImageLayer {
                timestamp: now_millis(),
                digest: layer.digest.clone(),
                path: layer_path.to_string(),
            };

            let key = Key::<ImageLayer>::not_namespaced()
                .tenant(DEFAULT_AGENT_TENANT)
                .collection(Collections::ImageLayer)
                .key(&layer.digest);

            self.store.put(&key, &layer_entry)?;
        }

        // 3. create temp dir
        let temp_dir = tempfile::tempdir()?;

        // 4. unpack the layers
        for layer in manigest.layers.iter() {
            let layer_entry = self.layer(&layer.digest)?;
            let Some(layer_entry) = layer_entry else {
                bail!("Layer not found: {}", layer.digest);
            };

            let layer_path = PathBuf::from(&layer_entry.path);

            oci::uncompress_layer(&layer_path, &temp_dir.path()).await?;
        }

        let whiteout_path = temp_dir.path().to_path_buf();
        spawn_blocking(move || oci::remove_whiteouts(whiteout_path)).await??;

        // 5. write config for takeoff
        if let Some(config) = config.config {
            let config_path = temp_dir.path().join("./etc/lttle/oci-config.json");
            tokio::fs::create_dir_all(config_path.parent().unwrap()).await?;
            tokio::fs::write(config_path, serde_json::to_string_pretty(&config)?).await?;
        }

        // 6. create the volume from temp dir
        let dir_size_path = temp_dir.path().to_path_buf();
        let dir_size_bytes =
            spawn_blocking(move || fs::dir_size_in_bytes_recursive(dir_size_path)).await??;

        // convert to mb and add 15% to account for overhead
        let dir_size_mb = dir_size_bytes / 1024 / 1024;
        let sparse_size_mb = (dir_size_mb as f64 * 1.15).ceil() as u64;
        let sparse_size = sparse_size_mb * 1024 * 1024;

        let volume = self
            .volume_agent
            .volume_create_ext4_sparse(temp_dir.path().to_str().unwrap(), Some(sparse_size))
            .await?;

        // 7. create image
        let image_id = uuid::Uuid::new_v4().to_string();
        let key = Key::<Image>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::Image)
            .key(&image_id);

        let image = Image {
            id: image_id,
            reference: reference.to_string(),
            digest,
            timestamp: now_millis(),
            volume_id: volume.id,
            layer_ids: manigest.layers.iter().map(|l| l.digest.clone()).collect(),
        };
        self.store.put(&key, &image)?;

        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::volume::VolumeAgentConfig;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_image_pull() {
        let images_base_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let store_base_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let volume_base_dir = tempfile::tempdir().expect("Failed to create temp dir");

        let store = Arc::new(Store::new(store_base_dir.path()).await.unwrap());

        let volume_agent = Arc::new(
            VolumeAgent::new(
                VolumeAgentConfig {
                    base_path: volume_base_dir.path().to_str().unwrap().to_string(),
                },
                store.clone(),
            )
            .await
            .unwrap(),
        );

        let image_agent = ImageAgent::new(
            ImageAgentConfig {
                base_path: images_base_dir.path().to_str().unwrap().to_string(),
            },
            store,
            volume_agent,
        )
        .await
        .unwrap();

        let image = image_agent
            .image_pull("alpine:latest")
            .await
            .expect("Failed to pull image");

        println!("image: {:?}", image);

        assert_eq!(image.reference, "docker.io/library/alpine:latest");
        assert_eq!(image.layer_ids.len(), 1);

        let layer_files = std::fs::read_dir(images_base_dir.path().join("layers")).unwrap();
        assert_eq!(layer_files.count(), 1);

        let volume_files = std::fs::read_dir(volume_base_dir.path()).unwrap();
        assert_eq!(volume_files.count(), 1);

        let images = image_agent.image_list().unwrap();
        assert_eq!(images.len(), 1);

        // pulling again should not pull anything
        let new_image = image_agent
            .image_pull("alpine:latest")
            .await
            .expect("Failed to pull image");

        assert_eq!(new_image.reference, "docker.io/library/alpine:latest");
        assert_eq!(new_image.layer_ids.len(), 1);
        assert_eq!(new_image.id, image.id);

        let layer_files = std::fs::read_dir(images_base_dir.path().join("layers")).unwrap();
        assert_eq!(layer_files.count(), 1);

        let volume_files = std::fs::read_dir(volume_base_dir.path()).unwrap();
        assert_eq!(volume_files.count(), 1);

        let images = image_agent.image_list().unwrap();
        assert_eq!(images.len(), 1);
    }
}
