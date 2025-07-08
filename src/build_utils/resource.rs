use anyhow::Result;
use tokio::fs::write;

use crate::{
    build_utils::cargo,
    resources::{BuildableResource, ResourceBuildInfo, ResourceConfiguration},
};

pub struct ResourcesBuilder {
    resources: Vec<ResourceBuildInfo>,
}

impl ResourcesBuilder {
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
        }
    }

    pub fn resource<R: BuildableResource>(self) -> Self {
        self.resource_with_config::<R>(identity)
    }

    pub fn resource_with_config<R: BuildableResource>(
        mut self,
        configure: impl FnOnce(ResourceConfiguration) -> ResourceConfiguration,
    ) -> Self {
        let build_info = R::build_info(configure(ResourceConfiguration::new()));
        self.resources.push(build_info);
        self
    }

    pub async fn build(self) -> Result<()> {
        build_resource_index(&self.resources).await?;
        build_repository(&self.resources).await?;
        build_services(&self.resources).await?;
        Ok(())
    }
}

pub fn identity(config: ResourceConfiguration) -> ResourceConfiguration {
    config
}

async fn build_resource_index(resources: &[ResourceBuildInfo]) -> Result<()> {
    let resource_index_out_path = cargo::out_dir_path("resource_index.rs");

    let mut src = String::new();
    src.push_str("#[allow(dead_code, unused)]\n");
    src.push_str("pub mod resource_index {\n");

    src.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    src.push_str("pub enum ResourceKind {\n");

    for resource in resources {
        src.push_str(&format!("    {},\n", resource.name));
    }
    src.push_str("}\n\n");
    src.push_str("}\n\n");

    write(&resource_index_out_path, src).await?;

    Ok(())
}

async fn build_repository(resources: &[ResourceBuildInfo]) -> Result<()> {
    let repository_out_path = cargo::out_dir_path("repository.rs");

    let mut src = String::new();
    src.push_str("#[allow(dead_code, unused)]\n");
    src.push_str("pub mod repository {\n");
    src.push_str("use anyhow::Result;\n");
    src.push_str("use std::sync::{Arc, Weak};\n\n");
    src.push_str("use crate::{\n");
    src.push_str("    controller::{context::ControllerEvent, scheduler::Scheduler},\n");
    src.push_str("    machinery::store::Store,\n");
    src.push_str("    resources::{Convert, FromResourceAsync, ProvideKey, ProvideMetadata, metadata::Metadata},\n");

    // Add resource imports
    for resource in resources {
        if let Some(status) = &resource.status {
            src.push_str(&format!(
                "    {}::{{{}, {}}},\n",
                resource.crate_path, resource.name, status.struct_name
            ));
        } else {
            src.push_str(&format!(
                "    {}::{{{}}},\n",
                resource.crate_path, resource.name
            ));
        }
    }
    src.push_str("};\n\n");

    // Generate Repository struct
    src.push_str("pub struct Repository {\n");
    src.push_str("    store: Arc<Store>,\n");
    src.push_str("    scheduler: Weak<Scheduler>,\n");
    src.push_str("}\n\n");

    src.push_str("impl Repository {\n");
    src.push_str("    pub fn new(store: Arc<Store>, scheduler: Weak<Scheduler>) -> Self {\n");
    src.push_str("        Self { store, scheduler }\n");
    src.push_str("    }\n\n");
    src.push_str("    fn get_scheduler(&self) -> Option<Arc<Scheduler>> {\n");
    src.push_str("        self.scheduler.upgrade()\n");
    src.push_str("    }\n\n");

    // Generate repository methods for each resource
    for resource in resources {
        let resource_name = resource.name;
        let repository_name = format!("{}Repository", resource_name);

        src.push_str(&format!(
            "    pub fn {}(&self, tenant: impl AsRef<str>) -> {} {{\n",
            resource.collection, repository_name
        ));
        src.push_str(&format!(
            "        {}::new(self.store.clone(), tenant, self.scheduler.clone())\n",
            repository_name
        ));
        src.push_str("    }\n\n");
    }
    src.push_str("}\n\n");

    // Generate individual resource repositories
    for resource in resources {
        generate_resource_repository(&mut src, resource);
    }

    src.push_str("}\n\n");

    write(&repository_out_path, src).await?;

    Ok(())
}

