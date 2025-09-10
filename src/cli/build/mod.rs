pub mod docker_auth;

use std::{io::Write, path::Path};

use anyhow::{Result, bail};
use ignition::resources::machine::{MachineBuild, MachineBuildOptions, MachineDockerOptions};
use nixpacks::nixpacks::{
    builder::docker::DockerBuilderOptions, plan::generator::GeneratePlanOptions,
};
use tokio::process::Command;

use crate::{build::docker_auth::DockerAuthConfig, ui::message::message_detail};

pub async fn build_image(
    dir: impl AsRef<Path>,
    tenant: &str,
    build: MachineBuild,
    auth: DockerAuthConfig,
    debug: bool,
) -> Result<String> {
    let image = match build {
        MachineBuild::Nixpacks(options) => {
            build_image_nixpacks(dir, tenant, options, auth, debug).await
        }
        MachineBuild::Docker(options) => {
            build_image_docker(dir, tenant, auth, options, debug).await
        }
        MachineBuild::NixpacksAuto => {
            let id = uuid::Uuid::new_v4().to_string();
            build_image_nixpacks(
                dir,
                tenant,
                MachineBuildOptions {
                    dir: None,
                    image: None,
                    tag: None,
                    name: id,
                },
                auth,
                debug,
            )
            .await
        }
    };

    Ok(image?)
}

async fn build_image_nixpacks(
    dir: impl AsRef<Path>,
    tenant: &str,
    options: MachineBuildOptions,
    auth: DockerAuthConfig,
    debug: bool,
) -> Result<String> {
    let Some(registry) = auth.get_registry() else {
        bail!("No registry found in auth");
    };

    if debug {
        message_detail("Building image with nixpacks");
    }

    let image = options.image.unwrap_or_else(|| {
        format!(
            "{}/{}/{}:{}",
            registry,
            tenant,
            options.name,
            options.tag.unwrap_or("latest".to_string())
        )
    });

    if debug {
        message_detail(format!("Generated image reference: {}", image));
    }

    let envs = std::env::vars()
        .into_iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<String>>();

    let envs = envs.iter().map(|s| s.as_str()).collect::<Vec<&str>>();

    let path = dir
        .as_ref()
        .join(options.dir.unwrap_or(".".to_string()))
        .to_string_lossy()
        .to_string();

    if debug {
        message_detail(format!("Building image for path: {}", path));
    }

    let plan_options = GeneratePlanOptions::default();

    let mut build_options = DockerBuilderOptions::default();
    build_options.platform = vec!["linux/amd64".to_string()];
    build_options.quiet = true;
    build_options.name = Some(image.clone());

    if debug {
        build_options.verbose = true;
        build_options.quiet = false;

        let mut build_options = build_options.clone();
        build_options.print_dockerfile = true;

        message_detail("Generated docker file: ");
        nixpacks::create_docker_image(&path, envs.clone(), &plan_options, &build_options)
            .await
            .ok();
    }

    if debug {
        message_detail("Docker build output: ");
    }

    let Ok(_) = nixpacks::create_docker_image(&path, envs, &plan_options, &build_options).await
    else {
        if debug {
            bail!("Failed to build docker image");
        }

        bail!(
            "Failed to build docker image. Run the same command with --debug-build to see the full output"
        );
    };

    Ok(image)
}

async fn build_image_docker(
    dir: impl AsRef<Path>,
    tenant: &str,
    auth: DockerAuthConfig,
    options: MachineDockerOptions,
    debug: bool,
) -> Result<String> {
    let Some(registry) = auth.get_registry() else {
        bail!("No registry found in auth");
    };

    if debug {
        message_detail("Building image with nixpacks");
    }

    let image = options.image.unwrap_or_else(|| {
        format!(
            "{}/{}/{}:{}",
            registry,
            tenant,
            options.name,
            options.tag.unwrap_or("latest".to_string())
        )
    });

    let context = dir
        .as_ref()
        .join(options.context.unwrap_or(".".to_string()));

    let dockerfile_path = match options.dockerfile {
        Some(path) => dir.as_ref().join(path),
        None => context.join("Dockerfile"),
    };

    if !dockerfile_path.exists() {
        bail!("Dockerfile not found");
    }

    let output = Command::new("docker")
        .env("DOCKER_AUTH_CONFIG", auth.to_json()?)
        .args(&[
            "build",
            "--platform",
            "linux/amd64",
            "-t",
            &image,
            "-f",
            dockerfile_path.to_str().unwrap(),
            context.to_str().unwrap(),
        ])
        .output()
        .await?;

    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout)?;
        std::io::stderr().write_all(&output.stderr)?;
        bail!("Failed to build image");
    };

    Ok(image)
}

pub async fn push_image(image: impl AsRef<str>, auth: DockerAuthConfig) -> Result<()> {
    let output = Command::new("docker")
        .env("DOCKER_AUTH_CONFIG", auth.to_json()?)
        .args(&["push", image.as_ref()])
        .output()
        .await?;

    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout)?;
        std::io::stderr().write_all(&output.stderr)?;
        bail!("Failed to push image");
    };

    Ok(())
}
