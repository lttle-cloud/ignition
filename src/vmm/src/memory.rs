use util::result::Result;
use vm_allocator::AddressAllocator;
use vm_memory::{GuestAddress, GuestMemoryMmap};

use crate::{
    config::MemoryConfig,
    constants::{MMIO_SIZE, MMIO_START},
};

pub struct Memory {
    config: MemoryConfig,
    guest_memory: GuestMemoryMmap,
    mmio_allocator: AddressAllocator,
}

impl Memory {
    pub fn new(config: MemoryConfig) -> Result<Self> {
        let guest_memory = Memory::create_guest_memory(&config)?;
        let mmio_allocator = Memory::create_mmio_allocator()?;

        Ok(Memory {
            config,
            guest_memory,
            mmio_allocator,
        })
    }

    fn create_guest_memory(config: &MemoryConfig) -> Result<GuestMemoryMmap> {
        let mem_size = config.size_mib << 20;

        let memory = GuestMemoryMmap::from_ranges(&[(GuestAddress(0), mem_size)])?;

        Ok(memory)
    }

    fn create_mmio_allocator() -> Result<AddressAllocator> {
        let alloc = AddressAllocator::new(MMIO_START, MMIO_SIZE)?;

        Ok(alloc)
    }

    pub fn guest_memory(&self) -> &GuestMemoryMmap {
        &self.guest_memory
    }

    pub fn mmio_allocator(&self) -> &AddressAllocator {
        &self.mmio_allocator
    }
}