fn generate_resource_repository(src: &mut String, resource: &ResourceBuildInfo) {
    let resource_name = resource.name;
    let repository_name = format!("{}Repository", resource_name);
    let collection_name = resource.collection;

    src.push_str(&format!("pub struct {} {{\n", repository_name));
    src.push_str("    store: Arc<Store>,\n");
    src.push_str("    tenant: String,\n");
    src.push_str("    scheduler: Weak<Scheduler>,\n");
    src.push_str("}\n\n");

    src.push_str(&format!("impl {} {{\n", repository_name));

    // Constructor
    src.push_str("    pub fn new(store: Arc<Store>, tenant: impl AsRef<str>, scheduler: Weak<Scheduler>) -> Self {\n");
    src.push_str("        Self {\n");
    src.push_str("            store: store,\n");
    src.push_str("            tenant: tenant.as_ref().to_string(),\n");
    src.push_str("            scheduler,\n");
    src.push_str("        }\n");
    src.push_str("    }\n\n");

    src.push_str("    fn get_scheduler(&self) -> Option<Arc<Scheduler>> {\n");
    src.push_str("        self.scheduler.upgrade()\n");
    src.push_str("    }\n\n");

    // Get method
    src.push_str("    pub fn get(\n");
    src.push_str("        &self,\n");
    src.push_str("        namespace: impl AsRef<str>,\n");
    src.push_str("        name: impl AsRef<str>,\n");
    src.push_str(&format!("    ) -> Result<Option<{}>> {{\n", resource_name));
    src.push_str(&format!(
        "        let key = {}::key(self.tenant.clone(), Metadata::new(name, Some(namespace)));\n",
        resource_name
    ));
    src.push_str("        let resource = self.store.get(key)?;\n");
    src.push_str("        Ok(resource)\n");
    src.push_str("    }\n\n");

    // Set method
    src.push_str(&format!(
        "    pub async fn set(&self, resource: {}) -> Result<()> {{\n",
        resource_name
    ));
    src.push_str("        let metadata = resource.metadata();\n");
    src.push_str(&format!(
        "        let key = {}::key(self.tenant.clone(), metadata.clone());\n",
        resource_name
    ));
    src.push_str("        let mut resource = resource.latest();\n");
    src.push_str("        resource.name = metadata.name.clone();\n");
    if resource.namespaced {
        src.push_str("        resource.namespace = metadata.namespace.clone().into();\n");
    }
    src.push_str(&format!(
        "        let stored_resource: {} = resource.into();\n",
        resource_name
    ));
    src.push_str("        self.store.put(key, stored_resource)?;\n");
    src.push_str("        \n");
    src.push_str("        // Notify scheduler of resource change\n");
    src.push_str("        if let Some(scheduler) = self.get_scheduler() {\n");
    src.push_str(&format!(
            "            let event = ControllerEvent::ResourceChange(crate::resource_index::ResourceKind::{}, metadata);\n",
            resource_name
        ));
    src.push_str("            if let Err(e) = scheduler.push(&self.tenant, event).await {\n");
    src.push_str("                tracing::warn!(\"Failed to notify scheduler of resource change: {}\", e);\n");
    src.push_str("            }\n");
    src.push_str("        }\n");
    src.push_str("        \n");
    src.push_str("        Ok(())\n");
    src.push_str("    }\n\n");

    // Delete method
    src.push_str("    pub async fn delete(&self, namespace: impl AsRef<str>, name: impl AsRef<str>) -> Result<()> {\n");
    src.push_str("        let namespace_str = namespace.as_ref().to_string();\n");
    src.push_str("        let name_str = name.as_ref().to_string();\n");
    src.push_str(&format!(
        "        let key = {}::key(self.tenant.clone(), Metadata::new(&name_str, Some(&namespace_str)));\n",
        resource_name
    ));
    src.push_str("        let Some(_resource) = self.store.get(key.clone())? else {\n");
    src.push_str(&format!(
        "            return Err(anyhow::anyhow!(\"{collection_name} not found\"));\n"
    ));
    src.push_str("        };\n");
    src.push_str("        self.store.delete(key)?;\n");
    src.push_str("        \n");
    src.push_str("        // Notify scheduler of resource deletion\n");
    src.push_str("        if let Some(scheduler) = self.get_scheduler() {\n");
    src.push_str("            let metadata = Metadata::new(name_str, Some(namespace_str));\n");
    src.push_str(&format!(
            "            let event = ControllerEvent::ResourceChange(crate::resource_index::ResourceKind::{}, metadata);\n",
            resource_name
        ));
    src.push_str("            if let Err(e) = scheduler.push(&self.tenant, event).await {\n");
    src.push_str("                tracing::warn!(\"Failed to notify scheduler of resource deletion: {}\", e);\n");
    src.push_str("            }\n");
    src.push_str("        }\n");
    src.push_str("        \n");
    src.push_str("        Ok(())\n");
    src.push_str("    }\n\n");

    // List method
    src.push_str(&format!(
        "    pub fn list(&self, namespace: Option<String>) -> Result<Vec<{}>> {{\n",
        resource_name
    ));
    src.push_str(&format!(
        "        let key = {}::partial_key(self.tenant.clone(), namespace);\n",
        resource_name
    ));
    src.push_str("        let resources = self.store.list(key)?;\n");
    src.push_str("        Ok(resources)\n");
    src.push_str("    }\n");

    // Status methods if status exists
    if let Some(status_info) = &resource.status {
        let status_name = status_info.struct_name;

        src.push_str(&format!(
            "\n    pub async fn get_status(&self, metadata: Metadata) -> Result<{}> {{\n",
            status_name
        ));
        src.push_str(&format!(
            "        let key = {}::key(self.tenant.clone(), metadata.clone());\n",
            status_name
        ));
        src.push_str("        if let Some(status) = self.store.get(key.clone())? {\n");
        src.push_str("            return Ok(status);\n");
        src.push_str("        };\n\n");

        src.push_str(
            "        let Some(resource) = self.get(&metadata.namespace, &metadata.name)? else {\n",
        );
        src.push_str(&format!(
            "            return Err(anyhow::anyhow!(\"{collection_name} not found\"));\n"
        ));
        src.push_str("        };\n\n");
        src.push_str(&format!(
            "        let status = {}::from_resource(resource).await?;\n",
            status_name
        ));
        src.push_str("        self.set_status(metadata, status.clone()).await?;\n");
        src.push_str("        Ok(status)\n");
        src.push_str("    }\n\n");

        src.push_str(&format!(
            "    pub async fn set_status(&self, metadata: Metadata, status: {}) -> Result<()> {{\n",
            status_name
        ));
        src.push_str(&format!(
            "        let key = {}::key(self.tenant.clone(), metadata.clone());\n",
            status_name
        ));
        src.push_str("        self.store.put(key, status)?;\n");
        src.push_str("        \n");
        src.push_str("        // Notify scheduler of status change\n");
        src.push_str("        if let Some(scheduler) = self.get_scheduler() {\n");
        src.push_str(&format!(
            "            let event = ControllerEvent::ResourceStatusChange(crate::resource_index::ResourceKind::{}, metadata);\n",
            resource_name
        ));
        src.push_str("            if let Err(e) = scheduler.push(&self.tenant, event).await {\n");
        src.push_str("                tracing::warn!(\"Failed to notify scheduler of status change: {}\", e);\n");
        src.push_str("            }\n");
        src.push_str("        }\n");
        src.push_str("        \n");
        src.push_str("        Ok(())\n");
        src.push_str("    }\n");
    }

    src.push_str("}\n\n");
}

