use std::{
    cell::RefCell,
    fs::File,
    os::{
        fd::{FromRawFd, IntoRawFd},
        unix::io::AsRawFd,
    },
    sync::{Arc, Barrier, Mutex},
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Result, bail};
use kvm_bindings::{CpuId, Msrs, kvm_fpu, kvm_regs};
use kvm_ioctls::{Kvm, KvmRunWrapper, VcpuExit, VcpuFd, VmFd};
use libc::{SIGRTMAX, c_int, siginfo_t};
use tracing::warn;
use vm_device::{
    bus::{MmioAddress, PioAddress},
    device_manager::{IoManager, MmioManager, PioManager},
};
use vm_memory::{Address, Bytes, GuestAddress, GuestMemoryMmap};
use vmm_sys_util::signal::{Killable, register_signal_handler};

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
    devices::meta::guest_manager::{GUEST_MANAGER_MMIO_START, GuestManagerDevice},
};

#[derive(Debug, PartialEq)]
pub enum VcpuStatus {
    Idle,
    Preparing,
    Running,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct VcpuEvent {
    pub event_type: VcpuEventType,
    pub vcpu_index: u8,
}

#[derive(Debug, Clone)]
pub enum VcpuEventType {
    Errored,
    Stopped,
    Suspended,
    Restarted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VcpuExitReason {
    Normal,
    Suspend,
}

pub struct Vcpu {
    pub count: u8,
    pub index: u8,
    pub status: VcpuStatus,
    pub cpuid: CpuId,
    pub vcpu_fd: VcpuFd,
    pub supported_msrs: Msrs,
    run_size: usize,
    barrier: Arc<Barrier>,
    io_manager: Arc<IoManager>,
    vcpu_event_tx: async_broadcast::Sender<VcpuEvent>,
    guest_manager: Arc<Mutex<GuestManagerDevice>>,
}

thread_local!(static THIS_VCPU_FD: RefCell<Option<(usize, i32)>> = RefCell::new(None));

fn sig_stop() -> c_int {
    SIGRTMAX() - 1
}

fn sig_suspend() -> c_int {
    SIGRTMAX() - 2
}

pub enum VcpuRunResult {
    Ok(Vcpu),
    Error(anyhow::Error, Vcpu),
}

#[derive(Debug)]
pub struct RunningVcpuHandle(JoinHandle<VcpuRunResult>);

impl RunningVcpuHandle {
    pub async fn signal_stop(&self, exit_reason: VcpuExitReason) {
        self.0
            .kill(if exit_reason == VcpuExitReason::Suspend {
                sig_suspend()
            } else {
                sig_stop()
            })
            .ok();
    }

