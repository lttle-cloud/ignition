use std::{collections::BTreeMap, fmt::Debug, path::PathBuf};

use anyhow::{Result, bail};
use git2::Repository;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use tokio::fs::read_to_string;

use crate::{expr::std_lib, ui::message::message_warn};

pub struct ExprEvalContextConfig {
    pub env_file: Option<PathBuf>,
    pub var_file: Option<PathBuf>,
    // needs parsing
    pub aditional_vars: Option<BTreeMap<String, String>>,
    pub git_dir: PathBuf,
    pub env_ambient_override_behavior: EnvAmbientOverrideBehavior,
}

#[derive(PartialEq, Eq)]
pub enum EnvAmbientOverrideBehavior {
    /// If the environment variable is already set, it will be overridden by the value from the file
    Override,
    /// If the environment variable is already set, it will not be overridden
    Ignore,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ExprEvalContext {
    env: BTreeMap<String, String>,
    var: BTreeMap<String, Value>,
    git: Option<GitInfo>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GitInfo {
    branch: Option<String>,

    #[serde(rename = "commitSha")]
    commit_sha: String, // 8 chars

    #[serde(rename = "commitMessage")]
    commit_message: String,

    #[serde(rename = "tag")]
    tag: Option<String>,

    #[serde(rename = "latestTag")]
    latest_tag: Option<String>,

    #[serde(rename = "ref")]
    r#ref: String,
}

impl Debug for GitInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("")
            .field("branch", &self.branch)
            .field("commitSha", &self.commit_sha)
            .field("commitMessage", &self.commit_message)
            .field("tag", &self.tag)
            .field("latestTag", &self.latest_tag)
            .field("ref", &self.r#ref)
            .finish()
    }
}

impl Debug for ExprEvalContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("")
            .field("env", &self.env)
            .field("var", &self.var)
            .field("git", &self.git)
            .finish()
    }
}

impl ExprEvalContext {
    pub async fn new(config: ExprEvalContextConfig) -> Result<Self> {
        let mut envs = BTreeMap::new();
        if let Some(env_file) = config.env_file {
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

        let mut vars: BTreeMap<String, Value> = BTreeMap::new();
        if let Some(var_file) = config.var_file {
            let contents = read_to_string(var_file).await?;
            vars = serde_yaml::from_str(&contents)?;
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
            Err(e) => {
                message_warn(format!("failed to load git info for current directory"));
                None
            }
        };

        Ok(Self {
            env: envs,
            var: vars,
            git: git_info,
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
        None
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

        // custom string functions
        ctx.add_function("last", std_lib::str::last);
        ctx.add_function("slugify", std_lib::str::slugify);
        ctx.add_function("toSlug", std_lib::str::to_slug);

        // CEL extended string functions
        ctx.add_function("charAt", std_lib::str::char_at);
        ctx.add_function("indexOf", std_lib::str::index_of);
        ctx.add_function("join", std_lib::str::join_list);
        ctx.add_function("lastIndexOf", std_lib::str::last_index_of);
        ctx.add_function("lowerAscii", std_lib::str::lower_ascii);
        ctx.add_function("quote", std_lib::str::quote);
        ctx.add_function("replace", std_lib::str::replace);
        ctx.add_function("split", std_lib::str::split_string);
        ctx.add_function("substring", std_lib::str::substring);
        ctx.add_function("trim", std_lib::str::trim);
        ctx.add_function("upperAscii", std_lib::str::upper_ascii);
        ctx.add_function("reverse", std_lib::str::reverse);

        Ok(ctx)
    }
}
