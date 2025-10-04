use std::{collections::BTreeMap, str::FromStr};

use anyhow::{Result, bail};
use schemars::{JsonSchema, SchemaGenerator, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::machinery::api_schema::{ApiMethod, ApiPathSegment, ApiService, ApiVerb};

// Increment this when you want to force clients to update their version
pub const CLIENT_COMPAT_VERSION: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Me {
    pub tenant: String,
    pub sub: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListNamespaces {
    pub namespaces: Vec<Namespace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteNamespaceParams {
    pub namespace: String,
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteNamespaceResponse {
    pub resources: Vec<DeletedResource>,
    pub did_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeletedResource {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Namespace {
    pub name: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegistryRobot {
    pub registry: String,
    pub user: String,
    pub pass: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum LogStreamTarget {
    #[serde(rename = "stdout")]
    Stdout,
    #[serde(rename = "stderr")]
    Stderr,
}

impl FromStr for LogStreamTarget {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let target = match s {
            "stdout" => LogStreamTarget::Stdout,
            "stderr" => LogStreamTarget::Stderr,
            _ => bail!("Invalid log stream target: {}", s),
        };

        Ok(target)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LogStreamItem {
    pub timestamp: u64,
    pub message: String,
    pub target_stream: LogStreamTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum LogStreamParams {
    Machine {
        machine_name: String,
        start_ts_ns: Option<String>,
        end_ts_ns: Option<String>,
    },
    Group {
        group_name: String,
        start_ts_ns: Option<String>,
        end_ts_ns: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExecParams {
    pub machine_name: String,
    pub command: String,
    pub stdin: Option<bool>,
    pub tty: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryParams {
    pub query: String,
    pub env: BTreeMap<String, String>,
    pub var: BTreeMap<String, Value>,
    pub git: Option<QueryGitInfo>,
    pub lttle_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryGitInfo {
    pub branch: Option<String>,
    pub commit_sha: String,
    pub commit_message: String,
    pub tag: Option<String>,
    pub latest_tag: Option<String>,
    pub r#ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueryResponse {
    pub query_result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AllocatedBuilder {
    pub host: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
    pub ca_cert_pem: String,
}

pub fn core_api_service() -> ApiService {
    ApiService {
        name: "Core".to_string(),
        tag: "core".to_string(),
        crate_path: "resources::core".to_string(),
        namespaced: false,
        versioned: false,
        methods: vec![
            ApiMethod {
                name: "me".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "me".to_string(),
                    },
                ],
                namespaced: false,
                verb: ApiVerb::Get,
                request: None,
                response: Some(
                    crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                        list: false,
                        optional: false,
                        name: "Me".to_string(),
                    },
                ),
            },
            ApiMethod {
                name: "get_registry_robot".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "registry".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "robot".to_string(),
                    },
                ],
                namespaced: false,
                verb: ApiVerb::Get,
                request: None,
                response: Some(
                    crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                        list: false,
                        optional: false,
                        name: "RegistryRobot".to_string(),
                    },
                ),
            },
            ApiMethod {
                name: "get_registry_builder_robot".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "registry".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "builder-robot".to_string(),
                    },
                ],
                namespaced: false,
                verb: ApiVerb::Get,
                request: None,
                response: Some(
                    crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                        list: false,
                        optional: false,
                        name: "RegistryRobot".to_string(),
                    },
                ),
            },
            ApiMethod {
                name: "list_namespaces".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "namespaces".to_string(),
                    },
                ],
                namespaced: false,
                verb: ApiVerb::Get,
                request: None,
                response: Some(
                    crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                        list: false,
                        optional: false,
                        name: "ListNamespaces".to_string(),
                    },
                ),
            },
            ApiMethod {
                name: "delete_namespace".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "namespaces".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "delete".to_string(),
                    },
                ],
                namespaced: false,
                verb: ApiVerb::Put,
                request: Some(crate::machinery::api_schema::ApiRequest::SchemaDefinition {
                    name: "DeleteNamespaceParams".to_string(),
                }),
                response: Some(
                    crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                        list: false,
                        optional: false,
                        name: "DeleteNamespaceResponse".to_string(),
                    },
                ),
            },
            ApiMethod {
                name: "stream_logs".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "logs".to_string(),
                    },
                ],
                namespaced: true,
                verb: ApiVerb::WebSocket,
                response: Some(
                    crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                        list: false,
                        optional: false,
                        name: "LogStreamItem".to_string(),
                    },
                ),
                request: Some(crate::machinery::api_schema::ApiRequest::SchemaDefinition {
                    name: "LogStreamParams".to_string(),
                }),
            },
            ApiMethod {
                name: "exec".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "exec".to_string(),
                    },
                ],
                namespaced: true,
                verb: ApiVerb::WebSocket,
                request: Some(crate::machinery::api_schema::ApiRequest::SchemaDefinition {
                    name: "ExecParams".to_string(),
                }),
                response: Some(crate::machinery::api_schema::ApiResponse::RawSocket),
            },
            ApiMethod {
                name: "query".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "query".to_string(),
                    },
                ],
                namespaced: false,
                verb: ApiVerb::Put,
                request: Some(crate::machinery::api_schema::ApiRequest::SchemaDefinition {
                    name: "QueryParams".to_string(),
                }),
                response: Some(
                    crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                        list: false,
                        optional: false,
                        name: "QueryResponse".to_string(),
                    },
                ),
            },
            ApiMethod {
                name: "alloc_builder".to_string(),
                path: vec![
                    ApiPathSegment::Static {
                        value: "core".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "build".to_string(),
                    },
                    ApiPathSegment::Static {
                        value: "alloc".to_string(),
                    },
                ],
                namespaced: false,
                verb: ApiVerb::Put,
                request: None,
                response: Some(
                    crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                        name: "AllocatedBuilder".to_string(),
                        list: false,
                        optional: false,
                    },
                ),
            },
        ],
    }
}

pub fn add_core_service_schema_defs(
    _schema_generator: &mut SchemaGenerator,
    defs: &mut Map<String, Value>,
) -> Result<()> {
    defs.insert("Me".to_string(), schema_for!(Me).into());
    defs.insert("Namespace".to_string(), schema_for!(Namespace).into());
    defs.insert(
        "ListNamespaces".to_string(),
        schema_for!(ListNamespaces).into(),
    );
    defs.insert(
        "DeleteNamespaceParams".to_string(),
        schema_for!(DeleteNamespaceParams).into(),
    );
    defs.insert(
        "DeleteNamespaceResponse".to_string(),
        schema_for!(DeleteNamespaceResponse).into(),
    );
    defs.insert(
        "RegistryRobot".to_string(),
        schema_for!(RegistryRobot).into(),
    );
    defs.insert(
        "LogStreamItem".to_string(),
        schema_for!(LogStreamItem).into(),
    );
    defs.insert(
        "LogStreamParams".to_string(),
        schema_for!(LogStreamParams).into(),
    );
    defs.insert("ExecParams".to_string(), schema_for!(ExecParams).into());
    defs.insert("QueryParams".to_string(), schema_for!(QueryParams).into());
    defs.insert(
        "QueryResponse".to_string(),
        schema_for!(QueryResponse).into(),
    );
    defs.insert(
        "AllocatedBuilder".to_string(),
        schema_for!(AllocatedBuilder).into(),
    );

    Ok(())
}