async fn build_services(resources: &[ResourceBuildInfo]) -> Result<()> {
    let service_out_path = cargo::out_dir_path("services.rs");

    let mut src = String::new();
    src.push_str("#[allow(dead_code, unused)]\n");
    src.push_str("pub mod services {\n");
    src.push_str("use std::sync::Arc;\n\n");
    src.push_str("use axum::{\n");
    src.push_str("    Json, Router,\n");
    src.push_str("    extract::{Path, State},\n");
    src.push_str("    http::StatusCode,\n");
    src.push_str("    response::IntoResponse,\n");
    src.push_str("    routing::{delete, get, put},\n");
    src.push_str("};\n\n");
    src.push_str("use crate::{\n");
    src.push_str("    api::{\n");
    src.push_str("        ApiState,\n");
    src.push_str("        context::ServiceRequestContext,\n");
    src.push_str("        resource_service::{ResourceService, ResourceServiceRouter},\n");
    src.push_str("    },\n");
    src.push_str("    repository::Repository,\n");
    src.push_str("    resources::metadata::{DEFAULT_NAMESPACE, Metadata},\n");

    // Add resource imports
    for resource in resources {
        if resource.configuration.generate_service {
            src.push_str(&format!(
                "    {}::{},\n",
                resource.crate_path, resource.name
            ));
        }
    }
    src.push_str("};\n\n");

    // Generate service implementations for each resource
    for resource in resources {
        if !resource.configuration.generate_service {
            continue;
        }

        generate_resource_service(&mut src, resource);
    }

    src.push_str("}\n\n");

    write(&service_out_path, src).await?;

    Ok(())
}

