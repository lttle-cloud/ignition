use crate::{
    config::{BlockConfig, Config, NetConfig},
    constants::{
        self, CMDLINE_START, E820_RAM, EBDA_START, HIGH_RAM_START, KERNEL_BOOT_FLAG_MAGIC,
        KERNEL_HDR_MAGIC, KERNEL_LOADER_OTHER, KERNEL_MIN_ALIGNMENT_BYTES, SERIAL_IRQ,
        ZERO_PAGE_START,
    },
    device::{
        legacy::{i8042::I8042Wrapper, serial::SerialWrapper, trigger::EventFdTrigger},
        meta::guest_manager::GuestManagerDevice,
        virtio::{block::device::Block, mmio::MmioConfig, net::device::Net, Env},
        SharedDeviceManager,
    },
    memory::Memory,
    state::{BlockState, NetState, VmmState},
    vcpu::ExitHandler,
    vm::{Vm, VmConfig},
};
use event_manager::{EventManager, EventOps, EventSet, Events, MutEventSubscriber, SubscriberOps};
use kvm_ioctls::Kvm;
use linux_loader::{
    configurator::{linux::LinuxBootConfigurator, BootConfigurator, BootParams},
    loader::{bootparam, KernelLoader, KernelLoaderResult},
};
use std::{
    fs::OpenOptions,
    io::{stdout, BufWriter, Seek, SeekFrom},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use util::tracing::warn;
use util::{
    async_runtime::sync::broadcast,
    result::{anyhow, bail, Result},
};
use vm_allocator::AllocPolicy;
use vm_device::{
    bus::{BusRange, MmioAddress, PioAddress, PioRange},
    device_manager::PioManager,
};
use vm_memory::{
    Address, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion, ReadVolatile,
};
use vm_superio::{I8042Device, Serial};
use vmm_sys_util::eventfd::EventFd;

pub struct Vmm {
    config: Config,
    memory: Arc<Memory>,
    device_manager: SharedDeviceManager,
    state_controller: VmmStateController,
    event_manager: EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    exit_handler: SharedExitEventHandler,
    vm: Vm<SharedExitEventHandler>,
    state: Option<VmmState>,
    net_device: Option<Arc<Mutex<Net>>>,
    block_devices: Vec<Arc<Mutex<Block>>>,
}

#[derive(Debug, Clone)]
pub enum VmmStateControllerMessage {
    StopRequested,
    Stopping,
    Stopped(VmmState),
    NetworkReady,
    Error(String),
}

#[derive(Clone)]
pub struct VmmStateController {
    evt: Arc<broadcast::Sender<VmmStateControllerMessage>>,
}

impl VmmStateController {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);

        VmmStateController { evt: tx.into() }
    }

    pub fn send(&self, msg: VmmStateControllerMessage) {
        let _ = self.evt.send(msg);
    }

    pub fn request_stop(&self) {
        let _ = self.evt.send(VmmStateControllerMessage::StopRequested);
    }

    pub fn rx(&self) -> broadcast::Receiver<VmmStateControllerMessage> {
        self.evt.subscribe()
    }
}

impl Vmm {
    pub fn create_memory_from_config(config: &Config) -> Result<Arc<Memory>> {
        let memory = if let Some(path) = config.memory.path.as_ref() {
            let file = OpenOptions::new().read(true).write(true).open(path)?;
            Memory::new_backed_by_file(config.memory.clone(), file)?
        } else {
            Memory::new(config.memory.clone())?
        };
        let memory = Arc::new(memory);

        Ok(memory)
    }

