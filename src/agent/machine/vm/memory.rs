use std::{
    fs::{File, OpenOptions, create_dir_all},
    path::Path,
};

use anyhow::{Ok, Result};
use vm_allocator::AddressAllocator;
use vm_memory::{FileOffset, GuestAddress, GuestMemoryMmap};

use crate::agent::machine::{
    machine::{MachineConfig, MachineStateRetentionMode},
    vm::constants::{MMIO_LEN, MMIO_SIZE, MMIO_START},
};

pub async fn create_memory(machine_config: &MachineConfig) -> Result<GuestMemoryMmap> {
    let mem_size = machine_config.resources.memory << 20; // Mb to bytes

    let guest_memory: GuestMemoryMmap = match &machine_config.state_retention_mode {
        MachineStateRetentionMode::InMemory => {
            GuestMemoryMmap::from_ranges(&[(GuestAddress(0), mem_size as usize)])?
        }
        MachineStateRetentionMode::OnDisk { path } => {
            let dir = Path::new(path);
            if !dir.exists() {
                create_dir_all(dir)?;
            }

            let mem_file = dir.join("memory.bin");
            let mem_file = open_memory_file(mem_file, mem_size)?;

            GuestMemoryMmap::from_ranges_with_files(&[(
                GuestAddress(0),
                mem_size as usize,
                Some(FileOffset::new(mem_file, 0)),
            )])?
        }
    };

    Ok(guest_memory)
}

fn open_memory_file(path: impl AsRef<Path>, mem_size: u64) -> Result<File> {
    let path = path.as_ref();

    let mem_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)?;

    mem_file.set_len(mem_size)?;

    Ok(mem_file)
}

pub fn create_mmio_allocator() -> Result<AddressAllocator> {
    // We reserve the first MMIO_SIZE bytes for the GuestManager meta device;
    let allocable_memory_start = MMIO_START + MMIO_LEN;
    let alloc = AddressAllocator::new(allocable_memory_start, MMIO_SIZE)?;

    Ok(alloc)
}
