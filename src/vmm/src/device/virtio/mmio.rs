use std::ops::Deref;
use std::sync::{Arc, Mutex};

use event_manager::{MutEventSubscriber, RemoteEndpoint, SubscriberId};
use kvm_ioctls::{IoEventAddress, VmFd};
use libc::EFD_NONBLOCK;
use util::result::{bail, Result};
use virtio_device::VirtioConfig;
use virtio_queue::Queue;
use virtio_queue::QueueT;
use vm_device::bus::{BusRange, MmioAddress};
use vmm_sys_util::eventfd::EventFd;

use crate::memory::Memory;

use super::{features, Env, VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET};

#[derive(Debug, Clone)]
pub struct MmioConfig {
    pub range: BusRange<MmioAddress>,
    pub irq: u32,
}

pub type Subscriber = Arc<Mutex<dyn MutEventSubscriber + Send>>;

pub struct VirtioMmioDeviceConfig {
    pub mmio: MmioConfig,
    pub virtio: VirtioConfig<Queue>,
    pub endpoint: RemoteEndpoint<Subscriber>,
    pub memory: Arc<Memory>,
    pub vm_fd: Arc<VmFd>,
    pub irqfd: Arc<EventFd>,
}

impl VirtioMmioDeviceConfig {
    pub fn new(virtio_config: VirtioConfig<Queue>, env: &Env) -> Result<Self> {
        let irqfd = Arc::new(EventFd::new(EFD_NONBLOCK)?);

        env.vm_fd.register_irqfd(&irqfd, env.mmio_cfg.irq)?;

        Ok(VirtioMmioDeviceConfig {
            virtio: virtio_config,
            mmio: env.mmio_cfg.clone(),
            endpoint: env.event_mgr.remote_endpoint(),
            vm_fd: env.vm_fd.clone(),
            memory: env.mem.clone(),
            irqfd,
        })
    }

    pub fn prepare_activate(&self) -> Result<Vec<EventFd>> {
        for q in self.virtio.queues.iter() {
            if !q.is_valid(self.memory.deref().guest_memory()) {
                bail!("Queue is not valid");
            }
        }

        if self.virtio.device_activated {
            bail!("Device already activated");
        }

        if self.virtio.driver_features & (1 << features::VIRTIO_F_VERSION_1) == 0 {
            bail!("Legacy drivers are not supported");
        }

        let mut ioevents = Vec::new();

        for i in 0..self.virtio.queues.len() {
            let fd = EventFd::new(EFD_NONBLOCK)?;

            // Register the queue event fd.
            self.vm_fd.register_ioevent(
                &fd,
                &IoEventAddress::Mmio(self.mmio.range.base().0 + VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET),
                u32::try_from(i).unwrap(),
            )?;

            ioevents.push(fd);
        }

        Ok(ioevents)
    }

    pub fn finalize_activate(&mut self, handler: Subscriber) -> Result<()> {
        let sub_handler = handler.clone();
        let _sub_id = self
            .endpoint
            .call_blocking(move |mgr| -> Result<SubscriberId> {
                Ok(mgr.add_subscriber(sub_handler))
            })?;

        self.virtio.device_activated = true;

        Ok(())
    }
}
