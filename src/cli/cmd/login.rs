use anyhow::Result;
use ignition::api_client::ApiClientConfig;

use crate::{
    client::get_api_client,
    config::{Config, Profile},
    ui::message::message_info,
};
use clap::Args;

#[derive(Args)]
pub struct LoginArgs {
    /// API URL
    #[arg(long)]
    api: String,

    /// Profile name
    #[arg(long, default_value = "default")]
    profile: String,

    /// Overwrite existing profile
    #[arg(long, short = 'y')]
    overwrite: bool,

    /// Token
    token: String,
}

pub async fn run_login(config: &Config, args: LoginArgs) -> Result<()> {
    let mut config = config.clone();

    if !args.overwrite && config.profiles.iter().any(|p| p.name == args.profile) {
        return Err(anyhow::anyhow!(
            "Profile {} already exists. To overwrite, use the --overwrite (-y) flag.",
            args.profile
        ));
    }

    let api_config = ApiClientConfig {
        base_url: args.api.clone(),
        token: args.token.clone(),
    };

    let api_client = get_api_client(api_config);
    let me =
        api_client.core().me().await.map_err(|_| {
            anyhow::anyhow!(
                "Failed to authenticate. Please check if the API URL and token are correct",
            )
        })?;

    config.profiles = config
        .profiles
        .iter()
        .filter(|p| &p.name != &args.profile)
        .cloned()
        .collect();

    config.profiles.push(Profile {
        name: args.profile.clone(),
        api_url: args.api,
        token: args.token,
    });
    config.current_profile = args.profile;

    config.save().await?;

    message_info(format!(
        "Successfully logged in as {} (tenant: {})",
        me.sub, me.tenant
    ));
    message_info(format!(
        "Set current profile to: {}",
        config.current_profile
    ));

    Ok(())
}

pub async fn run_whoami(config: &Config) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let me = api_client.core().me().await?;
    message_info(format!("Current profile: {}", config.current_profile));
    message_info(format!(
        "You are logged in as {} (tenant: {})",
        me.sub, me.tenant
    ));

    Ok(())
}
