use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use ignition::{
    resource_index::Resources,
    resources::{
        app::{AppExpose, AppExposeExternal, AppExposeInternal, AppV1},
        gadget::{
            BuildPlanPhase, DirBuildPlan, DirBuildPlanArgs, GadgetClientMessage, GadgetClientReply,
            GadgetInitData, GadgetInitDiscoveryData, GadgetInitReasoningEffort,
            GadgetInitRunParams, GadgetServiceMessage, InitAppEnvValue, InitAppExposedPortMode,
            InitAppExposedPortProtocolExternal, InitAppImage, InitAppSnapshotStrategy,
            InitAppSource, ListDirArgs, ListDirItem, ListDirResult, ReadFileArgs, ReadFileResult,
        },
        machine::{
            MachineBuild, MachineBuildOptions, MachineDependency, MachineDockerOptions,
            MachineMode, MachineResources, MachineSnapshotStrategy, MachineVolumeBinding,
        },
        service::{ServiceBindExternalProtocol, ServiceTargetConnectionTracking},
        volume::{VolumeMode, VolumeV1},
    },
};
use serde_json::json;

use crate::{
    build::get_build_plan,
    client::get_api_client,
    config::Config,
    ui::message::{message_detail, message_error, message_info, message_warn},
};

#[derive(Args)]
pub struct GadgetInitArgs {
    #[arg(long = "debug")]
    debug: bool,

    /// How much reasoning effort should Gadget use when initializing your project
    #[arg(long = "reasoning-effort", short = 'r', value_enum)]
    reasoning_effort: Option<GadgetReasoningEffort>,
}

#[derive(ValueEnum, Clone, Copy)]
pub enum GadgetReasoningEffort {
    #[value(name = "minimal")]
    Minimal,
    #[value(name = "low")]
    Low,
    #[value(name = "medium")]
    Medium,
    #[value(name = "high")]
    High,
}

impl std::fmt::Display for GadgetReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GadgetReasoningEffort::Minimal => write!(f, "minimal"),
            GadgetReasoningEffort::Low => write!(f, "low"),
            GadgetReasoningEffort::Medium => write!(f, "medium"),
            GadgetReasoningEffort::High => write!(f, "high"),
        }
    }
}

impl From<GadgetReasoningEffort> for GadgetInitReasoningEffort {
    fn from(value: GadgetReasoningEffort) -> Self {
        match value {
            GadgetReasoningEffort::Minimal => GadgetInitReasoningEffort::Minimal,
            GadgetReasoningEffort::Low => GadgetInitReasoningEffort::Low,
            GadgetReasoningEffort::Medium => GadgetInitReasoningEffort::Medium,
            GadgetReasoningEffort::High => GadgetInitReasoningEffort::High,
        }
    }
}

