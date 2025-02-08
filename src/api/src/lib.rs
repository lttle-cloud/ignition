use ignition_proto::admin_server::AdminServer;
use sds::Store;
use services::admin::{admin_auth_interceptor, AdminService};
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
}

impl ApiServerConfig {
    pub fn new(addr: SocketAddr, store: Store, admin_token: String) -> Self {
        Self {
            addr,
            store,
            admin_token,
        }
    }
}

pub async fn start_api_server(config: ApiServerConfig) -> Result<()> {
    let admin_token = config.admin_token.clone();
    let admin_service = AdminService::new(config.store.clone())?;
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
