use anyhow::Result;

use crate::{
    build_utils::{cargo, fs::write_if_changed},
    resources::ResourceBuildInfo,
};

pub async fn build_cel_functions(resources: &[ResourceBuildInfo]) -> Result<()> {
    let cel_out_path = cargo::build_out_dir_path("cel_functions.rs");

    let mut src = String::new();
    src.push_str("#[allow(dead_code, unused)]\n");
    src.push_str("#[cfg(feature = \"daemon\")]\n");
    src.push_str("pub mod cel_functions {\n");
    src.push_str("use std::sync::Arc;\n\n");
    src.push_str("use cel::{FunctionContext, ResolveResult, extractors::Arguments};\n\n");
    src.push_str("use crate::{\n");
    src.push_str("    repository::Repository,\n");
    src.push_str("    resources::{Convert, metadata},\n");
    src.push_str("};\n\n");

    // Generate CelResourceExt trait
    src.push_str("pub trait CelResourceExt {\n");
    src.push_str(
        "    fn add_resource_functions(&mut self, repository: Arc<Repository>, tenant: String);\n",
    );
    src.push_str("}\n\n");

    // Generate implementation
    src.push_str("impl CelResourceExt for cel::Context<'_> {\n");
    src.push_str(
        "    fn add_resource_functions(&mut self, repository: Arc<Repository>, tenant: String) {\n",
    );

    // Generate functions for each resource
    for resource in resources {
        generate_resource_functions(&mut src, resource);
    }

    src.push_str("    }\n");
    src.push_str("}\n");
    src.push_str("}\n");

    write_if_changed(&cel_out_path, src).await?;

    Ok(())
}

fn generate_resource_functions(src: &mut String, resource: &ResourceBuildInfo) {
    let resource_name = resource.name;
    let collection_name = resource.collection;
    let resource_name_lower = resource_name.to_lowercase();

    // Generate single resource getter function (e.g., "app")
    src.push_str(&format!(
        "        let query_repository_{} = repository.{}(tenant.clone());\n",
        resource_name_lower, collection_name
    ));
    src.push_str(&format!("        self.add_function(\n"));
    src.push_str(&format!("            \"{}\",\n", resource_name_lower));
    src.push_str(&format!("            move |_ftx: &FunctionContext, name: Arc<String>, namespace: Arc<String>| -> ResolveResult {{\n"));
    src.push_str(&format!(
        "                let Ok(Some(resource)) = query_repository_{}.get(\n",
        resource_name_lower
    ));
    src.push_str(&format!(
        "                    metadata::Namespace::from_value(Some(namespace.to_string())),\n"
    ));
    src.push_str(&format!("                    name.as_ref(),\n"));
    src.push_str(&format!("                ) else {{\n"));
    src.push_str(&format!("                    return Err(cel::ExecutionError::function_error(\"{}\", \"{} not found\"));\n", resource_name_lower, resource_name_lower));
    src.push_str(&format!("                }};\n\n"));
    src.push_str(&format!(
        "                let resource = resource.latest();\n"
    ));
    src.push_str(&format!(
        "                let value = cel::to_value(resource)\n"
    ));
    src.push_str(&format!("                    .map_err(|e| cel::ExecutionError::function_error(\"{}\", e.to_string()))?;\n\n", resource_name_lower));
    src.push_str(&format!("                Ok(value)\n"));
    src.push_str(&format!("            }},\n"));
    src.push_str(&format!("        );\n\n"));

    // Generate status getter function (e.g., "appStatus")
    src.push_str(&format!(
        "        let query_repository_{}_status = repository.{}(tenant.clone());\n",
        resource_name_lower, collection_name
    ));
    src.push_str(&format!("        self.add_function(\n"));
    src.push_str(&format!("            \"{}Status\",\n", resource_name_lower));
    src.push_str(&format!("            move |_ftx: &FunctionContext, name: Arc<String>, namespace: Arc<String>| -> ResolveResult {{\n"));
    src.push_str(&format!("                let Ok(Some(status)) = query_repository_{}_status.get_status(metadata::Metadata {{\n", resource_name_lower));
    src.push_str(&format!("                    name: name.to_string(),\n"));
    src.push_str(&format!(
        "                    namespace: Some(namespace.to_string()),\n"
    ));
    src.push_str(&format!("                }}) else {{\n"));
    src.push_str(&format!(
        "                    return Err(cel::ExecutionError::function_error(\n"
    ));
    src.push_str(&format!(
        "                        \"{}Status\",\n",
        resource_name_lower
    ));
    src.push_str(&format!(
        "                        \"{} status not found\",\n",
        resource_name_lower
    ));
    src.push_str(&format!("                    ));\n"));
    src.push_str(&format!("                }};\n\n"));
    src.push_str(&format!(
        "                let value = cel::to_value(status)\n"
    ));
    src.push_str(&format!("                    .map_err(|e| cel::ExecutionError::function_error(\"{}Status\", e.to_string()))?;\n\n", resource_name_lower));
    src.push_str(&format!("                Ok(value)\n"));
    src.push_str(&format!("            }},\n"));
    src.push_str(&format!("        );\n\n"));

    // Generate list function (e.g., "apps")
    let plural_name = format!("{}s", resource_name_lower);
    src.push_str(&format!(
        "        let query_repository_{}_list = repository.{}(tenant.clone());\n",
        resource_name_lower, collection_name
    ));
    src.push_str(&format!("        self.add_function(\n"));
    src.push_str(&format!("            \"{}\",\n", plural_name));
    src.push_str(&format!("            move |_ftx: &FunctionContext, Arguments(args): Arguments| -> ResolveResult {{\n"));
    src.push_str(&format!(
        "                let namespace = match args.get(0) {{\n"
    ));
    src.push_str(&format!(
        "                    Some(cel::Value::String(namespace)) => Some(namespace.to_string()),\n"
    ));
    src.push_str(&format!(
        "                    Some(cel::Value::Null) => None,\n"
    ));
    src.push_str(&format!("                    None => None,\n"));
    src.push_str(&format!("                    _ => {{\n"));
    src.push_str(&format!(
        "                        return Err(cel::ExecutionError::function_error(\n"
    ));
    src.push_str(&format!(
        "                            \"{}\",\n",
        plural_name
    ));
    src.push_str(&format!(
        "                            \"invalid namespace\",\n"
    ));
    src.push_str(&format!("                        ));\n"));
    src.push_str(&format!("                    }}\n"));
    src.push_str(&format!("                }};\n"));
    src.push_str(&format!(
        "                let namespace = metadata::Namespace::from_value(namespace);\n\n"
    ));
    src.push_str(&format!(
        "                let Ok(resources) = query_repository_{}_list.list(namespace) else {{\n",
        resource_name_lower
    ));
    src.push_str(&format!(
        "                    return Err(cel::ExecutionError::function_error(\n"
    ));
    src.push_str(&format!("                        \"{}\",\n", plural_name));
    src.push_str(&format!(
        "                        \"failed to list {}\",\n",
        plural_name
    ));
    src.push_str(&format!("                    ));\n"));
    src.push_str(&format!("                }};\n\n"));
    src.push_str(&format!(
        "                let value = cel::to_value(resources.latest())\n"
    ));
    src.push_str(&format!("                    .map_err(|e| cel::ExecutionError::function_error(\"{}\", e.to_string()))?;\n\n", plural_name));
    src.push_str(&format!("                Ok(value)\n"));
    src.push_str(&format!("            }},\n"));
    src.push_str(&format!("        );\n\n"));
}
