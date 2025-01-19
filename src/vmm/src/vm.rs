use std::{
    sync::{Arc, Barrier, Mutex},
    thread::{self, JoinHandle},
};

use kvm_bindings::{
    kvm_irqchip, kvm_pit_config, kvm_userspace_memory_region, KVM_CLOCK_TSC_STABLE,
    KVM_IRQCHIP_IOAPIC, KVM_IRQCHIP_PIC_MASTER, KVM_IRQCHIP_PIC_SLAVE, KVM_PIT_SPEAKER_DUMMY,
};
use kvm_ioctls::{Kvm, VmFd};
use libc::SIGRTMIN;
use util::result::{bail, Result};
use vm_memory::{Address, GuestAddress, GuestMemory, GuestMemoryRegion};
use vmm_sys_util::{eventfd::EventFd, signal::Killable};

use crate::{
    constants::MAX_IRQ,
    cpu_ref::mptable::MpTable,
    device::{meta::guest_manager::GuestManagerDevice, SharedDeviceManager},
    memory::Memory,
    state::{VcpuState, VmState},
    vcpu::{ExitHandler, Vcpu, VcpuRunState, VcpusConfigList},
};

#[derive(Clone, Debug)]
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
    #[allow(unused)]
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
    vcpu_handles: Vec<JoinHandle<VcpuState>>,
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
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
    ) -> Result<Self> {
        let vcpus_config = config.vcpus_config.clone();
        let mut vm = Self::create_instance(kvm, memory, config, exit_handler)?;

        MpTable::new(vm.config.vcpus_count, MAX_IRQ as u8)?.write(memory.guest_memory())?;

        vm.setup_irq_controller()?;
        vm.setup_vcpus(device_manager, guest_manager, vcpus_config, memory)?;

        Ok(vm)
    }

    pub fn from_state(
        kvm: &Kvm,
        memory: &Memory,
        state: &VmState,
        exit_handler: EH,
        device_manager: SharedDeviceManager,
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
    ) -> Result<Self> {
        let mut vm = Self::create_instance(kvm, memory, state.config.clone(), exit_handler)?;

        vm.setup_irq_controller()?;
        vm.set_state(&state)?;
        vm.setup_vcpus_from_state(device_manager, guest_manager, &state)?;

        Ok(vm)
    }

    pub fn fd(&self) -> Arc<VmFd> {
        self.fd.clone()
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
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
        vcpus_config: VcpusConfigList,
        memory: &Memory,
    ) -> Result<()> {
        for vcpu_config in vcpus_config.configs.iter() {
            let vcpu = Vcpu::new(
                &self.fd,
                device_manager.clone(),
                guest_manager.clone(),
                vcpu_config.clone(),
                self.vcpu_barrier.clone(),
                self.vcpu_run_state.clone(),
                memory,
            )?;
            self.vcpus.push(vcpu);
        }

        Ok(())
    }

    fn setup_vcpus_from_state(
        &mut self,
        device_manager: SharedDeviceManager,
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
        state: &VmState,
    ) -> Result<()> {
        self.vcpus = state
            .vcpus_state
            .iter()
            .map(|vcpu_state| {
                Vcpu::from_state(
                    &self.fd,
                    device_manager.clone(),
                    guest_manager.clone(),
                    vcpu_state,
                    self.vcpu_barrier.clone(),
                    self.vcpu_run_state.clone(),
                )
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(())
    }

    pub fn register_irqfd(&self, fd: &EventFd, irq: u32) -> Result<()> {
        Ok(self.fd.register_irqfd(fd, irq)?)
    }

    pub fn run(&mut self, start_addr: Option<GuestAddress>) -> Result<()> {
        Vcpu::setup_signal_handler()?;

        for (id, mut vcpu) in self.vcpus.drain(..).enumerate() {
            let exit_handler = self.exit_handler.clone();
            let handle = thread::Builder::new()
                .name(format!("vcpu_{}", id))
                .spawn(move || {
                    vcpu.run(start_addr).unwrap();
                    let _ = exit_handler.kick();

                    vcpu.run_state.set_and_notify(VmRunState::Exiting);

                    return vcpu.save_state().unwrap();
                })?;
            self.vcpu_handles.push(handle);
        }

        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<VmState> {
        self.vcpu_run_state.set_and_notify(VmRunState::Exiting);
        let vcpus_state = self
            .vcpu_handles
            .drain(..)
            .map(|handle| {
                #[allow(clippy::identity_op)]
                let _ = handle.kill(SIGRTMIN() + 0);
                return handle.join().unwrap();
            })
            .collect::<Vec<_>>();

        return self.save_state(vcpus_state);
    }

    fn save_state(&mut self, vcpus_state: Vec<VcpuState>) -> Result<VmState> {
        let pitstate = self.fd.get_pit2()?;

        let mut clock = self.fd.get_clock()?;
        // This bit is not accepted in SET_CLOCK, clear it.
        clock.flags &= !KVM_CLOCK_TSC_STABLE;

        let mut pic_master = kvm_irqchip {
            chip_id: KVM_IRQCHIP_PIC_MASTER,
            ..Default::default()
        };
        self.fd.get_irqchip(&mut pic_master)?;

        let mut pic_slave = kvm_irqchip {
            chip_id: KVM_IRQCHIP_PIC_SLAVE,
            ..Default::default()
        };
        self.fd.get_irqchip(&mut pic_slave)?;

        let mut ioapic = kvm_irqchip {
            chip_id: KVM_IRQCHIP_IOAPIC,
            ..Default::default()
        };
        self.fd.get_irqchip(&mut ioapic)?;

        Ok(VmState {
            pitstate,
            clock,
            pic_master,
            pic_slave,
            ioapic,
            config: self.config.clone(),
            vcpus_state,
        })
    }

    fn set_state(&mut self, state: &VmState) -> Result<()> {
        self.fd.set_pit2(&state.pitstate)?;
        self.fd.set_clock(&state.clock)?;
        self.fd.set_irqchip(&state.pic_master)?;
        self.fd.set_irqchip(&state.pic_slave)?;
        self.fd.set_irqchip(&state.ioapic)?;

        Ok(())
    }
}
