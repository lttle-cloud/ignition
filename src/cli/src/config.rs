use std::path::PathBuf;

use util::{
    async_runtime::fs::{create_dir_all, read_to_string, write},
    encoding::codec,
    result::{bail, Result},
};

const DEFAULT_API_ADDR: &str = "127.0.0.1:5100";

#[codec]
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    #[serde(skip_serializing, skip_deserializing)]
    pub config_path: PathBuf,

    pub api_addr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
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
                api_addr: DEFAULT_API_ADDR.to_string(),
                admin_token: None,
                token: None,
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
