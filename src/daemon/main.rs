use std::sync::Arc;

use anyhow::Result;
use ignition::{
    api::{ApiServer, ApiServerConfig},
    machinery::store::Store,
    services,
    utils::tracing::init_tracing,
};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let store = Arc::new(Store::new("data").await?);

    let api_server = ApiServer::new(
        store,
        ApiServerConfig {
            host: "0.0.0.0".to_string(),
            port: 3000,
        },
    )
    .add_service::<services::MachineService>();

    api_server.start().await?;

    Ok(())
}
