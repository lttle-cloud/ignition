use std::path::PathBuf;

use anyhow::{Result, bail};
use ignition::api_client::ApiClientConfig;
use serde::{Deserialize, Serialize};
use tokio::fs::{create_dir_all, read_to_string, write};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(skip_serializing, skip_deserializing)]
    pub config_path: PathBuf,

    #[serde(rename = "current-profile")]
    pub current_profile: String,

    #[serde(rename = "profile")]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub name: String,
    #[serde(rename = "api-url")]
    pub api_url: String,
    pub token: String,
}

impl Config {
    pub fn get_profile(&self, name: &str) -> Result<Profile> {
        self.profiles
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Profile {} not found", name))
    }

    pub fn get_current_profile(&self) -> Result<Profile> {
        self.get_profile(&self.current_profile)
    }
}

impl Config {
    pub async fn load() -> Result<Self> {
        let config_path_env = std::env::var("LTTLE_CONFIG");
        let config_path = if let Ok(path) = config_path_env {
            &PathBuf::from(path)
        } else {
            let Some(project_dirs) = directories::ProjectDirs::from("cloud", "lttle", "lttle")
            else {
                bail!("Failed to get config dir");
            };

            let config_dir = project_dirs.config_dir();
            if !config_dir.exists() {
                create_dir_all(config_dir).await?;
            };

            &config_dir.join("config.toml")
        };

        if !config_path.exists() {
            let config = Self {
                config_path: config_path.clone(),
                current_profile: "default".to_string(),
                profiles: vec![],
            };

            config.save().await?;

            Ok(config)
        } else {
            let config_str = read_to_string(config_path).await?;
            let mut config: Self = toml::from_str(&config_str)?;
            config.config_path = config_path.clone();

            Ok(config)
        }
    }

    pub async fn save(&self) -> Result<()> {
        let config_str = toml::to_string_pretty(&self)?;
        write(&self.config_path, config_str).await?;

        Ok(())
    }
}

impl TryInto<ApiClientConfig> for &Config {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<ApiClientConfig, Self::Error> {
        if self.profiles.is_empty() {
            bail!(
                "No profiles found in config. Please make sure you have configured your CLI with `lttle login`"
            );
        }

        let profile = self.get_current_profile()?;

        Ok(ApiClientConfig {
            base_url: profile.api_url,
            token: profile.token,
        })
    }
}
