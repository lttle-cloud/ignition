use crate::{
    constants::{
        BOOT_STACK_POINTER, PDE_START, PDPTE_START, PML4_START, X86_CR0_PE, X86_CR0_PG,
        X86_CR4_PAE, ZEROPG_START,
    },
    cpu_ref::{
        self,
        gdt::{Gdt, BOOT_GDT_OFFSET},
        interrupts::{
            set_klapic_delivery_mode, DeliveryMode, APIC_LVT0_REG_OFFSET, APIC_LVT1_REG_OFFSET,
        },
        msr_index,
    },
    device::{
        meta::guest_manager::{GuestManagerDevice, GUEST_MANAGER_MMIO_START},
        SharedDeviceManager,
    },
    memory::Memory,
    state::VcpuState,
    vm::VmRunState,
};
use kvm_bindings::{kvm_fpu, kvm_regs, CpuId, Msrs};
use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};
use libc::{c_int, siginfo_t, SIGRTMIN};
use std::cell::RefCell;
use std::sync::{Arc, Barrier, Condvar, Mutex};
use util::result::{bail, Result};
use vm_device::{
    bus::{MmioAddress, PioAddress},
    device_manager::{MmioManager, PioManager},
};
use vm_memory::{Address, Bytes, GuestAddress};
use vmm_sys_util::signal::register_signal_handler;

#[derive(Clone, Debug)]
pub struct VcpuConfig {
    pub id: u8,
    pub cpuid: CpuId,
    pub msrs: Msrs,
}

#[derive(Clone, Debug)]
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
    guest_manager: Arc<Mutex<GuestManagerDevice>>,
    config: VcpuConfig,
    run_barrier: Arc<Barrier>,
    pub run_state: Arc<VcpuRunState>,
}

thread_local!(static TLS_VCPU_PTR: RefCell<Option<*mut Vcpu>> = RefCell::new(None));

