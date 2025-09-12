use std::{str::FromStr, sync::Arc};

use anyhow::{Result, bail};
use axum::{
    Json, Router,
    extract::{FromRequestParts, Query, State, WebSocketUpgrade, ws::Message},
    http::request::Parts,
    response::{IntoResponse, Response},
    routing::{get, put},
};
use base64::{DecodeError, Engine, prelude::BASE64_STANDARD};
use cel::Context;
use futures_util::{SinkExt, StreamExt};
use hyper::HeaderMap;
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::error;
use url::form_urlencoded;

use crate::{
    agent::logs::LogStreamOrigin,
    api::{
        ApiState,
        auth::RegistryRobotHmacClaims,
        context::ServiceRequestContext,
        resource_service::{ResourceService, ResourceServiceRouter},
    },
    controller::{context::ControllerKey, machine::machine_name_from_key},
    eval::{
        CelCtxExt, CelResourceExt,
        ctx::{GitInfo, LttleInfo},
    },
    repository::Repository,
    resource_index::ResourceKind,
    resources::core::{
        ExecParams, ListNamespaces, LogStreamParams, Me, Namespace, QueryParams, QueryResponse,
        RegistryRobot,
    },
};

pub struct CoreService {}

#[derive(Debug)]
struct RegistryTokenQuery {
    service: String,    // must equal registry config "service"
    scope: Vec<String>, // may appear multiple times
}

impl FromRequestParts<Arc<ApiState>> for RegistryTokenQuery {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &Arc<ApiState>,
    ) -> Result<Self, Self::Rejection> {
        let mut service = Option::<String>::None;
        let mut scope = vec![];

        for (k, v) in form_urlencoded::parse(parts.uri.query().unwrap_or_default().as_bytes()) {
            match &*k {
                "service" => service = Some(v.into_owned()),
                "scope" => scope.push(v.into_owned()),
                // ignore other params (account, client_id, offline_token, etc.)
                _ => {}
            }
        }

        let Some(service) = service else {
            return Err((StatusCode::BAD_REQUEST, "service is required").into_response());
        };

        Ok(Self { service, scope })
    }
}

#[derive(Debug, Serialize)]
struct RegistryTokenResponse {
    token: String,
    access_token: String,
}

impl RegistryTokenResponse {
    pub fn new(access_token: String) -> Self {
        Self {
            token: access_token.clone(),
            access_token,
        }
    }
}

impl ResourceService for CoreService {
    fn create_router(_state: Arc<ApiState>) -> ResourceServiceRouter {
        async fn me(ctx: ServiceRequestContext) -> impl IntoResponse {
            (
                StatusCode::OK,
                Json(Me {
                    tenant: ctx.tenant,
                    sub: ctx.sub,
                }),
            )
        }

        async fn registry_robot(
            state: State<Arc<ApiState>>,
            ctx: ServiceRequestContext,
        ) -> impl IntoResponse {
            let claims = RegistryRobotHmacClaims::new(&ctx.tenant, &ctx.sub);
            let Ok(pass) = state.auth_handler.generate_registry_hmac(&claims) else {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to generate registry robot hmac",
                )
                    .into_response();
            };

            let user = claims.to_string();

            (
                StatusCode::OK,
                Json(RegistryRobot {
                    user,
                    pass,
                    registry: state.auth_handler.registry_service.clone(),
                }),
            )
                .into_response()
        }

        async fn registry_auth(
            state: State<Arc<ApiState>>,
            headers: HeaderMap,
            query: RegistryTokenQuery,
        ) -> impl IntoResponse {
            let Some(auth) = headers
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split_once("Basic ").map(|v| v.1))
            else {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            };

            let Ok((user, pass)) = BASE64_STANDARD.decode(auth).and_then(|x| {
                let parts = String::from_utf8_lossy(&x);
                let parts = parts.split_once(":");
                if let Some((user, pass)) = parts {
                    Ok((user.to_string(), pass.to_string()))
                } else {
                    Err(DecodeError::InvalidPadding)
                }
            }) else {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            };

            let Ok(claims) = RegistryRobotHmacClaims::from_str(&user) else {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            };

            let Ok(_) = state
                .auth_handler
                .verify_registry_hmac(pass, &claims, query.service)
            else {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            };

            let Ok(token) = state
                .auth_handler
                .generate_registry_token(&claims, query.scope)
            else {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            };

            (StatusCode::OK, Json(RegistryTokenResponse::new(token))).into_response()
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
                            .and_then(|val| u64::from_str_radix(&val, 10).ok())
                            .unwrap_or_else(|| {
                                1 * 60 * 60 * 1_000_000_000 // 1h in ns
                            });

