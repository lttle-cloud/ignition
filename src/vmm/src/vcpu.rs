use std::sync::{Arc, Barrier, Condvar, Mutex};

use crate::{
    constants::{PDE_START, PDPTE_START, PML4_START, X86_CR0_PE, X86_CR0_PG, X86_CR4_PAE},
    cpu_ref::{
        self,
        gdt::{Gdt, BOOT_GDT_OFFSET},
        interrupts::{
            set_klapic_delivery_mode, DeliveryMode, APIC_LVT0_REG_OFFSET, APIC_LVT1_REG_OFFSET,
        },
        msr_index,
    },
    device::SharedDeviceManager,
    memory::{self, Memory},
    vm::VmRunState,
};
use kvm_bindings::{kvm_fpu, CpuId, Msrs};
use kvm_ioctls::{Kvm, VcpuFd, VmFd};
use util::result::{bail, Result};
use vm_memory::{Address, Bytes, GuestAddress};

#[derive(Clone)]
pub struct VcpuConfig {
    pub id: u8,
    pub cpuid: CpuId,
    pub msrs: Msrs,
}

#[derive(Clone)]
pub struct VcpusConfigList {
    pub configs: Vec<VcpuConfig>,
}

impl VcpusConfigList {
    pub fn new(kvm: &Kvm, vcpus_count: u8) -> Result<Self> {
        if vcpus_count == 0 {
            bail!("At least one vCPU must be defined");
        }

        let base_cpuid = kvm.get_supported_cpuid(kvm_bindings::KVM_MAX_CPUID_ENTRIES)?;
        let supported_msrs = cpu_ref::msrs::supported_guest_msrs(kvm)?;

        let mut configs = vec![];
        for index in 0..vcpus_count {
            let mut cpuid = base_cpuid.clone();
            cpu_ref::cpuid::filter_cpuid(kvm, index, vcpus_count, &mut cpuid);

            let vcpu_config = VcpuConfig {
                id: index,
                cpuid,
                msrs: supported_msrs.clone(),
            };

            configs.push(vcpu_config);
        }

        return Ok(VcpusConfigList { configs });
    }
}

pub trait ExitHandler: Clone {
    fn kick(&self) -> Result<()>;
}

#[derive(Default)]
pub struct VcpuRunState {
    pub vm_state: Mutex<VmRunState>,
    condvar: Condvar,
}

impl VcpuRunState {
    pub fn set_and_notify(&self, state: VmRunState) {
        *self.vm_state.lock().unwrap() = state;
        self.condvar.notify_all();
    }
}

pub struct Vcpu {
    pub vcpu_fd: VcpuFd,
    device_manager: SharedDeviceManager,
    config: VcpuConfig,
    run_barrier: Arc<Barrier>,
    pub run_state: Arc<VcpuRunState>,
}

impl Vcpu {
    pub fn new(
        vm_fd: &VmFd,
        device_manager: SharedDeviceManager,
        config: VcpuConfig,
        run_barrier: Arc<Barrier>,
        run_state: Arc<VcpuRunState>,
        memory: &Memory,
    ) -> Result<Self> {
        let vcpu = Vcpu {
            vcpu_fd: vm_fd.create_vcpu(config.id.into())?,
            device_manager,
            config,
            run_barrier,
            run_state,
        };

        vcpu.configure_cpuid()?;
        vcpu.configure_msrs()?;
        vcpu.configure_sregs(memory)?;
        vcpu.configure_lapic()?;
        vcpu.configure_fpu()?;

        Ok(vcpu)
    }

    fn configure_cpuid(&self) -> Result<()> {
        self.vcpu_fd.set_cpuid2(&self.config.cpuid)?;
        Ok(())
    }

    fn configure_msrs(&self) -> Result<()> {
        let msrs = cpu_ref::msrs::create_boot_msr_entries()?;
        let msrs_written = self.vcpu_fd.set_msrs(&msrs)?;

        if msrs_written as u32 != msrs.as_fam_struct_ref().nmsrs {
            bail!("Failed to configure all required MSRs");
        }

        Ok(())
    }

    fn configure_sregs(&self, memory: &Memory) -> Result<()> {
        let mut sregs = self.vcpu_fd.get_sregs()?;

        let gdt_table = Gdt::default();

        let code_seg = gdt_table.create_kvm_segment_for(1).unwrap();
        let data_seg = gdt_table.create_kvm_segment_for(2).unwrap();
        let tss_seg = gdt_table.create_kvm_segment_for(3).unwrap();

        gdt_table.write_to_mem(memory.guest_memory())?;

        sregs.gdt.base = BOOT_GDT_OFFSET as u64;
        sregs.gdt.limit = 0x7u16; // sizeof(u64) - 1

        sregs.cs = code_seg;
        sregs.ds = data_seg;
        sregs.es = data_seg;
        sregs.fs = data_seg;
        sregs.gs = data_seg;
        sregs.ss = data_seg;
        sregs.tr = tss_seg;

        sregs.cr0 |= X86_CR0_PE;
        sregs.efer = (msr_index::EFER_LME | msr_index::EFER_LMA) as u64;

        let boot_pml4_addr = GuestAddress(PML4_START);
        let boot_pdpte_addr = GuestAddress(PDPTE_START);
        let boot_pde_addr = GuestAddress(PDE_START);

        memory
            .guest_memory()
            .write_obj(boot_pdpte_addr.raw_value() | 0x03, boot_pml4_addr)?;

        memory
            .guest_memory()
            .write_obj(boot_pde_addr.raw_value() | 0x03, boot_pdpte_addr)?;

        for i in 0..512 {
            memory
                .guest_memory()
                .write_obj((i << 21) + 0x83u64, boot_pde_addr.unchecked_add(i * 8))?;
        }

        sregs.cr3 = boot_pml4_addr.raw_value();
        sregs.cr4 |= X86_CR4_PAE;
        sregs.cr0 |= X86_CR0_PG;

        self.vcpu_fd.set_sregs(&sregs)?;

        Ok(())
    }

    fn configure_lapic(&self) -> Result<()> {
        let mut klapic = self.vcpu_fd.get_lapic()?;

        set_klapic_delivery_mode(&mut klapic, APIC_LVT0_REG_OFFSET, DeliveryMode::ExtINT)?;
        set_klapic_delivery_mode(&mut klapic, APIC_LVT1_REG_OFFSET, DeliveryMode::NMI)?;

        self.vcpu_fd.set_lapic(&klapic)?;

        Ok(())
    }

    fn configure_fpu(&self) -> Result<()> {
        let fpu = kvm_fpu {
            fcw: 0x37f,
            mxcsr: 0x1f80,
            ..Default::default()
        };

        self.vcpu_fd.set_fpu(&fpu)?;

        Ok(())
    }
}