pub async fn run_gadget_init(config: &Config, cmd: GadgetInitArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);

    let cwd = std::env::current_dir()?;
    let Some(base_dir_name) = cwd.file_name().map(|s| s.to_string_lossy().to_string()) else {
        bail!("gadget: failed to initialize project. no base directory name found");
    };

    message_info("Gadget is working on getting your project ready for lttle.cloud");
    match cmd.reasoning_effort {
        Some(GadgetReasoningEffort::Minimal) | None => {
            message_detail("Will use reasoning model with default reasoning effort")
        }
        Some(effort) => message_detail(format!(
            "Will use reasoning model with {} reasoning effort",
            effort.to_string(),
        )),
    }

    let mut run_count = 0;
    let mut client_messages = vec![];
    'agent: loop {
        run_count += 1;
        if run_count > 30 {
            bail!("gadget: failed to initialize project. depth limit exceeded");
        }

        if cmd.debug {
            message_detail(format!(
                "running agent iteration {} with {} messages",
                run_count,
                client_messages.len()
            ));
        }

        let response = api_client
            .gadget()
            .run_init(GadgetInitRunParams {
                reasoning_effort: cmd.reasoning_effort.map(Into::into),
                discovery_data: GadgetInitDiscoveryData {
                    base_dir_name: base_dir_name.clone(),
                    base_dir_build_plan: dir_build_plan(
                        config,
                        &base_dir_name,
                        &DirBuildPlanArgs {
                            path: ".".to_string(),
                        },
                    )
                    .await?,
                },
                messages: client_messages.clone(),
            })
            .await?;

        for message in response.messages {
            match &message {
                GadgetServiceMessage::Error(e) => {
                    bail!("gadget: {}", e);
                }
                GadgetServiceMessage::Finish(init_data) => {
                    let (uses_env_file, app_name, app_namespace) =
                        write_config_to_disk(init_data.clone()).await?;
                    write_editor_config().await?;

                    for warning in init_data.plan.warnings.iter() {
                        message_warn(&warning.message);
                    }

                    message_info("Gadget has finished initializing your project");
                    message_info("Next steps:");
                    if uses_env_file {
                        eprintln!("  → Setup and check your .env file");
                        eprintln!("  → Deploy your app with `lttle deploy --env <your env file>`");
                    } else {
                        eprintln!("  → Deploy your app with `lttle deploy`");
                    }

                    let app_get_cmd = match (app_name, app_namespace) {
                        (Some(name), Some(namespace)) => {
                            Some(format!("lttle app get {} --ns {}", name, namespace))
                        }
                        (Some(name), None) => Some(format!("lttle app get {}", name)),
                        (None, _) => None,
                    };

                    match app_get_cmd {
                        Some(cmd) => eprintln!("  → Check on your app with `{}`", cmd),
                        None => eprintln!("  → Check on your apps with `lttle app ls -a`"),
                    };

                    eprintln!("  → See more things you can do with `lttle --help`");

                    eprintln!("🎉 Happy clouding!");

                    break 'agent;
                }
                GadgetServiceMessage::ReadFile(args) => {
                    if cmd.debug {
                        message_detail(format!("reading file {}", args.path));
                    }

                    let reply = match read_files(config, &base_dir_name, args).await {
                        Ok(result) => GadgetClientReply::ReadFile(result),
                        Err(e) => GadgetClientReply::Error(e.to_string()),
                    };
                    client_messages.push(GadgetClientMessage {
                        service_message: message.clone(),
                        client_reply: Some(reply),
                    });
                }
                GadgetServiceMessage::ListDir(args) => {
                    if cmd.debug {
                        message_detail(format!(
                            "listing directory {} (depth = {})",
                            args.path,
                            args.max_depth.unwrap_or(1),
                        ));
                    }

                    let reply = match list_dir(config, &base_dir_name, args).await {
                        Ok(result) => GadgetClientReply::ListDir(result),
                        Err(e) => GadgetClientReply::Error(e.to_string()),
                    };
                    client_messages.push(GadgetClientMessage {
                        service_message: message.clone(),
                        client_reply: Some(reply),
                    });
                }
                GadgetServiceMessage::DirBuildPlan(args) => {
                    if cmd.debug {
                        message_detail(format!("getting directory build plan {}", args.path));
                    }

                    let reply = match dir_build_plan(config, &base_dir_name, args).await {
                        Ok(result) => GadgetClientReply::DirBuildPlan(result),
                        Err(e) => GadgetClientReply::Error(e.to_string()),
                    };
                    client_messages.push(GadgetClientMessage {
                        service_message: message.clone(),
                        client_reply: Some(reply),
                    });
                }
            }
        }
    }

    Ok(())
}

async fn read_files(
    _config: &Config,
    base_dir: &str,
    args: &ReadFileArgs,
) -> Result<ReadFileResult> {
    let cwd = std::env::current_dir()?;

    let rel = PathBuf::from(&args.path);
    let rel = rel
        .strip_prefix(base_dir)
        .unwrap_or(&rel)
        .to_string_lossy()
        .to_string();

    let path = cwd.join(&rel);

    let content = std::fs::read_to_string(&path)?;

    // path relative to base dir
    let path = path.strip_prefix(&cwd)?.to_string_lossy().to_string();

    Ok(ReadFileResult { path, content })
}

async fn list_dir(_config: &Config, base_dir: &str, args: &ListDirArgs) -> Result<ListDirResult> {
    let cwd = std::env::current_dir()?;

    let mut items = vec![];

    let rel = PathBuf::from(&args.path);
    let rel = rel
        .strip_prefix(base_dir)
        .unwrap_or(&rel)
        .to_string_lossy()
        .to_string();

    let path = cwd.join(&rel);

    fn list_dir(
        cwd: &PathBuf,
        path: impl AsRef<Path>,
        items: &mut Vec<ListDirItem>,
        depth: u32,
    ) -> Result<()> {
        let path = path.as_ref();
        let entries = std::fs::read_dir(path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            let is_dir = metadata.is_dir();

            // path relative to base dir
            let path_rel = path.strip_prefix(&cwd)?.to_string_lossy().to_string();

            items.push(ListDirItem {
                path: path_rel,
                is_dir,
                size: metadata.len(),
            });

            if is_dir && depth > 0 {
                list_dir(&cwd, &path, items, depth - 1)?;
            }
        }

        Ok(())
    }

    // max depth is 1 by default
    let depth = args.max_depth.unwrap_or(1).max(1);
    list_dir(&cwd, &path, &mut items, depth)?;

    Ok(ListDirResult { items })
}

