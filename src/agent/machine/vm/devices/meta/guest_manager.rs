use std::{
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    time::Duration,
};

use tracing::warn;

use crate::agent::machine::{
    machine::SnapshotStrategy,
    vm::{
        constants::{MMIO_LEN, MMIO_START},
        devices::DeviceEvent,
    },
};

pub const GUEST_MANAGER_MMIO_START: u64 = MMIO_START;
pub const GUEST_MANAGER_MMIO_SIZE: u64 = MMIO_LEN;
pub const GUEST_MANAGER_MMIO_END: u64 = GUEST_MANAGER_MMIO_START + GUEST_MANAGER_MMIO_SIZE;

const TRIGGER_AFTER_OFFSET: u8 = 127;
const CMD_OFFSET: u8 = 64;

const TRIGGER_SYS_LISTEN: u8 = 1;
const TRIGGER_SYS_BIND: u8 = 2;
const TRIGGER_USER_SPACE_READY: u8 = 3;
const TRIGGER_MANUAL: u8 = 10;

const TRIGGER_SYS_LISTEN_AFTER: u8 = TRIGGER_AFTER_OFFSET + TRIGGER_SYS_LISTEN;
const TRIGGER_SYS_BIND_AFTER: u8 = TRIGGER_AFTER_OFFSET + TRIGGER_SYS_BIND;

const CMD_FLASH_LOCK: u8 = CMD_OFFSET + 0;
const CMD_FLASH_UNLOCK: u8 = CMD_OFFSET + 1;

const READ_OFFSET_LAST_BOOT_TIME: u64 = 0;
const READ_OFFSET_FIRST_BOOT_TIME: u64 = 8;

#[allow(unused)]
#[derive(Debug, Clone, Copy)]
enum TriggerCode {
    BeforeListen { port: u16, addr: Ipv4Addr },
    BeforeBind { port: u16, addr: Ipv4Addr },
    AfterListen { port: u16, addr: Ipv4Addr },
    AfterBind { port: u16, addr: Ipv4Addr },
    UserSpaceReady { data: [u8; 7] },
    Manual { data: [u8; 7] },
}

impl TriggerCode {
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        fn parse_port(bytes: &[u8]) -> Option<u16> {
            if bytes.len() != 2 {
                return None;
            }

            Some(u16::from_le_bytes([bytes[0], bytes[1]]))
        }

        fn parse_ipv4_addr(bytes: &[u8]) -> Option<Ipv4Addr> {
            if bytes.len() != 4 {
                return None;
            }

            Some(Ipv4Addr::from([bytes[3], bytes[2], bytes[1], bytes[0]]))
        }

        if bytes.len() != 8 {
            return None;
        }

