use std::{arch::asm, io::Write};

use nix::libc::ioperm;
use tracing_subscriber::fmt::MakeWriter;

pub struct SerialWriter;

impl SerialWriter {
    pub fn initialize_serial() {
        unsafe {
            let result = ioperm(0x3f8, 8, 1);
            if result != 0 {
                panic!("ioperm failed");
            }
        }
    }
}

impl Write for SerialWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        fn write_byte(b: u8) {
            unsafe {
                asm!("out dx, al", in("dx") 0x3f8, in("al") b);
            }
        }

        for c in buf {
            write_byte(*c);
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SerialWriter {
    type Writer = SerialWriter;

    fn make_writer(&self) -> Self::Writer {
        SerialWriter
    }
}
