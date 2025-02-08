use api::{start_api_server, ApiServerConfig};
use sds::{Store, StoreConfig};
use tracing_subscriber::FmtSubscriber;
use util::{
    async_runtime,
    result::{Context, Result},
};

async fn ignition() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    let store = Store::new(StoreConfig {
        dir_path: "./data/ignition_store".into(),
        size_mib: 128,
    })?;

    let api_config = ApiServerConfig {
        addr: "0.0.0.0:5100".parse()?,
        store,
        // TODO(@laurci): get this from env
        admin_token: "temp_admin_token".to_string(),
    };

    start_api_server(api_config).await?;

    Ok(())
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
