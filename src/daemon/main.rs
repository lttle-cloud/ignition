use std::sync::Arc;

use anyhow::Result;
use ignition::{
    api::{ApiServer, ApiServerConfig, auth::AuthHandler, core::CoreService},
    controller::{
        machine::MachineController,
        scheduler::{Scheduler, SchedulerConfig},
    },
    machinery::store::Store,
    repository::Repository,
    services,
    utils::tracing::init_tracing,
};

// TODO: get this from config
const TEMP_JWT_SECRET: &str = "dGVtcF9qd3Rfc2VjcmV0";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let store = Arc::new(Store::new("data").await?);

    let scheduler = Arc::new_cyclic(|scheduler_weak| {
        let repository = Arc::new(Repository::new(store.clone(), scheduler_weak.clone()));

        let scheduler = Scheduler::new(
            store.clone(),
            repository.clone(),
            SchedulerConfig { worker_count: 1 },
            vec![MachineController::new_boxed()],
        );

        scheduler
    });

    let repository = scheduler.repository.clone();

    let auth_handler = Arc::new(AuthHandler::new(TEMP_JWT_SECRET));

    let api_server = ApiServer::new(
        store.clone(),
        repository.clone(),
        scheduler.clone(),
        auth_handler.clone(),
        ApiServerConfig {
            host: "0.0.0.0".to_string(),
            port: 5100,
        },
    )
    .add_service::<CoreService>()
    .add_service::<services::MachineService>();

    scheduler.start_workers();
    api_server.start().await?;

    Ok(())
}
