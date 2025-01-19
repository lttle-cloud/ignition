use std::{
    borrow::{Borrow, BorrowMut},
    ops::Deref,
    sync::{Arc, Mutex},
    thread,
};

use event_manager::{MutEventSubscriber, RemoteEndpoint, SubscriberId};
use kvm_ioctls::{IoEventAddress, VmFd};
use libc::EFD_NONBLOCK;
use util::result::{anyhow, bail, Error, Result};
use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioDeviceType, VirtioMmioDevice};
use virtio_queue::{Queue, QueueT};
use vm_device::{
    bus::{MmioAddress, MmioAddressOffset},
    MutDeviceMmio,
};
use vmm_sys_util::eventfd::EventFd;

use crate::{
    config::NetConfig,
    device::virtio::{
        features::{
            self, VIRTIO_F_IN_ORDER, VIRTIO_F_RING_EVENT_IDX, VIRTIO_F_VERSION_1,
            VIRTIO_NET_F_CSUM, VIRTIO_NET_F_GUEST_CSUM, VIRTIO_NET_F_GUEST_TSO4,
            VIRTIO_NET_F_GUEST_TSO6, VIRTIO_NET_F_GUEST_UFO, VIRTIO_NET_F_HOST_TSO4,
            VIRTIO_NET_F_HOST_TSO6, VIRTIO_NET_F_HOST_UFO, VIRTIO_NET_F_MAC,
        },
        mmio::MmioConfig,
        virtio_config_from_state, virtio_state_from_config, Env, SingleFdSignalQueue,
        VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
    },
    memory::Memory,
    state::NetState,
};

use super::{
    bindings,
    handler::{NetHandler, QueueHandler},
    tap::Tap,
};

const QUEUE_MAX_SIZE: u16 = 256;

pub const VIRTIO_NET_HDR_SIZE: usize = 12;

pub const NET_DEVICE_ID: u32 = 1;

pub const RXQ_INDEX: u16 = 0;
pub const TXQ_INDEX: u16 = 1;

pub struct Net {
    config: NetConfig,
    memory: Arc<Memory>,
    vm_fd: Arc<VmFd>,
    mmio_conig: MmioConfig,
    virtio: VirtioConfig<Queue>,
    irqfd: Arc<EventFd>,
    endpoint: RemoteEndpoint<Arc<Mutex<dyn MutEventSubscriber + Send>>>,
    handler: Option<Arc<Mutex<QueueHandler>>>,
}

#[repr(C, packed)]
#[derive(Debug, Default, Copy, Clone)]
struct VirtioNetConfig {
    mac: [u8; 6],
}

impl VirtioNetConfig {
    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

fn mac_to_hw_addr(mac: &str) -> [u8; 6] {
    let mut hw_addr = [0u8; 6];
    let mac_bytes: Vec<u8> = mac
        .split(':')
        .map(|s| u8::from_str_radix(s, 16).unwrap())
        .collect();
    hw_addr.copy_from_slice(&mac_bytes);
    hw_addr
}

impl Net {
    pub fn new(env: &mut Env, config: NetConfig) -> Result<Arc<Mutex<Self>>> {
        let device_features: u64 = (1 << VIRTIO_F_VERSION_1)
            | (1 << VIRTIO_F_RING_EVENT_IDX)
            | (1 << VIRTIO_F_IN_ORDER)
            | (1 << VIRTIO_NET_F_CSUM)
            | (1 << VIRTIO_NET_F_GUEST_CSUM)
            | (1 << VIRTIO_NET_F_GUEST_TSO4)
            | (1 << VIRTIO_NET_F_GUEST_TSO6)
            | (1 << VIRTIO_NET_F_GUEST_UFO)
            | (1 << VIRTIO_NET_F_HOST_TSO4)
            | (1 << VIRTIO_NET_F_HOST_TSO6)
            | (1 << VIRTIO_NET_F_HOST_UFO)
            | (1 << VIRTIO_NET_F_MAC);

        let queues = vec![Queue::new(QUEUE_MAX_SIZE)?, Queue::new(QUEUE_MAX_SIZE)?];

        let cfg = VirtioNetConfig {
            mac: mac_to_hw_addr(&config.mac_addr),
        };

        let virtio_cfg = VirtioConfig::new(device_features, queues, cfg.as_bytes().to_vec());

        let irqfd =
            Arc::new(EventFd::new(EFD_NONBLOCK).map_err(|_| anyhow!("Failed to create EventFd"))?);

        env.vm_fd.register_irqfd(&irqfd, env.mmio_cfg.irq)?;

        env.kernel_cmdline.insert_str(format!(
            "ip={ip}::{gateway}:{netmask}::eth0:off",
            ip = config.ip_addr,
            gateway = config.gateway,
            netmask = config.netmask,
        ))?;

        let net = Net {
            config,
            memory: env.mem.clone(),
            vm_fd: env.vm_fd.clone(),
            mmio_conig: env.mmio_cfg.clone(),
            virtio: virtio_cfg,
            irqfd,
            endpoint: env.event_mgr.remote_endpoint(),
            handler: None,
        };
        let net = Arc::new(Mutex::new(net));

        env.register_mmio_device(net.clone())?;

        Ok(net)
    }

