use std::convert::TryInto;
use util::tracing::warn;
use vm_device::bus::{PioAddress, PioAddressOffset};
use vm_device::MutDevicePio;
use vm_superio::I8042Device;

use super::trigger::EventFdTrigger;

pub struct I8042Wrapper(pub I8042Device<EventFdTrigger>);

impl MutDevicePio for I8042Wrapper {
    fn pio_read(&mut self, _base: PioAddress, offset: PioAddressOffset, data: &mut [u8]) {
        if data.len() != 1 {
            warn!("Invalid I8042 data length on read: {}", data.len());
            return;
        }
        match offset.try_into() {
            Ok(offset) => {
                self.0.read(offset);
            }
            Err(_) => warn!("Invalid I8042 read offset."),
        }
    }

    fn pio_write(&mut self, _base: PioAddress, offset: PioAddressOffset, data: &[u8]) {
        if data.len() != 1 {
            warn!("Invalid I8042 data length on write: {}", data.len());
            return;
        }
        match offset.try_into() {
            Ok(offset) => {
                if self.0.write(offset, data[0]).is_err() {
                    warn!("Failed to write to I8042.");
                }
            }
            Err(_) => warn!("Invalid I8042 write offset"),
        }
    }
}
