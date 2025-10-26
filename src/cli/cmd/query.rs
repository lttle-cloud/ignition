use std::{collections::BTreeMap, path::PathBuf};

use anyhow::Result;
use clap::{ArgAction, Args};
use ignition::{
    eval::ctx::LttleInfo,
    resources::core::{QueryGitInfo, QueryParams},
};

use crate::{
    client::get_api_client,
    config::Config,
    expr::ctx::{EnvAmbientOverrideBehavior, ExprEvalContext, ExprEvalContextConfig},
    ui::message::message_warn,
};

#[derive(Args)]
pub struct QueryArgs {
    /// Environment file to use for the query
    #[arg(long = "env")]
    env_file: Option<PathBuf>,

    /// Variables file to use for the query
    #[arg(long = "vars")]
    var_file: Option<PathBuf>,

    /// Additional variables to use for the query
    #[arg(short = 'v', long = "var", value_name = "KEY=VALUE", action = ArgAction::Append)]
    additional_vars: Vec<String>,

    /// Disable environment variable ambient override
    #[arg(long = "no-env-ambient-override")]
    ignore_env_ambient_override: bool,

    /// Debug the expression evaluation context
    #[arg(long = "debug-context")]
    debug_context: bool,

    /// Dump the context to stdout as JSON
    #[arg(long = "dump-context-json")]
    dump_context_json: bool,

    /// Force the result to be JSON
    #[arg(long = "force-json-output")]
    force_json_output: bool,

    /// Expression to evaluate
    expression: String,
}

pub async fn run_query(config: &Config, args: QueryArgs) -> Result<()> {
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

    let context = ExprEvalContext::new(ExprEvalContextConfig {
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

    // TODO: This is a bit dirty
    let var: BTreeMap<String, serde_json::Value> =
        serde_json::from_value(serde_json::to_value(&context.var)?)?;

    let query_result = api_client
        .core()
        .query(QueryParams {
            query: args.expression,
            env: context.env.clone(),
            var,
            git: context.git.map(|g| QueryGitInfo {
                branch: g.branch.clone(),
                commit_sha: g.commit_sha.clone(),
                commit_message: g.commit_message.clone(),
                tag: g.tag.clone(),
                latest_tag: g.latest_tag.clone(),
                r#ref: g.r#ref.clone(),
            }),
            lttle_profile: config.current_profile.clone(),
        })
        .await?;

    if args.force_json_output {
        let json_str = serde_json::to_string_pretty(&query_result.query_result)?;
        println!("{}", json_str);
        return Ok(());
    }

    let value_str = match query_result.query_result {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.to_string(),
        serde_json::Value::Null => "null".to_string(),
        _ => serde_json::to_string_pretty(&query_result.query_result)?,
    };

    println!("{}", value_str);

    Ok(())
}