    pub fn new(config: Config, memory: Arc<Memory>) -> Result<Self> {
        let kvm = Kvm::new()?;
        Vmm::check_kvm_caps(&kvm)?;

        let device_manager = SharedDeviceManager::new(SERIAL_IRQ)?;
        let state_controller = VmmStateController::new();

        let vm_config = VmConfig::new(&kvm, config.vcpu.num)?;
        let exit_handler = SharedExitEventHandler::new()?;

        let guest_manger = GuestManagerDevice::new(
            exit_handler.clone(),
            state_controller.clone(),
            config.snapshot_policy.clone(),
        );

        let vm = Vm::new(
            &kvm,
            &memory,
            vm_config,
            exit_handler.clone(),
            device_manager.clone(),
            guest_manger,
        )?;

        let mut event_manager = EventManager::<Arc<Mutex<dyn MutEventSubscriber + Send>>>::new()?;
        event_manager.add_subscriber(exit_handler.0.clone());

        let mut vmm = Vmm {
            config: config.clone(),
            memory,
            device_manager,
            state_controller,
            event_manager,
            exit_handler,
            vm,
            state: None,
            net_device: None,
            block_devices: vec![],
        };

        vmm.add_serial_console()?;
        vmm.add_i8042_device()?;

        if let Some(net_config) = &config.net {
            vmm.add_net_device(net_config)?;
        }

        for block_config in config.block.iter() {
            vmm.add_block_device(block_config.clone())?;
        }

        Ok(vmm)
    }

    pub fn from_state(state: VmmState, memory: Arc<Memory>) -> Result<Self> {
        let kvm = Kvm::new()?;
        Vmm::check_kvm_caps(&kvm)?;

        let device_manager = SharedDeviceManager::new(SERIAL_IRQ)?;
        let state_controller = VmmStateController::new();
        let exit_handler = SharedExitEventHandler::new()?;

        let guest_manger = GuestManagerDevice::new(
            exit_handler.clone(),
            state_controller.clone(),
            state.config.snapshot_policy.clone(),
        );

        {
            let mut guest_manger = guest_manger.lock().unwrap();
            guest_manger.set_exit_enabled(false);
        }

        let vm = Vm::from_state(
            &kvm,
            &memory,
            &state.vm_state,
            exit_handler.clone(),
            device_manager.clone(),
            guest_manger,
        )?;

        let mut event_manager = EventManager::<Arc<Mutex<dyn MutEventSubscriber + Send>>>::new()?;
        event_manager.add_subscriber(exit_handler.0.clone());

        let mut vmm = Vmm {
            config: state.config.clone(),
            memory,
            device_manager,
            state_controller,
            event_manager,
            exit_handler,
            vm,
            state: Some(state.clone()),
            net_device: None,
            block_devices: vec![],
        };

        vmm.add_serial_console()?;
        vmm.add_i8042_device()?;

        if let Some(net_state) = &state.net_state {
            vmm.add_net_device_from_state(net_state)?;
        }

        for block_state in state.block_states.iter() {
            vmm.add_block_device_from_tate(block_state)?;
        }

        Ok(vmm)
    }

    pub fn controller(&self) -> VmmStateController {
        self.state_controller.clone()
    }

    pub fn run(&mut self) -> Result<(VmmState, Arc<Memory>)> {
        let kernel_load_addr = match &self.state {
            Some(state) => state.kernel_load_addr.clone(),
            None => {
                let kernel_load = self.load_kernel()?;

                let Some(kernel_load_addr) = self
                    .memory
                    .guest_memory()
                    .check_address(kernel_load.kernel_load)
                else {
                    bail!("Invalid kernel load address");
                };

                kernel_load_addr
            }
        };

        let start_address = if self.state.is_some() {
            None
        } else {
            Some(kernel_load_addr)
        };

        self.vm.run(start_address)?;

        let mut control_rx = self.state_controller.rx();
        loop {
            match self.event_manager.run_with_timeout(100) {
                Ok(_) => {}
                Err(e) => {
                    warn!("Error running event manager: {:?}", e);
                    break;
                }
            }

            if !self.exit_handler.keep_running() {
                break;
            }

            let ev = control_rx.try_recv();
            match ev {
                Ok(VmmStateControllerMessage::StopRequested) => {
                    self.state_controller
                        .evt
                        .send(VmmStateControllerMessage::Stopping)?;
                    self.stop()?;
                    break;
                }
                _ => {}
            }
        }

        let state = self.vm.shutdown()?;
        let net_state = if let Some(net_device) = &self.net_device {
            let mut net = net_device.lock().unwrap();
            Some(net.get_state()?)
        } else {
            None
        };

        let block_states = self
            .block_devices
            .iter()
            .map(|dev| {
                let mut dev = dev.lock().unwrap();
                dev.get_state()
            })
            .collect::<Result<Vec<_>>>()?;

        self.memory.reset_mmio_allocator()?;
        self.device_manager.reset_irq();

        let state = VmmState {
            config: self.config.clone(),
            vm_state: state,
            kernel_load_addr,
            net_state,
            block_states,
        };

        self.state_controller
            .evt
            .send(VmmStateControllerMessage::Stopped(state.clone()))?;

        Ok((state, self.memory.clone()))
    }