impl Vcpu {
    pub fn new(
        vm_fd: &VmFd,
        device_manager: SharedDeviceManager,
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
        config: VcpuConfig,
        run_barrier: Arc<Barrier>,
        run_state: Arc<VcpuRunState>,
        memory: &Memory,
    ) -> Result<Self> {
        let vcpu = Vcpu {
            vcpu_fd: vm_fd.create_vcpu(config.id.into())?,
            device_manager,
            guest_manager,
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

    pub fn from_state(
        vm_fd: &VmFd,
        device_manager: SharedDeviceManager,
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
        state: &VcpuState,
        run_barrier: Arc<Barrier>,
        run_state: Arc<VcpuRunState>,
    ) -> Result<Self> {
        let mut vcpu = Vcpu {
            vcpu_fd: vm_fd.create_vcpu(state.config.id.into())?,
            device_manager,
            guest_manager,
            config: state.config.clone(),
            run_barrier,
            run_state,
        };

        vcpu.set_state(&state)?;

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

    pub fn run(&mut self, start_addr: Option<GuestAddress>) -> Result<()> {
        if let Some(start_addr) = start_addr {
            self.setup_regs(start_addr)?;
        }

        self.init_tls()?;

        self.run_barrier.wait();
        'vcpu_loop: loop {
            let mut interrupt_by_signal = false;

            match self.vcpu_fd.run() {
                Ok(exist_reason) => match exist_reason {
                    VcpuExit::Shutdown | VcpuExit::Hlt => {
                        println!("Guest shutdown: {:?}.", exist_reason);

                        self.run_state.set_and_notify(VmRunState::Exiting);
                        break;
                    }
                    VcpuExit::IoOut(addr, data) => {
                        if (0x3f8..(0x3f8 + 8)).contains(&addr) {
                            // write to serial port

                            let io_manager = self.device_manager.io_manager.lock().unwrap();
                            if io_manager.pio_write(PioAddress(addr), data).is_err() {
                                eprintln!("Failed to write to serial port.");
                            }
                        } else if addr == 0x060 || addr == 0x061 || addr == 0x064 {
                            // write to i8042
                            let io_manager = self.device_manager.io_manager.lock().unwrap();
                            if io_manager.pio_write(PioAddress(addr), data).is_err() {
                                eprintln!("Failed to write to i8042.");
                            }
                        } else if (0x070..=0x07f).contains(&addr) {
                            // rtc port write
                        } else {
                            // unhandled io port write
                        }
                    }
                    VcpuExit::IoIn(addr, data) => {
                        if (0x3f8..(0x3f8 + 8)).contains(&addr) {
                            // read from serial port

                            let io_manager = self.device_manager.io_manager.lock().unwrap();
                            if io_manager.pio_read(PioAddress(addr), data).is_err() {
                                eprintln!("Failed to read from serial port.");
                            }
                        } else {
                            // unhandled io port read
                        }
                    }
                    VcpuExit::MmioRead(addr, data) => {
                        if GuestManagerDevice::should_handle_read(addr) {
                            let mut guest_manager = self.guest_manager.lock().unwrap();
                            guest_manager.mmio_read(addr - GUEST_MANAGER_MMIO_START, data);

                            if guest_manager.should_exit_immediately() {
                                break 'vcpu_loop;
                            }

                            continue;
                        }

                        let io_manager = self.device_manager.io_manager.lock().unwrap();
                        if io_manager.mmio_read(MmioAddress(addr), data).is_err() {
                            eprintln!("Failed to read from mmio.");
                        }
                    }
                    VcpuExit::MmioWrite(addr, data) => {
                        if GuestManagerDevice::should_handle_write(addr) {
                            let mut guest_manager = self.guest_manager.lock().unwrap();
                            guest_manager.mmio_write(addr - GUEST_MANAGER_MMIO_START, data);

                            if guest_manager.should_exit_immediately() {
                                break 'vcpu_loop;
                            }

                            continue;
                        }

                        let io_manager = self.device_manager.io_manager.lock().unwrap();
                        match io_manager.mmio_write(MmioAddress(addr), data) {
                            Err(e) => {
                                eprintln!("Failed to write to mmio: {:?} {}", addr, e);
                            }
                            _ => {}
                        }
                    }
                    _other => {
                        println!("Unhandled exit reason: {:?}", _other);
                    }
                },
                Err(e) => match e.errno() {
                    libc::EAGAIN => {}
                    libc::EINTR => {
                        interrupt_by_signal = true;
                    }
                    _ => {
                        println!("Vcpu run failed: {:?}", e);
                        break;
                    }
                },
            }

            if interrupt_by_signal {
                self.vcpu_fd.set_kvm_immediate_exit(0);
                let mut run_state_locked = self.run_state.vm_state.lock().unwrap();

                loop {
                    match *run_state_locked {
                        VmRunState::Running => {
                            break;
                        }
                        VmRunState::Suspending => {}
                        VmRunState::Exiting => {
                            break 'vcpu_loop;
                        }
                    }

                    run_state_locked = self.run_state.condvar.wait(run_state_locked).unwrap();
                }
            }
        }

        Ok(())
    }

    fn init_tls(&mut self) -> Result<()> {
        TLS_VCPU_PTR.with(|vcpu| {
            if vcpu.borrow().is_none() {
                *vcpu.borrow_mut() = Some(self as *mut Vcpu);
                Ok(())
            } else {
                bail!("TLS already initialized");
            }
        })?;

        Ok(())
    }

    pub fn setup_signal_handler() -> Result<()> {
        extern "C" fn handle_signal(_: c_int, _: *mut siginfo_t, _: *mut libc::c_void) {
            Vcpu::set_local_exit(1);
        }

        register_signal_handler(SIGRTMIN() + 0, handle_signal)?;

        Ok(())
    }

    fn set_local_exit(value: u8) {
        TLS_VCPU_PTR.with(|v| {
            if let Some(vcpu) = *v.borrow_mut() {
                // The block below modifies a mmaped memory region (`kvm_run` struct) which is valid
                // as long as the `VMM` is still in scope. This function is called in response to
                // SIGRTMIN(), while the vCPU threads are still active. Their termination are
                // strictly bound to the lifespan of the `VMM` and it precedes the `VMM` dropping.
                unsafe {
                    let vcpu_ref: &mut Vcpu = &mut *vcpu;
                    vcpu_ref.vcpu_fd.set_kvm_immediate_exit(value);
                };
            }
        });
    }

    pub fn save_state(&mut self) -> Result<VcpuState> {
        let mp_state = self.vcpu_fd.get_mp_state()?;
        let regs = self.vcpu_fd.get_regs()?;
        let sregs = self.vcpu_fd.get_sregs()?;
        let xsave = self.vcpu_fd.get_xsave()?;
        let xcrs = self.vcpu_fd.get_xcrs()?;
        let debug_regs = self.vcpu_fd.get_debug_regs()?;
        let lapic = self.vcpu_fd.get_lapic()?;

        let mut msrs = self.config.msrs.clone();
        let num_msrs = self.config.msrs.as_fam_struct_ref().nmsrs as usize;
        let nmsrs = self.vcpu_fd.get_msrs(&mut msrs)?;
        if nmsrs != num_msrs {
            bail!(
                "Failed to get all MSRs. Expected {}, got {}",
                num_msrs,
                nmsrs
            );
        }
        let vcpu_events = self.vcpu_fd.get_vcpu_events()?;

        let cpuid = self
            .vcpu_fd
            .get_cpuid2(kvm_bindings::KVM_MAX_CPUID_ENTRIES)?;

        Ok(VcpuState {
            cpuid,
            msrs,
            debug_regs,
            lapic,
            mp_state,
            regs,
            sregs,
            vcpu_events,
            xcrs,
            xsave,
            config: self.config.clone(),
        })
    }

    fn set_state(&mut self, state: &VcpuState) -> Result<()> {
        self.vcpu_fd.set_cpuid2(&state.cpuid)?;
        self.vcpu_fd.set_mp_state(state.mp_state)?;
        self.vcpu_fd.set_regs(&state.regs)?;
        self.vcpu_fd.set_sregs(&state.sregs)?;
        self.vcpu_fd.set_xsave(&state.xsave)?;
        self.vcpu_fd.set_xcrs(&state.xcrs)?;
        self.vcpu_fd.set_debug_regs(&state.debug_regs)?;
        self.vcpu_fd.set_lapic(&state.lapic)?;
        self.vcpu_fd.set_msrs(&state.msrs)?;
        self.vcpu_fd.set_vcpu_events(&state.vcpu_events)?;
        Ok(())
    }
}

impl Drop for Vcpu {
    fn drop(&mut self) {
        TLS_VCPU_PTR.with(|v| {
            *v.borrow_mut() = None;
        });
    }
}
