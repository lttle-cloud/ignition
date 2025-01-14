use kvm_bindings::{
    kvm_clock_data, kvm_debugregs, kvm_irqchip, kvm_lapic_state, kvm_mp_state, kvm_pit_state2,
    kvm_regs, kvm_sregs, kvm_vcpu_events, kvm_xcrs, kvm_xsave, CpuId, Msrs,
};
use vm_memory::GuestAddress;

use crate::{config::Config, vcpu::VcpuConfig, vm::VmConfig};

#[derive(Debug)]
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
    pub xsave: kvm_xsave,
    pub config: VcpuConfig,
}

#[derive(Debug)]
pub struct VmState {
    pub pitstate: kvm_pit_state2,
    pub clock: kvm_clock_data,
    pub pic_master: kvm_irqchip,
    pub pic_slave: kvm_irqchip,
    pub ioapic: kvm_irqchip,
    pub config: VmConfig,
    pub vcpus_state: Vec<VcpuState>,
}

#[derive(Debug)]
pub struct VmmState {
    pub config: Config,
    pub vm_state: VmState,
    pub kernel_load_addr: GuestAddress,
}
