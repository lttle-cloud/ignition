use crate::{
    config::Config,
    constants::{
        self, CMDLINE_START, E820_RAM, EBDA_START, HIGH_RAM_START, KERNEL_BOOT_FLAG_MAGIC,
        KERNEL_HDR_MAGIC, KERNEL_LOADER_OTHER, KERNEL_MIN_ALIGNMENT_BYTES, MMIO_LEN, SERIAL_IRQ,
        ZERO_PAGE_START,
    },
    device::{
        legacy::{i8042::I8042Wrapper, serial::SerialWrapper, trigger::EventFdTrigger},
        meta::guest_manager::GuestManagerDevice,
        SharedDeviceManager,
    },
    memory::Memory,
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
    io::{stdin, stdout, Seek, SeekFrom},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use util::result::{anyhow, bail, Result};
use vm_allocator::AllocPolicy;
use vm_device::{
    bus::{MmioAddress, MmioRange, PioAddress, PioRange},
    device_manager::{MmioManager, PioManager},
};
use vm_memory::{
    Address, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion, ReadVolatile,
};
use vm_superio::{I8042Device, Serial};
use vmm_sys_util::{eventfd::EventFd, terminal::Terminal};

pub struct Vmm {
    config: Config,
    memory: Arc<Memory>,
    device_manager: SharedDeviceManager,
    event_manager: EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    exit_handler: SharedExitEventHandler,
    vm: Vm<SharedExitEventHandler>,
}

impl Vmm {
    pub fn new(config: Config) -> Result<Self> {
        let kvm = Kvm::new()?;
        Vmm::check_kvm_caps(&kvm)?;

        let memory = Arc::new(Memory::new(config.memory.clone())?);
        let device_manager = SharedDeviceManager::new(SERIAL_IRQ)?;

        let vm_config = VmConfig::new(&kvm, config.vcpu.num)?;
        let exit_handler = SharedExitEventHandler::new()?;

        let vm = Vm::new(
            &kvm,
            &memory,
            vm_config,
            exit_handler.clone(),
            device_manager.clone(),
        )?;

        let mut event_manager = EventManager::<Arc<Mutex<dyn MutEventSubscriber + Send>>>::new()?;
        event_manager.add_subscriber(exit_handler.0.clone());

        let mut vmm = Vmm {
            config,
            memory,
            device_manager,
            event_manager,
            exit_handler,
            vm,
        };

        vmm.add_serial_console()?;
        vmm.add_i8042_device()?;
        vmm.add_guest_manager_device()?;

        Ok(vmm)
    }

    pub fn run(&mut self) -> Result<()> {
        let kernel_load = self.load_kernel()?;

        let Some(kernel_load_addr) = self
            .memory
            .guest_memory()
            .check_address(kernel_load.kernel_load)
        else {
            bail!("Invalid kernel load address");
        };

        println!("Kernel loaded at: {:?}", kernel_load_addr);

        if stdin().lock().set_raw_mode().is_err() {
            eprintln!("Failed to set raw mode on terminal. Stdin will echo.");
        }

        self.vm.run(kernel_load_addr)?;

        loop {
            match self.event_manager.run() {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Error running event manager: {:?}", e);
                    break;
                }
            }

            if !self.exit_handler.keep_running() {
                break;
            }
        }

        self.vm.shutdown();

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

        let serial = Serial::new(irq_fd.try_clone()?, stdout());
        let serial = SerialWrapper(serial);
        let serial = Arc::new(Mutex::new(serial));

        self.vm.register_irqfd(&irq_fd, SERIAL_IRQ)?;

        self.config.kernel.cmdline.insert_str("console=ttyS0")?;

        let range = PioRange::new(PioAddress(0x3f8), 8)?;
        let mut io_manager = self.device_manager.io_manager.lock().unwrap();
        io_manager.register_pio(range, serial.clone())?;

        self.event_manager.add_subscriber(serial);

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

    fn add_guest_manager_device(&mut self) -> Result<()> {
        let mut io_manager = self.device_manager.io_manager.lock().unwrap();

        let address = {
            let mut mmio_allocator = self.memory.lock_mmio_allocator();
            mmio_allocator.allocate(MMIO_LEN, MMIO_LEN, AllocPolicy::FirstMatch)?
        };

        let device = GuestManagerDevice::new(self.memory.clone(), self.exit_handler.clone());
        let device = Arc::new(Mutex::new(device));

        io_manager.register_mmio(
            MmioRange::new(MmioAddress(address.start()), address.len())?,
            device,
        )?;

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

struct ExitEventHandler {
    exit_event: EventFd,
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
        };

        Ok(SharedExitEventHandler(Arc::new(Mutex::new(exit_handler))))
    }

    fn keep_running(&self) -> bool {
        self.0.lock().unwrap().keep_running.load(Ordering::Acquire)
    }

    pub fn trigger_exit(&self) -> Result<()> {
        self.0.lock().unwrap().exit_event.write(1)?;
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
