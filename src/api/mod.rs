pub mod context;
pub mod resource_service;

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    api::resource_service::{ResourceService, ResourceServiceRouter},
    machinery::store::Store,
    repository::Repository,
};

pub struct ApiState {
    pub store: Arc<Store>,
    pub repository: Repository,
}

pub struct ApiServerConfig {
    pub host: String,
    pub port: u16,
}

pub struct ApiServer {
    state: Arc<ApiState>,
    config: ApiServerConfig,
    routers: Vec<ResourceServiceRouter>,
}

impl ApiServer {
    pub fn new(store: Arc<Store>, config: ApiServerConfig) -> Self {
        let repository = Repository::new(store.clone());

        Self {
            state: Arc::new(ApiState { store, repository }),
            config,
            routers: vec![],
        }
    }

    pub fn add_service<R: ResourceService>(mut self) -> Self {
        let router = R::create_router(self.state.clone());
        self.routers.push(router);
        self
    }

    pub async fn start(self) -> Result<()> {
        let mut app = Router::new();

        for router in self.routers {
            info!("adding service {} at {}", router.name, router.base_path);
            app = app.nest(&router.base_path, router.router);
        }

        let app = app.with_state(self.state);

        let addr = format!("{}:{}", self.config.host, self.config.port);
        info!("starting api server on {}", addr);

        let listener = TcpListener::bind(addr).await?;

        axum::serve(listener, app).await?;

        Ok(())
    }
}
