pub mod build;
pub mod client;
pub mod cmd;
pub mod config;
pub mod expr;
pub mod ui;

use anyhow::Result;

use crate::ui::message::message_error;

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = cmd::run_cli().await {
        message_error(e.to_string());
    }

    Ok(())
}
