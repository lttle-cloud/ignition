use std::path::PathBuf;

use ansi_term::{Color, Style};
use anyhow::{Result, bail};
use clap::{ArgAction, Args};
use ignition::{
    api_client::ApiClient,
    eval::ctx::LttleInfo,
    resource_index::Resources,
    resources::{
        ProvideMetadata,
        app::App,
        certificate::Certificate,
        machine::Machine,
        metadata::{Metadata, Namespace},
        service::Service,
        volume::Volume,
    },
};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use tokio::fs::{read_dir, read_to_string};

use crate::{
    build::{build_image, docker_auth::DockerAuthConfig, push_image},
    client::get_api_client,
    config::Config,
    expr::{
        ctx::{EnvAmbientOverrideBehavior, ExprEvalContext, ExprEvalContextConfig},
        eval::{eval_expr, transform_eval_expressions_root},
    },
    ui::message::{message_detail, message_info, message_warn},
};

/// Find deployment path using fallback logic
fn find_deployment_path(provided_path: Option<PathBuf>) -> Result<PathBuf> {
    // If path was provided, try to use it
    if let Some(path) = provided_path {
        if !path.exists() {
            bail!("Provided path does not exist: {:?}", path);
        }

        return Ok(path);
    }

    // Check fallback paths in order:
    // 1. .lttle/deploy directory
    let lttle_deploy_dir = PathBuf::from(".lttle/deploy");
    if lttle_deploy_dir.exists() && lttle_deploy_dir.is_dir() {
        return Ok(lttle_deploy_dir);
    }

    // 2. .lttle/deploy.yaml or .lttle/deploy.yml
    let lttle_deploy_yaml = PathBuf::from(".lttle/deploy.yaml");
    if lttle_deploy_yaml.exists() {
        return Ok(lttle_deploy_yaml);
    }
    let lttle_deploy_yml = PathBuf::from(".lttle/deploy.yml");
    if lttle_deploy_yml.exists() {
        return Ok(lttle_deploy_yml);
    }

    // 3. .lttle/ directory
    let lttle_dir = PathBuf::from(".lttle");
    if lttle_dir.exists() && lttle_dir.is_dir() {
        return Ok(lttle_dir);
    }

    // 4. lttle.yaml or lttle.yml at root
    let lttle_yaml = PathBuf::from("lttle.yaml");
    if lttle_yaml.exists() {
        return Ok(lttle_yaml);
    }
    let lttle_yml = PathBuf::from("lttle.yml");
    if lttle_yml.exists() {
        return Ok(lttle_yml);
    }

    // 5. app.lttle.yaml
    let app_lttle_yaml = PathBuf::from("app.lttle.yaml");
    if app_lttle_yaml.exists() {
        return Ok(app_lttle_yaml);
    }

    // If none of the fallback paths exist, suggest initialization
    bail!(
        "No deployment configuration found. Please create a valid deployment configuration or use `lttle gadget init` to automatically initialize your project to use lttle.cloud"
    );
}

#[derive(Args)]
pub struct DeployArgs {
    /// Environment file to use for the deployment
    #[arg(long = "env")]
    env_file: Option<PathBuf>,

    /// Variables file to use for the deployment
    #[arg(long = "vars")]
    var_file: Option<PathBuf>,

    /// Additional variables to use for the deployment
    #[arg(short = 'v', long = "var", value_name = "KEY=VALUE", action = ArgAction::Append)]
    additional_vars: Vec<String>,

    /// Disable environment variable ambient override
    #[arg(long = "no-env-ambient-override")]
    ignore_env_ambient_override: bool,

    /// Recursively parse all files in the directory
    #[arg(short = 'r', long = "recursive")]
    recursive: bool,

    /// Debug the build process
    #[arg(long = "debug-build")]
    debug_build: bool,

    /// Disable the build cache
    #[arg(long = "no-build-cache")]
    disable_build_cache: bool,

    /// Debug the expression evaluation context
    #[arg(long = "debug-context")]
    debug_context: bool,

    /// Print the changes that would be committed without applying them
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Dump the context to stdout as JSON
    #[arg(long = "dump-context-json")]
    dump_context_json: bool,

    /// Evaluate the expression and print the result
    #[arg(long = "eval", value_name = "EXPRESSION")]
    eval: Option<String>,

    /// Path to the deployment file/directory
    path: Option<PathBuf>,
}

