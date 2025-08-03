use anyhow::Result;

use crate::{client::get_api_client, config::Config, ui::message::message_info};
use clap::Args;

#[derive(Args)]
pub struct LoginArgs {
    #[arg(long)]
    api: String,

    token: String,
}

pub async fn run_login(config: &Config, args: LoginArgs) -> Result<()> {
    let mut config = config.clone();

    config.api_url = Some(args.api);
    config.token = Some(args.token);

    config.save().await?;

    let api_client = get_api_client(&config).await?;
    let me = api_client.core().me().await?;
    message_info(format!("Successfully logged in as {}", me.tenant));

    Ok(())
}
