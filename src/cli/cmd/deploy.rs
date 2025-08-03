use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use ignition::{
    api_client::ApiClient,
    resource_index::Resources,
    resources::{ProvideMetadata, machine::Machine, metadata::Namespace, service::Service},
};
use tokio::fs::read_to_string;

use crate::{
    client::get_api_client,
    cmd::{machine::MachineSummary, service::ServiceSummary},
    config::Config,
    ui::message::message_info,
};

#[derive(Args)]
pub struct DeployArgs {
    /// Path to the deployment file
    file: PathBuf,
}

pub async fn run_deploy(config: &Config, args: DeployArgs) -> Result<()> {
    let api_client = get_api_client(config).await?;

    let contents = read_to_string(args.file).await?;

    let resources = parse_all_resources(&contents).await?;

    for resource in resources {
        if let Ok(machine) = resource.clone().try_into() {
            deploy_machine(config, &api_client, machine).await?;
            continue;
        }

        if let Ok(service) = resource.clone().try_into() {
            deploy_service(config, &api_client, service).await?;
            continue;
        }

        unreachable!("Unknown resource type: {:?}", resource);
    }

    Ok(())
}

async fn parse_all_resources(contents: &str) -> Result<Vec<Resources>> {
    let mut resources = Vec::new();

    let de = serde_yaml::Deserializer::from_str(contents);
    for doc in de {
        let resource: Resources = serde_yaml::with::singleton_map_recursive::deserialize(doc)?;
        resources.push(resource);
    }

    Ok(resources)
}

async fn deploy_machine(_config: &Config, api_client: &ApiClient, machine: Machine) -> Result<()> {
    let metadata = machine.metadata();
    api_client.machine().apply(machine).await?;

    let (machine, status) = api_client
        .machine()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed machine: {}",
        machine.metadata().to_string()
    ));

    let summary = MachineSummary::from((machine, status));
    summary.print();

    Ok(())
}

async fn deploy_service(_config: &Config, api_client: &ApiClient, service: Service) -> Result<()> {
    let metadata = service.metadata();
    api_client.service().apply(service).await?;

    let (service, status) = api_client
        .service()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed service: {}",
        service.metadata().to_string()
    ));

    let summary = ServiceSummary::from((service, status));
    summary.print();

    Ok(())
}
