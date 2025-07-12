use anyhow::Result;

use crate::{client::get_api_client, config::Config};
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
    println!("Logged in as {}", me.tenant);

    Ok(())
}
