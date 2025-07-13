use std::sync::Arc;

use axum::{Json, Router, extract::State, response::IntoResponse, routing::get};
use reqwest::StatusCode;

use crate::{
    api::{
        ApiState,
        context::ServiceRequestContext,
        resource_service::{ResourceService, ResourceServiceRouter},
    },
    resources::core::{ListNamespaces, Me, Namespace},
};

pub struct CoreService {}

impl ResourceService for CoreService {
    fn create_router(_state: Arc<ApiState>) -> ResourceServiceRouter {
        async fn me(ctx: ServiceRequestContext) -> impl IntoResponse {
            (StatusCode::OK, Json(Me { tenant: ctx.tenant }))
        }

        async fn list_namespaces(
            state: State<Arc<ApiState>>,
            ctx: ServiceRequestContext,
        ) -> impl IntoResponse {
            let Ok(namespaces) = state.store.list_tracked_namespaces(ctx.tenant) else {
                return (StatusCode::OK, Json(ListNamespaces { namespaces: vec![] }))
                    .into_response();
            };

            (
                StatusCode::OK,
                Json(ListNamespaces {
                    namespaces: namespaces
                        .into_iter()
                        .map(|n| Namespace {
                            name: n.namespace.clone(),
                            created_at: n.created_at,
                        })
                        .collect(),
                }),
            )
                .into_response()
        }

        let mut router = Router::new();
        router = router.route("/me", get(me));
        router = router.route("/namespaces", get(list_namespaces));

        ResourceServiceRouter {
            name: "core".to_string(),
            base_path: "/core".to_string(),
            router,
        }
    }
}
