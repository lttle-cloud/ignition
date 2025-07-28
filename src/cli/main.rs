pub mod client;
pub mod cmd;
pub mod config;
pub mod ui;

use anyhow::Result;

use crate::{config::Config, ui::message::message_error};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load().await?;

    if let Err(e) = cmd::run_cli(&config).await {
        message_error(e.to_string());
    }

    Ok(())
}
