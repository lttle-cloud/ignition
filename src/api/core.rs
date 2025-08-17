use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State, WebSocketUpgrade, ws::Message},
    response::IntoResponse,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;

use crate::{
    agent::logs::LogStreamOrigin,
    api::{
        ApiState,
        context::ServiceRequestContext,
        resource_service::{ResourceService, ResourceServiceRouter},
    },
    resources::core::{ListNamespaces, LogStreamParams, Me, Namespace},
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

        // websocket endpoint for streaming logs
        async fn stream_logs(
            state: State<Arc<ApiState>>,
            ctx: ServiceRequestContext,
            Query(params): Query<LogStreamParams>,
            ws: WebSocketUpgrade,
        ) -> impl IntoResponse {
            let (origin, start_ts, end_ts) = match params {
                LogStreamParams::Machine {
                    machine_name,
                    start_ts_ns,
                    end_ts_ns,
                } => (
                    LogStreamOrigin::Machine {
                        tenant: ctx.tenant.clone(),
                        name: machine_name,
                        namespace: ctx.namespace.as_value(),
                    },
                    start_ts_ns,
                    end_ts_ns,
                ),
                LogStreamParams::Group {
                    group_name,
                    start_ts_ns,
                    end_ts_ns,
                } => (
                    LogStreamOrigin::Group {
                        tenant: ctx.tenant.clone(),
                        name: group_name,
                        namespace: ctx.namespace.as_value(),
                    },
                    start_ts_ns,
                    end_ts_ns,
                ),
            };

            ws.on_upgrade(move |socket| async move {
                let (mut write, _) = socket.split();

                // if there is no end_ts, we should tail the logs
                // if there is an end_ts, we should get the logs from the start_ts to the end_ts by query_range
                match end_ts {
                    Some(end_ts) => {
                        let start_ts = start_ts
                            .and_then(|val| u128::from_str_radix(&val, 10).ok())
                            .unwrap_or_else(|| {
                                1 * 60 * 60 * 1_000_000_000 // 1h in ns
                            });

                        let Ok(end_ts) = u128::from_str_radix(&end_ts, 10) else {
                            return;
                        };

                        let Ok(logs) = state
                            .scheduler
                            .agent
                            .logs()
                            .query(origin, start_ts..=end_ts)
                            .await
                        else {
                            return;
                        };

                        for log in logs {
                            let Ok(entry_text) = serde_json::to_string(&log) else {
                                return;
                            };

                            let Ok(_) = write.send(Message::Text(entry_text.into())).await else {
                                return;
                            };
                        }
                    }
                    None => {
                        let Ok(mut stream) = state.scheduler.agent.logs().stream(origin).await
                        else {
                            return;
                        };

                        while let Some(logs) = stream.next().await {
                            for log in logs {
                                let Ok(entry_text) = serde_json::to_string(&log) else {
                                    return;
                                };

                                let Ok(_) = write.send(Message::Text(entry_text.into())).await
                                else {
                                    return;
                                };
                            }
                        }
                    }
                }
            })
        }

        let mut router = Router::new();
        router = router.route("/me", get(me));
        router = router.route("/namespaces", get(list_namespaces));
        router = router.route("/logs", get(stream_logs));

        ResourceServiceRouter {
            name: "Core".to_string(),
            base_path: "/core".to_string(),
            router,
        }
    }
}
