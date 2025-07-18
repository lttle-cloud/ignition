use anyhow::Result;
use tokio::fs::write;

use crate::{build_utils::cargo, resources::ResourceBuildInfo};

pub async fn build_repository(resources: &[ResourceBuildInfo]) -> Result<()> {
    let repository_out_path = cargo::build_out_dir_path("repository.rs");

    let mut src = String::new();
    src.push_str("#[allow(dead_code, unused)]\n");
    src.push_str("pub mod repository {\n");
    src.push_str("use anyhow::Result;\n");
    src.push_str("use std::sync::{Arc, Weak};\n\n");
    src.push_str("use crate::{\n");
    src.push_str("    controller::{context::ControllerEvent, scheduler::Scheduler},\n");
    src.push_str("    machinery::store::Store,\n");
    src.push_str("    resources::{Convert, FromResource, ProvideKey, ProvideMetadata, metadata::{Metadata, Namespace}},\n");

    // Add resource imports
    for resource in resources {
        src.push_str(&format!(
            "    {}::{{{}, {}}},\n",
            resource.crate_path, resource.name, resource.status.struct_name
        ));
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
    src.push_str("        namespace: Namespace,\n");
    src.push_str("        name: impl AsRef<str>,\n");
    src.push_str(&format!("    ) -> Result<Option<{}>> {{\n", resource_name));
    src.push_str(&format!(
        "        let key = {}::key(self.tenant.clone(), Metadata::new(name, namespace))?;\n",
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
        "        let key = {}::key(self.tenant.clone(), metadata.clone())?;\n",
        resource_name
    ));
    src.push_str("        let mut resource = resource.latest();\n");
    src.push_str("        resource.name = metadata.name.clone();\n");
    if resource.namespaced {
        src.push_str("        resource.namespace = metadata.namespace.clone();\n");
    }
    src.push_str(&format!(
        "        let stored_resource: {} = resource.into();\n",
        resource_name
    ));
    src.push_str("        self.store.put(key, stored_resource.clone())?;\n");
    src.push_str("        \n");

    let status_name = resource.status.struct_name;
    src.push_str(&format!(
        "        let status_key = {}::key(self.tenant.clone(), metadata.clone())?;\n",
        status_name
    ));
    src.push_str("        if self.store.get(status_key.clone())?.is_none() {\n");
    src.push_str(&format!(
        "            let status = {}::from_resource(stored_resource)?;\n",
        status_name
    ));
    src.push_str("            self.store.put(status_key, status)?;\n");
    src.push_str("        };\n");
    src.push_str("        \n");

    src.push_str("        // Notify scheduler of resource change\n");
    src.push_str("        if let Some(scheduler) = self.get_scheduler() {\n");
    src.push_str(&format!(
            "            let event = ControllerEvent::ResourceChange(crate::resource_index::ResourceKind::{}, metadata.clone());\n",
            resource_name
        ));
    src.push_str("            if let Err(e) = scheduler.push(&self.tenant, event).await {\n");
    src.push_str("                tracing::warn!(\"Failed to notify scheduler of resource change: {}\", e);\n");
    src.push_str("            }\n");
    src.push_str("        }\n");

    src.push_str("        Ok(())\n");
    src.push_str("    }\n\n");

    // Delete method
    src.push_str("    pub async fn delete(&self, namespace: Namespace, name: impl AsRef<str>) -> Result<()> {\n");
    src.push_str("        let name_str = name.as_ref().to_string();\n");
    src.push_str(&format!(
        "        let key = {}::key(self.tenant.clone(), Metadata::new(&name_str, namespace.clone()))?;\n",
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
    src.push_str("            let metadata = Metadata::new(name_str, namespace);\n");
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
        "    pub fn list(&self, namespace: Namespace) -> Result<Vec<{}>> {{\n",
        resource_name
    ));
    src.push_str(&format!(
        "        let key = {}::partial_key(self.tenant.clone(), namespace)?;\n",
        resource_name
    ));
    src.push_str("        let resources = self.store.list(key)?;\n");
    src.push_str("        Ok(resources)\n");
    src.push_str("    }\n");

    // Status methods if status exists
    let status_name = resource.status.struct_name;
    src.push_str(&format!(
        "\n    pub fn get_status(&self, metadata: Metadata) -> Result<Option<{}>> {{\n",
        status_name
    ));
    src.push_str(&format!(
        "        let key = {}::key(self.tenant.clone(), metadata.clone())?;\n",
        status_name
    ));

    src.push_str("        self.store.get(key.clone())\n");
    src.push_str("    }\n\n");

    src.push_str(&format!(
        "    pub async fn set_status(&self, metadata: Metadata, status: {}) -> Result<()> {{\n",
        status_name
    ));
    src.push_str(&format!(
        "        let key = {}::key(self.tenant.clone(), metadata.clone())?;\n",
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
    src.push_str(
        "                tracing::warn!(\"Failed to notify scheduler of status change: {}\", e);\n",
    );
    src.push_str("            }\n");
    src.push_str("        }\n");
    src.push_str("        \n");
    src.push_str("        Ok(())\n");
    src.push_str("    }\n\n");

    src.push_str(&format!(
        "    pub fn get_with_status(&self, metadata: Metadata) -> Result<Option<({}, {})>> {{\n",
        resource_name, status_name
    ));
    src.push_str(&format!(
        "        let key = {}::key(self.tenant.clone(), metadata.clone())?;\n",
        resource_name
    ));
    src.push_str("        let Some(resource) = self.store.get(key.clone())? else {\n");
    src.push_str("            return Ok(None);\n");
    src.push_str("        };\n");
    src.push_str(&format!(
        "        let status_key = {}::key(self.tenant.clone(), metadata.clone())?;\n",
        status_name
    ));
    src.push_str("        let Some(status) = self.store.get(status_key.clone())? else {\n");
    src.push_str("            return Ok(None);\n");
    src.push_str("        };\n");
    src.push_str("        Ok(Some((resource, status)))\n");
    src.push_str("    }\n\n");

    src.push_str(&format!(
        "    pub async fn patch_status<F>(&self, metadata: Metadata, mut f: F) -> Result<{}>\n",
        status_name
    ));
    src.push_str("    where\n");
    src.push_str(&format!("        F: FnMut(&mut {}),\n", status_name));
    src.push_str("    {\n");
    src.push_str(&format!(
        "        let Some(mut status) = self.get_status(metadata.clone())? else {{\n"
    ));
    src.push_str(&format!(
        "            return Err(anyhow::anyhow!(\"{collection_name} status not found\"));\n"
    ));
    src.push_str("        };\n");
    src.push_str("        f(&mut status);\n");
    src.push_str("        self.set_status(metadata.clone(), status.clone()).await?;\n");
    src.push_str("        Ok(status)\n");
    src.push_str("    }\n");

    src.push_str("}\n\n");
}
