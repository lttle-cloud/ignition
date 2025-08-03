use std::io::Write;

use anyhow::Error;
use tracing::warn;
use vm_device::{
    MutDevicePio,
    bus::{PioAddress, PioAddressOffset},
};
use vm_superio::{
    Serial, Trigger,
    serial::{NoEvents, SerialEvents},
};

pub struct SerialWrapper<T: Trigger, EV: SerialEvents, W: Write>(pub Serial<T, EV, W>);

impl<T: Trigger<E = Error>, W: Write> MutDevicePio for SerialWrapper<T, NoEvents, W> {
    fn pio_read(&mut self, _base: PioAddress, offset: PioAddressOffset, data: &mut [u8]) {
        if data.len() != 1 {
            warn!("Serial console invalid data length on read: {}", data.len());
            return;
        }

        let Ok(offset) = offset.try_into() else {
            warn!("Invalid serial console read offset.");
            return;
        };

        let res = self.0.read(offset);
        data[0] = res;
    }

    fn pio_write(&mut self, _base: PioAddress, offset: PioAddressOffset, data: &[u8]) {
        if data.len() != 1 {
            warn!(
                "Serial console invalid data length on write: {}",
                data.len()
            );
            return;
        }

        let Ok(offset) = offset.try_into() else {
            warn!("Invalid serial console write offset.");
            return;
        };

        let res = self.0.write(offset, data[0]);
        if res.is_err() {
            warn!("Error writing to serial console: {:#?}", res.unwrap_err());
        }
    }
}
