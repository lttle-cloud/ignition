use std::str::FromStr;

use anyhow::{Result, bail};
use schemars::{JsonSchema, SchemaGenerator, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::machinery::api_schema::{ApiMethod, ApiPathSegment, ApiService, ApiVerb};

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
pub struct Namespace {
    pub name: String,
    pub created_at: u128,
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
    pub timestamp: u128,
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

pub fn core_api_service() -> ApiService {
    ApiService {
        name: "Core".to_string(),
        tag: "core".to_string(),
        crate_path: "resources::core".to_string(),
        namespaced: false,
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
        "LogStreamItem".to_string(),
        schema_for!(LogStreamItem).into(),
    );
    defs.insert(
        "LogStreamParams".to_string(),
        schema_for!(LogStreamParams).into(),
    );
    defs.insert("ExecParams".to_string(), schema_for!(ExecParams).into());

    Ok(())
}
