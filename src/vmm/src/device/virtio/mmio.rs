use vm_device::bus::{BusRange, MmioAddress};

#[derive(Debug, Clone)]
pub struct MmioConfig {
    pub range: BusRange<MmioAddress>,
    pub irq: u32,
}
