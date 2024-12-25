pub mod config;
mod constants;
mod cpu_ref;
mod device;
mod kernel;
mod memory;
mod vcpu;
pub mod vmm;

// use std::sync::{
//     atomic::{AtomicBool, Ordering},
//     Arc, Condvar, Mutex,
// };

// use event_manager::{EventManager, EventOps, EventSet, Events, MutEventSubscriber, SubscriberOps};
// use kvm_bindings::kvm_userspace_memory_region;
// use kvm_ioctls::{Kvm, VcpuFd, VmFd};
// use linux_loader::{
//     configurator::{linux::LinuxBootConfigurator, BootConfigurator, BootParams},
//     loader::{bootparam, KernelLoader, KernelLoaderResult},
// };
// use std::cell::RefCell;
// use util::result::{bail, Result};
// use vm_allocator::AddressAllocator;
// use vm_memory::{Address, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion};
// use vmm_sys_util::eventfd::EventFd;

// const MMIO_END: u64 = 1 << 32;
// const MMIO_SIZE: u64 = 768 << 20; // 768 mib
// const MMIO_START: u64 = MMIO_END - MMIO_SIZE;

// const ZERO_PAGE_START: u64 = 0x7000;
// const CMDLINE_START: u64 = 0x0002_0000;
// const HIGH_RAM_START: u64 = 0x0010_0000;

// const KERNEL_BOOT_FLAG_MAGIC: u16 = 0xaa55;
// const KERNEL_HDR_MAGIC: u32 = 0x5372_6448;
// const KERNEL_LOADER_OTHER: u8 = 0xff;
// const KERNEL_MIN_ALIGNMENT_BYTES: u32 = 0x0100_0000;

// const EBDA_START: u64 = 0x0009_fc00;
// const E820_RAM: u32 = 1;

// fn create_guest_memory(mem_size_mib: usize) -> Result<GuestMemoryMmap> {
//     let mem_size = mem_size_mib << 20;

//     let memory = GuestMemoryMmap::from_ranges(&[(GuestAddress(0), mem_size)])?;
//     Ok(memory)
// }

// fn create_mmio_allocator() -> Result<AddressAllocator> {
//     let alloc = AddressAllocator::new(MMIO_START, MMIO_SIZE)?;
//     Ok(alloc)
// }

// pub fn load_kernel(
//     guest_memory: &mut GuestMemoryMmap,
//     kernel_path: impl AsRef<str>,
//     kernel_cmd: impl AsRef<str>,
// ) -> Result<KernelLoaderResult> {
//     let mut kernel_image = std::fs::File::open(kernel_path.as_ref())?;

//     let kernel_load = linux_loader::loader::Elf::load(
//         guest_memory,
//         None,
//         &mut kernel_image,
//         Some(GuestAddress(HIGH_RAM_START)),
//     )?;

//     let mut boot_params = bootparam::boot_params::default();
//     boot_params.hdr.boot_flag = KERNEL_BOOT_FLAG_MAGIC;
//     boot_params.hdr.header = KERNEL_HDR_MAGIC;
//     boot_params.hdr.kernel_alignment = KERNEL_MIN_ALIGNMENT_BYTES;
//     boot_params.hdr.type_of_loader = KERNEL_LOADER_OTHER;

//     // EBDA
//     boot_params.e820_table[boot_params.e820_entries as usize].addr = 0;
//     boot_params.e820_table[boot_params.e820_entries as usize].size = EBDA_START;
//     boot_params.e820_table[boot_params.e820_entries as usize].type_ = E820_RAM;
//     boot_params.e820_entries += 1;

//     // Memory
//     boot_params.e820_table[boot_params.e820_entries as usize].addr = HIGH_RAM_START;
//     boot_params.e820_table[boot_params.e820_entries as usize].size = guest_memory
//         .last_addr()
//         .unchecked_offset_from(GuestAddress(HIGH_RAM_START));
//     boot_params.e820_table[boot_params.e820_entries as usize].type_ = E820_RAM;
//     boot_params.e820_entries += 1;

//     boot_params.hdr.cmd_line_ptr = CMDLINE_START as u32;
//     boot_params.hdr.cmdline_size = kernel_cmd.as_ref().len() as u32;

//     let mut cmdline = linux_loader::cmdline::Cmdline::new(4096)?;
//     cmdline.insert_str(kernel_cmd.as_ref())?;

//     linux_loader::loader::load_cmdline(guest_memory, GuestAddress(CMDLINE_START), &cmdline)?;

//     LinuxBootConfigurator::write_bootparams::<GuestMemoryMmap>(
//         &BootParams::new::<bootparam::boot_params>(&boot_params, GuestAddress(ZERO_PAGE_START)),
//         guest_memory,
//     )?;

