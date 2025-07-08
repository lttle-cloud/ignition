use std::sync::Arc;

use axum::Router;

use crate::api::ApiState;

pub struct ResourceServiceRouter {
    pub name: String,
    pub base_path: String,
    pub router: Router<Arc<ApiState>>,
}

impl ResourceServiceRouter {
    pub fn new(name: String, base_path: String, router: Router<Arc<ApiState>>) -> Self {
        Self {
            name,
            base_path,
            router,
        }
    }
}

pub trait ResourceService {
    fn create_router(state: Arc<ApiState>) -> ResourceServiceRouter;
}
