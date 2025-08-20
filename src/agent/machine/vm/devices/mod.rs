pub mod alloc;
pub mod legacy;
pub mod meta;
pub mod virtio;

use std::{
    fs::OpenOptions,
    io::BufWriter,
    sync::{Arc, Mutex},
};

use anyhow::{Result, bail};
use event_manager::{EventManager, MutEventSubscriber};
use kvm_bindings::{KVM_PIT_SPEAKER_DUMMY, kvm_pit_config, kvm_userspace_memory_region};
use kvm_ioctls::{Kvm, VmFd};
use linux_loader::loader::Cmdline;
use vm_allocator::{AddressAllocator, AllocPolicy};
use vm_device::{
    bus::{BusRange, MmioAddress, PioAddress, PioRange},
    device_manager::{IoManager, PioManager},
};
use vm_memory::{Address, GuestMemory, GuestMemoryMmap, GuestMemoryRegion};
use vm_superio::Serial;
use vmm_sys_util::eventfd::EventFd;

use crate::agent::machine::{
    machine::{MachineConfig, MachineMode, NetworkConfig, VolumeMountConfig},
    vm::{
        constants::{MAX_IRQ, SERIAL_IRQ},
        cpu_ref::mptable::MpTable,
        devices::{
            alloc::IrqAllocator,
            legacy::{serial::SerialWrapper, trigger::EventFdTrigger},
            meta::guest_manager::GuestManagerDevice,
            virtio::{Env, block::device::Block, mmio::MmioConfig, net::device::Net},
        },
    },
};

#[derive(Clone)]
pub struct VmDevices {
    pub guest_manager: Arc<Mutex<GuestManagerDevice>>,
    pub net: Arc<Mutex<Net>>,
    pub blocks: Vec<Arc<Mutex<Block>>>,
}

#[derive(Debug, Clone)]
pub enum DeviceEvent {
    UserSpaceReady,
    StopRequested,
    FlashLock,
    FlashUnlock,
    ExitCode(i32),
}

pub async fn setup_devices(
    machine_config: &MachineConfig,
    kvm: &Kvm,
    vm_fd: Arc<VmFd>,
    memory: &GuestMemoryMmap,
    irq_allocator: &mut IrqAllocator,
    mmio_allocator: &mut AddressAllocator,
    io_manager: &mut IoManager,
    event_manager: &mut EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    kernel_cmdline: &mut Cmdline,
    log_path: &str,
    device_event_tx: async_broadcast::Sender<DeviceEvent>,
) -> Result<VmDevices> {
    setup_memory_regions(kvm, vm_fd.clone(), memory)?;

    MpTable::new(machine_config.resources.cpu, MAX_IRQ as u8)?.write(memory)?;

    setup_irq_controller(vm_fd.clone())?;
    setup_serial_console(vm_fd.clone(), io_manager, log_path)?;

    let snapshot_strategy = match &machine_config.mode {
        MachineMode::Regular => None,
        MachineMode::Flash {
            snapshot_strategy, ..
        } => Some(snapshot_strategy.clone()),
    };
    let guest_manager = GuestManagerDevice::new(device_event_tx.clone(), snapshot_strategy);

    let net = setup_network_device(
        vm_fd.clone(),
        &machine_config.network,
        irq_allocator,
        mmio_allocator,
        io_manager,
        event_manager,
        memory,
        kernel_cmdline,
    )?;

    let mut blocks = vec![];

    for volume_mount in machine_config.volume_mounts.iter() {
        let block = setup_block_device(
            vm_fd.clone(),
            volume_mount,
            irq_allocator,
            mmio_allocator,
            io_manager,
            event_manager,
            memory,
            kernel_cmdline,
        )?;

        blocks.push(block);
    }

    Ok(VmDevices {
        guest_manager,
        net,
        blocks,
    })
}

fn register_irq_fd(vm_fd: Arc<VmFd>, fd: &EventFd, irq: u32) -> Result<()> {
    vm_fd.register_irqfd(fd, irq)?;
    Ok(())
}

