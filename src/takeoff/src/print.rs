use std::arch::asm;

use nix::libc::ioperm;

pub fn _print(input: impl AsRef<str>) {
    fn print_char(c: char) {
        unsafe {
            asm!("out dx, al", in("dx") 0x3f8, in("al") c as u8);
        }
    }

    for c in input.as_ref().chars() {
        print_char(c);
    }
}

pub fn init_print() {
    unsafe {
        let result = ioperm(0x3f8, 8, 1);
        if result != 0 {
            panic!("ioperm failed");
        }
    }
}

#[macro_export]
macro_rules! rprint {
    ($($arg:tt)*) => ($crate::print::_print(&format!($($arg)*)));
}

#[macro_export]
macro_rules! rprintln {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::rprint!($($arg)*); $crate::rprint!("\n"));
}
