use std::ops::Deref;

use util::result::Result;
use vm_superio::Trigger;
use vmm_sys_util::eventfd::EventFd;

pub struct EventFdTrigger(EventFd);

impl Trigger for EventFdTrigger {
    type E = util::result::Error;

    fn trigger(&self) -> Result<()> {
        Ok(())
    }
}

impl Deref for EventFdTrigger {
    type Target = EventFd;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl EventFdTrigger {
    pub fn new(flag: i32) -> Result<Self> {
        let fd = EventFd::new(flag)?;
        Ok(EventFdTrigger(fd))
    }

    pub fn try_clone(&self) -> Result<Self> {
        Ok(EventFdTrigger(self.0.try_clone()?))
    }
}
