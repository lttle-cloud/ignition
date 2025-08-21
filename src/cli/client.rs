use anyhow::Result;
use async_trait::async_trait;
use ignition::{
    api_client::{ApiClient, ApiClientConfig, MachineApiClient},
    resources::{machine::Machine, metadata::Namespace},
};

use crate::config::Config;

pub async fn get_api_client(config: &Config) -> Result<ApiClient> {
    let api_client_config: ApiClientConfig = config.try_into()?;
    let client = ApiClient::new(api_client_config);

    Ok(client)
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