//     Ok(kernel_load)
// }

// #[derive(Debug, Clone, PartialEq, Eq)]
// enum VmState {
//     Running,
//     Suspending,
//     Exiting,
// }

// impl Default for VmState {
//     fn default() -> Self {
//         VmState::Running
//     }
// }

// #[derive(Debug, Default)]
// struct VcpuState {
//     vm_state: Mutex<VmState>,
//     condvar: Condvar,
// }

// impl VcpuState {
//     fn set_and_notify(&self, state: VmState) {
//         let mut vm_state = self.vm_state.lock().unwrap();
//         *vm_state = state;
//         self.condvar.notify_all();
//     }
// }

// fn configure_memory_regions(
//     guest_memory: &GuestMemoryMmap,
//     kvm: &Kvm,
//     vm_fd: Arc<VmFd>,
// ) -> Result<()> {
//     if guest_memory.num_regions() > kvm.get_nr_memslots() {
//         bail!("Not enough KVM memory slots for guest memory regions");
//     }

//     for (index, region) in guest_memory.iter().enumerate() {
//         println!("mapping region:  {} {:?}", index, region);
//         let memory_region = kvm_userspace_memory_region {
//             slot: index as u32,
//             guest_phys_addr: region.start_addr().raw_value(),
//             memory_size: region.len() as u64,
//             userspace_addr: guest_memory.get_host_address(region.start_addr()).unwrap() as u64,
//             flags: 0,
//         };

//         unsafe {
//             vm_fd.set_user_memory_region(memory_region)?;
//         };
//     }

//     Ok(())
// }

// struct Vcpu {
//     vcpu_fd: VcpuFd,
//     state: Arc<VcpuState>,
// }

// impl Vcpu {
//     thread_local! {
//         static VCPU_PTR: RefCell<Option<*const Vcpu>> = RefCell::new(None);
//     }

//     fn new(id: u64, vm_fd: &VmFd) -> Result<Self> {
//         let vcpu_fd = vm_fd.create_vcpu(id)?;

//         let vcpu = Vcpu {
//             vcpu_fd,
//             state: Arc::new(VcpuState::default()),
//         };

//         Ok(vcpu)
//     }
// }

// pub fn create_vm(mem_size_mib: usize) -> Result<()> {
//     let kvm = Kvm::new()?;

//     let mut guest_memory = create_guest_memory(mem_size_mib)?;
//     let mmio_allocator = create_mmio_allocator()?;

//     // todo: device manager

//     let exit_handler = SharedExitHandler::new()?;

//     let mut event_manager = EventManager::<Arc<Mutex<dyn MutEventSubscriber + Send>>>::new()?;
//     event_manager.add_subscriber(exit_handler.0.clone());

//     let vm_fd = Arc::new(kvm.create_vm()?);
//     let vcpu_state = Arc::new(VcpuState::default());

//     configure_memory_regions(&guest_memory, &kvm, vm_fd.clone())?;

//     load_kernel(&mut guest_memory, "../linux/vmlinux", "")?;

//     Ok(())
// }

// struct ExitHandler {
//     exit_event: EventFd,
//     keep_running: AtomicBool,
// }

// #[derive(Clone)]
// struct SharedExitHandler(Arc<Mutex<ExitHandler>>);

// impl SharedExitHandler {
//     fn new() -> Result<Self> {
//         let exit_event = EventFd::new(libc::EFD_NONBLOCK)?;
//         let keep_running = AtomicBool::new(true);

//         let exit_handler = ExitHandler {
//             exit_event,
//             keep_running,
//         };

//         Ok(SharedExitHandler(Arc::new(Mutex::new(exit_handler))))
//     }

//     fn keep_running(&self) -> bool {
//         self.0.lock().unwrap().keep_running.load(Ordering::Acquire)
//     }

//     fn kick(&self) -> Result<()> {
//         self.0.lock().unwrap().exit_event.write(1)?;
//         Ok(())
//     }
// }

// impl MutEventSubscriber for ExitHandler {
//     fn process(&mut self, events: Events, ops: &mut EventOps) {
//         if events.event_set().contains(EventSet::IN) {
//             self.keep_running.store(false, Ordering::Release);
//         }

//         if events.event_set().contains(EventSet::ERROR) {
//             println!("Exit handler got error");
//             ops.remove(Events::new(&self.exit_event, EventSet::IN))
//                 .expect("Failed to remove exit handler");
//         }
//     }

//     fn init(&mut self, ops: &mut EventOps) {
//         ops.add(Events::new(&self.exit_event, EventSet::IN))
//             .expect("Failed to initialize exit handler");
//     }
// }
