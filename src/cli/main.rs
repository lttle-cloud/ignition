pub mod client;
pub mod cmd;
pub mod config;
pub mod ui;

use crate::{config::Config, ui::message::message_error};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = cmd::run_cli().await {
        message_error(e.to_string());
    }

    Ok(())
}
