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
use tracing::info;
use util::async_runtime::spawn;
use util::async_runtime::time::sleep;
use util::result::Result;

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
    // starting progress task
    controller.start_progress_task().await?;

    controller
        .deploy(DeployRequest {
            instance_name: "test-caddy".to_string(),
            image: "caddy:latest".to_string(),
        })
        .await?;

    println!("progress task started");

    let controller_clone = controller.clone();
    spawn(async move {
        sleep(Duration::from_secs(45)).await;
        controller_clone.stop_progress_task().await;
    });

    Ok(())
}
