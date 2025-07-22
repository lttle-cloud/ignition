use std::{collections::HashMap, sync::Arc};

use anyhow::{Result, bail};
use kvm_ioctls::VmFd;
use tokio::sync::Barrier;
use vm_allocator::AddressAllocator;
use vm_memory::{GuestAddress, GuestMemory, GuestMemoryMmap};

use crate::{
    agent::{
        image::Image,
        machine::{
            MachineAgentConfig,
            vm::{
                kernel::{create_cmdline, load_kernel},
                kvm::create_and_verify_kvm,
                memory::{create_memory, create_mmio_allocator},
                vcpu::{Vcpu, VcpuRef},
            },
        },
    },
    takeoff::proto::TakeoffInitArgs,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineState {
    Idle,
    Booting,
    Ready,
    Suspending,
    Suspended,
    Stopping,
    Stopped,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineStateRetentionMode {
    InMemory,
    OnDisk { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineMode {
    Regular,
    Flash(SnapshotStrategy),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotStrategy {
    WaitForNthListen(u16),
    WaitForFirstListen,
    WaitForListenOnPort(u16),
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineResources {
    pub cpu: u8,
    pub memory: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineConfig {
    pub name: String,
    pub mode: MachineMode,
    pub state_retention_mode: MachineStateRetentionMode,
    pub resources: MachineResources,
    pub image: Image,
    pub envs: HashMap<String, String>,
}

#[derive(Debug)]
pub struct Machine {
    pub config: MachineConfig,
    pub state: MachineState,

    guest_memory: GuestMemoryMmap,
    mmio_allocator: AddressAllocator,
    kernel_start_address: GuestAddress,

    vm_fd: VmFd,
    vcpu_start_barrier: Arc<Barrier>,
    vcpus: Vec<VcpuRef>,
}

pub type MachineRef = Arc<Machine>;

impl Machine {
    pub async fn new(agent_config: &MachineAgentConfig, config: MachineConfig) -> Result<Self> {
        let kvm = create_and_verify_kvm()?;
        let vm_fd = kvm.create_vm()?;

        // create memory
        let guest_memory = create_memory(&config).await?;
        let mmio_allocator = create_mmio_allocator()?;

        // init kernel cmdline
        let mut kernel_cmd = create_cmdline(&config)?;
        kernel_cmd.insert_str(&agent_config.kernel_cmd_init)?;

        let takeoff_args = TakeoffInitArgs {
            envs: config.envs.clone(),
        };
        let takeoff_args_str = takeoff_args.encode()?;
        kernel_cmd.insert_str(format!("--takeoff-args={}", takeoff_args_str))?;

        // TODO: add and init devices

        // load the kernel
        let kernel_load_result = load_kernel(
            &config,
            &guest_memory,
            &agent_config.kernel_path,
            &agent_config.initrd_path,
            &kernel_cmd,
        )
        .await?;

        let Some(kernel_start_address) = guest_memory.check_address(kernel_load_result.kernel_load)
        else {
            bail!("Kernel load result is not in guest memory");
        };

        // add vcpus
        let barrier = Arc::new(Barrier::new(config.resources.cpu as usize));

        let mut vcpus = vec![];
        for i in 0..config.resources.cpu {
            let vcpu = Vcpu::new(
                &kvm,
                &vm_fd,
                &guest_memory,
                barrier.clone(),
                kernel_start_address.clone(),
                config.resources.cpu as u8,
                i,
            )
            .await?;
            vcpus.push(Arc::new(vcpu));
        }

        let machine = Self {
            config,
            state: MachineState::Idle,

            guest_memory,
            mmio_allocator,
            kernel_start_address,

            vm_fd,
            vcpu_start_barrier: barrier,
            vcpus,
        };

        Ok(machine)
    }

    pub async fn start(&self) -> Result<()> {
        Ok(())
    }
}
