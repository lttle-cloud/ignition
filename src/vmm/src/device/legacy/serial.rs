use std::io::{stdin, Read, Write};

use event_manager::{EventOps, EventSet, Events, MutEventSubscriber};
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

impl<T: Trigger, W: Write> MutEventSubscriber for SerialWrapper<T, NoEvents, W> {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        // Respond to stdin events.
        // `EventSet::IN` => send what's coming from stdin to the guest.
        // `EventSet::HANG_UP` or `EventSet::ERROR` => deregister the serial input.
        let mut out = [0u8; 32];
        match stdin().read(&mut out) {
            Err(e) => {
                eprintln!("Error while reading stdin: {:?}", e);
            }
            Ok(count) => {
                let event_set = events.event_set();
                let unregister_condition =
                    event_set.contains(EventSet::ERROR) | event_set.contains(EventSet::HANG_UP);
                if count > 0 {
                    if self.0.enqueue_raw_bytes(&out[..count]).is_err() {
                        eprintln!("Failed to send bytes to the guest via serial input");
                    }
                } else if unregister_condition {
                    // Got 0 bytes from serial input; is it a hang-up or error?
                    ops.remove(events)
                        .expect("Failed to unregister serial input");
                }
            }
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        // Hook to stdin events.
        ops.add(Events::new(&stdin(), EventSet::IN))
            .expect("Failed to register serial input event");
    }
}

impl<T: Trigger<E = Error>, W: Write> SerialWrapper<T, NoEvents, W> {
    fn bus_read(&mut self, offset: u8, data: &mut [u8]) {
        if data.len() != 1 {
            eprintln!("Serial console invalid data length on read: {}", data.len());
            return;
        }

        // This is safe because we checked that `data` has length 1.
        data[0] = self.0.read(offset);
    }

    fn bus_write(&mut self, offset: u8, data: &[u8]) {
        if data.len() != 1 {
            eprintln!(
                "Serial console invalid data length on write: {}",
                data.len()
            );
            return;
        }

        // This is safe because we checked that `data` has length 1.
        let res = self.0.write(offset, data[0]);
        if res.is_err() {
            eprintln!("Error writing to serial console: {:#?}", res.unwrap_err());
        }
    }
}

impl<T: Trigger<E = Error>, W: Write> MutDevicePio for SerialWrapper<T, NoEvents, W> {
    fn pio_read(&mut self, _base: PioAddress, offset: PioAddressOffset, data: &mut [u8]) {
        // TODO: this function can't return an Err, so we'll mark error conditions
        // (data being more than 1 byte, offset overflowing an u8) with logs & metrics.

        match offset.try_into() {
            Ok(offset) => self.bus_read(offset, data),
            Err(_) => eprintln!("Invalid serial console read offset."),
        }
    }

    fn pio_write(&mut self, _base: PioAddress, offset: PioAddressOffset, data: &[u8]) {
        // TODO: this function can't return an Err, so we'll mark error conditions
        // (data being more than 1 byte, offset overflowing an u8) with logs & metrics.
        match offset.try_into() {
            Ok(offset) => self.bus_write(offset, data),
            Err(_) => eprintln!("Invalid serial console write offset."),
        }
    }
}