fn setup_memory_regions(kvm: &Kvm, vm_fd: Arc<VmFd>, memory: &GuestMemoryMmap) -> Result<()> {
    if memory.num_regions() > kvm.get_nr_memslots() {
        bail!("Not enough KVM memory slots for guest memory regions");
    }

    for (index, region) in memory.iter().enumerate() {
        let memory_region = kvm_userspace_memory_region {
            slot: index as u32,
            guest_phys_addr: region.start_addr().raw_value(),
            memory_size: region.len() as u64,
            userspace_addr: memory.get_host_address(region.start_addr()).unwrap() as u64,
            flags: 0,
        };

        unsafe {
            vm_fd.set_user_memory_region(memory_region)?;
        };
    }

    Ok(())
}

fn setup_irq_controller(vm_fd: Arc<VmFd>) -> Result<()> {
    vm_fd.create_irq_chip()?;

    let pit_config = kvm_pit_config {
        flags: KVM_PIT_SPEAKER_DUMMY,
        ..Default::default()
    };

    vm_fd.create_pit2(pit_config)?;

    Ok(())
}

fn setup_serial_console(
    vm_fd: Arc<VmFd>,
    io_manager: &mut IoManager,
    log_path: &str,
) -> Result<()> {
    let irq_fd = EventFdTrigger::new(libc::EFD_NONBLOCK)?;

    register_irq_fd(vm_fd, &irq_fd, SERIAL_IRQ)?;

    // serial port
    let range = PioRange::new(PioAddress(0x3f8), 8)?;

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;

    let log_file = BufWriter::new(log_file);

    let serial = Serial::new(irq_fd.try_clone()?, log_file);
    let serial = SerialWrapper(serial);
    let serial = Arc::new(Mutex::new(serial));

    io_manager.register_pio(range, serial.clone())?;

    Ok(())
}

fn setup_network_device(
    vm_fd: Arc<VmFd>,
    network: &NetworkConfig,
    irq_allocator: &mut IrqAllocator,
    mmio_allocator: &mut AddressAllocator,
    io_manager: &mut IoManager,
    event_manager: &mut EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    memory: &GuestMemoryMmap,
    kernel_cmdline: &mut Cmdline,
) -> Result<Arc<Mutex<Net>>> {
    let mmio_range = {
        let range = mmio_allocator.allocate(0x1000, 4, AllocPolicy::FirstMatch)?;
        BusRange::new(MmioAddress(range.start()), range.len())?
    };

    let irq = irq_allocator.next_irq()?;

    let mmio_config = MmioConfig {
        range: mmio_range,
        irq,
    };

    let mut env = Env {
        from_state: false,
        mem: memory.clone(),
        event_mgr: event_manager,
        mmio_cfg: mmio_config,
        vm_fd: vm_fd.clone(),
        kernel_cmdline,
    };

    let net = Net::new(&mut env, io_manager, network.clone())?;
    Ok(net)
}

fn setup_block_device(
    vm_fd: Arc<VmFd>,
    volume_mount: &VolumeMountConfig,
    irq_allocator: &mut IrqAllocator,
    mmio_allocator: &mut AddressAllocator,
    io_manager: &mut IoManager,
    event_manager: &mut EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    memory: &GuestMemoryMmap,
    kernel_cmdline: &mut Cmdline,
) -> Result<Arc<Mutex<Block>>> {
    let mmio_range = {
        let range = mmio_allocator.allocate(0x1000, 4, AllocPolicy::FirstMatch)?;
        BusRange::new(MmioAddress(range.start()), range.len())?
    };

    let irq = irq_allocator.next_irq()?;

    let mmio_config = MmioConfig {
        range: mmio_range,
        irq,
    };

    let mut env = Env {
        from_state: false,
        mem: memory.clone(),
        vm_fd: vm_fd.clone(),
        event_mgr: event_manager,
        mmio_cfg: mmio_config,
        kernel_cmdline,
    };

    let block = Block::new(&mut env, io_manager, volume_mount.clone())?;
    Ok(block)
}
