use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApiVerb {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "PUT")]
    Put,
    #[serde(rename = "DELETE")]
    Delete,
    #[serde(rename = "WS")]
    WebSocket,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ApiPathSegment {
    #[serde(rename = "static")]
    Static { value: String },
    #[serde(rename = "resource_name")]
    ResourceName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ApiRequest {
    #[serde(rename = "schema")]
    SchemaDefinition { name: String },
    #[serde(rename = "optional_schema")]
    OptionalSchemaDefinition { name: String },
    #[serde(rename = "tagged_schema")]
    TaggedSchemaDefinition { name: String, tag: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ApiResponse {
    #[serde(rename = "schema")]
    SchemaDefinition {
        list: bool,
        optional: bool,
        name: String,
    },
    #[serde(rename = "tuple")]
    TupleSchemaDefinition {
        list: bool,
        optional: bool,
        names: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMethod {
    pub name: String,
    pub namespaced: bool,
    pub verb: ApiVerb,
    pub path: Vec<ApiPathSegment>,
    pub request: Option<ApiRequest>,
    pub response: Option<ApiResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiService {
    pub name: String,
    pub tag: String,
    pub crate_path: String,
    pub namespaced: bool,
    pub methods: Vec<ApiMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSchema {
    pub services: Vec<ApiService>,
    pub defs: Map<String, Value>,
}

impl ApiSchema {
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
            defs: Map::new(),
        }
    }
}
