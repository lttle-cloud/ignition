use std::sync::Arc;

use vm_device::MutDeviceMmio;

use crate::{memory::Memory, vmm::SharedExitEventHandler};

#[derive(Debug, Clone, Copy)]
enum TriggerCode {
    Write,
    Listen,
}

impl TriggerCode {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(TriggerCode::Write),
            2 => Some(TriggerCode::Listen),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TriggerData {
    code: TriggerCode,
}

impl TriggerData {
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 8 {
            return None;
        }

        let code = TriggerCode::from_byte(bytes[0])?;

        Some(TriggerData { code })
    }
}

pub struct GuestManagerDevice {
    memory: Arc<Memory>,
    exit_handler: SharedExitEventHandler,
}

impl GuestManagerDevice {
    pub fn new(memory: Arc<Memory>, exit_handler: SharedExitEventHandler) -> Self {
        Self {
            memory,
            exit_handler,
        }
    }
}

impl MutDeviceMmio for GuestManagerDevice {
    fn mmio_read(
        &mut self,
        _base: vm_device::bus::MmioAddress,
        offset: vm_device::bus::MmioAddressOffset,
        data: &mut [u8],
    ) {
        println!(
            "GuestManagerDevice::mmio_read offset: {}, size: {:?}",
            offset,
            data.len()
        );
    }

    fn mmio_write(
        &mut self,
        _base: vm_device::bus::MmioAddress,
        offset: vm_device::bus::MmioAddressOffset,
        data: &[u8],
    ) {
        println!(
            "GuestManagerDevice::mmio_write offset: {}, data: {:?}",
            offset, data
        );

        let Some(trigger_data) = TriggerData::from_bytes(data) else {
            println!("Failed to parse trigger data");
            return;
        };

        println!("Trigger data: {:?}", trigger_data);

        if matches!(trigger_data.code, TriggerCode::Write) {
            println!("Trigger exit");
            self.exit_handler.trigger_exit().unwrap();
        }
    }
}
