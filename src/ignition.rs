use std::sync::Arc;

use api::{start_api_server, ApiServerConfig};
use controller::image::credentials::DockerCredentialsProvider;
use controller::image::{ImagePool, ImagePoolConfig};
use controller::net::ip::{IpPool, IpPoolConfig};
use controller::net::tap::{TapPool, TapPoolConfig};
use controller::volume::VolumePoolConfig;
use controller::{Controller, ControllerConfig};
use futures::executor::block_on;
use reqwest;
use sds::{Store, StoreConfig};
use tracing_subscriber::FmtSubscriber;
use util::tracing::{self, info, warn};
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
            log_dir_path: "./data/logs".to_string(),
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

    // Test basic deployment functionality
    test_deployment_workflow(controller.clone()).await?;

    // Test resource tracking
    let (tracked_images, tracked_volumes) = controller.list_tracked_resources().await?;
    info!(
        "Current tracked resources: {} images, {} volumes",
        tracked_images.len(),
        tracked_volumes.len()
    );

    Ok(controller)
}

async fn test_deployment_workflow(controller: Arc<Controller>) -> Result<()> {
    info!("🧪 Running Spark deployment workflow test...");

    let test_config = controller::deployment::DeploymentConfig {
        name: "test-spark".to_string(),
        image: "nginx:latest".to_string(),
        mode: controller::deployment::DeploymentMode::Spark {
            timeout_ms: 5000, // 5 second timeout for testing
            snapshot_policy: controller::machine::SparkSnapshotPolicy::OnUserspaceReady,
        },
        image_pull_policy: controller::image::PullPolicy::IfNotPresent,
        vcpu_count: 1,
        memory_mib: 512,
        envs: vec!["TEST_ENV=spark".to_string()],
        replicas: 1, // Spark deployments must have exactly 1 replica
    };

    // Test Spark deployment creation
    info!("Creating Spark deployment...");
    let deployment = controller.deploy(test_config.clone()).await?;
    info!("Created Spark deployment: {}", deployment.id);

    // Wait for deployment to be ready
    info!("⏳ Waiting 20 seconds for Spark deployment to be ready...");
    util::async_runtime::time::sleep(std::time::Duration::from_secs(20)).await;

    // Check if deployment is ready
    let retrieved = controller.get_deployment(&deployment.id).await?;
    if let Some(retrieved) = retrieved {
        info!("📊 Spark deployment status: {:?}", retrieved.status);
        info!("🏭 Instances: {}", retrieved.instances.len());

        for instance in &retrieved.instances {
            info!(
                "  Instance {}: {:?} (IP: {})",
                instance.id, instance.status, instance.ip_addr
            );
        }
    }

    // Test Spark connection management
    info!("Testing Spark connection system...");

    // Open first connection (should ensure instance is running)
    info!("Opening first connection...");
    let ip1 = controller.open_connection(deployment.id.clone()).await?;
    info!("Got IP from first connection: {}", ip1);

    // Test HTTP connectivity to the nginx instance
    info!("Testing HTTP connectivity to nginx...");
    let url = format!("http://{}:80", ip1);

    // Retry HTTP requests since nginx takes time to start up inside the container
    let mut retry_count = 0;
    let max_retries = 5;
    loop {
        match reqwest::get(&url).await {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                info!("HTTP GET {}: {} ({} bytes)", url, status, body.len());
                if body.contains("nginx") || body.contains("Welcome") {
                    info!("Confirmed nginx is serving content!");
                }
                break;
            }
            Err(e) => {
                retry_count += 1;
                if retry_count <= max_retries {
                    warn!(
                        "HTTP request attempt {}/{} failed: {}",
                        retry_count, max_retries, e
                    );
                    info!("Waiting 2s before retry (nginx might still be starting up)...");
                    util::async_runtime::time::sleep(std::time::Duration::from_secs(2)).await;
                } else {
                    warn!("HTTP request failed after {} attempts: {}", max_retries, e);
                    break;
                }
            }
        }
    }

    // Open second connection (should reuse running instance)
    info!("Opening second connection...");
    let ip2 = controller.open_connection(deployment.id.clone()).await?;
    info!("Got IP from second connection: {}", ip2);

    if ip1 == ip2 {
        info!("Both connections got same IP (correct!)");
    } else {
        warn!("Different IPs returned: {} vs {}", ip1, ip2);
    }

    // Test HTTP again to ensure consistency
    info!("Testing HTTP on second connection...");
    let url2 = format!("http://{}:80", ip2);
    match reqwest::get(&url2).await {
        Ok(response) => {
            info!("HTTP GET {}: {}", url2, response.status());
        }
        Err(e) => {
            warn!("Second HTTP request failed: {}", e);
        }
    }

    // Close first connection (instance should stay running)
    info!("Closing first connection...");
    controller.close_connection(deployment.id.clone()).await?;
    info!("First connection closed, instance should still be running");

    // Close second connection (should start timeout)
    info!("Closing second connection...");
    controller.close_connection(deployment.id.clone()).await?;
    info!("Second connection closed, timeout should start (5s)");

    // Wait for timeout to trigger suspension
    info!("Waiting 7 seconds for timeout to trigger suspension...");
    util::async_runtime::time::sleep(std::time::Duration::from_secs(7)).await;

    // Check if instance was suspended
    let retrieved = controller.get_deployment(&deployment.id).await?;
    if let Some(retrieved) = retrieved {
        info!("📊 Status after timeout: {:?}", retrieved.status);

        if matches!(
            retrieved.status,
            controller::deployment::DeploymentStatus::ReadyToResume
        ) {
            info!("Spark instance suspended correctly!");
        } else {
            warn!("Expected ReadyToResume status, got {:?}", retrieved.status);
        }
    }

    // Test resume by opening another connection
    info!("Opening connection to test resume...");
    let ip3 = controller.open_connection(deployment.id.clone()).await?;
    info!("Got IP from resume connection: {}", ip3);

    // Wait a moment for instance to be fully ready
    util::async_runtime::time::sleep(std::time::Duration::from_secs(3)).await;

    // Check final status
    let retrieved = controller.get_deployment(&deployment.id).await?;
    if let Some(retrieved) = retrieved {
        info!("📊 Final status: {:?}", retrieved.status);
    }

    // Close the connection
    controller.close_connection(deployment.id.clone()).await?;

    // Clean up
    info!("Cleaning up Spark deployment...");
    controller.delete_deployment(&deployment.id).await?;
    info!("Spark deployment deleted");

    info!("Spark deployment workflow test completed successfully!");
    Ok(())
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