fn generate_resource_service(src: &mut String, resource: &ResourceBuildInfo) {
    let resource_name = resource.name;
    let service_name = format!("{}Service", resource_name);
    let collection_name = resource.collection;
    let namespaced = resource.namespaced;

    src.push_str(&format!("pub struct {};\n\n", service_name));
    src.push_str(&format!("impl ResourceService for {} {{\n", service_name));
    src.push_str("    fn create_router(_state: Arc<ApiState>) -> ResourceServiceRouter {\n");

    // Generate list method if enabled
    if resource.configuration.generate_service_list {
        src.push_str("        async fn list(\n");
        src.push_str("            state: State<Arc<ApiState>>,\n");
        src.push_str("            ctx: ServiceRequestContext,\n");
        src.push_str("        ) -> impl IntoResponse {\n");
        src.push_str(&format!(
            "            let resources = state.repository.{}(ctx.tenant).list(ctx.namespace);\n\n",
            collection_name
        ));
        src.push_str("            match resources {\n");
        src.push_str(
            "                Ok(resources) => (StatusCode::OK, Json(resources)).into_response(),\n",
        );
        src.push_str("                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),\n");
        src.push_str("            }\n");
        src.push_str("        }\n\n");
    }

    // Generate get_one method if enabled
    if resource.configuration.generate_service_get {
        src.push_str("        async fn get_one(\n");
        src.push_str("            state: State<Arc<ApiState>>,\n");
        src.push_str("            ctx: ServiceRequestContext,\n");
        src.push_str("            Path(name): Path<String>,\n");
        src.push_str("        ) -> impl IntoResponse {\n");
        if namespaced {
            src.push_str("            let namespace = ctx.namespace.unwrap_or(DEFAULT_NAMESPACE.to_string());\n\n");
            src.push_str(&format!(
                "            let resource = state.repository.{}(ctx.tenant).get(namespace, name);\n",
                collection_name
            ));
        } else {
            src.push_str(&format!(
                "            let resource = state.repository.{}(ctx.tenant).get(\"default\", name);\n",
                collection_name
            ));
        }
        src.push_str("\n            match resource {\n");
        src.push_str(
            "                Ok(resource) => (StatusCode::OK, Json(resource)).into_response(),\n",
        );
        src.push_str("                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),\n");
        src.push_str("            }\n");
        src.push_str("        }\n\n");
    }

    if resource.configuration.generate_service_get_status {
        src.push_str("        async fn get_status(\n");
        src.push_str("            state: State<Arc<ApiState>>,\n");
        src.push_str("            ctx: ServiceRequestContext,\n");
        src.push_str("            Path(name): Path<String>,\n");
        src.push_str("        ) -> impl IntoResponse {\n");
        src.push_str(&format!(
            "            let metadata = Metadata::new(name, ctx.namespace);\n\n",
        ));
        src.push_str(&format!(
            "            let result = state.repository.{}(ctx.tenant).get_status(metadata).await;\n\n",
            collection_name
        ));
        src.push_str("            match result {\n");
        src.push_str(
            "                Ok(status) => (StatusCode::OK, Json(status)).into_response(),\n",
        );
        src.push_str("                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),\n");
        src.push_str("            }\n");
        src.push_str("        }\n\n");
    }

    // Generate set method if enabled
    if resource.configuration.generate_service_set {
        src.push_str(&format!("        async fn set(\n"));
        src.push_str("            state: State<Arc<ApiState>>,\n");
        src.push_str("            ctx: ServiceRequestContext,\n");
        src.push_str(&format!(
            "            Json(resource): Json<{}>,\n",
            resource_name
        ));
        src.push_str("        ) -> impl IntoResponse {\n");
        src.push_str(&format!(
            "            let result = state.repository.{}(ctx.tenant).set(resource).await;\n\n",
            collection_name
        ));
        src.push_str("            match result {\n");
        src.push_str("                Ok(()) => StatusCode::OK.into_response(),\n");
        src.push_str("                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),\n");
        src.push_str("            }\n");
        src.push_str("        }\n\n");
    }

    // Generate remove method if enabled
    if resource.configuration.generate_service_delete {
        src.push_str("        async fn remove(\n");
        src.push_str("            state: State<Arc<ApiState>>,\n");
        src.push_str("            ctx: ServiceRequestContext,\n");
        src.push_str("            Path(name): Path<String>,\n");
        src.push_str("        ) -> impl IntoResponse {\n");
        if namespaced {
            src.push_str("            let namespace = ctx.namespace.unwrap_or(DEFAULT_NAMESPACE.to_string());\n\n");
            src.push_str(&format!(
                "            let result = state.repository.{}(ctx.tenant).delete(namespace, name).await;\n",
                collection_name
            ));
        } else {
            src.push_str(&format!(
                "            let result = state.repository.{}(ctx.tenant).delete(\"default\", name).await;\n",
                collection_name
            ));
        }
        src.push_str("\n            match result {\n");
        src.push_str("                Ok(()) => StatusCode::OK.into_response(),\n");
        src.push_str("                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),\n");
        src.push_str("            }\n");
        src.push_str("        }\n\n");
    }

    // Build router based on enabled methods
    src.push_str("        let mut router = Router::new();\n");

    if resource.configuration.generate_service_list {
        src.push_str("        router = router.route(\"/\", get(list));\n");
    }

    if resource.configuration.generate_service_get {
        src.push_str("        router = router.route(\"/{name}\", get(get_one));\n");
    }

    if resource.configuration.generate_service_set {
        src.push_str("        router = router.route(\"/\", put(set));\n");
    }

    if resource.configuration.generate_service_delete {
        src.push_str("        router = router.route(\"/{name}\", delete(remove));\n");
    }

    if resource.configuration.generate_service_get_status {
        src.push_str("        router = router.route(\"/{name}/status\", get(get_status));\n");
    }

    src.push_str("\n");
    src.push_str(&format!(
        "        ResourceServiceRouter::new(\"{}\".to_string(), \"/{}\".to_string(), router)\n",
        resource_name, collection_name
    ));
    src.push_str("    }\n");
    src.push_str("}\n\n");
}