        // first byte is the trigger code
        match bytes[0] {
            TRIGGER_SYS_LISTEN => {
                let port = parse_port(&bytes[1..3])?;
                let addr = parse_ipv4_addr(&bytes[3..7])?;
                Some(TriggerCode::BeforeListen { port, addr })
            }
            TRIGGER_SYS_LISTEN_AFTER => {
                let port = parse_port(&bytes[1..3])?;
                let addr = parse_ipv4_addr(&bytes[3..7])?;
                Some(TriggerCode::AfterListen { port, addr })
            }
            TRIGGER_SYS_BIND => {
                let port = parse_port(&bytes[1..3])?;
                let addr = parse_ipv4_addr(&bytes[3..7])?;
                Some(TriggerCode::BeforeBind { port, addr })
            }
            TRIGGER_SYS_BIND_AFTER => {
                let port = parse_port(&bytes[1..3])?;
                let addr = parse_ipv4_addr(&bytes[3..7])?;
                Some(TriggerCode::AfterBind { port, addr })
            }
            TRIGGER_USER_SPACE_READY => {
                let data = bytes[1..].try_into().ok()?;
                Some(TriggerCode::UserSpaceReady { data })
            }
            TRIGGER_MANUAL => {
                let data = bytes[1..].try_into().ok()?;
                Some(TriggerCode::Manual { data })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
enum Cmd {
    FlashLock,
    FlashUnlock,
}

impl Cmd {
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 1 {
            return None;
        }

        match bytes[0] {
            CMD_FLASH_LOCK => Some(Cmd::FlashLock),
            CMD_FLASH_UNLOCK => Some(Cmd::FlashUnlock),
            _ => None,
        }
    }
}

pub struct GuestManagerDevice {
    listen_trigger_count: u32,
    device_event_tx: async_broadcast::Sender<DeviceEvent>,
    first_boot_duration: Option<Duration>,
    last_boot_duration: Option<Duration>,
    snapshot_strategy: Option<SnapshotStrategy>,
}

impl GuestManagerDevice {
    pub fn should_handle_read(addr: u64) -> bool {
        addr >= GUEST_MANAGER_MMIO_START && addr < GUEST_MANAGER_MMIO_END
    }

    pub fn should_handle_write(addr: u64) -> bool {
        addr >= GUEST_MANAGER_MMIO_START && addr < GUEST_MANAGER_MMIO_END
    }

    pub fn new(
        device_event_tx: async_broadcast::Sender<DeviceEvent>,
        snapshot_strategy: Option<SnapshotStrategy>,
    ) -> Arc<Mutex<Self>> {
        let guest_manager = Self {
            snapshot_strategy,
            listen_trigger_count: 0,
            first_boot_duration: None,
            last_boot_duration: None,
            device_event_tx,
        };
        let guest_manager = Arc::new(Mutex::new(guest_manager));
        guest_manager
    }

    pub fn set_boot_duration(&mut self, duration: Duration) {
        if self.first_boot_duration.is_none() {
            self.first_boot_duration = Some(duration);
        }
        self.last_boot_duration = Some(duration);
    }

    pub fn set_snapshot_strategy(&mut self, snapshot_strategy: Option<SnapshotStrategy>) {
        self.snapshot_strategy = snapshot_strategy;
    }

    pub fn mmio_read(&mut self, offset: vm_device::bus::MmioAddressOffset, data: &mut [u8]) {
        if data.len() != 8 {
            warn!("invalid read data length {}", data.len());
            return;
        }

        let result = match offset {
            READ_OFFSET_LAST_BOOT_TIME => self
                .last_boot_duration
                .map(|duration| duration.as_micros() as u64),
            READ_OFFSET_FIRST_BOOT_TIME => self
                .first_boot_duration
                .map(|duration| duration.as_micros() as u64),
            _ => {
                warn!("unhandled read offset {}", offset);
                return;
            }
        };

        let result = result.unwrap_or(0);
        data.copy_from_slice(&result.to_le_bytes());
    }

    pub fn mmio_write(&mut self, offset: vm_device::bus::MmioAddressOffset, data: &[u8]) -> bool {
        if offset == 0 {
            return self.process_trigger(data);
        } else if offset == 8 {
            return self.process_cmd(data);
        }

        false
    }

    fn process_trigger(&mut self, data: &[u8]) -> bool {
        let Some(trigger_code) = TriggerCode::from_bytes(data) else {
            warn!("Failed to parse trigger data");
            return false;
        };

        if matches!(trigger_code, TriggerCode::AfterListen { port: _, addr: _ }) {
            self.listen_trigger_count += 1;
        }

        if matches!(trigger_code, TriggerCode::UserSpaceReady { data: _ }) {
            self.device_event_tx
                .try_broadcast(DeviceEvent::UserSpaceReady)
                .ok();
        }

        match (trigger_code, &self.snapshot_strategy) {
            (
                TriggerCode::UserSpaceReady { data: _ },
                Some(SnapshotStrategy::WaitForUserSpaceReady),
            ) => {
                return true;
            }
            (TriggerCode::AfterListen { .. }, Some(SnapshotStrategy::WaitForFirstListen)) => {
                return true;
            }
            (TriggerCode::AfterListen { .. }, Some(SnapshotStrategy::WaitForNthListen(count))) => {
                if self.listen_trigger_count >= *count {
                    return true;
                }
            }
            (
                TriggerCode::AfterListen { port, .. },
                Some(SnapshotStrategy::WaitForListenOnPort(target_port)),
            ) => {
                if port == *target_port {
                    return true;
                }
            }
            (TriggerCode::Manual { data: _ }, Some(SnapshotStrategy::Manual)) => {
                return true;
            }
            _ => {}
        };

        return false;
    }

    fn process_cmd(&mut self, data: &[u8]) -> bool {
        let Some(cmd) = Cmd::from_bytes(data) else {
            warn!("Failed to parse cmd data");
            return false;
        };

        let event = match cmd {
            Cmd::FlashLock => DeviceEvent::FlashLock,
            Cmd::FlashUnlock => DeviceEvent::FlashUnlock,
        };

        self.device_event_tx.try_broadcast(event).ok();

        return false;
    }
}
