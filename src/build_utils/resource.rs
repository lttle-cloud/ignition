use anyhow::Result;

use crate::{
    build_utils::cargo,
    resources::{BuildableResource, ResourceBuildInfo},
};

pub struct ResourcesRepositoryBuilder {
    resources: Vec<ResourceBuildInfo>,
}

impl ResourcesRepositoryBuilder {
    pub fn new() -> Self {
        Self {
            resources: Vec::new(),
        }
    }

    pub fn resource<R: BuildableResource>(mut self) -> Self {
        let build_info = R::build_info();
        self.resources.push(build_info);
        self
    }

    pub async fn build(self) -> Result<()> {
        cargo::warn(format!("build: {:?}", self.resources));
        Ok(())
    }
}
