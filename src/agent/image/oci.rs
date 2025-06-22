use std::{fs::File, io::BufReader, path::Path};

use anyhow::Result;
use flate2::bufread::GzDecoder;
use oci_client::{
    Client, Reference,
    client::{ClientConfig, ClientProtocol},
    config::ConfigFile,
    manifest::{OciDescriptor, OciImageManifest},
    secrets::RegistryAuth,
};
use tar::Archive;
use tracing::{error, info};

use crate::agent::image::credentials::{AnonymousOciCredentialsProvider, OciCredentialsProvider};

pub fn remove_whiteouts(dir: impl AsRef<Path>) -> Result<()> {
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

pub const SUPPORTED_LAYER_MEDIA_TYPES: &[&str] = &[
    "application/vnd.docker.image.rootfs.diff.tar.gzip",
    "application/vnd.oci.image.layer.v1.tar+gzip",
];

pub async fn create_default_oci_client(reference: &Reference) -> Result<(Client, RegistryAuth)> {
    let credentials_provider = AnonymousOciCredentialsProvider;
    let auth = credentials_provider.get_credentials_for_reference(reference)?;

    let client = Client::new(ClientConfig {
        protocol: ClientProtocol::Https,
        ..Default::default()
    });

    client
        .store_auth_if_needed(reference.resolve_registry(), &auth)
        .await;

    Ok((client, auth))
}

pub async fn is_layer_supported(layer: &OciDescriptor) -> Result<bool> {
    Ok(SUPPORTED_LAYER_MEDIA_TYPES.contains(&layer.media_type.as_str()))
}

pub async fn fetch_manifest(
    reference: &Reference,
) -> Result<(OciImageManifest, String, ConfigFile)> {
    let (client, auth) = create_default_oci_client(reference).await?;

    let (manifest, digest, config) = client.pull_manifest_and_config(reference, &auth).await?;

    let config: ConfigFile = serde_json::from_slice(&config.as_bytes())?;

    Ok((manifest, digest, config))
}

pub async fn pull_layer(
    reference: &Reference,
    layer: &OciDescriptor,
    file_path: impl AsRef<Path>,
) -> Result<()> {
    let (client, _) = create_default_oci_client(reference).await?;

    let mut data: Vec<u8> = Vec::new();

    let pull_result = client.pull_blob(reference, layer, &mut data).await;
    match pull_result {
        Ok(_) => {
            info!(
                "Layer {} pulled successfully ({} bytes)",
                layer.digest,
                data.len()
            );
        }
        Err(e) => {
            error!("Failed to pull layer {}: {}", layer.digest, e);
            return Err(e.into());
        }
    }

    println!("file_path: {:?}", file_path.as_ref());
    tokio::fs::write(&file_path, data).await?;

    Ok(())
}

pub async fn uncompress_layer(
    file_path: impl AsRef<Path>,
    dir_path: impl AsRef<Path>,
) -> Result<()> {
    let file_path = file_path.as_ref();
    let dir_path = dir_path.as_ref();

    let file = File::open(&file_path)?;
    info!("Unpacking layer {}", file_path.display());

    let file_reader = BufReader::new(file);
    let decoder = GzDecoder::new(file_reader);
    let mut archive = Archive::new(decoder);

    // Set up archive to preserve permissions
    archive.set_preserve_permissions(true);
    archive.set_preserve_mtime(true);

    let unpack_result = archive.unpack(&dir_path);
    match unpack_result {
        Ok(_) => {
            info!("Layer {} unpacked successfully", file_path.display());
        }
        Err(e) => {
            error!(
                "Failed to unpack layer {} to {}: {}",
                file_path.display(),
                dir_path.display(),
                e
            );
            error!("Error details: {:?}", e);

            return Err(e.into());
        }
    };

    Ok(())
}