    pub fn from_state(env: &mut Env, state: &NetState) -> Result<Arc<Mutex<Self>>> {
        let config = state.config.clone();

        let mut virtio_cfg = virtio_config_from_state(&state.virtio_state);
        virtio_cfg.device_activated = false;

        let irqfd =
            Arc::new(EventFd::new(EFD_NONBLOCK).map_err(|_| anyhow!("Failed to create EventFd"))?);

        env.vm_fd.register_irqfd(&irqfd, env.mmio_cfg.irq)?;

        let net = Net {
            config,
            memory: env.mem.clone(),
            vm_fd: env.vm_fd.clone(),
            mmio_conig: env.mmio_cfg.clone(),
            virtio: virtio_cfg,
            irqfd,
            endpoint: env.event_mgr.remote_endpoint(),
            handler: None,
        };
        let net = Arc::new(Mutex::new(net));

        env.register_mmio_device(net.clone())?;

        let activate_net = net.clone();
        thread::spawn(move || -> Result<()> {
            let mut net = activate_net.lock().unwrap();
            net.activate()?;
            Ok(())
        });

        Ok(net)
    }

    fn prepare_activate(&self) -> Result<Vec<EventFd>> {
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
                &IoEventAddress::Mmio(
                    self.mmio_conig.range.base().0 + VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
                ),
                u32::try_from(i).unwrap(),
            )?;

            ioevents.push(fd);
        }

        Ok(ioevents)
    }

    pub fn finalize_activate(&mut self, handler: Arc<Mutex<QueueHandler>>) -> Result<()> {
        let sub_handler = handler.clone();
        let _sub_id = self
            .endpoint
            .call_blocking(move |mgr| -> Result<SubscriberId> {
                Ok(mgr.add_subscriber(sub_handler))
            })?;

        self.virtio.device_activated = true;
        self.handler = Some(handler);

        Ok(())
    }

    pub fn get_state(&mut self) -> Result<NetState> {
        let handler = self
            .handler
            .take()
            .ok_or_else(|| anyhow!("Handler not found"))?;

        let mut handler = handler.lock().unwrap();
        let (rxq_state, txq_state) = handler.inner.get_queue_states()?;

        let mut virtio_state = virtio_state_from_config(&self.virtio);
        virtio_state.queues = vec![rxq_state, txq_state];

        Ok(NetState {
            config: self.config.clone(),
            virtio_state,
        })
    }
}

impl VirtioDeviceType for Net {
    fn device_type(&self) -> u32 {
        NET_DEVICE_ID
    }
}

impl Borrow<VirtioConfig<Queue>> for Net {
    fn borrow(&self) -> &VirtioConfig<Queue> {
        &self.virtio
    }
}
impl BorrowMut<VirtioConfig<Queue>> for Net {
    fn borrow_mut(&mut self) -> &mut VirtioConfig<Queue> {
        &mut self.virtio
    }
}

impl VirtioDeviceActions for Net {
    type E = Error;

    fn activate(&mut self) -> Result<()> {
        let Ok(tap) = Tap::open_named(&self.config.tap_name) else {
            bail!("Failed to open tap device: {}", self.config.tap_name);
        };

        tap.set_offload(
            bindings::TUN_F_CSUM
                | bindings::TUN_F_UFO
                | bindings::TUN_F_TSO4
                | bindings::TUN_F_TSO6,
        )
        .map_err(|_| {
            anyhow!(
                "Failed to set offload flags for tap device: {}",
                self.config.tap_name
            )
        })?;

        tap.set_vnet_hdr_size(VIRTIO_NET_HDR_SIZE as i32)
            .map_err(|_| {
                anyhow!(
                    "Failed to set vnet hdr size for tap device: {}",
                    self.config.tap_name
                )
            })?;

        let driver_notify = SingleFdSignalQueue {
            irqfd: self.irqfd.clone(),
            interrupt_status: self.virtio.interrupt_status.clone(),
        };

        let mut ioevents = self.prepare_activate()?;

        let rxq = self.virtio.queues.remove(0);
        let txq = self.virtio.queues.remove(0);

        let handler = NetHandler::new(self.memory.clone(), driver_notify, rxq, txq, tap);

        let handler = Arc::new(Mutex::new(QueueHandler {
            inner: handler,
            rx_ioevent: ioevents.remove(0),
            tx_ioevent: ioevents.remove(0),
        }));

        self.finalize_activate(handler)?;

        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        Ok(())
    }
}

impl VirtioMmioDevice for Net {}

impl MutDeviceMmio for Net {
    fn mmio_read(&mut self, _base: MmioAddress, offset: MmioAddressOffset, data: &mut [u8]) {
        self.read(offset, data);
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: MmioAddressOffset, data: &[u8]) {
        self.write(offset, data);
    }
}
