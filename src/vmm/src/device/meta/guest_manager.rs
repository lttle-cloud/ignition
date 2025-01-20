use tracing::{info, warn};

use crate::{
    constants::{MMIO_LEN, MMIO_START},
    vmm::{ExitHandlerReason, SharedExitEventHandler},
};

pub const GUEST_MANAGER_MMIO_START: u64 = MMIO_START;
pub const GUEST_MANAGER_MMIO_SIZE: u64 = MMIO_LEN;
pub const GUEST_MANAGER_MMIO_END: u64 = GUEST_MANAGER_MMIO_START + GUEST_MANAGER_MMIO_SIZE;

const TRIGGER_AFTER_OFFSET: u8 = 127;

const TRIGGER_SYS_LISTEN: u8 = 1;
const TRIGGER_SYS_SOCK: u8 = 2;
const TRIGGER_SYS_BIND: u8 = 3;

const TRIGGER_SYS_LISTEN_AFTER: u8 = TRIGGER_AFTER_OFFSET + TRIGGER_SYS_LISTEN;
const TRIGGER_SYS_SOCK_AFTER: u8 = TRIGGER_AFTER_OFFSET + TRIGGER_SYS_SOCK;
const TRIGGER_SYS_BIND_AFTER: u8 = TRIGGER_AFTER_OFFSET + TRIGGER_SYS_BIND;

const TRIGGER_BOOT_READY: u8 = 0xa;

#[derive(Debug, Clone, Copy)]
enum TriggerCode {
    BeforeListen,
    BeforeSock,
    BeforeBind,
    AfterListen,
    AfterSock,
    AfterBind,
    BootReadyMarker,
}

impl TriggerCode {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            TRIGGER_SYS_LISTEN => Some(TriggerCode::BeforeListen),
            TRIGGER_SYS_SOCK => Some(TriggerCode::BeforeSock),
            TRIGGER_SYS_BIND => Some(TriggerCode::BeforeBind),
            TRIGGER_SYS_LISTEN_AFTER => Some(TriggerCode::AfterListen),
            TRIGGER_SYS_SOCK_AFTER => Some(TriggerCode::AfterSock),
            TRIGGER_SYS_BIND_AFTER => Some(TriggerCode::AfterBind),
            TRIGGER_BOOT_READY => Some(TriggerCode::BootReadyMarker),
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
    exit_handler: SharedExitEventHandler,
    start_time: std::time::Instant,
    should_exit_immediately: bool,
    boot_ready_time: Option<std::time::Duration>,
}

impl GuestManagerDevice {
    pub fn should_handle_read(addr: u64) -> bool {
        addr >= GUEST_MANAGER_MMIO_START && addr < GUEST_MANAGER_MMIO_END
    }

    pub fn should_handle_write(addr: u64) -> bool {
        addr >= GUEST_MANAGER_MMIO_START && addr < GUEST_MANAGER_MMIO_END
    }

    pub fn new(exit_handler: SharedExitEventHandler) -> Self {
        Self {
            exit_handler,
            start_time: std::time::Instant::now(),
            should_exit_immediately: false,
            boot_ready_time: None,
        }
    }

    pub fn should_exit_immediately(&self) -> bool {
        self.should_exit_immediately
    }

    pub fn set_boot_ready(&mut self) {
        let boot_ready_time = self.start_time.elapsed();
        self.boot_ready_time = Some(boot_ready_time);

        let ms = boot_ready_time.as_millis();
        if ms > 2 {
            info!("boot ready {ms}ms");
        } else {
            info!("boot ready {}us", boot_ready_time.as_micros());
        }
    }

    pub fn mmio_read(&mut self, offset: vm_device::bus::MmioAddressOffset, data: &mut [u8]) {
        if offset != 0 {
            warn!("unhandled read offset {}", offset);
            return;
        }

        if data.len() != 8 {
            warn!("invalid read data length {}", data.len());
            return;
        }

        let mut boot_ready_time = 0u64;
        if let Some(duration) = self.boot_ready_time {
            boot_ready_time = duration.as_micros() as u64;
        }

        data.copy_from_slice(&boot_ready_time.to_le_bytes());
    }

    pub fn mmio_write(&mut self, _offset: vm_device::bus::MmioAddressOffset, data: &[u8]) {
        let Some(trigger_data) = TriggerData::from_bytes(data) else {
            warn!("Failed to parse trigger data");
            return;
        };

        match trigger_data.code {
            TriggerCode::AfterListen => {
                warn!("trigger exit");
                self.exit_handler
                    .trigger_exit(ExitHandlerReason::Snapshot)
                    .unwrap();

                self.should_exit_immediately = true;
            }
            TriggerCode::BootReadyMarker => {
                self.set_boot_ready();
            }
            _ => {
                warn!(
                    "Guest manager unhandled trigger code {:?}",
                    trigger_data.code
                );
            }
        }
    }
}
