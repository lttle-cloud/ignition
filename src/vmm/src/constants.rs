use crate::cpu_ref;

pub const MMIO_END: u64 = 1 << 32;
pub const MMIO_SIZE: u64 = 768 << 20; // 768 mib
pub const MMIO_START: u64 = MMIO_END - MMIO_SIZE;
pub const MMIO_LEN: u64 = 0x1000; // 4096 bytes (1 page)

pub const ZERO_PAGE_START: u64 = 0x7000;
pub const CMDLINE_START: u64 = 0x0002_0000;
pub const HIGH_RAM_START: u64 = 0x0010_0000;

pub const BOOT_STACK_POINTER: u64 = 0x8ff0;
pub const ZEROPG_START: u64 = 0x7000;
pub const PAGE_SIZE: usize = 4096;

pub const CMDLINE_CAPACITY: usize = 4096;

pub const X86_CR0_PE: u64 = 0x1;
pub const X86_CR0_PG: u64 = 0x8000_0000;
pub const X86_CR4_PAE: u64 = 0x20;

pub const PML4_START: u64 = 0x9000;
pub const PDPTE_START: u64 = 0xa000;
pub const PDE_START: u64 = 0xb000;

pub const KERNEL_BOOT_FLAG_MAGIC: u16 = 0xaa55;
pub const KERNEL_HDR_MAGIC: u32 = 0x5372_6448;
pub const KERNEL_LOADER_OTHER: u8 = 0xff;
pub const KERNEL_MIN_ALIGNMENT_BYTES: u32 = 0x0100_0000;

pub const EBDA_START: u64 = 0x0009_fc00;
pub const E820_RAM: u32 = 1;

pub const MAX_IRQ: u32 = cpu_ref::mptable::IRQ_MAX as u32;

pub const SERIAL_IRQ: u32 = 4;
