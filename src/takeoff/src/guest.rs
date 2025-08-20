use std::{
    ffi::c_void,
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
};

use anyhow::Result;
use nix::{
    fcntl::{OFlag, open},
    sys::{
        mman::{MapFlags, ProtFlags, mmap},
        stat::Mode,
    },
};

const PAGE_SIZE: usize = 4096;
const MAGIC_MMIO_ADDR: i64 = 0xd0000000;

pub struct GuestManager {
    map_base: ::core::ptr::NonNull<c_void>,
}
unsafe impl Send for GuestManager {}
unsafe impl Sync for GuestManager {}

impl GuestManager {
    pub fn new() -> Result<Self> {
        let fd = open(
            "/dev/mem",
            OFlag::O_RDWR | OFlag::O_SYNC | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;

        let fd = unsafe { File::from_raw_fd(fd.as_raw_fd()) };

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

    pub fn mark_user_space_ready(&self) {
        unsafe {
            let ptr = self.map_base.as_ptr() as *mut u64;
            ptr.write_volatile(0x00_00_00_00_00_00_00_03);
        }
    }

    pub fn set_exit_code(&self, code: i32) {
        unsafe {
            let ptr = self.map_base.as_ptr() as *mut u64;
            // first 4 bytes are the trigger code
            // last byte is 0x04

            let mut cmd = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04];
            cmd[..4].copy_from_slice(&code.to_le_bytes());

            let cmd_u64 = u64::from_be_bytes(cmd);

            ptr.write_volatile(cmd_u64);
        }
    }

    #[allow(dead_code)]
    pub fn trigger_manual_snapshot(&self) {
        unsafe {
            let ptr: *mut u64 = self.map_base.as_ptr() as *mut u64;
            ptr.write_volatile(0x00_00_00_00_00_00_00_0a);
        }
    }

    #[allow(dead_code)]
    pub fn get_last_boot_time_us(&self) -> u64 {
        unsafe {
            let ptr = self.map_base.as_ptr() as *mut u64;
            let time_us = ptr.read_volatile();

            time_us
        }
    }

    #[allow(dead_code)]
    pub fn get_first_boot_time_us(&self) -> u64 {
        unsafe {
            let ptr = self.map_base.as_ptr().add(8) as *mut u64;
            let time_us = ptr.read_volatile();

            time_us
        }
    }
}
