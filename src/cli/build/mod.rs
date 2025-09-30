pub mod docker_auth;

use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use ignition::{
    api_client::ApiClient,
    resources::machine::{MachineBuild, MachineBuildOptions, MachineDockerOptions},
};
use nixpacks::nixpacks::{
    builder::docker::DockerBuilderOptions,
    plan::{BuildPlan, generator::GeneratePlanOptions},
};
use tempfile::TempDir;
use tokio::{fs::create_dir_all, process::Command};

use crate::{
    build::docker_auth::DockerAuthConfig,
    ui::{
        message::message_detail,
        summary::{Summary, SummaryCellStyle, SummaryRow},
    },
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const BUILDCTL_BINARY: &[u8] = include_bytes!("../../../bins/buildctl_linux_amd64");

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const BUILDCTL_BINARY: &[u8] = include_bytes!("../../../bins/buildctl_darwin_arm64");

#[derive(Debug, Clone)]
pub enum BuildTarget {
    Local,
    Remote,
}

impl BuildTarget {
    pub async fn preferred() -> Self {
        // if docker is not installed, prefer remote
        match Command::new("docker").arg("--version").output().await {
            Err(_) => return Self::Remote,
            Ok(output) if !output.status.success() => return Self::Remote,
            _ => {}
        };

        // if platform is not linux/amd64, prefer remote
        if std::env::consts::OS != "linux" || std::env::consts::ARCH != "x86_64" {
            return Self::Remote;
        }

        Self::Local
    }
}

pub async fn build_and_push_image(
    api_client: &ApiClient,
    dir: impl AsRef<Path>,
    tenant: &str,
    build: MachineBuild,
    auth: DockerAuthConfig,
    debug: bool,
    disable_build_cache: bool,
    force_build_target: Option<BuildTarget>,
) -> Result<String> {
    let build_target = if let Some(force_build_target) = force_build_target {
        force_build_target
    } else {
        BuildTarget::preferred().await
    };

    match build_target {
        BuildTarget::Local => {
            let image =
                local_build_image(dir, tenant, build, auth.clone(), debug, disable_build_cache)
                    .await?;
            message_detail(format!("Built image {}", image));
            message_detail(format!("Pushing image {}", image));
            push_image(image.clone(), auth).await?;
            Ok(image)
        }
        BuildTarget::Remote => {
            let image = remote_build_and_push_image(
                api_client,
                dir,
                tenant,
                build,
                auth,
                debug,
                disable_build_cache,
            )
            .await?;
            Ok(image)
        }
    }
}

#[derive(Debug)]
struct RemoteBuildContext {
    #[allow(dead_code)]
    pub temp_dir: Option<TempDir>,
    pub image: String,
    pub build_args: BTreeMap<String, String>,
    pub context_dir: PathBuf,
    pub docker_file_path: PathBuf,
}

async fn remote_build_and_push_image(
    api_client: &ApiClient,
    dir: impl AsRef<Path>,
    tenant: &str,
    build: MachineBuild,
    auth: DockerAuthConfig,
    debug: bool,
    disable_build_cache: bool,
) -> Result<String> {
    message_detail("Building image remotely");

    let builder = api_client.core().alloc_builder().await?;

    let remote_build_context = match build {
        MachineBuild::Nixpacks(options) => {
            get_remote_build_context_nixpacks(
                dir,
                tenant,
                options,
                auth.clone(),
                debug,
                disable_build_cache,
            )
            .await?
        }
        MachineBuild::Docker(options) => {
            get_remote_build_context_docker(dir, tenant, auth.clone(), options, debug).await?
        }
        MachineBuild::NixpacksAuto => {
            get_remote_build_context_nixpacks(
                dir,
                tenant,
                MachineBuildOptions {
                    dir: None,
                    image: None,
                    tag: None,
                    name: None,
                },
                auth.clone(),
                debug,
                disable_build_cache,
            )
            .await?
        }
    };

    let buildctl_path = ensure_buildctl_binary().await?;

    let certs_dir = tempfile::tempdir()?;
    let certs_dir_path = certs_dir.path().to_path_buf();

    let (ca_cert, client_cert, client_key) = (
        certs_dir_path.join("ca.cert"),
        certs_dir_path.join("client.cert"),
        certs_dir_path.join("client.key"),
    );

    tokio::fs::write(&ca_cert, builder.ca_cert_pem).await?;
    tokio::fs::write(&client_cert, builder.client_cert_pem).await?;
    tokio::fs::write(&client_key, builder.client_key_pem).await?;

    let Some(registry) = auth.get_registry() else {
        bail!("No registry found in auth");
    };

    let cache_ref = format!("{}/{}/lttle-build-cache:bk", registry, tenant);

    let mut buildkit_args = vec![
        "--addr".to_string(),
        format!("tcp://{}:1234", builder.host),
        "--tlscacert".to_string(),
        ca_cert.to_string_lossy().to_string(),
        "--tlscert".to_string(),
        client_cert.to_string_lossy().to_string(),
        "--tlskey".to_string(),
        client_key.to_string_lossy().to_string(),
        "build".to_string(),
        "--frontend=dockerfile.v0".to_string(),
        "--local".to_string(),
        format!(
            "context={}",
            remote_build_context.context_dir.to_str().unwrap()
        ),
        "--local".to_string(),
        format!(
            "dockerfile={}",
            remote_build_context.docker_file_path.to_str().unwrap()
        ),
        "--opt".to_string(),
        "platform=linux/amd64".to_string(),
    ];

    for (key, value) in remote_build_context.build_args {
        buildkit_args.push("--opt".to_string());
        buildkit_args.push(format!("build-arg:{}={}", key, value));
    }

    if !disable_build_cache {
        // TODO: enable this when we need multi-builder support
        // buildkit_args.extend(vec![
        //     "--import-cache".to_string(),
        //     format!("type=registry,ref={}", cache_ref),
        //     "--export-cache".to_string(),
        //     format!("type=registry,ref={},mode=min", cache_ref),
        // ]);
    } else {
        buildkit_args.extend(vec!["--no-cache".to_string()]);
    }

    buildkit_args.extend(vec![
        "--output".to_string(),
        format!(
            "type=image,name={},push=true,compression=gzip,compression-level=1,oci-mediatypes=true",
            remote_build_context.image
        ),
    ]);

    if debug {
        message_detail("Buildkit args: ");
        for arg in buildkit_args.iter() {
            println!("{}", arg);
        }
    }

    let mut cmd = Command::new(buildctl_path);
    cmd.env("DOCKER_AUTH_CONFIG", auth.to_json()?);
    cmd.args(&buildkit_args);

    if debug {
        cmd.arg("--progress=plain");
        cmd.stdout(std::process::Stdio::inherit());
        cmd.stderr(std::process::Stdio::inherit());
    } else {
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
    }

    let status = cmd.status().await?;

    if !status.success() {
        bail!(
            "Failed to build image. Run the same command with --debug-build to see the full output"
        );
    }

    Ok(remote_build_context.image)
}

async fn ensure_buildctl_binary() -> Result<String> {
    let Some(project_dirs) = directories::ProjectDirs::from("cloud", "lttle", "lttle") else {
        bail!("Failed to get cache dir");
    };

    let cache_dir = project_dirs.cache_dir();
    if !cache_dir.exists() {
        create_dir_all(&cache_dir).await?;
    }
    let buildctl_path = cache_dir.join("buildctl");
    tokio::fs::write(&buildctl_path, BUILDCTL_BINARY).await?;
    std::fs::set_permissions(&buildctl_path, std::fs::Permissions::from_mode(0o755))?;

    Ok(buildctl_path.to_string_lossy().to_string())
}

async fn get_remote_build_context_nixpacks(
    dir: impl AsRef<Path>,
    tenant: &str,
    options: MachineBuildOptions,
    auth: DockerAuthConfig,
    debug: bool,
    disable_build_cache: bool,
) -> Result<RemoteBuildContext> {
    let out_dir = tempfile::tempdir()?;

    let out_dir_path = out_dir.path().to_path_buf();

    let (plan, _) = get_build_plan(dir.as_ref()).await?;

    let build_args = if let Some(args) = serde_json::to_value(&plan)?.get("variables") {
        serde_json::from_value(args.clone())?
    } else {
        BTreeMap::new()
    };

    let image = build_image_nixpacks(
        dir,
        tenant,
        options,
        auth,
        debug,
        disable_build_cache,
        Some(out_dir_path.to_string_lossy().to_string()),
    )
    .await?;

    let docker_file_path = out_dir.path().join(".nixpacks");

    Ok(RemoteBuildContext {
        temp_dir: Some(out_dir),
        image,
        build_args,
        context_dir: out_dir_path,
        docker_file_path,
    })
}

async fn get_remote_build_context_docker(
    dir: impl AsRef<Path>,
    tenant: &str,
    auth: DockerAuthConfig,
    options: MachineDockerOptions,
    debug: bool,
) -> Result<RemoteBuildContext> {
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

    let dockerfile_dir = dockerfile_path.parent().unwrap().to_path_buf();

    if !dockerfile_path.exists() {
        bail!("Dockerfile not found");
    }

    let args = options.args.unwrap_or_default();
    Ok(RemoteBuildContext {
        temp_dir: None,
        image,
        build_args: args,
        context_dir: context,
        docker_file_path: dockerfile_dir,
    })
}

async fn local_build_image(
    dir: impl AsRef<Path>,
    tenant: &str,
    build: MachineBuild,
    auth: DockerAuthConfig,
    debug: bool,
    disable_build_cache: bool,
) -> Result<String> {
    let image = match build {
        MachineBuild::Nixpacks(options) => {
            build_image_nixpacks(dir, tenant, options, auth, debug, disable_build_cache, None).await
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
                None,
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
    target_out_dir: Option<String>,
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

    build_options.out_dir = target_out_dir.clone();

    if debug && target_out_dir.is_none() {
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

pub async fn get_build_plan(path: impl AsRef<Path>) -> Result<(BuildPlan, Vec<String>)> {
    let envs = std::env::vars()
        .into_iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<String>>();

    let envs = envs.iter().map(|s| s.as_str()).collect::<Vec<&str>>();

    let mut plan_options = GeneratePlanOptions::default();
    // check if dir or pwd contains a nixpacks.toml
    let dir_nixpacks_toml = path.as_ref().join("nixpacks.toml");
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

    let path_str = path.as_ref().to_str().unwrap();

    let providers = nixpacks::get_plan_providers(path_str, envs.clone(), &plan_options)?;
    let plan = nixpacks::generate_build_plan(path_str, envs.clone(), &plan_options)?;

    Ok((plan, providers))
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
    } else {
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
    }

    if let Some(args) = options.args {
        for (key, value) in args {
            cmd.arg("--build-arg");
            cmd.arg(format!("{}={}", key, value));
        }
    }

    let status = cmd.status().await?;

    if !status.success() {
        bail!(
            "Failed to build image. Run the same command with --debug-build to see the full output"
        );
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
