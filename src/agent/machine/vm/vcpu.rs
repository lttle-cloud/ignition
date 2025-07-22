use std::sync::Arc;

use anyhow::{Result, bail};
use kvm_bindings::{CpuId, Msrs, kvm_fpu, kvm_regs};
use kvm_ioctls::{Kvm, VcpuFd, VmFd};
use tokio::sync::Barrier;
use vm_memory::{Address, Bytes, GuestAddress, GuestMemoryMmap};

use crate::agent::machine::vm::{
    constants::{
        BOOT_STACK_POINTER, PDE_START, PDPTE_START, PML4_START, X86_CR0_PE, X86_CR0_PG,
        X86_CR4_PAE, ZEROPG_START,
    },
    cpu_ref::{
        self,
        gdt::{BOOT_GDT_OFFSET, Gdt},
        interrupts::{
            APIC_LVT0_REG_OFFSET, APIC_LVT1_REG_OFFSET, DeliveryMode, set_klapic_delivery_mode,
        },
        msr_index,
    },
};

#[derive(Debug)]
enum VcpuStatus {
    Idle,
    Suspended,
    Running,
    Stopped,
}

#[derive(Debug)]
pub struct Vcpu {
    count: u8,
    index: u8,
    status: VcpuStatus,
    cpuid: CpuId,
    vcpu_fd: VcpuFd,
    supported_msrs: Msrs,
    barrier: Arc<Barrier>,
}

pub type VcpuRef = Arc<Vcpu>;

impl Vcpu {
    pub async fn new(
        kvm: &Kvm,
        vm_fd: &VmFd,
        memory: &GuestMemoryMmap,
        barrier: Arc<Barrier>,
        start_addr: GuestAddress,
        vcpu_count: u8,
        index: u8,
    ) -> Result<Self> {
        let base_cpuid = kvm.get_supported_cpuid(kvm_bindings::KVM_MAX_CPUID_ENTRIES)?;
        let supported_msrs = cpu_ref::msrs::supported_guest_msrs(kvm)?;

        let mut cpuid = base_cpuid.clone();
        cpu_ref::cpuid::filter_cpuid(kvm, index, vcpu_count, &mut cpuid);

        let vcpu_fd = vm_fd.create_vcpu(index as u64)?;

        let vcpu = Self {
            count: vcpu_count,
            index,
            status: VcpuStatus::Idle,
            cpuid,
            vcpu_fd,
            supported_msrs,
            barrier,
        };

        vcpu.configure_cpuid()?;
        vcpu.configure_msrs()?;
        vcpu.configure_sregs(memory)?;
        vcpu.configure_lapic()?;
        vcpu.configure_fpu()?;
        vcpu.setup_regs(start_addr)?;

        Ok(vcpu)
    }

    fn configure_cpuid(&self) -> Result<()> {
        self.vcpu_fd.set_cpuid2(&self.cpuid)?;
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

    fn configure_sregs(&self, memory: &GuestMemoryMmap) -> Result<()> {
        let mut sregs = self.vcpu_fd.get_sregs()?;

        let gdt_table = Gdt::default();

        let code_seg = gdt_table.create_kvm_segment_for(1).unwrap();
        let data_seg = gdt_table.create_kvm_segment_for(2).unwrap();
        let tss_seg = gdt_table.create_kvm_segment_for(3).unwrap();

        gdt_table.write_to_mem(memory)?;

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

        memory.write_obj(boot_pdpte_addr.raw_value() | 0x03, boot_pml4_addr)?;

        memory.write_obj(boot_pde_addr.raw_value() | 0x03, boot_pdpte_addr)?;

        for i in 0..512 {
            memory.write_obj((i << 21) + 0x83u64, boot_pde_addr.unchecked_add(i * 8))?;
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

    fn setup_regs(&self, start_addr: GuestAddress) -> Result<()> {
        let regs = kvm_regs {
            rflags: 0x0000_0000_0000_0002u64,
            rip: start_addr.raw_value(),
            rsp: BOOT_STACK_POINTER,
            rbp: BOOT_STACK_POINTER,
            rsi: ZEROPG_START,
            ..Default::default()
        };

        self.vcpu_fd.set_regs(&regs)?;

        Ok(())
    }
}
