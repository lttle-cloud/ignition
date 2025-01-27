pub mod block;
pub mod mmio;
pub mod net;

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};

use event_manager::{EventManager, MutEventSubscriber};
use kvm_ioctls::VmFd;
use linux_loader::loader::Cmdline;
use mmio::MmioConfig;
use util::result::Result;
use virtio_device::VirtioConfig;
use virtio_queue::{Queue, QueueState, QueueT};
use vm_device::{device_manager::MmioManager, DeviceMmio};
use vm_memory::GuestAddress;
use vmm_sys_util::eventfd::EventFd;

use crate::{memory::Memory, state::VirtioState};

use super::SharedDeviceManager;

pub mod features {
    pub const VIRTIO_F_RING_EVENT_IDX: u64 = 29;
    pub const VIRTIO_F_VERSION_1: u64 = 32;
    pub const VIRTIO_F_IN_ORDER: u64 = 35;

    pub const VIRTIO_NET_F_CSUM: u64 = 0;
    pub const VIRTIO_NET_F_GUEST_CSUM: u64 = 1;
    pub const VIRTIO_NET_F_MAC: u32 = 5;
    pub const VIRTIO_NET_F_GUEST_TSO4: u64 = 7;
    pub const VIRTIO_NET_F_GUEST_TSO6: u64 = 8;
    pub const VIRTIO_NET_F_GUEST_UFO: u64 = 10;
    pub const VIRTIO_NET_F_HOST_TSO4: u64 = 11;
    pub const VIRTIO_NET_F_HOST_TSO6: u64 = 12;
    pub const VIRTIO_NET_F_HOST_UFO: u64 = 14;
}

const VIRTIO_MMIO_INT_VRING: u8 = 0x01;
pub const VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET: u64 = 0x50;

pub struct Env<'a> {
    pub mem: Arc<Memory>,
    pub vm_fd: Arc<VmFd>,
    pub event_mgr: &'a mut EventManager<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    pub device_manager: SharedDeviceManager,
    pub mmio_cfg: MmioConfig,
    pub kernel_cmdline: &'a mut Cmdline,
}

impl<'a> Env<'a> {
    pub fn register_mmio_device(
        &mut self,
        device: Arc<dyn DeviceMmio + Send + Sync>,
    ) -> Result<()> {
        {
            let mut io_manager = self.device_manager.io_manager.lock().unwrap();
            io_manager.register_mmio(self.mmio_cfg.range, device)?;
        }

        self.kernel_cmdline.add_virtio_mmio_device(
            self.mmio_cfg.range.size(),
            GuestAddress(self.mmio_cfg.range.base().0),
            self.mmio_cfg.irq,
            None,
        )?;

        Ok(())
    }
}

pub trait SignalUsedQueue {
    fn signal_used_queue(&self, index: u16);
}

pub struct SingleFdSignalQueue {
    pub irqfd: Arc<EventFd>,
    pub interrupt_status: Arc<AtomicU8>,
}

impl SignalUsedQueue for SingleFdSignalQueue {
    fn signal_used_queue(&self, _index: u16) {
        self.interrupt_status
            .fetch_or(VIRTIO_MMIO_INT_VRING, Ordering::SeqCst);
        self.irqfd
            .write(1)
            .expect("Failed write to eventfd when signalling queue");
    }
}

pub fn queue_from_state(state: &QueueState) -> Result<Queue> {
    let mut q = Queue::new(state.max_size)?;
    q.set_desc_table_address(
        Some(state.desc_table as u32),
        Some((state.desc_table >> 32) as u32),
    );
    q.set_avail_ring_address(
        Some(state.avail_ring as u32),
        Some((state.avail_ring >> 32) as u32),
    );
    q.set_used_ring_address(
        Some(state.used_ring as u32),
        Some((state.used_ring >> 32) as u32),
    );
    q.set_next_avail(state.next_avail);
    q.set_next_used(state.next_used);
    q.set_event_idx(state.event_idx_enabled);
    q.set_size(state.size);
    q.set_ready(state.ready);

    Ok(q)
}

pub fn virtio_state_from_config(config: &VirtioConfig<Queue>) -> VirtioState {
    let queues = config.queues.iter().map(|q| q.state()).collect::<Vec<_>>();

    let state = VirtioState {
        config_generation: config.config_generation,
        conifg_space: config.config_space.clone(),
        device_activated: config.device_activated,
        device_features: config.device_features,
        device_features_sel: config.device_features_select,
        device_status: config.device_status,
        driver_features: config.driver_features,
        driver_features_sel: config.driver_features_select,
        interrupt_status: config.interrupt_status.load(Ordering::Relaxed),
        queue_sel: config.queue_select,
        queues,
    };

    state
}

pub fn virtio_config_from_state(state: &VirtioState) -> VirtioConfig<Queue> {
    let queues = state
        .queues
        .iter()
        .map(|q| queue_from_state(q).unwrap())
        .collect::<Vec<_>>();

    let mut config = VirtioConfig::new(state.device_features, queues, state.conifg_space.clone());

    config.config_generation = state.config_generation;
    config.device_activated = state.device_activated;
    config.device_features_select = state.device_features_sel;
    config.device_status = state.device_status;
    config.driver_features = state.driver_features;
    config.driver_features_select = state.driver_features_sel;
    config.interrupt_status = Arc::new(AtomicU8::new(state.interrupt_status));
    config.queue_select = state.queue_sel;

    config
}