    pub fn stop(&self) -> Result<()> {
        self.exit_handler.trigger_exit(ExitHandlerReason::Exit)?;

        Ok(())
    }

    fn check_kvm_caps(kvm: &Kvm) -> Result<()> {
        let required_caps = vec![
            kvm_ioctls::Cap::Irqchip,
            kvm_ioctls::Cap::Ioeventfd,
            kvm_ioctls::Cap::Irqfd,
            kvm_ioctls::Cap::UserMemory,
        ];

        for cap in required_caps {
            if !kvm.check_extension(cap) {
                bail!("required KVM cap not supported: {:?}", cap);
            }
        }

        Ok(())
    }

    fn add_serial_console(&mut self) -> Result<()> {
        let irq_fd = EventFdTrigger::new(libc::EFD_NONBLOCK)?;

        self.vm.register_irqfd(&irq_fd, SERIAL_IRQ)?;

        if self.state.is_none() {
            self.config.kernel.cmdline.insert_str("console=ttyS0")?;
        }

        let range = PioRange::new(PioAddress(0x3f8), 8)?;
        let mut io_manager = self.device_manager.io_manager.lock().unwrap();

        if let Some(log_file_path) = &self.config.log_file_path {
            let log_file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_file_path)?;

            let log_file = BufWriter::new(log_file);

            let serial = Serial::new(irq_fd.try_clone()?, log_file);
            let serial = SerialWrapper(serial);
            let serial = Arc::new(Mutex::new(serial));

            io_manager.register_pio(range, serial.clone())?;
        } else {
            let writer = BufWriter::new(stdout());

            let serial = Serial::new(irq_fd.try_clone()?, writer);
            let serial = SerialWrapper(serial);
            let serial = Arc::new(Mutex::new(serial));

            io_manager.register_pio(range, serial.clone())?;
        };

