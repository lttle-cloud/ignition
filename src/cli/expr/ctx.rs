use std::{collections::BTreeMap, fmt::Debug, path::PathBuf};

use anyhow::{Result, bail};
use git2::Repository;
use ignition::eval::{
    CelCtxExt,
    ctx::{GitInfo, LttleInfo},
};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use tokio::fs::read_to_string;

use crate::{expr::eval::transform_eval_expressions, ui::message::message_warn};

#[derive(Clone)]
pub struct ExprEvalContextConfig {
    pub env_file: Option<PathBuf>,
    pub var_file: Option<PathBuf>,
    // needs parsing
    pub initial_vars: Option<BTreeMap<String, Value>>,
    pub aditional_vars: Option<BTreeMap<String, String>>,
    pub git_dir: PathBuf,
    pub env_ambient_override_behavior: EnvAmbientOverrideBehavior,
    pub lttle_info: LttleInfo,
}

#[derive(PartialEq, Eq, Clone)]
pub enum EnvAmbientOverrideBehavior {
    /// If the environment variable is already set, it will be overridden by the value from the file
    Override,
    /// If the environment variable is already set, it will not be overridden
    Ignore,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ExprEvalContext {
    pub env: BTreeMap<String, String>,
    pub var: BTreeMap<String, Value>,
    pub git: Option<GitInfo>,
    pub lttle: LttleInfo,
    pub namespace: Option<String>,
}

impl Debug for ExprEvalContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("")
            .field("env", &self.env)
            .field("var", &self.var)
            .field("git", &self.git)
            .field("lttle", &self.lttle)
            .field("namespace", &self.namespace)
            .finish()
    }
}

impl ExprEvalContext {
    pub async fn new(config: ExprEvalContextConfig) -> Result<Self> {
        let mut envs = BTreeMap::new();
        if let Some(env_file) = config.env_file.clone() {
            let mut iter = dotenvy::from_filename_iter(env_file)?;
            while let Some(line) = iter.next() {
                let Ok((key, val)) = line else {
                    bail!("Failed to parse env: {:?}", line);
                };

                envs.insert(key, val);
            }
        };
        // also add env ambient variables
        for (key, val) in std::env::vars() {
            if envs.contains_key(&key) {
                if config.env_ambient_override_behavior == EnvAmbientOverrideBehavior::Ignore {
                    continue;
                }

                message_warn(format!(
                    "Environment variable '{key}' exists in env file but was overridden by ambient environment variable",
                ));
            }
            envs.insert(key, val);
        }

        let mut vars: BTreeMap<String, Value> = config.initial_vars.clone().unwrap_or_default();
        if let Some(var_file) = config.var_file.clone() {
            let contents = read_to_string(var_file).await?;
            let value = serde_yaml::from_str(&contents)?;

            let mut vars_eval_ctx_config = config.clone();
            vars_eval_ctx_config.var_file = None;
            vars_eval_ctx_config.initial_vars =
                Some(extract_top_level_vars_without_expressions(&value)?);

            let vars_eval_ctx = Box::pin(ExprEvalContext::new(vars_eval_ctx_config)).await?;

            let value = transform_eval_expressions(&value, &vars_eval_ctx)?;
            vars = serde_yaml::from_value(value)?;
        }

        for (key, val) in config.aditional_vars.unwrap_or_default() {
            if vars.contains_key(&key) {
                message_warn(format!(
                    "Variable '{key}' exists in var file but was overridden by set variable",
                ));
            }

            let value = serde_yaml::from_str(&val)?;
            vars.insert(key, value);
        }

        // git info
        let git_info = match get_git_info(config.git_dir) {
            Ok(git_info) => Some(git_info),
            Err(_e) => {
                message_warn(format!("failed to load git info for current directory"));
                None
            }
        };

        Ok(Self {
            env: envs,
            var: vars,
            git: git_info,
            lttle: config.lttle_info,
            namespace: None,
        })
    }
}

