use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use ignition::{
    api_client::ApiClient,
    resource_index::Resources,
    resources::{
        ProvideMetadata, certificate::Certificate, machine::Machine, metadata::Namespace, 
        service::Service, volume::Volume,
    },
};
use tokio::fs::read_to_string;

use crate::{
    client::get_api_client,
    cmd::{machine::MachineSummary, service::ServiceSummary, volume::VolumeSummary},
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
        if let Ok(certificate) = resource.clone().try_into() {
            deploy_certificate(config, &api_client, certificate).await?;
            continue;
        }

        if let Ok(machine) = resource.clone().try_into() {
            deploy_machine(config, &api_client, machine).await?;
            continue;
        }

        if let Ok(service) = resource.clone().try_into() {
            deploy_service(config, &api_client, service).await?;
            continue;
        }

        if let Ok(volume) = resource.clone().try_into() {
            deploy_volume(config, &api_client, volume).await?;
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

async fn deploy_certificate(_config: &Config, api_client: &ApiClient, certificate: Certificate) -> Result<()> {
    let metadata = certificate.metadata();
    api_client.certificate().apply(certificate).await?;

    let (certificate, status) = api_client
        .certificate()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed certificate: {}",
        certificate.metadata().to_string()
    ));

    // For now, just print basic status info since we don't have a CertificateSummary yet
    println!("Status: {:?}", status);

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

async fn deploy_volume(_config: &Config, api_client: &ApiClient, volume: Volume) -> Result<()> {
    let metadata = volume.metadata();
    api_client.volume().apply(volume).await?;

    let (volume, status) = api_client
        .volume()
        .get(
            Namespace::from_value_or_default(metadata.namespace),
            metadata.name,
        )
        .await?;

    message_info(format!(
        "Successfully deployed volume: {}",
        volume.metadata().to_string()
    ));

    let summary = VolumeSummary::from((volume, status));
    summary.print();

    Ok(())
}
