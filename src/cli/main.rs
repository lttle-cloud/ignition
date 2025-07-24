pub mod client;
pub mod cmd;
pub mod config;
pub mod ui;

use anyhow::Result;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load().await?;

    cmd::run_cli(&config).await?;

    Ok(())
}
