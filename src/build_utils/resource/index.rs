use anyhow::Result;

use crate::{
    build_utils::{cargo, fs::write_if_changed},
    resources::ResourceBuildInfo,
};

pub async fn build_resource_index(resources: &[ResourceBuildInfo]) -> Result<()> {
    let resource_index_out_path = cargo::build_out_dir_path("resource_index.rs");

    let mut src = String::new();
    src.push_str("#[allow(dead_code, unused)]\n");
    src.push_str("pub mod resource_index {\n");

    src.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    src.push_str("pub enum ResourceKind {\n");

    for resource in resources {
        src.push_str(&format!("    {},\n", resource.name));
    }
    src.push_str("}\n\n");

    src.push_str(
        "#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]\n",
    );
    src.push_str("pub enum Resources {\n");

    for resource in resources {
        let latest_version = resource.versions.iter().find(|v| v.latest);

        if let Some(latest_version) = latest_version {
            let variant_value = format!(
                "crate::{}::{}",
                resource.crate_path, latest_version.struct_name
            );
            src.push_str(&format!("#[serde(rename = \"{}\")]\n", resource.tag));
            src.push_str(&format!("    {}({}),\n", resource.name, variant_value));
        }

        for version in &resource.versions {
            if !version.served && !version.latest {
                continue;
            }

            let variant_name = format!("{}{}", resource.name, version.variant_name);
            let variant_value = format!("crate::{}::{}", resource.crate_path, version.struct_name);
            let tag_name = format!("{}.{}", resource.tag, version.variant_name).to_lowercase();

            src.push_str(&format!("#[serde(rename = \"{}\")]\n", tag_name));
            src.push_str(&format!("    {}({}),\n", variant_name, variant_value));
        }
    }
    src.push_str("}\n\n");

    // Generate TryFrom implementations for ResourceKind
    src.push_str("impl TryFrom<Resources> for ResourceKind {\n");
    src.push_str("    type Error = anyhow::Error;\n\n");
    src.push_str("    fn try_from(value: Resources) -> Result<Self, Self::Error> {\n");
    src.push_str("        match value {\n");

    for resource in resources {
        let latest_version = resource.versions.iter().find(|v| v.latest);

        if let Some(_) = latest_version {
            src.push_str(&format!(
                "            Resources::{}(_) => Ok(ResourceKind::{}),\n",
                resource.name, resource.name
            ));
        }

        for version in &resource.versions {
            if !version.served && !version.latest {
                continue;
            }

            let variant_name = format!("{}{}", resource.name, version.variant_name);
            src.push_str(&format!(
                "            Resources::{}(_) => Ok(ResourceKind::{}),\n",
                variant_name, resource.name
            ));
        }
    }

    src.push_str("        }\n");
    src.push_str("    }\n");
    src.push_str("}\n\n");

    // Generate TryFrom implementations for each resource
    for resource in resources {
        let resource_name = resource.name;
        let resource_path = format!("crate::{}::{}", resource.crate_path, resource_name);

        src.push_str(&format!(
            "impl TryFrom<Resources> for {} {{\n",
            resource_path
        ));
        src.push_str("    type Error = anyhow::Error;\n\n");
        src.push_str("    fn try_from(value: Resources) -> Result<Self, Self::Error> {\n");
        src.push_str("        match value {\n");

        let latest_version = resource.versions.iter().find(|v| v.latest);

        if let Some(latest_version) = latest_version {
            src.push_str(&format!(
                "            Resources::{}(m) => Ok({}::{}(m)),\n",
                resource_name, resource_path, latest_version.variant_name
            ));
        }

        for version in &resource.versions {
            if !version.served && !version.latest {
                continue;
            }

            let variant_name = format!("{}{}", resource.name, version.variant_name);
            src.push_str(&format!(
                "            Resources::{}(m) => Ok({}::{}(m)),\n",
                variant_name, resource_path, version.variant_name
            ));
        }

        src.push_str(&format!(
            "            _ => Err(anyhow::anyhow!(\"resource index does not contain a {}\")),\n",
            resource_name.to_lowercase()
        ));
        src.push_str("        }\n");
        src.push_str("    }\n");
        src.push_str("}\n\n");
    }

    src.push_str("}\n\n");

    write_if_changed(&resource_index_out_path, src).await?;

    Ok(())
}
