use std::{
    borrow::{Borrow, BorrowMut},
    sync::{self, mpsc::Sender, Arc, Mutex},
    thread,
};

use tracing::warn;
use util::result::{anyhow, bail, Error, Result};
use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioDeviceType, VirtioMmioDevice};
use virtio_queue::{Queue, QueueT};
use vm_device::{
    bus::{MmioAddress, MmioAddressOffset},
    MutDeviceMmio,
};

use crate::{
    config::NetConfig,
    device::{
        meta::guest_manager::GuestManagerDevice,
        virtio::{
            features::{
                VIRTIO_F_IN_ORDER, VIRTIO_F_RING_EVENT_IDX, VIRTIO_F_VERSION_1, VIRTIO_NET_F_CSUM,
                VIRTIO_NET_F_GUEST_CSUM, VIRTIO_NET_F_GUEST_TSO4, VIRTIO_NET_F_GUEST_TSO6,
                VIRTIO_NET_F_GUEST_UFO, VIRTIO_NET_F_HOST_TSO4, VIRTIO_NET_F_HOST_TSO6,
                VIRTIO_NET_F_HOST_UFO, VIRTIO_NET_F_MAC,
            },
            mmio::VirtioMmioDeviceConfig,
            virtio_config_from_state, virtio_state_from_config, Env, SingleFdSignalQueue,
        },
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
    device: VirtioMmioDeviceConfig,
    memory: Arc<Memory>,
    handler: Option<Arc<Mutex<QueueHandler>>>,
    ready_tx: Option<Sender<()>>,
    guest_manager: Arc<Mutex<GuestManagerDevice>>,
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
    pub fn new(
        env: &mut Env,
        config: NetConfig,
        net_ready_tx: Option<sync::mpsc::Sender<()>>,
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
    ) -> Result<Arc<Mutex<Self>>> {
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

        env.kernel_cmdline.insert_str(format!(
            "ip={ip}::{gateway}:{netmask}::eth0:off",
            ip = config.ip_addr,
            gateway = config.gateway,
            netmask = config.netmask,
        ))?;

        let device = VirtioMmioDeviceConfig::new(virtio_cfg, env)?;

        let net = Net {
            config,
            memory: env.mem.clone(),
            device,
            handler: None,
            ready_tx: net_ready_tx,
            guest_manager,
        };
        let net = Arc::new(Mutex::new(net));

        env.register_mmio_device(net.clone())?;

        Ok(net)
    }

    pub fn from_state(
        env: &mut Env,
        state: &NetState,
        net_ready_tx: Option<sync::mpsc::Sender<()>>,
        guest_manager: Arc<Mutex<GuestManagerDevice>>,
    ) -> Result<Arc<Mutex<Self>>> {
        let config = state.config.clone();

        let mut virtio_cfg = virtio_config_from_state(&state.virtio_state);
        virtio_cfg.device_activated = false;

        let device = VirtioMmioDeviceConfig::new(virtio_cfg, env)?;

        let net = Net {
            config,
            memory: env.mem.clone(),
            device,
            handler: None,
            ready_tx: net_ready_tx,
            guest_manager,
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

    pub fn finalize_activate(&mut self, handler: Arc<Mutex<QueueHandler>>) -> Result<()> {
        self.device.finalize_activate(handler.clone())?;
        self.handler = Some(handler);

        if let Some(ready_tx) = self.ready_tx.as_ref() {
            if let Err(e) = ready_tx.send(()) {
                warn!("Failed to send net ready signal: {}", e);
            }
        }

        // TODO: this is a hack, we need a central event bus
        {
            let mut gm = self.guest_manager.lock().unwrap();
            gm.set_boot_ready();
        }

        Ok(())
    }

    pub fn get_state(&mut self) -> Result<NetState> {
        let handler = self
            .handler
            .take()
            .ok_or_else(|| anyhow!("Handler not found"))?;

        let handler = handler.lock().unwrap();
        let (rxq_state, txq_state) = handler.inner.get_queue_states();

        let mut virtio_state = virtio_state_from_config(&self.device.virtio);
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
        &self.device.virtio
    }
}
impl BorrowMut<VirtioConfig<Queue>> for Net {
    fn borrow_mut(&mut self) -> &mut VirtioConfig<Queue> {
        &mut self.device.virtio
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
            irqfd: self.device.irqfd.clone(),
            interrupt_status: self.device.virtio.interrupt_status.clone(),
        };

        let mut ioevents = self.device.prepare_activate()?;

        let rxq = self.device.virtio.queues.remove(0);
        let txq = self.device.virtio.queues.remove(0);

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