        Ok(())
    }

    fn add_i8042_device(&mut self) -> Result<()> {
        let reset_fd = EventFdTrigger::new(libc::EFD_NONBLOCK)?;

        let i8042 = I8042Device::new(reset_fd.try_clone()?);
        let i8042 = I8042Wrapper(i8042);
        let i8042 = Arc::new(Mutex::new(i8042));

        self.vm.register_irqfd(&reset_fd, 1)?;

        let range = PioRange::new(PioAddress(0x060), 8)?;

        let mut io_manager = self.device_manager.io_manager.lock().unwrap();
        io_manager.register_pio(range, i8042)?;

        Ok(())
    }

    fn add_net_device(&mut self, config: &NetConfig) -> Result<()> {
        let mmio_range = {
            let mut alloc = self.memory.lock_mmio_allocator();
            let range = alloc.allocate(0x1000, 4, AllocPolicy::FirstMatch)?;
            BusRange::new(MmioAddress(range.start()), range.len())?
        };

        let irq = self.device_manager.next_irq()?;

        let mmio_config = MmioConfig {
            range: mmio_range,
            irq,
        };

        let mut env = Env {
            from_state: false,
            mem: self.memory.clone(),
            vm_fd: self.vm.fd(),
            device_manager: self.device_manager.clone(),
            event_mgr: &mut self.event_manager,
            mmio_cfg: mmio_config,
            kernel_cmdline: &mut self.config.kernel.cmdline,
        };

        let net = Net::new(&mut env, config.clone(), self.state_controller.clone())?;
        self.net_device = Some(net);

        Ok(())
    }

    fn add_net_device_from_state(&mut self, state: &NetState) -> Result<()> {
        let mmio_range = {
            let mut alloc = self.memory.lock_mmio_allocator();
            let range = alloc.allocate(0x1000, 4, AllocPolicy::FirstMatch)?;
            BusRange::new(MmioAddress(range.start()), range.len())?
        };

        let irq = self.device_manager.next_irq()?;

        let mmio_config = MmioConfig {
            range: mmio_range,
            irq,
        };

        let mut env = Env {
            from_state: true,
            mem: self.memory.clone(),
            vm_fd: self.vm.fd(),
            device_manager: self.device_manager.clone(),
            event_mgr: &mut self.event_manager,
            mmio_cfg: mmio_config,
            kernel_cmdline: &mut self.config.kernel.cmdline,
        };

        let net = Net::from_state(&mut env, &state, self.state_controller.clone())?;
        self.net_device = Some(net);

        Ok(())
    }

    fn add_block_device(&mut self, config: BlockConfig) -> Result<()> {
        let mmio_range = {
            let mut alloc = self.memory.lock_mmio_allocator();
            let range = alloc.allocate(0x1000, 4, AllocPolicy::FirstMatch)?;
            BusRange::new(MmioAddress(range.start()), range.len())?
        };

        let irq = self.device_manager.next_irq()?;

        let mmio_config = MmioConfig {
            range: mmio_range,
            irq,
        };

        let mut env = Env {
            from_state: false,
            mem: self.memory.clone(),
            vm_fd: self.vm.fd(),
            device_manager: self.device_manager.clone(),
            event_mgr: &mut self.event_manager,
            mmio_cfg: mmio_config,
            kernel_cmdline: &mut self.config.kernel.cmdline,
        };

        let block = Block::new(&mut env, config.clone())?;
        self.block_devices.push(block);

        Ok(())
    }

    fn add_block_device_from_tate(&mut self, state: &BlockState) -> Result<()> {
        let mmio_range = {
            let mut alloc = self.memory.lock_mmio_allocator();
            let range = alloc.allocate(0x1000, 4, AllocPolicy::FirstMatch)?;
            BusRange::new(MmioAddress(range.start()), range.len())?
        };

        let irq = self.device_manager.next_irq()?;

        let mmio_config = MmioConfig {
            range: mmio_range,
            irq,
        };

        let mut env = Env {
            from_state: true,
            mem: self.memory.clone(),
            vm_fd: self.vm.fd(),
            device_manager: self.device_manager.clone(),
            event_mgr: &mut self.event_manager,
            mmio_cfg: mmio_config,
            kernel_cmdline: &mut self.config.kernel.cmdline,
        };

        let block = Block::from_state(&mut env, state)?;
        self.block_devices.push(block);

        Ok(())
    }

    fn load_kernel(&mut self) -> Result<KernelLoaderResult> {
        let mut kernel_image = std::fs::File::open(&self.config.kernel.path)?;

        let kernel_load = linux_loader::loader::Elf::load(
            self.memory.guest_memory(),
            None,
            &mut kernel_image,
            Some(GuestAddress(HIGH_RAM_START)),
        )?;

        let mut boot_params = bootparam::boot_params::default();
        boot_params.hdr.boot_flag = KERNEL_BOOT_FLAG_MAGIC;
        boot_params.hdr.header = KERNEL_HDR_MAGIC;
        boot_params.hdr.kernel_alignment = KERNEL_MIN_ALIGNMENT_BYTES;
        boot_params.hdr.type_of_loader = KERNEL_LOADER_OTHER;

        // EBDA
        boot_params.e820_table[boot_params.e820_entries as usize].addr = 0;
        boot_params.e820_table[boot_params.e820_entries as usize].size = EBDA_START;
        boot_params.e820_table[boot_params.e820_entries as usize].type_ = E820_RAM;
        boot_params.e820_entries += 1;

        // Memory
        boot_params.e820_table[boot_params.e820_entries as usize].addr = HIGH_RAM_START;
        boot_params.e820_table[boot_params.e820_entries as usize].size = self
            .memory
            .guest_memory()
            .last_addr()
            .unchecked_offset_from(GuestAddress(HIGH_RAM_START));
        boot_params.e820_table[boot_params.e820_entries as usize].type_ = E820_RAM;
        boot_params.e820_entries += 1;

        if let Some(initrd_path) = self.config.kernel.initrd_path.as_ref() {
            let (size, addr) = self.load_initrd(initrd_path.clone())?;

            boot_params.hdr.ramdisk_image = addr.raw_value() as u32;
            boot_params.hdr.ramdisk_size = size as u32;
        }

        boot_params.hdr.cmd_line_ptr = CMDLINE_START as u32;
        boot_params.hdr.cmdline_size =
            self.config.kernel.cmdline.as_cstring()?.as_bytes().len() as u32;

        linux_loader::loader::load_cmdline(
            self.memory.guest_memory(),
            GuestAddress(CMDLINE_START),
            &self.config.kernel.cmdline,
        )?;

        LinuxBootConfigurator::write_bootparams::<GuestMemoryMmap>(
            &BootParams::new::<bootparam::boot_params>(&boot_params, GuestAddress(ZERO_PAGE_START)),
            self.memory.guest_memory(),
        )?;

        Ok(kernel_load)
    }

    fn load_initrd(&mut self, initrd_path: String) -> Result<(usize, GuestAddress)> {
        let mut initrd_image = std::fs::File::open(&initrd_path)?;

        let size = match initrd_image.seek(SeekFrom::End(0)) {
            Err(err) => bail!("Initrd image seek failed: {}", err),
            Ok(0) => {
                bail!("Initrd image is empty");
            }
            Ok(s) => s as usize,
        };

        initrd_image.seek(SeekFrom::Start(0))?;

        let first_region = self
            .memory
            .guest_memory()
            .find_region(GuestAddress(0))
            .ok_or(anyhow!(
                "Failed to find a suitable region for the initrd image"
            ))?;

        let first_region_size = first_region.len() as usize;

        if first_region_size < size {
            bail!("First memory region is too small for the initrd image");
        }

        let address = first_region_size - size;
        let aligned_address = (address & !(constants::PAGE_SIZE - 1)) as u64;

        let mut mem = self
            .memory
            .guest_memory()
            .get_slice(GuestAddress(aligned_address), size)?;

        initrd_image.read_exact_volatile(&mut mem)?;

        Ok((size, GuestAddress(aligned_address)))
    }
}