                        let Ok(end_ts) = u64::from_str_radix(&end_ts, 10) else {
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

        // websocket endpoint for machine exec
        async fn exec(
            state: State<Arc<ApiState>>,
            ctx: ServiceRequestContext,
            Query(params): Query<ExecParams>,
            ws: WebSocketUpgrade,
        ) -> impl IntoResponse {
            ws.on_upgrade(move |socket| async move {
                let (mut ws_write, mut ws_read) = socket.split();

                let machine_name = machine_name_from_key(&ControllerKey::new(
                    ctx.tenant.clone(),
                    ResourceKind::Machine,
                    ctx.namespace.as_value(),
                    params.machine_name,
                ));

                // Find the machine
                let Some(machine) = state.scheduler.agent.machine().get_machine(&machine_name)
                else {
                    let _ = ws_write
                        .send(Message::Text("Machine not found".into()))
                        .await;
                    return;
                };

                // Get connection to the machine's exec server (port 50051)
                let Ok(mut connection) = machine.get_connection(50051, None).await else {
                    let _ = ws_write
                        .send(Message::Text("Failed to connect to machine".into()))
                        .await;
                    return;
                };

                let tcp_stream = connection.upstream_socket();

                // Send the exec request to the exec server
                // Protocol: [cmd_len: u32][cmd: string][stdin_flag: u8][tty_flag: u8]
                let cmd_bytes = params.command.as_bytes();
                let cmd_len = cmd_bytes.len() as u32;
                let stdin_flag = if params.stdin.unwrap_or(false) {
                    1u8
                } else {
                    0u8
                };
                let tty_flag = if params.tty.unwrap_or(false) {
                    1u8
                } else {
                    0u8
                };

                if tcp_stream.write_all(&cmd_len.to_le_bytes()).await.is_err() {
                    return;
                }
                if tcp_stream.write_all(cmd_bytes).await.is_err() {
                    return;
                }
                if tcp_stream.write_all(&[stdin_flag]).await.is_err() {
                    return;
                }
                if tcp_stream.write_all(&[tty_flag]).await.is_err() {
                    return;
                }

                let (tcp_read, tcp_write) = tcp_stream.split();
                let tcp_read = Arc::new(tokio::sync::Mutex::new(tcp_read));
                let tcp_write = Arc::new(tokio::sync::Mutex::new(tcp_write));

                // Handle bidirectional data flow
                let ws_to_tcp = async {
                    while let Some(msg) = ws_read.next().await {
                        match msg {
                            Ok(Message::Binary(data)) => {
                                if tcp_write.lock().await.write_all(&data).await.is_err() {
                                    break;
                                }
                            }
                            Ok(Message::Text(text)) => {
                                if tcp_write
                                    .lock()
                                    .await
                                    .write_all(text.as_bytes())
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Ok(Message::Close(_)) => break,
                            _ => {}
                        }
                    }
                };

                let tcp_to_ws = async {
                    let mut buf = [0; 1024];
                    loop {
                        match tcp_read.lock().await.read(&mut buf).await {
                            Ok(0) => {
                                // TCP connection closed (command finished)
                                break;
                            }
                            Ok(n) => {
                                if ws_write
                                    .send(Message::Binary(buf[..n].to_vec().into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(_) => {
                                // TCP error (machine suspended or connection dropped)
                                break;
                            }
                        }
                    }
                    // Always send close message when TCP ends
                    let _ = ws_write.send(Message::Close(None)).await;
                };

                tokio::select! {
                    _ = ws_to_tcp => {},
                    _ = tcp_to_ws => {},
                }
            })
        }

        async fn query(
            state: State<Arc<ApiState>>,
            ctx: ServiceRequestContext,
            Json(params): Json<QueryParams>,
        ) -> impl IntoResponse {
            let value = match evaluate_query(state.repository.clone(), ctx, params) {
                Ok(value) => value,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            };

            (
                StatusCode::OK,
                Json(QueryResponse {
                    query_result: value.clone(),
                }),
            )
                .into_response()
        }

        let mut router = Router::new();
        router = router.route("/me", get(me));
        router = router.route("/registry/robot", get(registry_robot));
        router = router.route("/registry/auth", get(registry_auth));
        router = router.route("/namespaces", get(list_namespaces));
        router = router.route("/logs", get(stream_logs));
        router = router.route("/exec", get(exec));
        router = router.route("/query", put(query));

        ResourceServiceRouter {
            name: "Core".to_string(),
            base_path: "/core".to_string(),
            router,
        }
    }
}

fn evaluate_query(
    repository: Arc<Repository>,
    ctx: ServiceRequestContext,
    params: QueryParams,
) -> Result<Value> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let program = cel::Program::compile(params.query.as_str())?;
        let mut exec_ctx = Context::default();

        exec_ctx.add_variable("env", params.env)?;
        exec_ctx.add_variable("var", params.var)?;
        exec_ctx.add_variable(
            "git",
            params.git.map(|g| GitInfo {
                branch: g.branch,
                commit_sha: g.commit_sha,
                commit_message: g.commit_message,
                tag: g.tag,
                latest_tag: g.latest_tag,
                r#ref: g.r#ref,
            }),
        )?;
        exec_ctx.add_variable(
            "lttle",
            LttleInfo {
                tenant: ctx.tenant.clone(),
                user: ctx.sub.clone(),
                profile: params.lttle_profile,
            },
        )?;

        exec_ctx.add_stdlib_functions();
        exec_ctx.add_resource_functions(repository.clone(), ctx.tenant.clone());

        let output = match program.execute(&exec_ctx) {
            Ok(output) => output,
            Err(e) => {
                bail!("Failed to execute query: {}", e.to_string());
            }
        };

        let value = match output.json() {
            Ok(value) => value,
            Err(e) => {
                bail!("Failed to deserialize query result: {}", e.to_string());
            }
        };

        Ok(value.clone())
    }));

    match result {
        Ok(result) => result,
        Err(panic_info) => {
            let panic_msg = if let Some(msg) = panic_info.downcast_ref::<&str>() {
                msg.to_string()
            } else if let Some(msg) = panic_info.downcast_ref::<String>() {
                msg.clone()
            } else {
                "Unknown panic occurred in CEL parser".to_string()
            };
            error!("Query evaluation panicked: {}", panic_msg);

            bail!("Failed to evaluate query");
        }
    }
}
