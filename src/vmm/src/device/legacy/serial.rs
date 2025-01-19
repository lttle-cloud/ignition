use std::io::Write;

use util::result::Error;
use vm_device::{
    bus::{PioAddress, PioAddressOffset},
    MutDevicePio,
};
use vm_superio::{
    serial::{NoEvents, SerialEvents},
    Serial, Trigger,
};

pub struct SerialWrapper<T: Trigger, EV: SerialEvents, W: Write>(pub Serial<T, EV, W>);

impl<T: Trigger<E = Error>, W: Write> MutDevicePio for SerialWrapper<T, NoEvents, W> {
    fn pio_read(&mut self, _base: PioAddress, offset: PioAddressOffset, data: &mut [u8]) {
        if data.len() != 1 {
            eprintln!("Serial console invalid data length on read: {}", data.len());
            return;
        }

        let Ok(offset) = offset.try_into() else {
            eprintln!("Invalid serial console read offset.");
            return;
        };

        let res = self.0.read(offset);
        data[0] = res;
    }

    fn pio_write(&mut self, _base: PioAddress, offset: PioAddressOffset, data: &[u8]) {
        if data.len() != 1 {
            eprintln!(
                "Serial console invalid data length on write: {}",
                data.len()
            );
            return;
        }

        let Ok(offset) = offset.try_into() else {
            eprintln!("Invalid serial console write offset.");
            return;
        };

        let res = self.0.write(offset, data[0]);
        if res.is_err() {
            eprintln!("Error writing to serial console: {:#?}", res.unwrap_err());
        }
    }
}
