pub mod docker_auth;

use std::{io::Write, path::Path};

use anyhow::{Result, bail};
use ignition::resources::machine::{MachineBuild, MachineBuildOptions, MachineDockerOptions};
use nixpacks::nixpacks::{
    builder::docker::DockerBuilderOptions, plan::generator::GeneratePlanOptions,
};
use tokio::process::Command;

use crate::{
    build::docker_auth::DockerAuthConfig,
    ui::{
        message::message_detail,
        summary::{Summary, SummaryCellStyle, SummaryRow},
    },
};

pub async fn build_image(
    dir: impl AsRef<Path>,
    tenant: &str,
    build: MachineBuild,
    auth: DockerAuthConfig,
    debug: bool,
    disable_build_cache: bool,
) -> Result<String> {
    let image = match build {
        MachineBuild::Nixpacks(options) => {
            build_image_nixpacks(dir, tenant, options, auth, debug, disable_build_cache).await
        }
        MachineBuild::Docker(options) => {
            build_image_docker(dir, tenant, auth, options, debug, disable_build_cache).await
        }
        MachineBuild::NixpacksAuto => {
            build_image_nixpacks(
                dir,
                tenant,
                MachineBuildOptions {
                    dir: None,
                    image: None,
                    tag: None,
                    name: None,
                },
                auth,
                debug,
                disable_build_cache,
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
    disable_build_cache: bool,
) -> Result<String> {
    let Some(registry) = auth.get_registry() else {
        bail!("No registry found in auth");
    };

    if debug {
        message_detail("Building image with nixpacks");
    }

    let image = options.image.unwrap_or_else(|| {
        let id = uuid::Uuid::new_v4().to_string();

        format!(
            "{}/{}/{}:{}",
            registry,
            tenant,
            options.name.unwrap_or(id),
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

    let mut plan_options = GeneratePlanOptions::default();
    // check if dir or pwd contains a nixpacks.toml
    let dir_nixpacks_toml = dir.as_ref().join("nixpacks.toml");
    let pwd_nixpacks_toml = std::env::current_dir().unwrap().join("nixpacks.toml");

    let nixpacks_toml = if dir_nixpacks_toml.exists() {
        Some(dir_nixpacks_toml)
    } else if pwd_nixpacks_toml.exists() {
        Some(pwd_nixpacks_toml)
    } else {
        None
    };

    if let Some(nixpacks_toml) = nixpacks_toml {
        message_detail(format!(
            "Using nixpacks options from: {}",
            nixpacks_toml.to_str().unwrap()
        ));
        plan_options.config_file = Some(nixpacks_toml.to_str().unwrap().to_string());
    };

    let providers = nixpacks::get_plan_providers(&path, envs.clone(), &plan_options)?;
    if providers.is_empty() {
        bail!(
            "No compatible providers found for auto-build. Check the documentation for auto-build supported targets: https://docs.lttle.cloud/build/auto-build#supported-targets"
        );
    }

    message_detail(format!(
        "Auto-build using providers: {}",
        providers.join(", ")
    ));

    let plan = nixpacks::generate_build_plan(&path, envs.clone(), &plan_options)?;
    let phase_info_desc = plan.get_phase_info_desc()?;

    if debug {
        message_detail("Build summary: ");
        let mut build_summary = Summary {
            rows: phase_info_desc
                .phases
                .iter()
                .map(|(name, content)| SummaryRow {
                    name: name.clone(),
                    cell_style: SummaryCellStyle::Default,
                    value: content.clone(),
                })
                .collect(),
        };
        build_summary.rows.push(SummaryRow {
            name: "start".to_string(),
            cell_style: SummaryCellStyle::Default,
            value: vec![phase_info_desc.start],
        });
        build_summary.print();
    }

    let mut build_options = DockerBuilderOptions::default();
    build_options.platform = vec!["linux/amd64".to_string()];
    build_options.quiet = true;
    if disable_build_cache {
        build_options.no_cache = true;
    }
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
    disable_build_cache: bool,
) -> Result<String> {
    let Some(registry) = auth.get_registry() else {
        bail!("No registry found in auth");
    };

    if debug {
        message_detail("Building image with docker");
    }

    let id = uuid::Uuid::new_v4().to_string();

    let image = options.image.unwrap_or_else(|| {
        format!(
            "{}/{}/{}:{}",
            registry,
            tenant,
            options.name.unwrap_or(id),
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

    let mut cmd = Command::new("docker");
    cmd.env("DOCKER_AUTH_CONFIG", auth.to_json()?);
    cmd.args(&[
        "build",
        "--platform",
        "linux/amd64",
        "-t",
        &image,
        "-f",
        dockerfile_path.to_str().unwrap(),
        context.to_str().unwrap(),
    ]);

    if disable_build_cache {
        cmd.arg("--no-cache");
    }

    if debug {
        cmd.arg("--progress=plain");
        cmd.stdout(std::process::Stdio::inherit());
        cmd.stderr(std::process::Stdio::inherit());
    }

    if let Some(args) = options.args {
        for (key, value) in args {
            cmd.arg("--build-arg");
            cmd.arg(format!("{}={}", key, value));
        }
    }

    let output = cmd.output().await?;

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
