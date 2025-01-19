pub mod legacy;
pub mod meta;
pub mod virtio;

use std::sync::{Arc, Mutex};
use util::result::{anyhow, bail, Result};
use vm_device::device_manager::IoManager;

use crate::constants::MAX_IRQ;

#[derive(Clone)]
pub struct SharedDeviceManager {
    pub io_manager: Arc<Mutex<IoManager>>,
    irq_allocator: IrqAllocator,
}

impl SharedDeviceManager {
    pub fn new(last_irq: u32) -> Result<Self> {
        let io_manager = Arc::new(Mutex::new(IoManager::new()));
        let irq_allocator = IrqAllocator::new(last_irq)?;

        Ok(SharedDeviceManager {
            io_manager,
            irq_allocator,
        })
    }

    pub fn next_irq(&self) -> Result<u32> {
        return self.irq_allocator.next_irq();
    }

    pub fn reset_irq(&self) {
        self.irq_allocator.reset();
    }
}

#[derive(Clone)]
pub struct IrqAllocator {
    initial_irq: u32,
    last_irq: Arc<Mutex<u32>>,
}

impl IrqAllocator {
    fn new(last_irq: u32) -> Result<Self> {
        if last_irq >= MAX_IRQ {
            bail!("No more IRQs are available");
        }

        Ok(IrqAllocator {
            initial_irq: last_irq,
            last_irq: Arc::new(Mutex::new(last_irq)),
        })
    }

    fn next_irq(&self) -> Result<u32> {
        let mut last_irq = self
            .last_irq
            .lock()
            .map_err(|_| anyhow!("Failed to lock last IRQ"))?;

        if *last_irq == MAX_IRQ {
            bail!("No more IRQs are available");
        }

        *last_irq += 1;
        Ok(*last_irq)
    }

    fn reset(&self) {
        let mut last_irq = self.last_irq.lock().expect("Failed to lock last IRQ");

        *last_irq = self.initial_irq;
    }
}