pub async fn run_deploy(config: &Config, args: DeployArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);

    let me = api_client.core().me().await?;
    let profile = config.current_profile.clone();

    let additional_vars = args
        .additional_vars
        .iter()
        .filter_map(|v| {
            let parts: Vec<&str> = v.split('=').collect();
            if parts.len() != 2 {
                message_warn(format!("Invalid variable: {}", v));
                return None;
            }
            Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
        })
        .collect();

    let mut context = ExprEvalContext::new(ExprEvalContextConfig {
        env_file: args.env_file,
        var_file: args.var_file,
        initial_vars: None,
        aditional_vars: Some(additional_vars),
        git_dir: std::env::current_dir()?,
        env_ambient_override_behavior: if args.ignore_env_ambient_override {
            EnvAmbientOverrideBehavior::Ignore
        } else {
            EnvAmbientOverrideBehavior::Override
        },
        lttle_info: LttleInfo {
            tenant: me.tenant,
            user: me.sub,
            profile: profile,
        },
    })
    .await?;

    if args.debug_context {
        let dbg_str = format!("{:#?}", context).trim().to_string();
        println!("{}", dbg_str);
        return Ok(());
    }

    if args.dump_context_json {
        let json_str = serde_json::to_string_pretty(&context)?;
        println!("{}", json_str);
        return Ok(());
    }

    if let Some(eval) = args.eval {
        let value = eval_expr(&eval, &context)?;
        let value_str = match value {
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.to_string(),
            Value::Null => "null".to_string(),
            _ => bail!(
                "Invalid value '{:?}' returned by expression '{}'",
                value,
                &eval
            ),
        };
        println!("{}", value_str);
        return Ok(());
    }

    let path = find_deployment_path(args.path)?;

    if args.dry_run {
        message_info("Dry run mode enabled. No changes will be committed.");
    }

    let mut resources = Vec::new();
    if path.is_file() {
        let contents = read_to_string(&path).await?;
        parse_all_resources(path, &contents, &mut resources, &mut context).await?;
    } else if path.is_dir() {
        parse_all_resources_in_dir(&path, &mut resources, &mut context, args.recursive).await?;
    } else {
        bail!("Invalid path: {:?}", path);
    }

    let me = api_client.core().me().await?;

    let registry_robot = api_client.core().get_registry_robot().await?;
    let auth = DockerAuthConfig::internal(
        &registry_robot.registry,
        &registry_robot.user,
        &registry_robot.pass,
    );

    for (path, resource) in resources.iter_mut() {
        let (resource_name, mut_image, mut_build) = match resource {
            Resources::Machine(machine) => (
                machine.metadata().to_string(),
                &mut machine.image,
                &mut machine.build,
            ),
            Resources::MachineV1(machine) => (
                machine.metadata().to_string(),
                &mut machine.image,
                &mut machine.build,
            ),
            Resources::App(app) => (app.metadata().to_string(), &mut app.image, &mut app.build),
            Resources::AppV1(app) => (app.metadata().to_string(), &mut app.image, &mut app.build),
            _ => continue,
        };

        let Some(build) = mut_build.clone() else {
            continue;
        };

        let Some(dir) = path.parent() else {
            bail!("No parent directory for path: {:?}", path);
        };

        message_detail(format!("Building image for {}", resource_name));
        let image = build_image(
            dir,
            &me.tenant,
            build,
            auth.clone(),
            args.debug_build,
            args.disable_build_cache,
        )
        .await?;
        message_detail(format!("Pushing image for {} → {}", resource_name, image));
        push_image(image.clone(), auth.clone()).await?;
        message_info(format!(
            "Successfully built and pushed image for {}",
            resource_name
        ));

        *mut_build = None;
        *mut_image = Some(image);
    }

    for (_path, resource) in resources {
        match resource {
            Resources::Certificate(certificate) | Resources::CertificateV1(certificate) => {
                if args.dry_run {
                    deploy_dry_run::<Certificate>(
                        config,
                        &api_client,
                        "certificate",
                        certificate.metadata(),
                        certificate.into(),
                    )?;
                    continue;
                }
                deploy_certificate(config, &api_client, certificate.into()).await?;
            }
            Resources::App(app) | Resources::AppV1(app) => {
                if args.dry_run {
                    deploy_dry_run::<App>(config, &api_client, "app", app.metadata(), app.into())?;
                    continue;
                }
                deploy_app(config, &api_client, app.into()).await?;
            }
            Resources::Machine(machine) | Resources::MachineV1(machine) => {
                if args.dry_run {
                    deploy_dry_run::<Machine>(
                        config,
                        &api_client,
                        "machine",
                        machine.metadata(),
                        machine.into(),
                    )?;
                    continue;
                }

                deploy_machine(config, &api_client, machine.into()).await?;
            }
            Resources::Service(service) | Resources::ServiceV1(service) => {
                if args.dry_run {
                    deploy_dry_run::<Service>(
                        config,
                        &api_client,
                        "service",
                        service.metadata(),
                        service.into(),
                    )?;
                    continue;
                }
                deploy_service(config, &api_client, service.into()).await?;
            }
            Resources::Volume(volume) | Resources::VolumeV1(volume) => {
                if args.dry_run {
                    deploy_dry_run::<Volume>(
                        config,
                        &api_client,
                        "volume",
                        volume.metadata(),
                        volume.into(),
                    )?;
                    continue;
                }
                deploy_volume(config, &api_client, volume.into()).await?;
            }
        };
    }

    Ok(())
}

