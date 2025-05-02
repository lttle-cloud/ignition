use anyhow::{Result, bail};
use clap::Parser;
use docker_credential::{CredentialRetrievalError, DockerCredential};
use flate2::read::GzDecoder;
use oci_client::{
    Client, Reference,
    client::{ClientConfig, ClientProtocol},
    config::ConfigFile,
    secrets::RegistryAuth,
};
use std::fs::{self, OpenOptions};
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use tar::Archive;

#[derive(Parser, Debug)]
#[command(name = "convert-image")]
#[command(about = "Pull and convert OCI images to ext4 sparse volumes", long_about = None)]
struct Cli {
    #[arg(value_name = "IMAGE")]
    image: String,

    #[arg(short, long, value_name = "OUTPUT")]
    output: PathBuf,
}

fn build_auth(reference: &Reference, anonymous: bool) -> RegistryAuth {
    let server = reference
        .resolve_registry()
        .strip_suffix('/')
        .unwrap_or_else(|| reference.resolve_registry());

    if anonymous {
        return RegistryAuth::Anonymous;
    }

    match docker_credential::get_credential(server) {
        Err(CredentialRetrievalError::ConfigNotFound) => RegistryAuth::Anonymous,
        Err(CredentialRetrievalError::NoCredentialConfigured) => RegistryAuth::Anonymous,
        Err(e) => {
            println!("Error handling docker configuration file: {}", e);
            RegistryAuth::Anonymous
        }
        Ok(DockerCredential::UsernamePassword(username, password)) => {
            println!("Found docker credentials");
            RegistryAuth::Basic(username, password)
        }
        Ok(DockerCredential::IdentityToken(_)) => {
            println!(
                "Cannot use contents of docker config, identity token not supported. Using anonymous auth"
            );
            RegistryAuth::Anonymous
        }
    }
}

async fn extract_image_to_dir(
    reference: &Reference,
    auth: &RegistryAuth,
    dir_path: impl AsRef<Path>,
) -> Result<()> {
    let config = ClientConfig {
        protocol: ClientProtocol::Https,
        ..Default::default()
    };

    let client = Client::new(config);

    println!("Pulling manifest and config: {}", reference);
    let (manifest, digest, config) = client.pull_manifest_and_config(reference, auth).await?;
    println!("Manifest and config pulled: {} {}", reference, digest);

    let config: ConfigFile = serde_json::from_slice(&config.as_bytes())?;

    for layer in manifest.layers.iter() {
        if layer.media_type != "application/vnd.oci.image.layer.v1.tar+gzip" {
            bail!("Unsupported layer media type: {}", layer.media_type);
        }
    }

    println!("Layers: {}", manifest.layers.len());
    for (index, layer) in manifest.layers.iter().enumerate() {
        println!("Pulling layer: {}", index);
        let mut data: Vec<u8> = Vec::new();
        client.pull_blob(reference, layer, &mut data).await?;

        println!("Unpacking layer: {}", index);
        let cursor = Cursor::new(&*data);
        let decoder = GzDecoder::new(cursor);

        let mut archive = Archive::new(decoder);
        archive.unpack(&dir_path)?;
    }

    if let Some(config) = config.config {
        let config_path = dir_path.as_ref().join("./etc/lttle/oci-config.json");
        fs::create_dir_all(config_path.parent().unwrap())?;
        fs::write(config_path, serde_json::to_string_pretty(&config)?)?;
    }

    Ok(())
}

fn dir_size_in_bytes_recursive(dir_path: impl AsRef<Path>) -> Result<u64> {
    let dir_path = dir_path.as_ref();
    let mut size = 0;
    for entry in fs::read_dir(dir_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            size += fs::metadata(path)?.len();
        } else if path.is_dir() {
            size += dir_size_in_bytes_recursive(&path)?;
        }
    }
    Ok(size)
}

fn remove_whiteouts(dir: impl AsRef<Path>) -> Result<()> {
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with(".wh.") {
            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        } else if path.is_dir() {
            remove_whiteouts(&path)?;
        }
    }
    Ok(())
}

fn create_sparse_image(path: impl AsRef<Path>, size_bytes: u64) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)?;

    file.set_len(size_bytes)?;
    Ok(())
}

fn create_ext4_image_from_dir(
    dir_path: impl AsRef<Path>,
    image_path: impl AsRef<Path>,
) -> Result<()> {
    let dir_path = dir_path.as_ref();
    let image_path = image_path.as_ref();

    let output = Command::new("mkfs.ext4")
        .arg("-F")
        .arg("-d")
        .arg(dir_path)
        .arg(image_path)
        .output()?;

    if !output.status.success() {
        bail!(
            "Failed to create ext4 image: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let reference: Reference = args.image.parse()?;
    let auth = build_auth(&reference, false);

    let temp_dir = tempfile::tempdir()?;

    extract_image_to_dir(&reference, &auth, &temp_dir.path()).await?;
    remove_whiteouts(&temp_dir.path())?;
    let size_bytes = dir_size_in_bytes_recursive(&temp_dir.path())?;
    let size_mb = size_bytes / 1024 / 1024;

    let sparse_image_size_mb = (size_mb as f64 * 1.15).ceil() as u64;
    let sparse_image_size = sparse_image_size_mb * 1024 * 1024;
    create_sparse_image(&args.output, sparse_image_size)?;

    create_ext4_image_from_dir(&temp_dir.path(), &args.output)?;

    Ok(())
}
