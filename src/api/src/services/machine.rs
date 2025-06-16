use std::sync::Arc;

use controller::machine::{MachineInfo, MachineStatus, SnapshotPolicy};
use controller::{Controller, DeployMachineInput};
use tonic::{Request, Response, Status};
use util::async_runtime::task::spawn_blocking;
use util::futures::executor::block_on;
use util::result::Result;

use crate::ignition_proto::machine::{
    self, DeleteMachineRequest, DeleteMachineResponse, DeployMachineRequest, DeployMachineResponse,
    GetLogsRequest, GetLogsResponse, GetMachineRequest, GetMachineResponse, ListMachinesResponse,
    StartMachineRequest, StartMachineResponse, StopMachineRequest, StopMachineResponse,
};
use crate::ignition_proto::machine_server::Machine;
use crate::ignition_proto::util::Empty;

fn ctoa_machine_status(status: MachineStatus) -> machine::MachineStatus {
    match status {
        MachineStatus::New => machine::MachineStatus::New,
        MachineStatus::Running => machine::MachineStatus::Running,
        MachineStatus::Ready => machine::MachineStatus::Ready,
        MachineStatus::Stopping => machine::MachineStatus::Stopping,
        MachineStatus::Suspended => machine::MachineStatus::Suspended,
        MachineStatus::Stopped => machine::MachineStatus::Stopped,
        MachineStatus::Error(_e) => machine::MachineStatus::Error,
    }
}

fn ctoa_snapshot_policy(snapshot_policy: SnapshotPolicy) -> machine::MachineSnapshotPolicy {
    match snapshot_policy {
        SnapshotPolicy::OnNthListenSyscall(n) => machine::MachineSnapshotPolicy {
            policy: Some(
                machine::machine_snapshot_policy::Policy::OnNthListenSyscall(
                    machine::OnNthListenSyscall { n },
                ),
            ),
        },
        SnapshotPolicy::OnListenOnPort(port) => machine::MachineSnapshotPolicy {
            policy: Some(machine::machine_snapshot_policy::Policy::OnListenOnPort(
                machine::OnListenOnPort { port: port as u32 },
            )),
        },
        SnapshotPolicy::OnUserspaceReady => machine::MachineSnapshotPolicy {
            policy: Some(machine::machine_snapshot_policy::Policy::OnUserspaceReady(
                machine::OnUserspaceReady {},
            )),
        },
        SnapshotPolicy::Manual => machine::MachineSnapshotPolicy {
            policy: Some(machine::machine_snapshot_policy::Policy::Manual(
                machine::Manual {},
            )),
        },
    }
}

fn atoc_snapshot_policy(snapshot_policy: machine::MachineSnapshotPolicy) -> Option<SnapshotPolicy> {
    match snapshot_policy.policy {
        Some(machine::machine_snapshot_policy::Policy::OnNthListenSyscall(n)) => {
            Some(SnapshotPolicy::OnNthListenSyscall(n.n))
        }
        Some(machine::machine_snapshot_policy::Policy::OnListenOnPort(port)) => {
            Some(SnapshotPolicy::OnListenOnPort(port.port as u16))
        }
        Some(machine::machine_snapshot_policy::Policy::OnUserspaceReady(_)) => {
            Some(SnapshotPolicy::OnUserspaceReady)
        }
        Some(machine::machine_snapshot_policy::Policy::Manual(_)) => Some(SnapshotPolicy::Manual),
        None => None,
    }
}

fn ctoa_machine_info(machine_info: &MachineInfo) -> machine::MachineInfo {
    let machine_info = machine_info.clone();
    machine::MachineInfo {
        id: machine_info.id,
        name: machine_info.name,
        status: ctoa_machine_status(machine_info.status).into(),
        image_reference: machine_info.image_reference,
        ip_addr: machine_info.ip_addr,
        snapshot_policy: machine_info
            .snapshot_policy
            .map(|sp| ctoa_snapshot_policy(sp)),
    }
}

