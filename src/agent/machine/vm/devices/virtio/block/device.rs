use std::{
    borrow::{Borrow, BorrowMut},
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use virtio_blk::{defs::SECTOR_SHIFT, stdio_executor::StdIoBackend};
use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioDeviceType, VirtioMmioDevice};
use virtio_queue::{Queue, QueueT};
use vm_device::{MutDeviceMmio, bus::MmioAddress, device_manager::IoManager};

use crate::agent::machine::{
    machine::VolumeMountConfig,
    vm::devices::virtio::{
        Env, SingleFdSignalQueue,
        block::overlay_backend::OverlayBackend,
        features::{VIRTIO_F_IN_ORDER, VIRTIO_F_RING_EVENT_IDX, VIRTIO_F_VERSION_1},
        mmio::VirtioMmioDeviceConfig,
    },
};

use super::handler::{BlockHandler, QueueHandler};

pub const BLOCK_DEVICE_ID: u32 = 2;

pub const VIRTIO_BLK_F_RO: u64 = 5;
pub const VIRTIO_BLK_F_FLUSH: u64 = 9;

const QUEUE_MAX_SIZE: u16 = 256;

#[repr(C, packed)]
#[derive(Debug, Default, Copy, Clone)]
struct VirtioBlockConfig {
    sectors: [u8; 8],
}

impl VirtioBlockConfig {
    pub fn new(file_path: &PathBuf) -> Result<Self> {
        let file_size = File::open(file_path)?.seek(SeekFrom::End(0))?;

        let sectors = file_size >> SECTOR_SHIFT;

        Ok(Self {
            sectors: sectors.to_le_bytes(),
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                (self as *const Self) as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

pub struct Block {
    device: VirtioMmioDeviceConfig,
    config: VolumeMountConfig,
    handler: Option<Arc<Mutex<QueueHandler>>>,
}

impl Block {
    pub fn new(
        env: &mut Env,
        io_manager: &mut IoManager,
        config: VolumeMountConfig,
    ) -> Result<Arc<Mutex<Self>>> {
        let mut device_features: u64 = 1 << VIRTIO_F_VERSION_1
            | 1 << VIRTIO_F_IN_ORDER
            | 1 << VIRTIO_F_RING_EVENT_IDX
            | 1 << VIRTIO_BLK_F_FLUSH;

        if config.read_only {
            device_features |= 1 << VIRTIO_BLK_F_RO;
        }

        let queues = vec![Queue::new(QUEUE_MAX_SIZE)?];
        let cfg = VirtioBlockConfig::new(&config.volume.path.clone().into())?;

        let virtio_config = VirtioConfig::new(device_features, queues, cfg.as_bytes().to_vec());

        let device = VirtioMmioDeviceConfig::new(virtio_config, &env)?;

        let block = Block {
            device,
            config,
            handler: None,
        };
        let block = Arc::new(Mutex::new(block));

        env.register_mmio_device(io_manager, block.clone())?;

        Ok(block)
    }
}

impl VirtioDeviceType for Block {
    fn device_type(&self) -> u32 {
        BLOCK_DEVICE_ID
    }
}

impl Borrow<VirtioConfig<Queue>> for Block {
    fn borrow(&self) -> &VirtioConfig<Queue> {
        &self.device.virtio
    }
}
impl BorrowMut<VirtioConfig<Queue>> for Block {
    fn borrow_mut(&mut self) -> &mut VirtioConfig<Queue> {
        &mut self.device.virtio
    }
}

impl VirtioDeviceActions for Block {
    type E = anyhow::Error;

    fn activate(&mut self) -> Result<()> {
        let mut features = self.device.virtio.driver_features;
        if self.config.read_only {
            features |= 1 << VIRTIO_BLK_F_RO;
        }

        let backend = if self.config.read_only {
            let src_file = OpenOptions::new()
                .read(true)
                .open(&self.config.volume.path)?;

            OverlayBackend::new_readonly(src_file)
        } else {
            let src_file = OpenOptions::new()
                .read(true)
                .open(&self.config.volume.path)?;

            let ov_file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.config.volume.ov_path)?;

            OverlayBackend::new_readwrite(src_file, ov_file)?
        };
        let disk =
            StdIoBackend::new(backend, features).map_err(|_| anyhow!("failed to create disk"))?;

        let driver_notify = SingleFdSignalQueue {
            irqfd: self.device.irqfd.clone(),
            interrupt_status: self.device.virtio.interrupt_status.clone(),
        };

        let mut ioevents = self.device.prepare_activate()?;

        let handler = BlockHandler {
            driver_notify,
            queue: self.device.virtio.queues.remove(0),
            memory: self.device.memory.clone(),
            disk,
        };

        let handler = Arc::new(Mutex::new(QueueHandler {
            inner: handler,
            ioeventfd: ioevents.remove(0),
        }));

        self.handler = Some(handler.clone());

        self.device.finalize_activate(handler)?;

        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        Ok(())
    }
}

impl VirtioMmioDevice for Block {}

impl MutDeviceMmio for Block {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        self.read(offset, data);
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data);
    }
}
