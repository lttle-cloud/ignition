use anyhow::Result;
use clap::Args;

use crate::{
    config::{Config, Profile},
    ui::message::{message_detail, message_info, message_warn},
};

#[derive(Args)]
pub struct ProfileSetArgs {
    profile: String,
}

#[derive(Args)]
pub struct ProfileDeleteArgs {
    profile: String,

    #[arg(long = "yes", short = 'y')]
    confirm: bool,
}

pub async fn run_profile_current(config: &Config) -> Result<()> {
    message_info(format!("Current profile: {}", config.current_profile));
    Ok(())
}

pub async fn run_profile_list(config: &Config) -> Result<()> {
    message_info("Available profiles:");
    for profile in &config.profiles {
        if profile.name == config.current_profile {
            message_detail(format!("* {}", profile.name));
        } else {
            message_info(format!("  {}", profile.name));
        }
    }

    Ok(())
}

pub async fn run_profile_set(config: &Config, args: ProfileSetArgs) -> Result<()> {
    let mut config = config.clone();
    let profile = config.get_profile(&args.profile)?;

    config.current_profile = profile.name.clone();
    config.save().await?;

    message_info(format!("Current profile set to: {}", profile.name));

    Ok(())
}

pub async fn run_profile_delete(config: &Config, args: ProfileDeleteArgs) -> Result<()> {
    let mut config = config.clone();
    let profile = config.get_profile(&args.profile)?;

    if !args.confirm {
        message_warn(format!(
            "You are about to delete the profile '{}'. This action cannot be undone. To confirm, run the command with --yes (or -y).",
            profile.name
        ));
        return Ok(());
    }

    config.profiles = config
        .profiles
        .iter()
        .filter(|p| p.name != profile.name)
        .cloned()
        .collect::<Vec<Profile>>();

    let first_config = config
        .profiles
        .first()
        .map(|p| p.name.clone())
        .unwrap_or("default".to_string());

    config.current_profile = first_config;
    config.save().await?;

    message_info(format!("Profile '{}' deleted", profile.name));
    message_info(format!(
        "Current profile set to: {}",
        config.current_profile
    ));

    Ok(())
}