    pub async fn join(self) -> VcpuRunResult {
        loop {
            if self.0.is_finished() {
                return self.0.join().unwrap();
            }

            // check every 10ms if the thread is finished
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn join_with_timeout(self, timeout: Duration) -> Result<VcpuRunResult, ()> {
        use std::time::Instant;

        let start = Instant::now();

        loop {
            if self.0.is_finished() {
                return Ok(self.0.join().unwrap());
            }

            if start.elapsed() > timeout {
                warn!(
                    "VCPU join timed out after {:?}, thread may be stuck",
                    timeout
                );
                return Err(()); // Return error to indicate timeout
            }

            // check every 10ms if the thread is finished
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn signal_stop_and_join(self, exit_reason: VcpuExitReason) -> VcpuRunResult {
        self.signal_stop(exit_reason).await;
        self.join().await
    }

    pub async fn signal_stop_and_join_with_timeout(
        self,
        exit_reason: VcpuExitReason,
        timeout: Duration,
    ) -> Result<VcpuRunResult, ()> {
        self.signal_stop(exit_reason).await;
        self.join_with_timeout(timeout).await
    }
}

impl Vcpu {
    pub async fn new(
        kvm: &Kvm,
        vm_fd: &VmFd,
        memory: &GuestMemoryMmap,
        io_manager: Arc<IoManager>,
        barrier: Arc<Barrier>,
        vcpu_event_tx: async_broadcast::Sender<VcpuEvent>,
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
        start_addr: GuestAddress,
        vcpu_count: u8,
        index: u8,
    ) -> Result<Self> {
        let base_cpuid = kvm.get_supported_cpuid(kvm_bindings::KVM_MAX_CPUID_ENTRIES)?;
        let supported_msrs = cpu_ref::msrs::supported_guest_msrs(kvm)?;

        let mut cpuid = base_cpuid.clone();
        cpu_ref::cpuid::filter_cpuid(kvm, index, vcpu_count, &mut cpuid);

        let run_size = vm_fd.run_size();
        let vcpu_fd = vm_fd.create_vcpu(index as u64)?;

        let vcpu = Self {
            count: vcpu_count,
            index,
            status: VcpuStatus::Idle,
            cpuid,
            vcpu_fd,
            supported_msrs,
            run_size,
            io_manager,
            barrier,
            vcpu_event_tx,
            guest_manager,
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

    fn setup_thread_local(&self) -> Result<()> {
        THIS_VCPU_FD.with(|fd| {
            *fd.borrow_mut() = Some((self.run_size, self.vcpu_fd.as_raw_fd()));
        });

        Ok(())
    }

    pub fn setup_signal_handler() -> Result<()> {
        extern "C" fn handle_signal(signal_num: c_int, _: *mut siginfo_t, _: *mut libc::c_void) {
            // Primary: signal causes EINTR which is sufficient
            // Secondary: set immediate exit via raw fd (100% crash-safe)
            let _ = THIS_VCPU_FD.try_with(|fd| {
                if let Ok(fd_ref) = fd.try_borrow() {
                    if let Some((run_size, vcpu_fd)) = *fd_ref {
                        let immediate_exit = if signal_num == sig_stop() {
                            1
                        } else if signal_num == sig_suspend() {
                            2
                        } else {
                            warn!("unhandled signal: {:?}", signal_num);
                            0
                        };

                        unsafe {
                            let fd = File::from_raw_fd(vcpu_fd);
                            let Ok(mut run_wrapper) = KvmRunWrapper::mmap_from_fd(&fd, run_size)
                            else {
                                warn!("Failed to mmap run wrapper");
                                return;
                            };

                            run_wrapper.as_mut_ref().immediate_exit = immediate_exit;

                            // drop the file; but don't close the fd
                            let _ = fd.into_raw_fd();
                        }
                    }
                }
            });
        }

        register_signal_handler(sig_stop(), handle_signal)?;
        register_signal_handler(sig_suspend(), handle_signal)?;

        Ok(())
    }

    fn run(&mut self) -> Result<VcpuExitReason> {
        Self::setup_signal_handler()?;
        self.setup_thread_local()?;

        // Clear any lingering immediate_exit flag from previous suspend
        self.vcpu_fd.set_kvm_immediate_exit(0);

        // if the vcpu is stopped, and we're restarting, we need to send the restarted event
        let restart = self.status == VcpuStatus::Stopped;

        self.status = VcpuStatus::Preparing;
        self.barrier.wait();
        self.status = VcpuStatus::Running;

        if restart {
            self.vcpu_event_tx
                .try_broadcast(VcpuEvent {
                    event_type: VcpuEventType::Restarted,
                    vcpu_index: self.index,
                })
                .ok();
        }

        'vcpu_loop: loop {
            match self.vcpu_fd.run() {
                Ok(exit) => match exit {
                    VcpuExit::Shutdown | VcpuExit::Hlt => {
                        warn!("Guest shutdown: {:?}.", exit);
                        break 'vcpu_loop;
                    }
                    VcpuExit::IoOut(addr, data) => {
                        if (0x3f8..(0x3f8 + 8)).contains(&addr) {
                            // write to serial port

                            if self.io_manager.pio_write(PioAddress(addr), data).is_err() {
                                warn!("Failed to write to serial port.");
                            }
                        } else if addr == 0x060 || addr == 0x061 || addr == 0x064 {
                            // write to i8042
                            if self.io_manager.pio_write(PioAddress(addr), data).is_err() {
                                warn!("Failed to write to i8042.");
                            }
                        } else if (0x070..=0x07f).contains(&addr) {
                            // rtc port write
                            warn!("unhandled rtc port write: {:x}", addr);
                        } else {
                            warn!("unhandled io port write: {:x}", addr);
                        }
                    }
                    VcpuExit::IoIn(addr, data) => {
                        if (0x3f8..(0x3f8 + 8)).contains(&addr) {
                            // read from serial port
                            if self.io_manager.pio_read(PioAddress(addr), data).is_err() {
                                warn!("Failed to read from serial port.");
                            }
                        } else {
                            warn!("unhandled io port read: {:x}", addr);
                        }
                    }
                    VcpuExit::MmioRead(addr, data) => {
                        if GuestManagerDevice::should_handle_read(addr) {
                            let mut guest_manager = self.guest_manager.lock().unwrap();
                            guest_manager.mmio_read(addr - GUEST_MANAGER_MMIO_START, data);
                            continue;
                        }

                        if self.io_manager.mmio_read(MmioAddress(addr), data).is_err() {
                            warn!("Failed to read from mmio.");
                        }
                    }
                    VcpuExit::MmioWrite(addr, data) => {
                        if GuestManagerDevice::should_handle_write(addr) {
                            let mut guest_manager = self.guest_manager.lock().unwrap();
                            let should_exit =
                                guest_manager.mmio_write(addr - GUEST_MANAGER_MMIO_START, data);

                            if should_exit {
                                return Ok(VcpuExitReason::Suspend);
                            }

                            continue;
                        }

                        match self.io_manager.mmio_write(MmioAddress(addr), data) {
                            Err(e) => {
                                warn!("Failed to write to mmio: {:?} {}", addr, e);
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        warn!("unhandled vcpu run exit: {:?}", exit);
                    }
                },
                Err(e) if e.errno() == libc::EAGAIN => {}
                Err(e) if e.errno() == libc::EINTR => {
                    // Clear the immediate exit flag after handling the interrupt
                    let k_run = self.vcpu_fd.get_kvm_run();
                    warn!("Vcpu run interrupt: {} {}", e, k_run.immediate_exit);

                    let exit_reason = match k_run.immediate_exit {
                        1 => VcpuExitReason::Normal,
                        2 => VcpuExitReason::Suspend,
                        _ => {
                            warn!("unhandled immediate exit: {:?}", k_run.immediate_exit);
                            VcpuExitReason::Normal
                        }
                    };

                    self.vcpu_fd.set_kvm_immediate_exit(0);

                    return Ok(exit_reason);
                }
                Err(e) => {
                    warn!("Vcpu run error: {}", e);
                    return Err(e.into());
                }
            };
        }

        THIS_VCPU_FD.with(|fd| {
            *fd.borrow_mut() = None;
        });

        Ok(VcpuExitReason::Normal)
    }

    pub async fn start(mut self) -> Result<RunningVcpuHandle> {
        let vcpu_event_tx = self.vcpu_event_tx.clone();
        let handle = std::thread::Builder::new()
            .name(format!("vcpu-{}", self.index))
            .spawn(move || match self.run() {
                Ok(exit_reason) => {
                    self.status = VcpuStatus::Stopped;

                    vcpu_event_tx
                        .try_broadcast(VcpuEvent {
                            event_type: if exit_reason == VcpuExitReason::Suspend {
                                VcpuEventType::Suspended
                            } else {
                                VcpuEventType::Stopped
                            },
                            vcpu_index: self.index,
                        })
                        .ok();

                    warn!("Vcpu {} stopped", self.index);

                    VcpuRunResult::Ok(self)
                }
                Err(e) => {
                    self.status = VcpuStatus::Stopped;

                    vcpu_event_tx
                        .try_broadcast(VcpuEvent {
                            event_type: VcpuEventType::Errored,
                            vcpu_index: self.index,
                        })
                        .ok();

                    VcpuRunResult::Error(e, self)
                }
            })?;

        Ok(RunningVcpuHandle(handle))
    }
}
