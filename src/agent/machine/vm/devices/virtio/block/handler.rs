use std::fs::File;

use anyhow::{Result, anyhow};
use event_manager::{EventOps, EventSet, Events, MutEventSubscriber};
use tracing::warn;
use virtio_blk::{request::Request, stdio_executor::StdIoBackend};
use virtio_queue::{Queue, QueueOwnedT, QueueState, QueueT};
use vm_memory::GuestMemoryMmap;
use vmm_sys_util::eventfd::EventFd;

use crate::agent::machine::vm::devices::virtio::{SignalUsedQueue, SingleFdSignalQueue};

const IOEVENT_DATA: u32 = 0;

pub struct BlockHandler<S: SignalUsedQueue> {
    pub driver_notify: S,
    pub queue: Queue,
    pub memory: GuestMemoryMmap,
    pub disk: StdIoBackend<File>,
}

impl<S: SignalUsedQueue> BlockHandler<S> {
    pub fn process(&mut self) -> Result<()> {
        loop {
            self.queue.disable_notification(&self.memory)?;

            while let Some(mut chain) = self.queue.iter(&self.memory)?.next() {
                let used_len = match Request::parse(&mut chain) {
                    Ok(request) => self
                        .disk
                        .process_request(&self.memory, &request)
                        .map_err(|_| anyhow!("block: failed to process request"))?,
                    Err(e) => {
                        warn!("block: failed to parse request: {}", e);
                        0
                    }
                };

                self.queue
                    .add_used(&self.memory, chain.head_index(), used_len)?;

                if self.queue.needs_notification(&self.memory)? {
                    self.driver_notify.signal_used_queue(0);
                }
            }

            if !self.queue.enable_notification(&self.memory)? {
                break;
            }
        }

        Ok(())
    }

    pub fn get_queue_state(&self) -> QueueState {
        self.queue.state()
    }
}

pub struct QueueHandler {
    pub inner: BlockHandler<SingleFdSignalQueue>,
    pub ioeventfd: EventFd,
}

impl MutEventSubscriber for QueueHandler {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        let mut error = true;

        if events.event_set() != EventSet::IN {
            warn!("unexpected event_set");
        } else if events.data() != IOEVENT_DATA {
            warn!("unexpected events data {}", events.data());
        } else if self.ioeventfd.read().is_err() {
            warn!("ioeventfd read error")
        } else if let Err(e) = self.inner.process() {
            warn!("error processing block queue {:?}", e);
        } else {
            error = false;
        }

        if error {
            ops.remove(events)
                .expect("Failed to remove fd from event handling loop");
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        ops.add(Events::with_data(
            &self.ioeventfd,
            IOEVENT_DATA,
            EventSet::IN,
        ))
        .expect("Failed to init block queue handler");
    }
}
