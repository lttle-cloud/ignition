use anyhow::Result;
use schemars::{JsonSchema, SchemaGenerator, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::machinery::api_schema::{ApiMethod, ApiPathSegment, ApiService, ApiVerb};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Me {
    pub tenant: String,
}

pub fn core_api_service() -> ApiService {
    ApiService {
        name: "Core".to_string(),
        tag: "core".to_string(),
        crate_path: "resources::core".to_string(),
        namespaced: false,
        methods: vec![ApiMethod {
            name: "me".to_string(),
            path: vec![
                ApiPathSegment::Static {
                    value: "core".to_string(),
                },
                ApiPathSegment::Static {
                    value: "me".to_string(),
                },
            ],
            verb: ApiVerb::Get,
            request: None,
            response: Some(
                crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                    list: false,
                    optional: false,
                    name: "Me".to_string(),
                },
            ),
        }],
    }
}

pub fn add_core_service_schema_defs(
    _schema_generator: &mut SchemaGenerator,
    defs: &mut Map<String, Value>,
) -> Result<()> {
    defs.insert("Me".to_string(), schema_for!(Me).into());

    Ok(())
}
