use util::{result::Result, tracing::info};

use crate::config::Config;

pub async fn run_login(config: Config, token: String) -> Result<()> {
    let mut config = config.clone();

    config.token = Some(token);
    config.save().await?;

    info!("Token saved to config");
    Ok(())
}
