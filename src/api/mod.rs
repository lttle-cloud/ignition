pub mod auth;
pub mod context;
pub mod core;
pub mod gadget;
pub mod resource_service;

use std::sync::Arc;

use anyhow::Result;
use axum::{
    Router,
    body::Body,
    extract::Request,
    middleware::{self, Next},
    response::Response,
};
use hyper::StatusCode;
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    api::{
        auth::AuthHandler,
        resource_service::{ResourceService, ResourceServiceRouter},
    },
    controller::scheduler::Scheduler,
    machinery::store::Store,
    repository::Repository,
    resources::core::CLIENT_COMPAT_VERSION,
};

pub struct ApiState {
    pub store: Arc<Store>,
    pub repository: Arc<Repository>,
    pub scheduler: Arc<Scheduler>,
    pub auth_handler: Arc<AuthHandler>,
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
    pub fn new(
        store: Arc<Store>,
        repository: Arc<Repository>,
        scheduler: Arc<Scheduler>,
        auth_handler: Arc<AuthHandler>,
        config: ApiServerConfig,
    ) -> Self {
        Self {
            state: Arc::new(ApiState {
                store,
                repository,
                scheduler,
                auth_handler,
            }),
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

        let app = app.route_layer(middleware::from_fn(check_client_compat));
        let app = app.with_state(self.state);

        let addr = format!("{}:{}", self.config.host, self.config.port);
        info!("starting api server on {}", addr);

        let listener = TcpListener::bind(addr).await?;

        axum::serve(listener, app).await?;

        Ok(())
    }
}

const SKIP_CLIENT_COMPAT_CHECK: &[&str] = &["/core/registry/auth"];

async fn check_client_compat(request: Request, next: Next) -> Response {
    let compat_version = request
        .headers()
        .get("x-ignition-compat")
        .and_then(|v| v.to_str().map(|s| s.to_string()).ok())
        .unwrap_or("".to_string());

    let is_skip_client_compat_check = SKIP_CLIENT_COMPAT_CHECK.contains(&request.uri().path());

    if compat_version != CLIENT_COMPAT_VERSION && !is_skip_client_compat_check {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(
                "incompatible client version.\nHere's how you can install the latest version: https://github.com/lttle-cloud/ignition?tab=readme-ov-file#installation",
            ))
            .unwrap();
    }

    let response = next.run(request).await;
    response
}
