pub mod credentials;

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
    tracing::{debug, error, info},
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
    let dir_path = dir.as_ref();

    let entries = match std::fs::read_dir(&dir_path) {
        Ok(entries) => entries,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                return Ok(());
            } else {
                return Err(e.into());
            }
        }
    };

    for entry in entries {
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
    pub reference: String,
    pub digest: String,
    pub size_mib: u64,
    pub volume_id: String,
    pub created_at: u128,
}

pub struct ImagePoolConfig {
    pub volume_pool: VolumePool,
    pub credentials_provider: Arc<dyn OciCredentialsProvider + Send + Sync>,
}

#[codec]
#[derive(Clone, Debug)]
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
    credentials_provider: Arc<dyn OciCredentialsProvider + Send + Sync>,
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
        let images = txn.get_all_values_prefix(&self.images_collection, &reference.to_string())?;
        // find the image with the latest created_at
        let image = images
            .into_iter()
            .max_by_key(|image| image.created_at)
            .map(|image| image);

        Ok(image)
    }

    pub async fn get_volume(&self, volume_id: &str) -> Result<Option<crate::volume::Volume>> {
        self.volume_pool.get(volume_id).await
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
        info!("Starting image pull from manifest for {}", reference);
        debug!("Image digest: {}", digest);
        debug!("Layers count: {}", manifest.layers.len());

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

        info!("Creating OCI client for image pull");
        let (client, _) = self.create_oci_client(reference).await?;

        info!("Creating temporary directory for image extraction");
        let temp_dir = tempfile::tempdir().map_err(|e| {
            error!("Failed to create temp directory: {}", e);
            e
        })?;
        let temp_path = temp_dir.path();
        info!("Temp directory created: {}", temp_path.display());

        for (index, layer) in manifest.layers.iter().enumerate() {
            info!("Pulling layer {} of {}", index + 1, manifest.layers.len());
            debug!("Layer digest: {}", layer.digest);
            debug!("Layer media type: {}", layer.media_type);
            debug!("Layer size: {} bytes", layer.size);

            let mut data: Vec<u8> = Vec::new();

            let pull_result = client.pull_blob(reference, layer, &mut data).await;
            match pull_result {
                Ok(_) => {
                    info!("Layer {} pulled successfully ({} bytes)", index, data.len());
                }
                Err(e) => {
                    error!("Failed to pull layer {}: {}", index, e);
                    return Err(e.into());
                }
            }

            info!("Unpacking layer {} to {}", index, temp_path.display());

            let cursor = Cursor::new(&*data);
            let decoder = GzDecoder::new(cursor);
            let mut archive = Archive::new(decoder);

            // Set up archive to preserve permissions
            archive.set_preserve_permissions(true);
            archive.set_preserve_mtime(true);

            let unpack_result = archive.unpack(&temp_path);
            match unpack_result {
                Ok(_) => {
                    info!("Layer {} unpacked successfully", index);
                }
                Err(e) => {
                    error!(
                        "Failed to unpack layer {} to {}: {}",
                        index,
                        temp_path.display(),
                        e
                    );
                    error!("Error details: {:?}", e);

                    return Err(e.into());
                }
            }
        }

        info!("Removing whiteouts from extracted image");
        let temp_dir_path = temp_dir.path().to_path_buf();
        let whiteout_result = spawn_blocking(move || remove_whiteouts(&temp_dir_path)).await?;
        match whiteout_result {
            Ok(_) => {
                info!("Whiteouts removed successfully");
            }
            Err(e) => {
                error!("Failed to remove whiteouts: {}", e);
                return Err(e);
            }
        }

        if let Some(config) = config.config {
            info!("Writing OCI config to image");
            let config_path = temp_dir.path().join("./etc/lttle/oci-config.json");

            let create_dir_result = fs::create_dir_all(config_path.parent().unwrap()).await;
            match create_dir_result {
                Ok(_) => {
                    debug!("Config directory created");
                }
                Err(e) => {
                    error!("Failed to create config directory: {}", e);
                    return Err(e.into());
                }
            }

            let write_config_result =
                fs::write(config_path, serde_json::to_string_pretty(&config)?).await;
            match write_config_result {
                Ok(_) => {
                    info!("OCI config written successfully");
                }
                Err(e) => {
                    error!("Failed to write OCI config: {}", e);
                    return Err(e.into());
                }
            }
        }

        info!("Measuring extracted image size");
        let size_bytes = dir_size_in_bytes_recursive(&temp_dir.path())?;
        let size_mb = size_bytes / 1024 / 1024;
        info!(
            "Extracted image size: {} MB ({} bytes)",
            size_mb, size_bytes
        );

        let sparse_image_size_mb = (size_mb as f64 * 1.15).ceil() as u64;
        info!(
            "Creating volume with size: {} MB (15% padding)",
            sparse_image_size_mb
        );

        let volume_result = self
            .volume_pool
            .create_volume(VolumeConfig {
                name: Some(format!("{}:{}", reference.to_string(), digest)),
                size_mib: sparse_image_size_mb,
                read_only: false,
            })
            .await;

        let volume = match volume_result {
            Ok(volume) => {
                info!("Volume created successfully: {}", volume.id);
                info!("Volume path: {}", volume.path);
                volume
            }
            Err(e) => {
                error!("Failed to create volume: {}", e);
                return Err(e);
            }
        };

        info!("Formatting volume as ext4 from directory");
        let format_result = volume
            .format_as_ext4_volume_from_dir(&temp_dir.path())
            .await;

        match format_result {
            Ok(_) => {
                info!("Volume formatted successfully as ext4");
            }
            Err(e) => {
                error!("Failed to format volume as ext4: {}", e);
                return Err(e);
            }
        }

        drop(temp_dir);
        info!("Temporary directory cleaned up");

        let image = Image {
            reference: reference.to_string(),
            digest,
            size_mib: sparse_image_size_mb,
            volume_id: volume.id,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        };

        info!("Storing image metadata in database");
        let mut txn = self.store.write_txn()?;

        // Store by reference for quick lookup
        txn.put(&self.images_collection, &reference.to_string(), &image)?;

        // Also store by reference:digest for specific version lookup
        txn.put(
            &self.images_collection,
            &format!("{}:{}", reference.to_string(), image.digest),
            &image,
        )?;

        txn.commit().map_err(|e| {
            error!("Failed to commit image to database: {}", e);
            e
        })?;

        info!("Image metadata stored successfully");
        info!("Image pull completed successfully for {}", reference);
        debug!(
            "Final image: volume_id={}, size={}MB",
            image.volume_id, image.size_mib
        );

        Ok(image)
    }
}
