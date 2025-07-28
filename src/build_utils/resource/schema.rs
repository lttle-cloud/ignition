use std::collections::HashMap;

use anyhow::Result;
use schemars::{Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::fs::write;

use crate::{
    build_utils::cargo,
    machinery::api_schema::{
        ApiMethod, ApiPathSegment, ApiRequest, ApiResponse, ApiSchema, ApiService, ApiVerb,
    },
    resources::{
        ResourceBuildInfo,
        core::{add_core_service_schema_defs, core_api_service},
    },
};

#[derive(Serialize, Deserialize, Clone)]
struct PartialRootSchema {
    #[serde(rename = "oneOf")]
    one_of: Vec<Value>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde()]
struct RootSchema {
    #[serde(rename = "$schema")]
    schema: String,
    title: String,
    #[serde(rename = "oneOf")]
    one_of: Vec<Value>,
    #[serde(rename = "$defs")]
    defs: HashMap<String, Value>,
}

impl RootSchema {
    fn new(name: &str) -> Self {
        Self {
            schema: "https://json-schema.org/draft/2020-12/schema".to_string(),
            title: name.to_string(),
            one_of: Vec::new(),
            defs: HashMap::new(),
        }
    }

    fn add_one_of(&mut self, schema: &PartialRootSchema) {
        self.one_of.extend(schema.one_of.clone());
    }

    fn set_defs(&mut self, defs: &Map<String, Value>) {
        self.defs.clear();

        let mut sorted_keys = defs.keys().collect::<Vec<_>>();
        sorted_keys.sort_by(|a, b| b.cmp(a));

        for key in sorted_keys {
            self.defs.insert(key.clone(), defs[key].clone());
        }
    }
}

pub fn merge_json_schemas(schemas: Vec<Schema>, schema_generator: &mut SchemaGenerator) -> Value {
    let schemas = schemas
        .into_iter()
        .map(|s| s.to_value())
        .collect::<Vec<_>>();
    let schemas = schemas
        .into_iter()
        .map(|s| {
            serde_json::from_value::<PartialRootSchema>(s.clone()).expect(&format!(
                "Failed to parse partial root schema {}",
                s.to_string()
            ))
        })
        .collect::<Vec<_>>();

    let mut root_schema = RootSchema::new("Resources");

    for schema in schemas {
        root_schema.add_one_of(&schema);
    }

    root_schema.defs.clear();

    let definitions = schema_generator.definitions();
    let mut sorted_keys = definitions.keys().collect::<Vec<_>>();
    sorted_keys.sort_by(|a, b| a.cmp(b)); // Ascending order
    for key in sorted_keys {
        if let Some(value) = definitions.get(key) {
            root_schema.defs.insert(key.clone(), value.clone());
        }
    }

    serde_json::to_value(root_schema).expect("Failed to convert root schema to value")
}

pub async fn build_schema(
    resources: &[ResourceBuildInfo],
    schema_generator: &mut SchemaGenerator,
) -> Result<ApiSchema> {
    build_resources_json_schema(resources, schema_generator).await?;
    build_api_schema(resources, schema_generator).await
}

async fn build_api_schema(
    resources: &[ResourceBuildInfo],
    schema_generator: &mut SchemaGenerator,
) -> Result<ApiSchema> {
    let schema_out_path = cargo::workspace_root_dir_path("schemas/api.json").await?;

    let mut api_schema = ApiSchema::new();
    api_schema.services.push(core_api_service());
    add_core_service_schema_defs(schema_generator, &mut api_schema.defs)?;

    for resource in resources {
        if !resource.configuration.generate_service {
            continue;
        }

        let latest_version = resource
            .versions
            .iter()
            .find(|v| v.latest)
            .expect("No latest version found");

        let mut service = ApiService {
            crate_path: resource.crate_path.to_string(),
            name: resource.name.to_string(),
            tag: resource.tag.to_string(),
            namespaced: resource.namespaced,
            methods: Vec::new(),
        };

        if resource.configuration.generate_service_get {
            let method = ApiMethod {
                name: "get".to_string(),
                verb: ApiVerb::Get,
                path: vec![
                    ApiPathSegment::Static {
                        value: resource.tag.to_string(),
                    },
                    ApiPathSegment::ResourceName,
                ],
                request: None,
                response: Some(ApiResponse::TupleSchemaDefinition {
                    list: false,
                    optional: false,
                    names: vec![
                        latest_version.struct_name.to_string(),
                        resource.status.struct_name.to_string(),
                    ],
                }),
            };

            service.methods.push(method);
        }

        if resource.configuration.generate_service_list {
            let method = ApiMethod {
                name: "list".to_string(),
                verb: ApiVerb::Get,
                path: vec![ApiPathSegment::Static {
                    value: resource.tag.to_string(),
                }],
                request: None,
                response: Some(ApiResponse::TupleSchemaDefinition {
                    list: true,
                    optional: false,
                    names: vec![
                        latest_version.struct_name.to_string(),
                        resource.status.struct_name.to_string(),
                    ],
                }),
            };

            service.methods.push(method);
        }

        if resource.configuration.generate_service_delete {
            let method = ApiMethod {
                name: "delete".to_string(),
                verb: ApiVerb::Delete,
                path: vec![ApiPathSegment::Static {
                    value: resource.tag.to_string(),
                }],
                request: None,
                response: None,
            };

            service.methods.push(method);
        }

        if resource.configuration.generate_service_set {
            let method = ApiMethod {
                name: "apply".to_string(),
                verb: ApiVerb::Put,
                path: vec![ApiPathSegment::Static {
                    value: resource.tag.to_string(),
                }],
                request: Some(ApiRequest::SchemaDefinition {
                    name: resource.name.to_string(),
                }),
                response: None,
            };

            service.methods.push(method);
        }

        if resource.configuration.generate_service_get_status {
            let method = ApiMethod {
                name: "get_status".to_string(),
                verb: ApiVerb::Get,
                path: vec![
                    ApiPathSegment::Static {
                        value: resource.tag.to_string(),
                    },
                    ApiPathSegment::ResourceName,
                    ApiPathSegment::Static {
                        value: "status".to_string(),
                    },
                ],
                request: None,
                response: Some(ApiResponse::SchemaDefinition {
                    list: false,
                    optional: false,
                    name: resource.status.struct_name.to_string(),
                }),
            };

            service.methods.push(method);
        }

        api_schema.services.push(service);

        let partial_schema = resource.schema.clone().to_value();
        let partial_schema = serde_json::from_value::<PartialRootSchema>(partial_schema)
            .expect(&format!("Failed to parse partial root schema"));

        api_schema.defs.insert(
            resource.name.to_string(),
            serde_json::to_value(partial_schema)
                .expect("Failed to convert partial root schema to value"),
        );

        api_schema.defs.insert(
            resource.status.struct_name.to_string(),
            resource.status_schema.clone().to_value(),
        );
    }
    api_schema
        .defs
        .extend(schema_generator.definitions().clone());

    api_schema.defs.sort_keys();

    let src = serde_json::to_string_pretty(&api_schema)?;

    write(&schema_out_path, src).await?;

    Ok(api_schema)
}

async fn build_resources_json_schema(
    resources: &[ResourceBuildInfo],
    schema_generator: &mut SchemaGenerator,
) -> Result<()> {
    let schema_out_path = cargo::workspace_root_dir_path("schemas/resources.json").await?;

    let schema = merge_json_schemas(
        resources
            .iter()
            .map(|r| r.schema.clone())
            .collect::<Vec<_>>(),
        schema_generator,
    );

    let src = serde_json::to_string_pretty(&schema)?;

    write(&schema_out_path, src).await?;

    Ok(())
}