async fn dir_build_plan(
    _config: &Config,
    base_dir: &str,
    args: &DirBuildPlanArgs,
) -> Result<DirBuildPlan> {
    let path = std::env::current_dir()?;
    let rel = PathBuf::from(&args.path);
    let rel = rel
        .strip_prefix(base_dir)
        .unwrap_or(&rel)
        .to_string_lossy()
        .to_string();

    let path = path.join(&rel);
    let (plan, providers) = get_build_plan(&path).await?;

    let descriptions = plan.get_phase_info_desc()?;

    Ok(DirBuildPlan {
        detected_providers: providers,
        phases: descriptions
            .phases
            .iter()
            .map(|(name, content)| BuildPlanPhase {
                name: name.clone(),
                build_info: content.join("\n"),
            })
            .collect(),
    })
}

fn remove_null_values(value: &mut serde_yaml::Value) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            // Remove entries where the value is null
            map.retain(|_, v| !v.is_null());

            // Recursively process remaining values
            for (_, v) in map.iter_mut() {
                remove_null_values(v);
            }

            // Remove entries where the value is an empty mapping after cleaning
            map.retain(|_, v| {
                if let serde_yaml::Value::Mapping(inner_map) = v {
                    !inner_map.is_empty()
                } else {
                    true
                }
            });
        }
        serde_yaml::Value::Sequence(seq) => {
            // Recursively process sequence elements
            for item in seq.iter_mut() {
                remove_null_values(item);
            }
            // Remove null items from sequences
            seq.retain(|item| !item.is_null());
        }
        _ => {
            // For other types (String, Number, Bool, Null), do nothing
        }
    }
}

