use std::{
    cell::RefCell,
    sync::{Arc, Barrier},
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Result, bail};
use kvm_bindings::{CpuId, Msrs, kvm_fpu, kvm_regs};
use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};
use libc::{SIGRTMAX, c_int, siginfo_t};
use tracing::{debug, warn};
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
};

#[derive(Debug, PartialEq)]
pub enum VcpuStatus {
    Idle,
    Preparing,
    Running,
    Stopped,
}

#[derive(Debug)]
pub struct VcpuEvent {
    pub event_type: VcpuEventType,
    pub vcpu_index: u8,
}

#[derive(Debug)]
pub enum VcpuEventType {
    Errored,
    Stopped,
    Restarted,
}

pub struct Vcpu {
    pub count: u8,
    pub index: u8,
    pub status: VcpuStatus,
    pub cpuid: CpuId,
    pub vcpu_fd: VcpuFd,
    pub supported_msrs: Msrs,
    barrier: Arc<Barrier>,
    io_manager: Arc<IoManager>,
    vcpu_event_tx: async_channel::Sender<VcpuEvent>,
}

thread_local!(static THIS_VCPU_PTR: RefCell<Option<*mut Vcpu>> = RefCell::new(None));

pub enum VcpuRunResult {
    Ok(Vcpu),
    Error(anyhow::Error, Vcpu),
}

#[derive(Debug)]
pub struct RunningVcpuHandle(JoinHandle<VcpuRunResult>);

impl RunningVcpuHandle {
    pub async fn signal_stop(&self) {
        self.0.kill(SIGRTMAX() - 1).ok();
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

    pub async fn signal_stop_and_join(self) -> VcpuRunResult {
        self.signal_stop().await;
        self.join().await
    }
}

impl Vcpu {
    pub async fn new(
        kvm: &Kvm,
        vm_fd: &VmFd,
        memory: &GuestMemoryMmap,
        io_manager: Arc<IoManager>,
        barrier: Arc<Barrier>,
        vcpu_event_tx: async_channel::Sender<VcpuEvent>,
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
            io_manager,
            barrier,
            vcpu_event_tx,
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
        THIS_VCPU_PTR.with(|vcpu| {
            if vcpu.borrow().is_none() {
                let vcpu_ptr = (self as *const Self).cast_mut();
                *vcpu.borrow_mut() = Some(vcpu_ptr);
            }
        });

        Ok(())
    }

    pub fn setup_signal_handler() -> Result<()> {
        extern "C" fn handle_signal(_: c_int, _: *mut siginfo_t, _: *mut libc::c_void) {
            THIS_VCPU_PTR.with(|vcpu| {
                if let Some(vcpu_ptr) = *vcpu.borrow() {
                    let vcpu = unsafe { &mut *vcpu_ptr };
                    vcpu.vcpu_fd.set_kvm_immediate_exit(1);
                }
            });
        }

        register_signal_handler(SIGRTMAX() - 1, handle_signal)?;

        Ok(())
    }

    fn run(&mut self) -> Result<()> {
        Self::setup_signal_handler()?;
        self.setup_thread_local()?;

        // if the vcpu is stopped, and we're restarting, we need to send the restarted event
        let restart = self.status == VcpuStatus::Stopped;

        self.status = VcpuStatus::Preparing;
        self.barrier.wait();
        self.status = VcpuStatus::Running;

        if restart {
            self.vcpu_event_tx
                .try_send(VcpuEvent {
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
                        // if GuestManagerDevice::should_handle_read(addr) {
                        //     let mut guest_manager = self.guest_manager.lock().unwrap();
                        //     guest_manager.mmio_read(addr - GUEST_MANAGER_MMIO_START, data);

                        //     if guest_manager.should_exit_immediately() {
                        //         break 'vcpu_loop;
                        //     }

                        //     continue;
                        // }

                        if self.io_manager.mmio_read(MmioAddress(addr), data).is_err() {
                            warn!("Failed to read from mmio.");
                        }
                    }
                    VcpuExit::MmioWrite(addr, data) => {
                        // if GuestManagerDevice::should_handle_write(addr) {
                        //     let mut guest_manager = self.guest_manager.lock().unwrap();
                        //     guest_manager.mmio_write(addr - GUEST_MANAGER_MMIO_START, data);

                        //     if guest_manager.should_exit_immediately() {
                        //         break 'vcpu_loop;
                        //     }

                        //     continue;
                        // }

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
                    warn!("Vcpu run interrupt: {}", e);
                    // Clear the immediate exit flag after handling the interrupt
                    self.vcpu_fd.set_kvm_immediate_exit(0);
                    break 'vcpu_loop;
                }
                Err(e) => {
                    warn!("Vcpu run error: {}", e);
                    return Err(e.into());
                }
            };
        }

        Ok(())
    }

    pub async fn start(mut self) -> Result<RunningVcpuHandle> {
        let vcpu_event_tx = self.vcpu_event_tx.clone();
        let handle = std::thread::Builder::new()
            .name(format!("vcpu-{}", self.index))
            .spawn(move || match self.run() {
                Ok(_) => {
                    self.status = VcpuStatus::Stopped;

                    vcpu_event_tx
                        .try_send(VcpuEvent {
                            event_type: VcpuEventType::Stopped,
                            vcpu_index: self.index,
                        })
                        .ok();

                    warn!("Vcpu {} stopped", self.index);

                    VcpuRunResult::Ok(self)
                }
                Err(e) => {
                    self.status = VcpuStatus::Stopped;

                    vcpu_event_tx
                        .try_send(VcpuEvent {
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
