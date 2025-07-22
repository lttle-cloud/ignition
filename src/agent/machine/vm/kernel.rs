use std::{
    io::{Seek, SeekFrom},
    path::Path,
};

use anyhow::{Result, anyhow, bail};
use linux_loader::{
    configurator::{BootConfigurator, BootParams, linux::LinuxBootConfigurator},
    loader::{Cmdline, KernelLoader, KernelLoaderResult, bootparam},
};
use vm_memory::{
    Address, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion, ReadVolatile,
};

use crate::agent::machine::{
    machine::MachineConfig,
    vm::constants::{
        CMDLINE_CAPACITY, CMDLINE_START, E820_RAM, EBDA_START, HIGH_RAM_START,
        KERNEL_BOOT_FLAG_MAGIC, KERNEL_HDR_MAGIC, KERNEL_LOADER_OTHER, KERNEL_MIN_ALIGNMENT_BYTES,
        PAGE_SIZE, ZERO_PAGE_START,
    },
};

pub async fn load_kernel(
    machine_config: &MachineConfig,
    memory: &GuestMemoryMmap,
    kernel_path: impl AsRef<Path>,
    initrd_path: impl AsRef<Path>,
    kernel_cmd: &Cmdline,
) -> Result<KernelLoaderResult> {
    let kernel_path = kernel_path.as_ref();

    let mut kernel_image = std::fs::File::open(kernel_path)?;
    let kernel_load = linux_loader::loader::Elf::load(
        memory,
        None,
        &mut kernel_image,
        Some(GuestAddress(HIGH_RAM_START)),
    )?;

    let mut boot_params = bootparam::boot_params::default();
    boot_params.hdr.boot_flag = KERNEL_BOOT_FLAG_MAGIC;
    boot_params.hdr.header = KERNEL_HDR_MAGIC;
    boot_params.hdr.kernel_alignment = KERNEL_MIN_ALIGNMENT_BYTES;
    boot_params.hdr.type_of_loader = KERNEL_LOADER_OTHER;

    // EBDA
    boot_params.e820_table[boot_params.e820_entries as usize].addr = 0;
    boot_params.e820_table[boot_params.e820_entries as usize].size = EBDA_START;
    boot_params.e820_table[boot_params.e820_entries as usize].type_ = E820_RAM;
    boot_params.e820_entries += 1;

    // Memory
    boot_params.e820_table[boot_params.e820_entries as usize].addr = HIGH_RAM_START;
    boot_params.e820_table[boot_params.e820_entries as usize].size = memory
        .last_addr()
        .unchecked_offset_from(GuestAddress(HIGH_RAM_START));

    boot_params.e820_table[boot_params.e820_entries as usize].type_ = E820_RAM;
    boot_params.e820_entries += 1;

    let (initrd_addr, initrd_size) = load_initrd(initrd_path, memory)?;

    boot_params.hdr.ramdisk_image = initrd_addr.raw_value() as u32;
    boot_params.hdr.ramdisk_size = initrd_size as u32;

    boot_params.hdr.cmd_line_ptr = CMDLINE_START as u32;
    boot_params.hdr.cmdline_size = kernel_cmd.as_cstring()?.as_bytes().len() as u32;

    linux_loader::loader::load_cmdline(memory, GuestAddress(CMDLINE_START), kernel_cmd)?;

    LinuxBootConfigurator::write_bootparams::<GuestMemoryMmap>(
        &BootParams::new::<bootparam::boot_params>(&boot_params, GuestAddress(ZERO_PAGE_START)),
        memory,
    )?;

    Ok(kernel_load)
}

pub fn create_cmdline(_machine_config: &MachineConfig) -> Result<Cmdline> {
    let cmdline = Cmdline::new(CMDLINE_CAPACITY)?;
    Ok(cmdline)
}

fn load_initrd(
    initrd_path: impl AsRef<Path>,
    memory: &GuestMemoryMmap,
) -> Result<(GuestAddress, usize)> {
    let initrd_path = initrd_path.as_ref();
    let mut initrd_image = std::fs::File::open(&initrd_path)?;

    let size = match initrd_image.seek(SeekFrom::End(0)) {
        Err(err) => bail!("Initrd image seek failed: {}", err),
        Ok(0) => {
            bail!("Initrd image is empty");
        }
        Ok(s) => s as usize,
    };

    initrd_image.seek(SeekFrom::Start(0))?;

    let first_region = memory.find_region(GuestAddress(0)).ok_or(anyhow!(
        "Failed to find a suitable region for the initrd image"
    ))?;

    let first_region_size = first_region.len() as usize;

    if first_region_size < size {
        bail!("First memory region is too small for the initrd image");
    }

    let address = first_region_size - size;
    let aligned_address = (address & !(PAGE_SIZE - 1)) as u64;

    let mut mem = memory.get_slice(GuestAddress(aligned_address), size)?;

    initrd_image.read_exact_volatile(&mut mem)?;

    Ok((GuestAddress(aligned_address), size))
}
