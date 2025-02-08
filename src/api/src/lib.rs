use ignition_proto::admin_server::AdminServer;
use sds::Store;
use services::admin::{admin_auth_interceptor, AdminService, AdminServiceConfig};
use std::net::SocketAddr;
use tonic::transport::Server;
use tracing::info;
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
}

pub struct ApiServerConfig {
    pub addr: SocketAddr,
    pub store: Store,
    pub admin_token: String,
    pub jwt_private_key: String,
    pub default_token_duration: u32,
}

pub async fn start_api_server(config: ApiServerConfig) -> Result<()> {
    let admin_token = config.admin_token.clone();
    let admin_service = AdminService::new(
        config.store.clone(),
        AdminServiceConfig {
            jwt_private_key: config.jwt_private_key,
            default_token_duration: config.default_token_duration,
        },
    )?;
    let admin_server = AdminServer::with_interceptor(admin_service, move |req| {
        admin_auth_interceptor(req, admin_token.clone())
    });

    info!("api server listening on {:?}", config.addr);

    Server::builder()
        .add_service(admin_server)
        .serve(config.addr)
        .await?;

    Ok(())
}
