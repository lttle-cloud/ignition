use std::sync::{Mutex, MutexGuard};

use util::result::Result;
use vm_allocator::AddressAllocator;
use vm_memory::{FileOffset, GuestAddress, GuestMemoryMmap};

use crate::{
    config::MemoryConfig,
    constants::{MMIO_LEN, MMIO_SIZE, MMIO_START},
};

pub struct Memory {
    guest_memory: GuestMemoryMmap,
    mmio_allocator: Mutex<AddressAllocator>,
}

impl Memory {
    pub fn new(config: MemoryConfig) -> Result<Self> {
        let guest_memory = Memory::create_guest_memory(&config)?;
        let mmio_allocator = Memory::create_mmio_allocator()?;

        Ok(Memory {
            guest_memory,
            mmio_allocator: Mutex::new(mmio_allocator),
        })
    }

    pub fn new_backed_by_file(config: MemoryConfig, file: std::fs::File) -> Result<Self> {
        let guest_memory = Memory::create_guest_memory_backed_by_file(&config, file)?;
        let mmio_allocator = Memory::create_mmio_allocator()?;

        Ok(Memory {
            guest_memory,
            mmio_allocator: Mutex::new(mmio_allocator),
        })
    }

    fn create_guest_memory(config: &MemoryConfig) -> Result<GuestMemoryMmap> {
        let mem_size = config.size_mib << 20;

        let memory = GuestMemoryMmap::from_ranges(&[(GuestAddress(0), mem_size)])?;

        Ok(memory)
    }

    fn create_guest_memory_backed_by_file(
        config: &MemoryConfig,
        file: std::fs::File,
    ) -> Result<GuestMemoryMmap> {
        let mem_size = config.size_mib << 20;

        let memory = GuestMemoryMmap::from_ranges_with_files(&[(
            GuestAddress(0),
            mem_size,
            Some(FileOffset::new(file, 0)),
        )])?;

        Ok(memory)
    }

    fn create_mmio_allocator() -> Result<AddressAllocator> {
        // We reserve the first MMIO_SIZE bytes for the GuestManager meta device;
        let allocable_memory_start = MMIO_START + MMIO_LEN;
        let alloc = AddressAllocator::new(allocable_memory_start, MMIO_SIZE)?;

        Ok(alloc)
    }

    pub fn guest_memory(&self) -> &GuestMemoryMmap {
        &self.guest_memory
    }

    pub fn lock_mmio_allocator(&self) -> MutexGuard<AddressAllocator> {
        self.mmio_allocator.lock().unwrap()
    }

    pub fn reset_mmio_allocator(&self) -> Result<()> {
        let mut alloc = self.mmio_allocator.lock().unwrap();
        *alloc = Memory::create_mmio_allocator()?;
        Ok(())
    }
}
