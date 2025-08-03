use anyhow::{Result, bail};

use crate::agent::machine::vm::constants::MAX_IRQ;

#[derive(Clone)]
pub struct IrqAllocator {
    initial_irq: u32,
    last_irq: u32,
}

impl IrqAllocator {
    pub fn new(last_irq: u32) -> Result<Self> {
        if last_irq >= MAX_IRQ {
            bail!("No more IRQs are available");
        }

        Ok(IrqAllocator {
            initial_irq: last_irq,
            last_irq,
        })
    }

    pub fn next_irq(&mut self) -> Result<u32> {
        if self.last_irq == MAX_IRQ {
            bail!("No more IRQs are available");
        }

        self.last_irq += 1;
        Ok(self.last_irq)
    }

    pub fn reset(&mut self) {
        self.last_irq = self.initial_irq;
    }
}
