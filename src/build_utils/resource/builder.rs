use anyhow::Result;
use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};

use crate::{
    build_utils::resource::api_client::{build_api_spec, build_ts_client},
    resources::{
        BuildableResource, DamascusBuildableResource, ResourceBuildInfo, ResourceConfiguration,
    },
};

use super::{
    cel::build_cel_functions, client::build_internal_rust_api_client, index::build_resource_index,
    repository::build_repository, schema::build_schema, services::build_services,
};

pub struct ResourcesBuilder {
    resources: Vec<ResourceBuildInfo>,
    schema_generator: SchemaGenerator,
}

impl ResourcesBuilder {
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
            schema_generator: SchemaGenerator::new(SchemaSettings::default()),
        }
    }

    #[allow(dead_code)]
    pub fn resource<R: BuildableResource + DamascusBuildableResource>(self) -> Self {
        self.resource_with_config::<R>(identity)
    }

    pub fn resource_with_config<R: BuildableResource + DamascusBuildableResource>(
        mut self,
        configure: impl FnOnce(ResourceConfiguration) -> ResourceConfiguration,
    ) -> Self {
        let schema = R::SchemaProvider::json_schema(&mut self.schema_generator);
        let status_schema = R::StatusSchemaProvider::json_schema(&mut self.schema_generator);

        let build_info = R::build_info(
            configure(ResourceConfiguration::new()),
            schema,
            status_schema,
        );
        self.resources.push(build_info);
        self
    }

    pub async fn build(mut self) -> Result<()> {
        build_resource_index(&self.resources).await?;
        build_repository(&self.resources).await?;
        build_services(&self.resources).await?;
        build_cel_functions(&self.resources).await?;

        let internal_api_schema = build_schema(&self.resources, &mut self.schema_generator).await?;
        build_internal_rust_api_client(&internal_api_schema).await?;

        let api_sepc = build_api_spec(&self.resources).await?;
        build_ts_client(&api_sepc).await?;

        Ok(())
    }
}

pub fn identity(config: ResourceConfiguration) -> ResourceConfiguration {
    config
}
