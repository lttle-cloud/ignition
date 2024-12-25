use std::sync::{Arc, Mutex};
use vm_device::device_manager::IoManager;

#[derive(Clone)]
pub struct SharedDeviceManager(Arc<Mutex<IoManager>>);

impl SharedDeviceManager {
    pub fn new() -> Self {
        SharedDeviceManager(Arc::new(Mutex::new(IoManager::new())))
    }
}
