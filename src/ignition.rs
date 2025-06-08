use std::sync::Arc;

use api::{start_api_server, ApiServerConfig};
use controller::image::credentials::DockerCredentialsProvider;
use controller::image::{ImagePool, ImagePoolConfig};
use controller::net::ip::{IpPool, IpPoolConfig};
use controller::net::tap::{TapPool, TapPoolConfig};
use controller::volume::VolumePoolConfig;
use controller::{Controller, ControllerConfig};
use futures::executor::block_on;
use sds::{Store, StoreConfig};
use tracing_subscriber::FmtSubscriber;
use util::tracing::{self, info};
use util::{
    async_runtime::{self, task::spawn_blocking},
    result::{Context, Result},
};

async fn ignition() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    let store = Store::new(StoreConfig {
        dir_path: "./data/store".into(),
        size_mib: 128,
    })?;

    let controller = create_and_start_controller(store.clone()).await?;

    let api_config = ApiServerConfig {
        addr: "0.0.0.0:5100".parse()?,
        store,
        controller,
        // TODO(@laurci): get this from env
        admin_token: "temp_admin_token".to_string(),
        jwt_secret: "dGVtcF9qd3Rfc2VjcmV0".to_string(), // Note: In production, this should be a proper secret
        default_token_duration: 3600,
    };

    start_api_server(api_config).await?;

    Ok(())
}

async fn create_and_start_controller(store: Store) -> Result<Arc<Controller>> {
    let image_volume_pool = controller::volume::VolumePool::new(
        store.clone(),
        VolumePoolConfig {
            name: "image".to_string(),
            root_dir: "./data/volumes/images".to_string(),
        },
    )?;

    let image_pool = Arc::new(ImagePool::new(
        store.clone(),
        ImagePoolConfig {
            volume_pool: image_volume_pool,
            credentials_provider: Arc::new(DockerCredentialsProvider {}),
        },
    )?);

    let tap_pool = TapPool::new(TapPoolConfig {
        bridge_name: "ltbr0".to_string(),
    })
    .await?;

    let ip_pool = IpPool::new(
        IpPoolConfig {
            name: "vm".to_string(),
            cidr: "10.0.0.0/16".to_string(),
        },
        store.clone(),
    )?;

    // Create controller
    let controller = Controller::new(
        ControllerConfig {
            reconcile_interval_secs: 2, // slow for demo and testing
        },
        store.clone(),
        image_pool,
        tap_pool.clone(),
        ip_pool,
    )?;

    // Start reconciliation in background
    let reconcile_controller = controller.clone();
    spawn_blocking(move || {
        block_on(async {
            let _ = reconcile_controller.run_reconciliation().await;
        });
    });

    info!("Controller reconciliation started");

    // Test resource tracking
    let (tracked_images, tracked_volumes) = controller.list_tracked_resources().await?;
    info!(
        "Current tracked resources: {} images, {} volumes",
        tracked_images.len(),
        tracked_volumes.len()
    );

    Ok(controller)
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
