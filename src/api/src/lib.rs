use controller::{Controller, DeployRequest};
use ignition_proto::admin_server::AdminServer;
use ignition_proto::image_server::ImageServer;
use sds::Store;
use services::admin::{admin_auth_interceptor, AdminService, AdminServiceConfig};
use services::auth::{AuthInterceptor, AuthInterceptorConfig};
use services::image::ImageService;
use std::net::SocketAddr;
use std::time::Duration;
use tonic::transport::Server;
use util::async_runtime::time;
use util::result::Result;
use util::tracing::info;

pub(crate) mod data;
pub(crate) mod services;

pub(crate) mod ignition_proto {
    tonic::include_proto!("ignition");
    pub mod util {
        tonic::include_proto!("ignition.util");
    }
    pub mod admin {
        tonic::include_proto!("ignition.admin");
    }
    pub mod image {
        tonic::include_proto!("ignition.image");
    }
    pub mod deployment {
        tonic::include_proto!("ignition.deployment");
    }
}

pub struct ApiServerConfig {
    pub addr: SocketAddr,
    pub store: Store,
    pub controller: Controller,
    pub admin_token: String,
    pub jwt_secret: String,
    pub default_token_duration: u32,
}

pub async fn start_api_server(config: ApiServerConfig) -> Result<()> {
    let admin_token = config.admin_token.clone();
    let admin_service = AdminService::new(
        config.store.clone(),
        AdminServiceConfig {
            jwt_secret: config.jwt_secret.clone(),
            default_token_duration: config.default_token_duration,
        },
    )?;
    let admin_server = AdminServer::with_interceptor(admin_service, move |req| {
        admin_auth_interceptor(req, admin_token.clone())
    });

    let auth_interceptor = AuthInterceptor::new(
        config.store.clone(),
        AuthInterceptorConfig {
            jwt_secret: config.jwt_secret,
        },
    )?;

    let image_service = ImageService::new(config.store.clone())?;
    let image_server = ImageServer::with_interceptor(image_service, move |req| {
        auth_interceptor.validate_request(req)
    });

    test_controller(config.controller.clone()).await?;

    info!("api server listening on {:?}", config.addr);

    Server::builder()
        .add_service(admin_server)
        .add_service(image_server)
        .serve(config.addr)
        .await?;

    Ok(())
}

async fn test_controller(controller: Controller) -> Result<()> {
    println!("Controller initialized successfully");

    // Deploy an instance (this now returns immediately and queues the work)
    println!("Deploying instance: test-instance");

    // This will:
    // 1. Queue the deployment request
    // 2. Return immediately
    // 3. Background reconciliation loop will handle actual deployment
    controller
        .deploy("test-instance".to_string(), "alpine:latest".to_string())
        .await?;

    println!("Deployment queued successfully!");

    // Wait a bit for deployment to progress
    time::sleep(Duration::from_secs(5)).await;

    // Check deployment status
    let instances = controller.list_instances().await;
    println!("Current instances:");
    for instance in instances {
        println!(
            "  {}: {:?} on {} ({})",
            instance.name, instance.status, instance.ip_addr, instance.tap_name
        );
    }

    // Try deploying the same instance again (should trigger redeployment with new generation)
    // println!("\nDeploying the same instance again (should trigger redeployment)...");
    // controller
    //     .deploy("test-instance".to_string(), "caddy:latest".to_string())
    //     .await?;

    // println!("Redeployment queued!");

    // // Wait for redeployment to progress
    // time::sleep(Duration::from_secs(3)).await;

    // // List instances again
    // let instances = controller.list_instances().await;
    // println!("\nInstances after redeployment:");
    // for instance in instances {
    //     println!(
    //         "  {}: {:?} on {} ({})",
    //         instance.name, instance.status, instance.ip_addr, instance.tap_name
    //     );
    // }

    // // Deploy a second instance
    // println!("\nDeploying second instance...");
    // controller
    //     .deploy("test-instance-2".to_string(), "caddy:latest".to_string())
    //     .await?;

    // // Wait and check
    // time::sleep(Duration::from_secs(2)).await;
    // let instances = controller.list_instances().await;
    // println!("\nAll instances:");
    // for instance in instances {
    //     println!(
    //         "  {}: {:?} on {} ({})",
    //         instance.name, instance.status, instance.ip_addr, instance.tap_name
    //     );
    // }

    // // Destroy the first instance
    // println!("\nDestroying first instance...");
    // controller.destroy_instance("test-instance").await?;

    // // Wait for destruction
    // time::sleep(Duration::from_secs(2)).await;
    // let instances = controller.list_instances().await;
    // println!("\nRemaining instances:");
    // for instance in instances {
    //     println!(
    //         "  {}: {:?} on {} ({})",
    //         instance.name, instance.status, instance.ip_addr, instance.tap_name
    //     );
    // }

    Ok(())
}
