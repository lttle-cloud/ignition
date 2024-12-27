use std::{
    sync::{Arc, Barrier},
    thread::{self, JoinHandle},
};

use kvm_bindings::{kvm_pit_config, kvm_userspace_memory_region, KVM_PIT_SPEAKER_DUMMY};
use kvm_ioctls::{Kvm, VmFd};
use libc::SIGRTMIN;
use util::result::{bail, Result};
use vm_memory::{Address, GuestAddress, GuestMemory, GuestMemoryRegion};
use vmm_sys_util::{eventfd::EventFd, signal::Killable};

use crate::{
    constants::MAX_IRQ,
    cpu_ref::mptable::MpTable,
    device::SharedDeviceManager,
    memory::Memory,
    vcpu::{ExitHandler, Vcpu, VcpuRunState, VcpusConfigList},
};

pub struct VmConfig {
    pub vcpus_count: u8,
    pub vcpus_config: VcpusConfigList,
}

impl VmConfig {
    pub fn new(kvm: &Kvm, vcpus_count: u8) -> Result<Self> {
        Ok(VmConfig {
            vcpus_count,
            vcpus_config: VcpusConfigList::new(kvm, vcpus_count)?,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VmRunState {
    Running,
    Suspending,
    Exiting,
}

impl Default for VmRunState {
    fn default() -> Self {
        VmRunState::Running
    }
}

pub struct Vm<EH: ExitHandler + Send> {
    fd: Arc<VmFd>,
    config: VmConfig,
    vcpus: Vec<Vcpu>,
    vcpu_handles: Vec<JoinHandle<()>>,
    exit_handler: EH,
    vcpu_barrier: Arc<Barrier>,
    vcpu_run_state: Arc<VcpuRunState>,
}

impl<EH: ExitHandler + Send + 'static> Vm<EH> {
    fn create_instance(
        kvm: &Kvm,
        memory: &Memory,
        config: VmConfig,
        exit_handler: EH,
    ) -> Result<Self> {
        let vm_fd = kvm.create_vm()?;
        let vcpu_run_state = Arc::new(VcpuRunState::default());

        let vcpus_count = config.vcpus_count as usize;

        let vm = Vm {
            fd: Arc::new(vm_fd),
            config,
            vcpus: Vec::new(),
            vcpu_handles: Vec::new(),
            exit_handler,
            vcpu_barrier: Arc::new(Barrier::new(vcpus_count)),
            vcpu_run_state,
        };

        vm.configure_memory_regions(kvm, memory)?;

        Ok(vm)
    }

    pub fn new(
        kvm: &Kvm,
        memory: &Memory,
        config: VmConfig,
        exit_handler: EH,
        device_manager: SharedDeviceManager,
    ) -> Result<Self> {
        let vcpus_config = config.vcpus_config.clone();
        let mut vm = Self::create_instance(kvm, memory, config, exit_handler)?;

        MpTable::new(vm.config.vcpus_count, MAX_IRQ as u8)?.write(memory.guest_memory())?;

        vm.setup_irq_controller()?;
        vm.setup_vcpus(device_manager, vcpus_config, memory)?;

        Ok(vm)
    }

    fn configure_memory_regions(&self, kvm: &Kvm, memory: &Memory) -> Result<()> {
        let guest_memory = memory.guest_memory();

        if guest_memory.num_regions() > kvm.get_nr_memslots() {
            bail!("Not enough KVM memory slots for guest memory regions");
        }

        for (index, region) in guest_memory.iter().enumerate() {
            let memory_region = kvm_userspace_memory_region {
                slot: index as u32,
                guest_phys_addr: region.start_addr().raw_value(),
                memory_size: region.len() as u64,
                userspace_addr: guest_memory.get_host_address(region.start_addr()).unwrap() as u64,
                flags: 0,
            };

            unsafe {
                self.fd.set_user_memory_region(memory_region)?;
            };
        }

        Ok(())
    }

    fn setup_irq_controller(&mut self) -> Result<()> {
        self.fd.create_irq_chip()?;

        let pit_config = kvm_pit_config {
            flags: KVM_PIT_SPEAKER_DUMMY,
            ..Default::default()
        };

        self.fd.create_pit2(pit_config)?;

        Ok(())
    }

    fn setup_vcpus(
        &mut self,
        device_manager: SharedDeviceManager,
        vcpus_config: VcpusConfigList,
        memory: &Memory,
    ) -> Result<()> {
        for vcpu_config in vcpus_config.configs.iter() {
            let vcpu = Vcpu::new(
                &self.fd,
                device_manager.clone(),
                vcpu_config.clone(),
                self.vcpu_barrier.clone(),
                self.vcpu_run_state.clone(),
                memory,
            )?;
            self.vcpus.push(vcpu);
        }

        Ok(())
    }

    pub fn register_irqfd(&self, fd: &EventFd, irq: u32) -> Result<()> {
        Ok(self.fd.register_irqfd(fd, irq)?)
    }

    pub fn run(&mut self, start_addr: GuestAddress) -> Result<()> {
        Vcpu::setup_signal_handler()?;

        for (id, mut vcpu) in self.vcpus.drain(..).enumerate() {
            let exit_handler = self.exit_handler.clone();
            let handle = thread::Builder::new()
                .name(format!("vcpu_{}", id))
                .spawn(move || {
                    vcpu.run(start_addr).unwrap();
                    let _ = exit_handler.kick();

                    vcpu.run_state.set_and_notify(VmRunState::Exiting);
                })?;
            self.vcpu_handles.push(handle);
        }

        Ok(())
    }

    pub fn shutdown(&mut self) {
        self.vcpu_run_state.set_and_notify(VmRunState::Exiting);
        self.vcpu_handles.drain(..).for_each(|handle| {
            #[allow(clippy::identity_op)]
            let _ = handle.kill(SIGRTMIN() + 0);
            let _ = handle.join();
        })
    }
}
