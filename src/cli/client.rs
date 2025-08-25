use anyhow::Result;
use async_trait::async_trait;
use ignition::{
    api_client::{ApiClient, ApiClientConfig, MachineApiClient},
    resources::{machine::Machine, metadata::Namespace},
};

pub fn get_api_client(config: ApiClientConfig) -> ApiClient {
    ApiClient::new(config)
}

#[async_trait]
pub trait MachineClientExt {
    async fn add_tag(&self, namespace: Namespace, name: String, tag: String) -> Result<()>;
}

#[async_trait]
impl MachineClientExt for MachineApiClient {
    async fn add_tag(&self, namespace: Namespace, name: String, tag: String) -> Result<()> {
        let (mut machine, _) = self.get(namespace, name).await?;

        let mut tags = machine
            .tags
            .unwrap_or_default()
            .into_iter()
            .filter(|tag| tag.to_string() != tag.to_string())
            .collect::<Vec<String>>();
        tags.push(tag.to_string());
        machine.tags = Some(tags);

        let machine: Machine = machine.into();

        self.apply(machine).await?;
        Ok(())
    }
}