async fn write_config_to_disk(
    init_data: GadgetInitData,
) -> Result<(bool, Option<String>, Option<String>)> {
    if !init_data.plan.issues.is_empty() {
        for issue in init_data.plan.issues {
            message_error(issue.message);
        }

        std::process::exit(1);
    }

    let cwd = std::env::current_dir()?;
    let config_path = cwd.join("lttle.yaml");

    let mut append_to_files = vec![];

    let mut resources = vec![];

    for volume in init_data.plan.volumes {
        resources.push(Resources::Volume(VolumeV1 {
            name: volume.name,
            namespace: volume.namespace,
            tags: None,
            mode: VolumeMode::Writeable,
            size: "100Mi".to_string(),
        }));
    }

    let mut uses_env_file = false;
    let mut app_name = None;
    let mut app_namespace = None;

    for app in init_data.plan.apps {
        if app_name.is_none() {
            app_name = Some(app.name.clone());
        }

        if app_namespace.is_none() {
            app_namespace = app.namespace.clone();
        }

        let mut app_v1 = AppV1 {
            name: app.name,
            namespace: app.namespace,
            tags: None,
            image: None,
            build: None,
            resources: MachineResources {
                cpu: 1,
                memory: 256,
            },
            command: None,
            depends_on: None,
            environment: None,
            expose: None,
            restart_policy: None,
            mode: None,
            volumes: None,
        };

        match app.source {
            InitAppSource::Image(InitAppImage { image }) => {
                app_v1.image = Some(image);
            }
            InitAppSource::BuildAutomatically(build) => {
                app_v1.build = Some(MachineBuild::NixpacksAuto);

                if let Some(dir_path) = build.dir_path {
                    if !dir_path.is_empty() && dir_path != "." {
                        app_v1.build = Some(MachineBuild::Nixpacks(MachineBuildOptions {
                            dir: Some(dir_path),
                            name: None,
                            tag: None,
                            image: None,
                        }));
                    }
                }

                if let Some(append_docker_ignore_extra) = build.append_docker_ignore_extra {
                    append_to_files.push((
                        append_docker_ignore_extra.path,
                        append_docker_ignore_extra.lines,
                    ));
                }
            }
            InitAppSource::BuildWithDockerfile(build) => {
                app_v1.build = Some(MachineBuild::Docker(MachineDockerOptions {
                    name: None,
                    tag: None,
                    image: None,
                    context: Some(build.dir_path),
                    dockerfile: build.dockerfile_name,
                    args: None,
                }));
            }
        }

        let mode = match app.snapshot_strategy {
            None => None,
            Some(InitAppSnapshotStrategy::SuspendManually) => Some(MachineMode::Flash {
                strategy: MachineSnapshotStrategy::Manual,
                timeout: None,
            }),
            Some(InitAppSnapshotStrategy::SuspendBeforeStart) => Some(MachineMode::Flash {
                strategy: MachineSnapshotStrategy::WaitForUserSpaceReady,
                timeout: None,
            }),
            Some(InitAppSnapshotStrategy::SuspendAfterListenOnAnyPort) => {
                Some(MachineMode::Flash {
                    strategy: MachineSnapshotStrategy::WaitForFirstListen,
                    timeout: None,
                })
            }
            Some(InitAppSnapshotStrategy::SuspendAfterListenOnPort(port)) => {
                Some(MachineMode::Flash {
                    strategy: MachineSnapshotStrategy::WaitForListenOnPort(port),
                    timeout: None,
                })
            }
        };

        app_v1.mode = mode;

        let mut envs = BTreeMap::new();
        for env in app.envs.unwrap_or_default() {
            let value = match env.value {
                InitAppEnvValue::Literal(value) => value,
                InitAppEnvValue::Expression(value) => value,
                InitAppEnvValue::CopyFromEnvFile { var_name } => {
                    format!("${{{{ env.{} }}}}", var_name)
                }
            };

            if value.contains("env.") {
                uses_env_file = true;
            }

            envs.insert(env.name, value);
        }
        if !envs.is_empty() {
            app_v1.environment = Some(envs);
        }

        let mut exposed_ports = BTreeMap::new();
        for exposed_port in app.exposed_ports.unwrap_or_default() {
            let mut app_expose = AppExpose {
                port: exposed_port.port,
                internal: None,
                connection_tracking: None,
                external: None,
            };

            match exposed_port.mode {
                InitAppExposedPortMode::Internal { .. } => {
                    app_expose.internal = Some(AppExposeInternal { port: None });
                    app_expose.connection_tracking =
                        Some(ServiceTargetConnectionTracking::TrafficAware {
                            inactivity_timeout: None,
                        });
                }
                InitAppExposedPortMode::External { protocol } => {
                    app_expose.external = Some(AppExposeExternal {
                        port: None,
                        protocol: match protocol {
                            InitAppExposedPortProtocolExternal::Tls => {
                                ServiceBindExternalProtocol::Tls
                            }
                            InitAppExposedPortProtocolExternal::Https => {
                                ServiceBindExternalProtocol::Https
                            }
                        },
                        host: None,
                    });
                }
            }

            exposed_ports.insert(exposed_port.name, app_expose);
        }
        if !exposed_ports.is_empty() {
            app_v1.expose = Some(exposed_ports);
        }

        let mut binded_volumes = vec![];
        for binded_volume in app.binded_volumes.unwrap_or_default() {
            binded_volumes.push(MachineVolumeBinding {
                name: binded_volume.name,
                namespace: binded_volume.namespace,
                path: binded_volume.path,
            });
        }
        if !binded_volumes.is_empty() {
            app_v1.volumes = Some(binded_volumes);
        }

        let mut dependencies = vec![];
        for dependency in app.depends_on.unwrap_or_default() {
            dependencies.push(MachineDependency {
                name: dependency.name,
                namespace: dependency.namespace,
            });
        }
        if !dependencies.is_empty() {
            app_v1.depends_on = Some(dependencies);
        }

        resources.push(Resources::App(app_v1));
    }

    let mut output = String::new();
    for resource in resources {
        let mut buf = Vec::new();
        let mut serializer = serde_yaml::Serializer::new(&mut buf);
        serde_yaml::with::singleton_map_recursive::serialize(&resource, &mut serializer).unwrap();

        // Parse YAML and remove null values
        let yaml_str = String::from_utf8(buf).unwrap();
        let mut yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml_str).unwrap();
        remove_null_values(&mut yaml_value);

        // Serialize back to clean YAML
        let clean_yaml = serde_yaml::to_string(&yaml_value).unwrap();
        output.push_str(&clean_yaml);
        output.push_str("---\n");
    }
    let output = output.trim().trim_end_matches("---").trim().to_string() + "\n";

    let config_path_exists = config_path.exists();
    std::fs::write(&config_path, output)?;
    if config_path_exists {
        message_warn("Overwriting lttle.yaml as it already exists");
    } else {
        message_info("Created lttle.yaml");
    }

    for (path, lines) in append_to_files {
        let original_path = path.clone();

        let path = cwd.join(path);
        let Some(dir) = path.parent() else {
            message_warn(format!(
                "Failed to get parent directory of {}",
                path.display()
            ));
            continue;
        };

        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
        }

        if !path.exists() {
            std::fs::write(&path, lines.join("\n"))?;
            message_info(format!("Created {}", original_path));
            continue;
        }

        let mut existing_content = std::fs::read_to_string(&path)?;
        for line in lines {
            if !existing_content.contains(&line) {
                existing_content.push_str(&line);
                existing_content.push_str("\n");
            }
        }

        std::fs::write(&path, existing_content)?;
        message_info(format!("Updated {}", original_path));
    }

    Ok((uses_env_file, app_name, app_namespace))
}

