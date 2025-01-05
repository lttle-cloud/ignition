use std::{fs, os::fd::FromRawFd};

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

fn test_user_mmio_wirte() -> Result<()> {
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

    unsafe {
        let ptr = map_base.as_ptr() as *mut u8;
        *ptr = 123u8;
    }

    Ok(())
}

async fn takeoff() {
    for i in 0..5 {
        println!("{}...", 5 - i);
        time::sleep(time::Duration::from_secs(1)).await;
    }

    println!("takeoff");

    time::sleep(time::Duration::from_secs(1)).await;
    test_kernel_mmio_write().unwrap();

    time::sleep(time::Duration::from_secs(1)).await;
    test_user_mmio_wirte().unwrap();

    time::sleep(time::Duration::from_secs(5)).await;
}

fn main() -> Result<()> {
    runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(takeoff());

    Ok(())
}