fn parse_all_resources_in_dir<'a>(
    path: &'a PathBuf,
    resources: &'a mut Vec<(PathBuf, Resources)>,
    context: &'a mut ExprEvalContext,
    recursive: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
    Box::pin(async move {
        let mut files = read_dir(path).await?;
        while let Some(file) = files.next_entry().await? {
            if file.path().is_file() {
                let path = file.path();
                let base_name = path.file_stem().unwrap_or_default();
                let extension = path.extension().unwrap_or_default();
                if (extension != "yaml" && extension != "yml")
                    || base_name.to_string_lossy().starts_with("_")
                {
                    continue;
                }

                let contents = read_to_string(file.path()).await?;
                parse_all_resources(file.path(), &contents, resources, context).await?;
            }

            if file.path().is_dir() && recursive {
                parse_all_resources_in_dir(&file.path(), resources, context, recursive).await?;
            }
        }

        Ok(())
    })
}

async fn parse_all_resources(
    path: PathBuf,
    contents: &str,
    resources: &mut Vec<(PathBuf, Resources)>,
    expr_eval_context: &mut ExprEvalContext,
) -> Result<()> {
    let de = serde_yaml::Deserializer::from_str(contents);
    for doc in de {
        let value: Value = Value::deserialize(doc)?;
        let value = eval_and_validate_resource(&value, expr_eval_context)?;
        resources.push((path.clone(), value));
    }

    Ok(())
}

fn eval_and_validate_resource(
    resource_src: &Value,
    context: &mut ExprEvalContext,
) -> Result<Resources> {
    let value = transform_eval_expressions_root(resource_src, context)?;
    let resource: Resources = serde_yaml::with::singleton_map_recursive::deserialize(value)?;

    Ok(resource)
}

async fn deploy_machine(_config: &Config, api_client: &ApiClient, machine: Machine) -> Result<()> {
    let metadata = machine.metadata();
    api_client.machine().apply(machine).await?;

    let (machine, _status) = api_client
        .machine()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed machine: {}",
        machine.metadata().to_string()
    ));

    Ok(())
}

async fn deploy_certificate(
    _config: &Config,
    api_client: &ApiClient,
    certificate: Certificate,
) -> Result<()> {
    let metadata = certificate.metadata();
    api_client.certificate().apply(certificate).await?;

    let (certificate, _status) = api_client
        .certificate()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed certificate: {}",
        certificate.metadata().to_string()
    ));

    Ok(())
}

async fn deploy_service(_config: &Config, api_client: &ApiClient, service: Service) -> Result<()> {
    let metadata = service.metadata();
    api_client.service().apply(service).await?;

    let (service, _status) = api_client
        .service()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed service: {}",
        service.metadata().to_string()
    ));

    Ok(())
}

async fn deploy_volume(_config: &Config, api_client: &ApiClient, volume: Volume) -> Result<()> {
    let metadata = volume.metadata();
    api_client.volume().apply(volume).await?;

    let (volume, _status) = api_client
        .volume()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed volume: {}",
        volume.metadata().to_string()
    ));

    Ok(())
}

async fn deploy_app(_config: &Config, api_client: &ApiClient, app: App) -> Result<()> {
    let metadata = app.metadata();
    api_client.app().apply(app).await?;

    let (app, _status) = api_client
        .app()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed app: {}",
        app.metadata().to_string()
    ));

    Ok(())
}

fn deploy_dry_run<T: Serialize>(
    _config: &Config,
    _api_client: &ApiClient,
    resource_type_name: &'static str,
    metadata: Metadata,
    resource: T,
) -> Result<()> {
    let resource = serde_yaml::to_string(&resource)?;

    let type_style = Style::new().fg(Color::Yellow);
    let metadata_style = Style::new().bold().fg(Color::Blue);

    eprintln!(
        "→ {} {} as: \n{}",
        type_style.paint(resource_type_name),
        metadata_style.paint(metadata.to_string()),
        resource
    );

    Ok(())
}
