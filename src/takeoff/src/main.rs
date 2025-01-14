use std::{ffi::c_void, fs, os::fd::FromRawFd};

use nix::{
    fcntl::{open, OFlag},
    sys::{
        mman::{mmap, MapFlags, ProtFlags},
        stat::Mode,
    },
    unistd::write,
};
use util::{
    async_runtime::{runtime, time},
    result::Result,
};

const PAGE_SIZE: usize = 4096;
const MAGIC_MMIO_ADDR: i64 = 0xd0000000;

fn test_kernel_mmio_write() -> Result<()> {
    let fd = unsafe { fs::File::from_raw_fd(100) };
    write(fd, &[123u8])?;
    Ok(())
}

struct GuestManager {
    map_base: ::core::ptr::NonNull<c_void>,
}

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
}

async fn takeoff() {
    let guest_manager = GuestManager::new().unwrap();
    guest_manager.mark_boot_ready();

    for i in 0..5 {
        println!("{}...", 5 - i);
        time::sleep(time::Duration::from_secs(1)).await;
    }

    println!("takeoff");

    guest_manager.trigger_snapshot(); // here the guest is suspended. will comtinue from here.

    guest_manager.mark_boot_ready();
    println!("guess who's back");

    test_kernel_mmio_write().unwrap(); // here the guest is suspended by the kernel. will comtinue from here.

    guest_manager.mark_boot_ready();
    println!("back again");
}

fn main() -> Result<()> {
    runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(takeoff());

    Ok(())
}
