use std::path::Path;

use anyhow::Result;
use oci_client::{
    Client, Reference,
    client::{ClientConfig, ClientProtocol},
    config::ConfigFile,
    manifest::{OciDescriptor, OciImageManifest},
    secrets::RegistryAuth,
};
use tracing::{error, info};

use crate::agent::image::{credentials::OciCredentialsProvider, unpacker};

pub const SUPPORTED_LAYER_MEDIA_TYPES: &[&str] = &[
    "application/vnd.docker.image.rootfs.diff.tar.gzip",
    "application/vnd.oci.image.layer.v1.tar+gzip",
];

pub async fn create_default_oci_client(
    credentials_provider: &impl OciCredentialsProvider,
    reference: &Reference,
) -> Result<(Client, RegistryAuth)> {
    let auth = credentials_provider.get_credentials_for_reference(reference)?;

    let client = Client::new(ClientConfig {
        protocol: ClientProtocol::Https,
        ..Default::default()
    });

    println!(
        "store auth if needed {} {:?}",
        reference.resolve_registry(),
        auth
    );

    client
        .store_auth_if_needed(reference.resolve_registry(), &auth)
        .await;

    Ok((client, auth))
}

pub async fn is_layer_supported(layer: &OciDescriptor) -> Result<bool> {
    Ok(SUPPORTED_LAYER_MEDIA_TYPES.contains(&layer.media_type.as_str()))
}

pub async fn fetch_manifest(
    credentials_provider: &impl OciCredentialsProvider,
    reference: &Reference,
) -> Result<(OciImageManifest, String, ConfigFile)> {
    let (client, auth) = create_default_oci_client(credentials_provider, reference).await?;

    let (manifest, digest, config) = client.pull_manifest_and_config(reference, &auth).await?;

    let config: ConfigFile = serde_json::from_slice(&config.as_bytes())?;

    Ok((manifest, digest, config))
}

pub async fn pull_layer(
    credentials_provider: &impl OciCredentialsProvider,
    reference: &Reference,
    layer: &OciDescriptor,
    file_path: impl AsRef<Path>,
) -> Result<()> {
    let (client, _) = create_default_oci_client(credentials_provider, reference).await?;

    let mut data: Vec<u8> = Vec::new();

    println!("pulling layer {} {}", layer.digest, layer.media_type);

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

    tokio::fs::write(&file_path, data).await?;

    Ok(())
}

pub async fn uncompress_layer(
    file_path: impl AsRef<Path>,
    dir_path: impl AsRef<Path>,
) -> Result<()> {
    let file_path = file_path.as_ref().to_owned();
    let dir_path = dir_path.as_ref().to_owned();

    tokio::task::spawn_blocking(move || unpacker::unpack_gzipped_tar(file_path, dir_path))
        .await??;

    Ok(())
}