fn get_git_info(git_dir: PathBuf) -> Result<GitInfo> {
    let repo = Repository::open(git_dir)?;

    // Get current HEAD commit
    let head = repo.head()?;
    let commit = head.peel_to_commit()?;
    let commit_sha = commit.id().to_string();
    let commit_message = commit.message().unwrap_or("").to_string();

    // Get current branch name (None if detached HEAD)
    let branch = if head.is_branch() {
        head.shorthand().map(|s| s.to_string())
    } else {
        // maybe we are in github actions
        if let Ok(github_head_ref) = std::env::var("GITHUB_HEAD_REF") {
            // PRs: source branch
            Some(github_head_ref)
        } else if let Ok(github_ref_name) = std::env::var("GITHUB_REF_NAME") {
            // Pushes/manual triggers with a branch name
            if let Ok(github_ref_type) = std::env::var("GITHUB_REF_TYPE") {
                if github_ref_type == "branch" {
                    Some(github_ref_name)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    // Get tag if current commit is directly tagged
    let tag = {
        let mut found_tag = None;
        let current_oid = commit.id();

        repo.tag_foreach(|oid, name| {
            if oid == current_oid {
                let name_str = std::str::from_utf8(name).unwrap();
                if let Some(tag_name) = name_str.strip_prefix("refs/tags/") {
                    found_tag = Some(tag_name.to_string());
                    return false; // Stop iteration
                }
            }
            true // Continue iteration
        })
        .ok();

        found_tag
    };

    // Get the most recent tag (by commit date)
    let latest_tag = {
        let mut latest_tag_info: Option<(String, i64)> = None;

        repo.tag_foreach(|oid, name| {
            let name_str = std::str::from_utf8(name).unwrap();
            if let Some(tag_name) = name_str.strip_prefix("refs/tags/") {
                let obj = repo.find_object(oid, None).unwrap();
                let commit = obj.peel_to_commit().unwrap();
                let commit_time = commit.time().seconds();

                match &latest_tag_info {
                    None => {
                        latest_tag_info = Some((tag_name.to_string(), commit_time));
                    }
                    Some((_, existing_time)) => {
                        if commit_time > *existing_time {
                            latest_tag_info = Some((tag_name.to_string(), commit_time));
                        }
                    }
                }
            }
            true // Continue iteration
        })
        .ok();

        latest_tag_info.map(|(name, _)| name)
    };

    // Ref (use tag if available, otherwise branch, otherwise commit SHA)
    let r#ref = tag
        .clone()
        .or_else(|| branch.clone())
        .unwrap_or_else(|| commit_sha.clone());

    Ok(GitInfo {
        branch,
        commit_sha,
        commit_message,
        tag,
        latest_tag,
        r#ref,
    })
}

impl TryFrom<&ExprEvalContext> for cel::Context<'_> {
    type Error = anyhow::Error;

    fn try_from(context: &ExprEvalContext) -> Result<Self> {
        let mut ctx = cel::Context::default();
        ctx.add_variable("var", context.var.clone())?;
        ctx.add_variable("env", context.env.clone())?;
        ctx.add_variable("git", context.git.clone())?;
        ctx.add_variable("lttle", context.lttle.clone())?;
        ctx.add_variable("namespace", context.namespace.clone())?;

        ctx.add_stdlib_functions();

        Ok(ctx)
    }
}

fn extract_top_level_vars_without_expressions(value: &Value) -> Result<BTreeMap<String, Value>> {
    let mut vars = BTreeMap::new();
    if let Some(map) = value.as_mapping() {
        for (key, value) in map {
            let Value::String(key_str) = key else {
                continue;
            };

            if let Some(str) = value.as_str() {
                if str.contains("${{") && str.contains("}}") {
                    continue;
                }
            }

            vars.insert(key_str.clone(), value.clone());
        }
    }

    Ok(vars)
}
