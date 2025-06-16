use ignition_client::ignition_proto::{
    image::PullImageRequest,
    machine::{DeployMachineRequest, Machine, MachineEnvironmentVariable},
    service::{
        service_mode::Mode, service_protocol::Protocol, DeployServiceRequest, External, Http,
        Internal, Service, ServiceMode, ServiceProtocol, ServiceTarget, Tcp, Tls,
    },
};
use std::path::PathBuf;
use util::{
    async_runtime::fs::read_to_string,
    result::{bail, Result},
    tracing::info,
};

use crate::{
    client::get_client,
    config::Config,
    resource::{self, parse_all_resources, Resource},
};

pub async fn run_deploy(config: Config, file: PathBuf) -> Result<()> {
    let file_contents = read_to_string(file).await?;
    let resources = parse_all_resources(&file_contents)?;

    let machines = resources
        .iter()
        .filter_map(|r| match r {
            Resource::Machine(m) => Some(m),
            _ => None,
        })
        .collect::<Vec<_>>();

    let images: Vec<String> = machines.iter().map(|r| r.image.clone()).collect();

    let services = resources
        .iter()
        .filter_map(|r| match r {
            Resource::Service(s) => Some(s),
            _ => None,
        })
        .collect::<Vec<_>>();

    let client = get_client(config).await?;

    for image in images {
        info!("Pulling image: {}", image);
        let result = client
            .image()
            .pull(PullImageRequest {
                image: image.clone(),
            })
            .await?
            .into_inner();

        if !result.was_downloaded {
            info!(
                "Image already exists: {} @ {}",
                result.image_name, result.digest
            );
            continue;
        }

        info!("Image pulled: {} @ {}", result.image_name, result.digest);
    }

    for machine in machines {
        info!("Deploying machine: {}", machine.name);

        let env = machine
            .environment
            .as_ref()
            .map(|envs| {
                envs.iter()
                    .map(|e| MachineEnvironmentVariable {
                        name: e.name.clone(),
                        value: e.value.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let machine_info = client
            .machine()
            .deploy(DeployMachineRequest {
                machine: Some(Machine {
                    name: machine.name.clone(),
                    memory: machine.memory,
                    vcpus: machine.vcpus as u32,
                    image: machine.image.clone(),
                    environment: env,
                    snapshot_policy: None,
                }),
            })
            .await?
            .into_inner();

        let Some(machine_info) = machine_info.machine else {
            bail!("Failed to deploy machine: {}", machine.name);
        };

        info!(
            "Machine deployed: {} ({})",
            machine_info.name, machine_info.id
        );
    }

    for service in services {
        info!("Deploying service: {}", service.name);

        let service_protocol = match service.protocol {
            resource::ServiceProtocol::Http => ServiceProtocol {
                protocol: Some(Protocol::Http(Http {})),
            },
            resource::ServiceProtocol::Tcp { port } => ServiceProtocol {
                protocol: Some(Protocol::Tcp(Tcp { port: port as u32 })),
            },
            resource::ServiceProtocol::Tls { port } => ServiceProtocol {
                protocol: Some(Protocol::Tls(Tls { port: port as u32 })),
            },
        };

        let service_mode = match &service.mode {
            resource::ServiceMode::Internal => ServiceMode {
                mode: Some(Mode::Internal(Internal {})),
            },
            resource::ServiceMode::External { host } => ServiceMode {
                mode: Some(Mode::External(External { host: host.clone() })),
            },
        };

        let service_info = client
            .service()
            .deploy(DeployServiceRequest {
                service: Some(Service {
                    name: service.name.clone(),
                    target: Some(ServiceTarget {
                        name: service.target.name.clone(),
                        port: service.target.port as u32,
                    }),
                    protocol: Some(service_protocol),
                    mode: Some(service_mode),
                }),
            })
            .await?
            .into_inner();

        let Some(service_info) = service_info.service else {
            bail!("Failed to deploy service: {}", service.name);
        };

        info!("Service deployed: {}", service_info.name);
    }

    Ok(())
}
