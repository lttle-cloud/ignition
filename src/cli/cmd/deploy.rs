use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{ArgAction, Args};
use ignition::{
    api_client::ApiClient,
    resource_index::Resources,
    resources::{
        ProvideMetadata, certificate::Certificate, machine::Machine, metadata::Namespace,
        service::Service, volume::Volume,
    },
};
use serde::Deserialize;
use serde_yaml::{Mapping, Sequence, Value};
use tokio::fs::{read_dir, read_to_string};

use crate::{
    client::get_api_client,
    cmd::{
        certificate::CertificateSummary, machine::MachineSummary, service::ServiceSummary,
        volume::VolumeSummary,
    },
    config::Config,
    expr::{
        ctx::{EnvAmbientOverrideBehavior, ExprEvalContext, ExprEvalContextConfig},
        eval::eval_expr,
    },
    ui::message::{message_info, message_warn},
};

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

    /// Debug the expression evaluation context
    #[arg(long = "debug-context")]
    debug_context: bool,

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

    let context = ExprEvalContext::new(ExprEvalContextConfig {
        env_file: args.env_file,
        var_file: args.var_file,
        aditional_vars: Some(additional_vars),
        git_dir: std::env::current_dir()?,
        env_ambient_override_behavior: if args.ignore_env_ambient_override {
            EnvAmbientOverrideBehavior::Ignore
        } else {
            EnvAmbientOverrideBehavior::Override
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

    let Some(path) = args.path else {
        bail!("No path provided");
    };

    if !path.exists() {
        bail!("Path does not exist: {:?}", path);
    }

    let mut resources = Vec::new();
    if path.is_file() {
        let contents = read_to_string(&path).await?;
        parse_all_resources(&contents, &mut resources, &context).await?;
    } else if path.is_dir() {
        parse_all_resources_in_dir(&path, &mut resources, &context, args.recursive).await?;
    } else {
        bail!("Invalid path: {:?}", path);
    }

    for resource in resources {
        if let Ok(certificate) = resource.clone().try_into() {
            deploy_certificate(config, &api_client, certificate).await?;
            continue;
        }

        if let Ok(machine) = resource.clone().try_into() {
            deploy_machine(config, &api_client, machine).await?;
            continue;
        }

        if let Ok(service) = resource.clone().try_into() {
            deploy_service(config, &api_client, service).await?;
            continue;
        }

        if let Ok(volume) = resource.clone().try_into() {
            deploy_volume(config, &api_client, volume).await?;
            continue;
        }

        unreachable!("Unknown resource type: {:?}", resource);
    }

    Ok(())
}

fn parse_all_resources_in_dir<'a>(
    path: &'a PathBuf,
    resources: &'a mut Vec<Resources>,
    context: &'a ExprEvalContext,
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

                message_info(format!("Deploying file: {:?}", file.path()));
                let contents = read_to_string(file.path()).await?;
                parse_all_resources(&contents, resources, &context).await?;
            }

            if file.path().is_dir() && recursive {
                parse_all_resources_in_dir(&file.path(), resources, context, recursive).await?;
            }
        }

        Ok(())
    })
}

async fn parse_all_resources(
    contents: &str,
    resources: &mut Vec<Resources>,
    expr_eval_context: &ExprEvalContext,
) -> Result<()> {
    let de = serde_yaml::Deserializer::from_str(contents);
    for doc in de {
        let value: Value = Value::deserialize(doc)?;
        let value = eval_and_validate_resource(&value, expr_eval_context)?;
        resources.push(value);
    }

    Ok(())
}

fn eval_and_validate_resource(
    resource_src: &Value,
    context: &ExprEvalContext,
) -> Result<Resources> {
    fn transform_eval_expressions(value: &Value, context: &ExprEvalContext) -> Result<Value> {
        if let Some(str) = value.as_str() {
            let new_value = parse_and_eval_expr(str, context)?;
            return Ok(new_value.unwrap_or(value.clone()));
        }

        if let Some(map) = value.as_mapping() {
            let mut new_map = Mapping::new();
            for (key, value) in map {
                new_map.insert(key.clone(), transform_eval_expressions(value, context)?);
            }
            Ok(Value::Mapping(new_map))
        } else if let Some(seq) = value.as_sequence() {
            let mut new_seq = Sequence::new();
            for value in seq {
                new_seq.push(transform_eval_expressions(value, context)?);
            }
            Ok(Value::Sequence(new_seq))
        } else {
            Ok(value.clone())
        }
    }

    let value = transform_eval_expressions(resource_src, context)?;
    let resource: Resources = serde_yaml::with::singleton_map_recursive::deserialize(value)?;

    Ok(resource)
}

fn parse_and_eval_expr(expr: &str, context: &ExprEvalContext) -> Result<Option<Value>> {
    // either
    // 1. it starts with ${{ and ends with }} => we eval the expression and return the result as a value
    // 2. or it contains ${{ and }} => we eval the expression/s, convert the result to a string and replace in the original string
    // 3. or is just a regular string => we return the original string

    let expr = expr.trim();

    let expr_start_marker_count = expr.matches("${{").count();
    let expr_end_marker_count = expr.matches("}}").count();

    if expr_start_marker_count == 0 && expr_end_marker_count == 0 {
        return Ok(None);
    }

    if expr.starts_with("${{")
        && expr.ends_with("}}")
        && expr_start_marker_count == 1
        && expr_end_marker_count == 1
    {
        let expr = expr
            .trim_start_matches("${{")
            .trim_end_matches("}}")
            .trim()
            .to_string();

        return eval_expr(&expr, context).map(|v| Some(v));
    }

    // loop should be find, split, eval, replace, repeat\
    let mut output = expr.to_string();
    loop {
        let start = output.find("${{").unwrap_or(0);
        let end = output.find("}}").unwrap_or(0);

        if start == 0 && end == 0 {
            break;
        }

        let expr = output[start + 3..end - 1].trim();

        if expr.is_empty() {
            break;
        }

        let value = eval_expr(&expr, context)?;
        let value_str = match value {
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.to_string(),
            Value::Null => "null".to_string(),
            _ => bail!(
                "Invalid value '{:?}' returned by expression '{}'",
                value,
                expr
            ),
        };
        output = output[..start].to_string() + &value_str + &output[end + 2..];
    }

    return Ok(Some(Value::String(output)));
}

async fn deploy_machine(_config: &Config, api_client: &ApiClient, machine: Machine) -> Result<()> {
    let metadata = machine.metadata();
    api_client.machine().apply(machine).await?;

    let (machine, status) = api_client
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

    let summary = MachineSummary::from((machine, status));
    summary.print();

    Ok(())
}

async fn deploy_certificate(
    _config: &Config,
    api_client: &ApiClient,
    certificate: Certificate,
) -> Result<()> {
    let metadata = certificate.metadata();
    api_client.certificate().apply(certificate).await?;

    let (certificate, status) = api_client
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

    let summary = CertificateSummary::from((certificate, status));
    summary.print();

    Ok(())
}

async fn deploy_service(_config: &Config, api_client: &ApiClient, service: Service) -> Result<()> {
    let metadata = service.metadata();
    api_client.service().apply(service).await?;

    let (service, status) = api_client
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

    let summary = ServiceSummary::from((service, status));
    summary.print();

    Ok(())
}

async fn deploy_volume(_config: &Config, api_client: &ApiClient, volume: Volume) -> Result<()> {
    let metadata = volume.metadata();
    api_client.volume().apply(volume).await?;

    let (volume, status) = api_client
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

    let summary = VolumeSummary::from((volume, status));
    summary.print();

    Ok(())
}
