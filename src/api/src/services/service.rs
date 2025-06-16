use std::sync::Arc;

use controller::Controller;
use tonic::{Request, Response, Status};
use util::{
    async_runtime::task::spawn_blocking,
    futures::executor::block_on,
    result::{bail, Result},
};

use crate::ignition_proto::{
    service::{
        self, DeleteServiceRequest, DeleteServiceResponse, DeployServiceRequest,
        DeployServiceResponse, GetServiceRequest, GetServiceResponse, ListServicesResponse,
    },
    service_server::Service,
    util::Empty,
};

fn ctoa_service_target(target: controller::ServiceTarget) -> service::ServiceTarget {
    service::ServiceTarget {
        name: target.name,
        port: target.port as u32,
    }
}

fn ctoa_service_protocol(protocol: controller::ServiceProtocol) -> service::ServiceProtocol {
    match protocol {
        controller::ServiceProtocol::Tcp { port } => service::ServiceProtocol {
            protocol: Some(service::service_protocol::Protocol::Tcp(service::Tcp {
                port: port as u32,
            })),
        },
        controller::ServiceProtocol::Tls { port } => service::ServiceProtocol {
            protocol: Some(service::service_protocol::Protocol::Tls(service::Tls {
                port: port as u32,
            })),
        },
        controller::ServiceProtocol::Http => service::ServiceProtocol {
            protocol: Some(service::service_protocol::Protocol::Http(service::Http {})),
        },
    }
}

fn ctoa_service_mode(mode: controller::ServiceMode) -> service::ServiceMode {
    match mode {
        controller::ServiceMode::Internal => service::ServiceMode {
            mode: Some(service::service_mode::Mode::Internal(service::Internal {})),
        },
        controller::ServiceMode::External { host } => service::ServiceMode {
            mode: Some(service::service_mode::Mode::External(service::External {
                host,
            })),
        },
    }
}

fn ctoa_service_info(service: controller::Service) -> service::ServiceInfo {
    service::ServiceInfo {
        name: service.name,
        target: Some(ctoa_service_target(service.target)),
        protocol: Some(ctoa_service_protocol(service.protocol)),
        mode: Some(ctoa_service_mode(service.mode)),
        internal_ip: service.internal_ip,
    }
}

fn atoc_service_target(target: service::ServiceTarget) -> controller::ServiceTarget {
    controller::ServiceTarget {
        name: target.name,
        port: target.port as u16,
    }
}

fn atoc_service_protocol(
    protocol: service::ServiceProtocol,
) -> Result<controller::ServiceProtocol> {
    match protocol.protocol {
        Some(service::service_protocol::Protocol::Http(_http)) => {
            Ok(controller::ServiceProtocol::Http)
        }
        Some(service::service_protocol::Protocol::Tcp(tcp)) => {
            Ok(controller::ServiceProtocol::Tcp {
                port: tcp.port as u16,
            })
        }
        Some(service::service_protocol::Protocol::Tls(tls)) => {
            Ok(controller::ServiceProtocol::Tls {
                port: tls.port as u16,
            })
        }
        None => bail!("service protocol is not set"),
    }
}

fn atoc_service_mode(mode: service::ServiceMode) -> Result<controller::ServiceMode> {
    match mode.mode {
        Some(service::service_mode::Mode::Internal(_internal)) => {
            Ok(controller::ServiceMode::Internal)
        }
        Some(service::service_mode::Mode::External(external)) => {
            Ok(controller::ServiceMode::External {
                host: external.host,
            })
        }
        None => bail!("service mode is not set"),
    }
}

fn atoc_deploy_service_input(service: service::Service) -> Result<controller::DeployServiceInput> {
    let Some(target) = service.target else {
        bail!("service target is not set");
    };
    let Some(protocol) = service.protocol else {
        bail!("service protocol is not set");
    };
    let Some(mode) = service.mode else {
        bail!("service mode is not set");
    };

    Ok(controller::DeployServiceInput {
        name: service.name,
        target: atoc_service_target(target),
        protocol: atoc_service_protocol(protocol)?,
        mode: atoc_service_mode(mode)?,
    })
}

pub struct ServiceApiConfig {}

pub struct ServiceApi {
    config: ServiceApiConfig,
    controller: Arc<Controller>,
}

impl ServiceApi {
    pub fn new(controller: Arc<Controller>, config: ServiceApiConfig) -> Result<Self> {
        Ok(Self { controller, config })
    }
}

#[tonic::async_trait]
impl Service for ServiceApi {
    async fn deploy(
        &self,
        request: Request<DeployServiceRequest>,
    ) -> Result<Response<DeployServiceResponse>, Status> {
        println!("deploying service");
        let request = request.into_inner();

        let service = request
            .service
            .ok_or_else(|| Status::invalid_argument("service is not set"))?;

        let input = atoc_deploy_service_input(service)
            .map_err(|_| Status::invalid_argument("invalid service"))?;

        let deploy_ctrl = self.controller.clone();
        let task = spawn_blocking(move || {
            block_on(async move {
                deploy_ctrl
                    .deploy_service(input)
                    .await
                    .map_err(|_| Status::internal("failed to deploy service"))
            })
        })
        .await;
        let service = task
            .map_err(|_| Status::internal("failed to deploy service"))?
            .map_err(|_| Status::internal("failed to deploy service"))?;

        let service_info = ctoa_service_info(service);

        Ok(Response::new(DeployServiceResponse {
            service: Some(service_info),
        }))
    }

    async fn get(
        &self,
        request: Request<GetServiceRequest>,
    ) -> Result<Response<GetServiceResponse>, Status> {
        let request = request.into_inner();

        let service_name = request.name;

        let service = self
            .controller
            .get_service(&service_name)
            .await
            .map_err(|_| Status::not_found("service not found"))?;

        let service_info = service.map(ctoa_service_info);

        Ok(Response::new(GetServiceResponse {
            service: service_info,
        }))
    }

    async fn list(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ListServicesResponse>, Status> {
        let services = self
            .controller
            .list_services()
            .await
            .map_err(|_| Status::internal("failed to list services"))?;

        let service_infos = services.into_iter().map(ctoa_service_info).collect();

        Ok(Response::new(ListServicesResponse {
            services: service_infos,
        }))
    }

    async fn delete(
        &self,
        request: Request<DeleteServiceRequest>,
    ) -> Result<Response<DeleteServiceResponse>, Status> {
        let request = request.into_inner();

        let service_name = request.name;

        self.controller
            .delete_service(&service_name)
            .await
            .map_err(|_| Status::not_found("service not found"))?;

        Ok(Response::new(DeleteServiceResponse {}))
    }
}
