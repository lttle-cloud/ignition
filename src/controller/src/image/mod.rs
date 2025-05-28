mod credentials;

use credentials::OciCredentialsProvider;
use flate2::read::GzDecoder;
use oci_client::{
    Client, Reference,
    client::{ClientConfig, ClientProtocol},
    config::ConfigFile,
    manifest::OciImageManifest,
    secrets::RegistryAuth,
};
use sds::{Collection, Store};
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use tar::Archive;
use util::{
    async_runtime::{fs, task::spawn_blocking},
    encoding::codec,
    result::{Result, bail},
};

use crate::volume::{VolumeConfig, VolumePool};

fn dir_size_in_bytes_recursive(dir_path: impl AsRef<Path>) -> Result<u64> {
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

fn remove_whiteouts(dir: impl AsRef<Path>) -> Result<()> {
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with(".wh.") {
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        } else if path.is_dir() {
            remove_whiteouts(&path)?;
        }
    }
    Ok(())
}

const SUPPORTED_LAYER_MEDIA_TYPES: &[&str] = &[
    "application/vnd.docker.image.rootfs.diff.tar.gzip",
    "application/vnd.oci.image.layer.v1.tar+gzip",
];

#[codec]
pub struct Image {
    pub reference: Reference,
    pub digest: String,
    pub size_mib: u64,
    pub volume_id: String,
    pub created_at: u128,
}

pub struct ImagePoolConfig {
    pub volume_pool: VolumePool,
    pub credentials_provider: Box<dyn OciCredentialsProvider>,
}

#[derive(Clone)]
pub enum PullPolicy {
    Always,
    IfNotPresent,
    IfChanged,
}

#[derive(Clone)]
pub struct ImagePool {
    volume_pool: VolumePool,
    store: Store,
    images_collection: Arc<Collection<Image>>,
    credentials_provider: Arc<dyn OciCredentialsProvider>,
}

impl ImagePool {
    pub fn new(store: Store, config: ImagePoolConfig) -> Result<Self> {
        let images_collection = store.collection::<Image>("images")?;

        Ok(Self {
            volume_pool: config.volume_pool,
            store,
            images_collection: Arc::new(images_collection),
            credentials_provider: Arc::from(config.credentials_provider),
        })
    }

    pub async fn get_by_reference_and_digest(
        &self,
        reference: &Reference,
        digest: &str,
    ) -> Result<Option<Image>> {
        let txn = self.store.read_txn()?;
        let image = txn.get(
            &self.images_collection,
            &format!("{}:{}", reference.to_string(), digest),
        );
        Ok(image)
    }

    pub async fn get_by_reference(&self, reference: &Reference) -> Result<Option<Image>> {
        let txn = self.store.read_txn()?;
        let image = txn.get(&self.images_collection, &reference.to_string());
        Ok(image)
    }

    pub async fn create_oci_client(&self, reference: &Reference) -> Result<(Client, RegistryAuth)> {
        let auth = self
            .credentials_provider
            .get_credentials_for_reference(reference)?;

        let client = Client::new(ClientConfig {
            protocol: ClientProtocol::Https,
            ..Default::default()
        });

        client
            .store_auth_if_needed(reference.resolve_registry(), &auth)
            .await;

        Ok((client, auth))
    }

    pub async fn fetch_manifest(
        &self,
        reference: &Reference,
    ) -> Result<(OciImageManifest, String, ConfigFile)> {
        let (client, auth) = self.create_oci_client(reference).await?;

        let (manifest, digest, config) = client.pull_manifest_and_config(reference, &auth).await?;

        let config: ConfigFile = serde_json::from_slice(&config.as_bytes())?;

        Ok((manifest, digest, config))
    }

    pub async fn pull_image_if_needed(
        &self,
        reference: &Reference,
        policy: PullPolicy,
    ) -> Result<Image> {
        match policy {
            PullPolicy::Always => self.pull_image_always(reference).await,
            PullPolicy::IfNotPresent => self.pull_image_if_not_present(reference).await,
            PullPolicy::IfChanged => self.pull_image_if_changed(reference).await,
        }
    }

    async fn pull_image_always(&self, reference: &Reference) -> Result<Image> {
        let (manifest, digest, config) = self.fetch_manifest(reference).await?;
        self.pull_image_from_manifest(reference, manifest, digest, config)
            .await
    }

    async fn pull_image_if_not_present(&self, reference: &Reference) -> Result<Image> {
        let image = self.get_by_reference(reference).await?;
        match image {
            Some(image) => Ok(image),
            None => self.pull_image_always(reference).await,
        }
    }

    async fn pull_image_if_changed(&self, reference: &Reference) -> Result<Image> {
        let image = self.get_by_reference(reference).await?;
        let Some(image) = image else {
            return self.pull_image_always(reference).await;
        };

        let (manifest, digest, config) = self.fetch_manifest(reference).await?;

        if image.digest == digest {
            Ok(image)
        } else {
            self.pull_image_from_manifest(reference, manifest, digest, config)
                .await
        }
    }

    async fn pull_image_from_manifest(
        &self,
        reference: &Reference,
        manifest: OciImageManifest,
        digest: String,
        config: ConfigFile,
    ) -> Result<Image> {
        // 0. validate manifest
        // 1. fetch image to temp dir
        // 2. meassure size of temp dir
        // 3. create sparse volume image
        // 4. ext4 volume from temp dir
        // 5. create image record

        for layer in manifest.layers.iter() {
            if !SUPPORTED_LAYER_MEDIA_TYPES.contains(&layer.media_type.as_str()) {
                bail!("Unsupported layer media type: {}", layer.media_type);
            }
        }

        let (client, _) = self.create_oci_client(reference).await?;

        let temp_dir = tempfile::tempdir()?;
        for (index, layer) in manifest.layers.iter().enumerate() {
            println!("Pulling layer: {}", index);
            let mut data: Vec<u8> = Vec::new();
            client.pull_blob(reference, layer, &mut data).await?;

            println!("Unpacking layer: {}", index);
            let cursor = Cursor::new(&*data);
            let decoder = GzDecoder::new(cursor);

            let mut archive = Archive::new(decoder);
            archive.unpack(&temp_dir.path())?;
        }
        let temp_dir_path = temp_dir.path().to_path_buf();
        spawn_blocking(move || remove_whiteouts(&temp_dir_path)).await?;

        if let Some(config) = config.config {
            let config_path = temp_dir.path().join("./etc/lttle/oci-config.json");
            fs::create_dir_all(config_path.parent().unwrap()).await?;
            fs::write(config_path, serde_json::to_string_pretty(&config)?).await?;
        }

        let size_bytes = dir_size_in_bytes_recursive(&temp_dir.path())?;
        let size_mb = size_bytes / 1024 / 1024;

        let sparse_image_size_mb = (size_mb as f64 * 1.15).ceil() as u64;

        let volume = self
            .volume_pool
            .create_volume(VolumeConfig {
                name: Some(format!("{}:{}", reference.to_string(), digest)),
                size_mib: sparse_image_size_mb,
                read_only: false,
            })
            .await?;

        volume
            .format_as_ext4_volume_from_dir(&temp_dir.path())
            .await?;

        drop(temp_dir);

        let image = Image {
            reference: reference.clone(),
            digest,
            size_mib: sparse_image_size_mb,
            volume_id: volume.id,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        };

        Ok(image)
    }
}