fn atoc_machine_to_deploy_input(machine: machine::Machine) -> DeployMachineInput {
    let snapshot_policy = machine
        .snapshot_policy
        .and_then(|sp| atoc_snapshot_policy(sp));

    DeployMachineInput {
        name: machine.name,
        image_name: machine.image,
        vcpu_count: machine.vcpus as u8,
        memory_size_mib: machine.memory as usize,
        envs: machine
            .environment
            .iter()
            .map(|e| (e.name.clone(), e.value.clone()))
            .collect(),
        snapshot_policy,
    }
}

pub struct MachineApiConfig {}

pub struct MachineApi {
    config: MachineApiConfig,
    controller: Arc<Controller>,
}

impl MachineApi {
    pub fn new(controller: Arc<Controller>, config: MachineApiConfig) -> Result<Self> {
        Ok(Self { controller, config })
    }
}

#[tonic::async_trait]
impl Machine for MachineApi {
    async fn deploy(
        &self,
        request: Request<DeployMachineRequest>,
    ) -> Result<Response<DeployMachineResponse>, Status> {
        let request = request.into_inner();

        let machine = request
            .machine
            .ok_or_else(|| Status::invalid_argument("machine is not set"))?;

        let deploy_input = atoc_machine_to_deploy_input(machine);

        // TODO: horrible stuff.
        let controller_clone = self.controller.clone();
        let machine_info_task = spawn_blocking(move || {
            block_on(async move { controller_clone.deploy_machine(deploy_input).await })
        })
        .await;

        let machine_info = machine_info_task
            .map_err(|e| Status::internal(format!("failed to deploy machine: {}", e)))?
            .map_err(|e| Status::internal(format!("failed to deploy machine: {}", e)))?;

        let machine_info = ctoa_machine_info(&machine_info);

        Ok(Response::new(DeployMachineResponse {
            machine: Some(machine_info),
        }))
    }

    async fn get(
        &self,
        request: Request<GetMachineRequest>,
    ) -> Result<Response<GetMachineResponse>, Status> {
        let request = request.into_inner();

        let machine_info = self
            .controller
            .get_machine(&request.id)
            .await
            .map_err(|e| Status::internal(format!("failed to get machine: {}", e)))?;

        let machine_info = machine_info.ok_or_else(|| Status::not_found("machine not found"))?;

        Ok(Response::new(GetMachineResponse {
            machine: Some(ctoa_machine_info(&machine_info)),
        }))
    }

    async fn list(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ListMachinesResponse>, Status> {
        let machines = self
            .controller
            .list_machines()
            .await
            .map_err(|e| Status::internal(format!("failed to list machines: {}", e)))?;

        let machines: Vec<machine::MachineInfo> =
            machines.iter().map(|m| ctoa_machine_info(m)).collect();
        Ok(Response::new(ListMachinesResponse { machines }))
    }

    async fn delete(
        &self,
        request: Request<DeleteMachineRequest>,
    ) -> Result<Response<DeleteMachineResponse>, Status> {
        let request = request.into_inner();

        self.controller
            .delete_machine(&request.id)
            .await
            .map_err(|e| Status::internal(format!("failed to delete machine: {}", e)))?;

        Ok(Response::new(DeleteMachineResponse {}))
    }

    async fn start(
        &self,
        request: Request<StartMachineRequest>,
    ) -> Result<Response<StartMachineResponse>, Status> {
        let request = request.into_inner();

        self.controller
            .start_machine(&request.id)
            .await
            .map_err(|e| Status::internal(format!("failed to start machine: {}", e)))?;

        Ok(Response::new(StartMachineResponse {}))
    }

    async fn stop(
        &self,
        request: Request<StopMachineRequest>,
    ) -> Result<Response<StopMachineResponse>, Status> {
        let request = request.into_inner();

        self.controller
            .stop_machine(&request.id)
            .await
            .map_err(|e| Status::internal(format!("failed to stop machine: {}", e)))?;

        Ok(Response::new(StopMachineResponse {}))
    }

    async fn get_logs(
        &self,
        _request: Request<GetLogsRequest>,
    ) -> Result<Response<GetLogsResponse>, Status> {
        Ok(Response::new(GetLogsResponse {
            logs: "no logs".to_string(),
        }))
    }
}
