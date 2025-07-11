use anyhow::Result;
use tokio::fs::write;

use crate::{build_utils::cargo, resources::ResourceBuildInfo};

pub async fn build_services(resources: &[ResourceBuildInfo]) -> Result<()> {
    let service_out_path = cargo::build_out_dir_path("services.rs");

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
    src.push_str("    resources::Convert,\n");
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
            "                Ok(resources) => (StatusCode::OK, Json(resources.latest())).into_response(),\n",
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
            "                Ok(resource) => (StatusCode::OK, Json(resource.map(|r| r.latest()))).into_response(),\n",
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
