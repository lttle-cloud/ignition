use controller::Controller;
use ignition_proto::admin_server::AdminServer;
use sds::Store;
use services::admin::{admin_auth_interceptor, AdminApi, AdminApiConfig};
use services::auth::{AuthInterceptor, AuthInterceptorConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use tonic::transport::Server;
use util::result::Result;
use util::tracing::info;

use crate::ignition_proto::image_server::ImageServer;
use crate::ignition_proto::machine_server::MachineServer;
use crate::ignition_proto::service_server::ServiceServer;
use crate::services::auth::user_auth_interceptor;
use crate::services::image::{ImageApi, ImageApiConfig};
use crate::services::machine::{MachineApi, MachineApiConfig};
use crate::services::service::{ServiceApi, ServiceApiConfig};

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
    pub mod service {
        tonic::include_proto!("ignition.service");
    }
    pub mod machine {
        tonic::include_proto!("ignition.machine");
    }
}

pub struct ApiServerConfig {
    pub addr: SocketAddr,
    pub store: Store,
    pub controller: Arc<Controller>,
    pub admin_token: String,
    pub jwt_secret: String,
    pub default_token_duration: u32,
}

pub async fn start_api_server(config: ApiServerConfig) -> Result<()> {
    let admin_token = config.admin_token.clone();
    let admin_service = AdminApi::new(
        config.store.clone(),
        AdminApiConfig {
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

    let image_api = ImageApi::new(config.controller.clone(), ImageApiConfig {})?;
    let image_server =
        ImageServer::with_interceptor(image_api, user_auth_interceptor(auth_interceptor.clone()));

    let machine_api = MachineApi::new(config.controller.clone(), MachineApiConfig {})?;
    let machine_server = MachineServer::with_interceptor(
        machine_api,
        user_auth_interceptor(auth_interceptor.clone()),
    );

    let service_api = ServiceApi::new(config.controller.clone(), ServiceApiConfig {})?;
    let service_server = ServiceServer::with_interceptor(
        service_api,
        user_auth_interceptor(auth_interceptor.clone()),
    );

    info!("api server listening on {:?}", config.addr);

    Server::builder()
        .add_service(admin_server)
        .add_service(image_server)
        .add_service(machine_server)
        .add_service(service_server)
        .serve(config.addr)
        .await?;

    Ok(())
}