pub enum ExitHandlerReason {
    Exit,
    Snapshot,
}

struct ExitEventHandler {
    exit_event: EventFd,
    exit_reason: Option<ExitHandlerReason>,
    keep_running: AtomicBool,
}

#[derive(Clone)]
pub struct SharedExitEventHandler(Arc<Mutex<ExitEventHandler>>);

impl SharedExitEventHandler {
    pub fn new() -> Result<Self> {
        let exit_event = EventFd::new(libc::EFD_NONBLOCK)?;
        let keep_running = AtomicBool::new(true);

        let exit_handler = ExitEventHandler {
            exit_event,
            keep_running,
            exit_reason: None,
        };

        Ok(SharedExitEventHandler(Arc::new(Mutex::new(exit_handler))))
    }

    fn keep_running(&self) -> bool {
        self.0.lock().unwrap().keep_running.load(Ordering::Acquire)
    }

    pub fn trigger_exit(&self, reason: ExitHandlerReason) -> Result<()> {
        let mut handler = self.0.lock().unwrap();

        handler.exit_reason = Some(reason);
        handler.exit_event.write(1)?;

        Ok(())
    }
}

impl ExitHandler for SharedExitEventHandler {
    fn kick(&self) -> Result<()> {
        Ok(self.0.lock().unwrap().exit_event.write(1)?)
    }
}

impl MutEventSubscriber for ExitEventHandler {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        if events.event_set().contains(EventSet::IN) {
            self.keep_running.store(false, Ordering::Release);
        }
        if events.event_set().contains(EventSet::ERROR) {
            let _ = ops.remove(Events::new(&self.exit_event, EventSet::IN));
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        ops.add(Events::new(&self.exit_event, EventSet::IN))
            .expect("Cannot initialize exit handler.");
    }
}
