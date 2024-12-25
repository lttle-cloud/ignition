fn takeoff() -> ! {
    println!("takeoff");

    // todo: lets see if we can intercept this write from the kernel
    let dummy = "dummy";
    unsafe {
        libc::write(100, dummy.as_ptr() as *const libc::c_void, dummy.len());
    };

    loop {}
}

fn main() -> ! {
    takeoff();
}
