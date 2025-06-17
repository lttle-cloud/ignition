pub(crate) mod client;
mod cmd;
mod config;
mod resource;

use tracing_subscriber::FmtSubscriber;
use util::{
    async_runtime,
    result::{bail, Context, Result},
    tracing::{self, error},
};

use crate::{cmd::run_cli, config::Config};

async fn ignition() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    let Ok(config) = Config::load().await else {
        bail!("Failed to load config");
    };

    if let Err(e) = run_cli(config).await {
        error!("Error: {}", e);
        std::process::exit(1);
    };

    Ok(())
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
