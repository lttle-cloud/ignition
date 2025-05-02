use std::{ffi::c_void, os::fd::FromRawFd};

use nix::{
    fcntl::{open, OFlag},
    sys::{
        mman::{mmap, MapFlags, ProtFlags},
        stat::Mode,
    },
};
use util::{async_runtime::fs, result::Result};

const PAGE_SIZE: usize = 4096;
const MAGIC_MMIO_ADDR: i64 = 0xd0000000;

pub struct GuestManager {
    map_base: ::core::ptr::NonNull<c_void>,
}
unsafe impl Send for GuestManager {}
unsafe impl Sync for GuestManager {}

impl GuestManager {
    pub fn new() -> Result<GuestManager> {
        let fd = open(
            "/dev/mem",
            OFlag::O_RDWR | OFlag::O_SYNC | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;

        let fd = unsafe { fs::File::from_raw_fd(fd) };

        let map_base = unsafe {
            mmap(
                None,
                PAGE_SIZE.try_into()?,
                ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED,
                fd,
                MAGIC_MMIO_ADDR,
            )?
        };

        Ok(GuestManager { map_base })
    }

    #[allow(unused)]
    pub fn trigger_snapshot(&self) {
        unsafe {
            let ptr: *mut u64 = self.map_base.as_ptr() as *mut u64;
            ptr.write_volatile(0x00_00_00_00_00_00_00_01);
        }
    }

    pub fn mark_boot_ready(&self) {
        unsafe {
            let ptr = self.map_base.as_ptr() as *mut u64;
            ptr.write_volatile(0x00_00_00_00_00_00_00_0a);
        }
    }

    pub fn get_boot_ready_time_us(&self) -> u64 {
        unsafe {
            let ptr = self.map_base.as_ptr() as *mut u64;
            let time_us = ptr.read_volatile();

            time_us
        }
    }
}
