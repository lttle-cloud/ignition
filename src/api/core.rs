use std::sync::Arc;

use axum::{Json, Router, response::IntoResponse, routing::get};
use reqwest::StatusCode;

use crate::{
    api::{
        ApiState,
        context::ServiceRequestContext,
        resource_service::{ResourceService, ResourceServiceRouter},
    },
    resources::core::Me,
};

pub struct CoreService {}

impl ResourceService for CoreService {
    fn create_router(_state: Arc<ApiState>) -> ResourceServiceRouter {
        async fn me(ctx: ServiceRequestContext) -> impl IntoResponse {
            (StatusCode::OK, Json(Me { tenant: ctx.tenant }))
        }

        let mut router = Router::new();
        router = router.route("/me", get(me));

        ResourceServiceRouter {
            name: "core".to_string(),
            base_path: "/core".to_string(),
            router,
        }
    }
}
