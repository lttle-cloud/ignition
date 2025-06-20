use std::path::PathBuf;
use std::sync::Arc;

use api::{start_api_server, ApiServerConfig};
use controller::image::credentials::DockerCredentialsProvider;
use controller::image::{ImagePool, ImagePoolConfig};
use controller::logs::{LogsPool, LogsPoolConfig};
use controller::machine::MachinePool;
use controller::net::ip::{IpPool, IpPoolConfig};
use controller::net::tap::{TapPool, TapPoolConfig};
use controller::proxy::{Proxy, ProxyConfig, ProxyTlsTerminationConfig};
use controller::service::ServicePool;
use controller::volume::{VolumePool, VolumePoolConfig};
use controller::{Controller, ControllerConfig};
use sds::{Store, StoreConfig};
use tracing_subscriber::FmtSubscriber;
use util::async_runtime::task;
use util::tracing::{self, info};
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
    let image_volume_pool = VolumePool::new(
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

    let tap_pool = Arc::new(
        TapPool::new(TapPoolConfig {
            bridge_name: "ltbr0".to_string(),
        })
        .await?,
    );

    let vm_ip_pool = Arc::new(IpPool::new(
        IpPoolConfig {
            name: "vm".to_string(),
            cidr: "10.0.0.0/16".to_string(),
        },
        store.clone(),
    )?);

    let svc_ip_pool = Arc::new(IpPool::new(
        IpPoolConfig {
            name: "svc".to_string(),
            cidr: "10.1.0.0/16".to_string(),
        },
        store.clone(),
    )?);

    let logs_pool = Arc::new(
        LogsPool::new(LogsPoolConfig {
            base_path: "./data/logs".to_string(),
        })
        .await?,
    );

    let machine_pool = Arc::new(MachinePool::new(store.clone())?);

    let service_pool = Arc::new(ServicePool::new(store.clone())?);

    let controller = Controller::new(
        ControllerConfig {
            garbage_collection_interval_secs: 60 * 5, // 5 minutes
            default_tls_termination: ProxyTlsTerminationConfig {
                ssl_cert_path: PathBuf::from("./certs/server.cert"),
                ssl_key_path: PathBuf::from("./certs/server.key"),
            },
        },
        image_pool,
        tap_pool.clone(),
        vm_ip_pool,
        svc_ip_pool,
        logs_pool,
        machine_pool,
        service_pool,
    )?;

    let proxy = Proxy::new(
        ProxyConfig {
            external_host: "151.80.18.214".to_string(),
            http_port: 80,
            https_port: 443,
        },
        controller.clone(),
    )
    .await?;
    controller.set_proxy(proxy.clone()).await;

    controller.bring_up().await?;

    let garbage_collection_controller = controller.clone();
    task::spawn(async move {
        garbage_collection_controller
            .garbage_collection_task()
            .await;
    });
    info!("Garbage collection task started");

    let machines = controller.list_machines().await?;
    println!("Machines: {:?}", machines);

    Ok(controller)
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