async fn write_vscode_settings() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let vscode_settings_dir = cwd.join(".vscode");
    if !vscode_settings_dir.exists() {
        std::fs::create_dir_all(&vscode_settings_dir)?;
    }

    let vscode_settings_file = vscode_settings_dir.join("settings.json");
    if !vscode_settings_file.exists() {
        std::fs::write(
            &vscode_settings_file,
            serde_json::to_string_pretty(&json!({
                "yaml.schemas": {
                    "https://resources.lttle.sh": "lttle.yaml"
                }
            }))?,
        )?;

        message_info("Created .vscode/settings.json");
        return Ok(());
    }

    let existing_content = std::fs::read_to_string(&vscode_settings_file)?;
    let mut existing_content = serde_json::from_str::<serde_json::Value>(&existing_content)?;
    let Some(existing_content) = existing_content.as_object_mut() else {
        message_warn("Failed to parse .vscode/settings.json");
        return Ok(());
    };

    let content = if let Some(serde_json::Value::Object(map)) = existing_content.get("yaml.schemas")
    {
        let mut map = map.clone();
        map.insert(
            "https://resources.lttle.sh".to_string(),
            serde_json::Value::String("lttle.yaml".to_string()),
        );
        serde_json::Value::Object(map)
    } else {
        json!({
            "yaml.schemas": {
                "https://resources.lttle.sh": "lttle.yaml"
            }
        })
    };
    existing_content.insert("yaml.schemas".to_string(), content);

    std::fs::write(
        vscode_settings_file,
        serde_json::to_string_pretty(&existing_content)?,
    )?;

    message_info("Updated .vscode/settings.json");
    Ok(())
}

async fn write_vscode_extensions() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let vscode_settings_dir = cwd.join(".vscode");
    if !vscode_settings_dir.exists() {
        std::fs::create_dir_all(&vscode_settings_dir)?;
    }

    let extensions_file = vscode_settings_dir.join("extensions.json");
    if !extensions_file.exists() {
        std::fs::write(
            extensions_file,
            serde_json::to_string_pretty(&json!({
                "recommendations": ["redhat.vscode-yaml"]
            }))?,
        )?;
        message_info("Created .vscode/extensions.json");
        return Ok(());
    }

    let existing_content = std::fs::read_to_string(&extensions_file)?;
    let mut existing_content = serde_json::from_str::<serde_json::Value>(&existing_content)?;
    let Some(existing_content) = existing_content.as_object_mut() else {
        message_warn("Failed to parse .vscode/extensions.json");
        return Ok(());
    };

    let mut recommendations = existing_content
        .get("recommendations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if !recommendations.contains(&serde_json::Value::String("redhat.vscode-yaml".to_string())) {
        recommendations.push(serde_json::Value::String("redhat.vscode-yaml".to_string()));
    }

    existing_content.insert(
        "recommendations".to_string(),
        serde_json::Value::Array(recommendations.clone()),
    );

    std::fs::write(
        extensions_file,
        serde_json::to_string_pretty(&existing_content)?,
    )?;

    message_info("Updated .vscode/extensions.json");
    Ok(())
}

async fn write_editor_config() -> Result<()> {
    write_vscode_settings().await?;
    write_vscode_extensions().await?;
    Ok(())
}
