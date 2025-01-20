use kvm_bindings::{
    kvm_clock_data, kvm_debugregs, kvm_irqchip, kvm_lapic_state, kvm_mp_state, kvm_pit_state2,
    kvm_regs, kvm_sregs, kvm_vcpu_events, kvm_xcrs, kvm_xsave, CpuId, Msrs, __IncompleteArrayField,
};
use virtio_queue::QueueState;
use vm_memory::GuestAddress;

use crate::{
    config::{Config, NetConfig},
    vcpu::VcpuConfig,
    vm::VmConfig,
};

#[derive(Debug, Clone)]
pub struct VcpuState {
    pub cpuid: CpuId,
    pub msrs: Msrs,
    pub debug_regs: kvm_debugregs,
    pub lapic: kvm_lapic_state,
    pub mp_state: kvm_mp_state,
    pub regs: kvm_regs,
    pub sregs: kvm_sregs,
    pub vcpu_events: kvm_vcpu_events,
    pub xcrs: kvm_xcrs,
    pub xsave: KvmXsave,
    pub config: VcpuConfig,
}

#[derive(Debug)]
pub struct KvmXsave(pub kvm_xsave);

impl Clone for KvmXsave {
    fn clone(&self) -> Self {
        Self(kvm_xsave {
            region: self.0.region.clone(),
            extra: __IncompleteArrayField::default(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct NetState {
    pub config: NetConfig,
    pub virtio_state: VirtioState,
}

#[derive(Debug, Clone)]
pub struct VirtioState {
    pub config_generation: u8,
    pub conifg_space: Vec<u8>,
    pub device_activated: bool,
    pub device_features: u64,
    pub device_features_sel: u32,
    pub device_status: u8,
    pub driver_features: u64,
    pub driver_features_sel: u32,
    pub interrupt_status: u8,
    pub queue_sel: u16,
    pub queues: Vec<QueueState>,
}

#[derive(Debug, Clone)]
pub struct VmState {
    pub pitstate: kvm_pit_state2,
    pub clock: kvm_clock_data,
    pub pic_master: kvm_irqchip,
    pub pic_slave: kvm_irqchip,
    pub ioapic: kvm_irqchip,
    pub config: VmConfig,
    pub vcpus_state: Vec<VcpuState>,
}

#[derive(Debug, Clone)]
pub struct VmmState {
    pub config: Config,
    pub vm_state: VmState,
    pub kernel_load_addr: GuestAddress,
    pub net_state: Option<NetState>,
}
