use std::{
    io::{Read, Write},
    ops::Deref,
    sync::Arc,
};

use event_manager::{EventOps, EventSet, Events, MutEventSubscriber};
use util::result::Result;
use util::tracing::warn;
use virtio_queue::{Queue, QueueOwnedT, QueueState, QueueT};
use vm_memory::Bytes;
use vmm_sys_util::eventfd::EventFd;

use crate::{
    device::virtio::{SignalUsedQueue, SingleFdSignalQueue},
    memory::Memory,
};

use super::{
    device::{RXQ_INDEX, TXQ_INDEX},
    tap::Tap,
};

const MAX_BUFFER_SIZE: usize = 65562;

const TAPFD_DATA: u32 = 0;
const RX_IOEVENT_DATA: u32 = 1;
const TX_IOEVENT_DATA: u32 = 2;

pub struct NetHandler<S: SignalUsedQueue> {
    pub memory: Arc<Memory>,
    pub driver_notify: S,
    pub rxq: Queue,
    pub rxbuf_current: usize,
    pub rxbuf: [u8; MAX_BUFFER_SIZE],
    pub txq: Queue,
    pub txbuf: [u8; MAX_BUFFER_SIZE],
    pub tap: Tap,
}

impl<S: SignalUsedQueue> NetHandler<S> {
    pub fn new(memory: Arc<Memory>, driver_notify: S, rxq: Queue, txq: Queue, tap: Tap) -> Self {
        NetHandler {
            memory,
            driver_notify,
            rxq,
            rxbuf_current: 0,
            rxbuf: [0u8; MAX_BUFFER_SIZE],
            txq,
            txbuf: [0u8; MAX_BUFFER_SIZE],
            tap,
        }
    }

    pub fn process_tap(&mut self) -> Result<()> {
        loop {
            if self.rxbuf_current == 0 {
                match self.tap.read(&mut self.rxbuf) {
                    Ok(n) => self.rxbuf_current = n,
                    Err(_) => {
                        // TODO: Do something (logs, metrics, etc.) in response to an error when
                        // reading from tap. EAGAIN means there's nothing available to read anymore
                        // (because we open the TAP as non-blocking).
                        break;
                    }
                }
            }

            if !self.write_frame_to_guest()?
                && !self
                    .rxq
                    .enable_notification(self.memory.deref().guest_memory())?
            {
                break;
            }
        }

        if self
            .rxq
            .needs_notification(self.memory.deref().guest_memory())?
        {
            self.driver_notify.signal_used_queue(RXQ_INDEX);
        }

        Ok(())
    }

    pub fn process_txq(&mut self) -> Result<()> {
        loop {
            self.txq
                .disable_notification(self.memory.deref().guest_memory())?;

            while let Some(mut chain) = self.txq.iter(self.memory.deref().guest_memory())?.next() {
                let mut count = 0;
                while let Some(desc) = chain.next() {
                    let left = self.txbuf.len() - count;
                    let len = desc.len() as usize;

                    if len > left {
                        warn!("tx frame too large");
                        break;
                    }

                    chain
                        .memory()
                        .read_slice(&mut self.txbuf[count..count + len], desc.addr())?;

                    count += len;
                }

                self.tap.write(&self.txbuf[..count])?;

                self.txq
                    .add_used(self.memory.deref().guest_memory(), chain.head_index(), 0)?;

                if self
                    .txq
                    .needs_notification(self.memory.deref().guest_memory())?
                {
                    self.driver_notify.signal_used_queue(TXQ_INDEX);
                }
            }

            if !self
                .txq
                .enable_notification(self.memory.deref().guest_memory())?
            {
                return Ok(());
            }
        }
    }

    pub fn process_rxq(&mut self) -> Result<()> {
        self.rxq
            .disable_notification(self.memory.deref().guest_memory())?;
        self.process_tap()
    }

    fn write_frame_to_guest(&mut self) -> Result<bool> {
        let num_bytes = self.rxbuf_current;

        let mut chain = match self.rxq.iter(self.memory.deref().guest_memory())?.next() {
            Some(c) => c,
            _ => return Ok(false),
        };

        let mut count = 0;
        let buf = &mut self.rxbuf[..num_bytes];

        while let Some(desc) = chain.next() {
            let left = buf.len() - count;

            if left == 0 {
                break;
            }

            let len = std::cmp::min(left, desc.len() as usize);
            chain
                .memory()
                .write_slice(&buf[count..count + len], desc.addr())?;

            count += len;
        }

        if count != buf.len() {
            // The frame was too large for the chain.
            eprint!("rx frame too large");
        }

        self.rxq.add_used(
            self.memory.deref().guest_memory(),
            chain.head_index(),
            count as u32,
        )?;

        self.rxbuf_current = 0;

        Ok(true)
    }

    pub fn get_queue_states(&self) -> (QueueState, QueueState) {
        let rxq_state = self.rxq.state();
        let txq_state = self.txq.state();

        (rxq_state, txq_state)
    }
}

pub struct QueueHandler {
    pub inner: NetHandler<SingleFdSignalQueue>,
    pub rx_ioevent: EventFd,
    pub tx_ioevent: EventFd,
}

impl QueueHandler {
    fn handle_error<S: AsRef<str>>(&self, s: S, ops: &mut EventOps) {
        warn!("{}", s.as_ref());

        ops.remove(Events::empty(&self.rx_ioevent))
            .expect("Failed to remove rx ioevent");
        ops.remove(Events::empty(&self.tx_ioevent))
            .expect("Failed to remove tx ioevent");
        ops.remove(Events::empty(&self.inner.tap))
            .expect("Failed to remove tap event");
    }
}

impl MutEventSubscriber for QueueHandler {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        if events.event_set() != EventSet::IN {
            self.handle_error("Unexpected event_set", ops);
            return;
        }

        match events.data() {
            TAPFD_DATA => {
                if let Err(e) = self.inner.process_tap() {
                    self.handle_error(format!("Process tap error {:?}", e), ops);
                }
            }
            RX_IOEVENT_DATA => {
                if self.rx_ioevent.read().is_err() {
                    self.handle_error("Rx ioevent read", ops);
                } else if let Err(e) = self.inner.process_rxq() {
                    self.handle_error(format!("Process rx error {:?}", e), ops);
                }
            }
            TX_IOEVENT_DATA => {
                if self.tx_ioevent.read().is_err() {
                    self.handle_error("Tx ioevent read", ops);
                }
                if let Err(e) = self.inner.process_txq() {
                    self.handle_error(format!("Process tx error {:?}", e), ops);
                }
            }
            _ => self.handle_error("Unexpected data", ops),
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        ops.add(Events::with_data(
            &self.inner.tap,
            TAPFD_DATA,
            EventSet::IN | EventSet::EDGE_TRIGGERED,
        ))
        .expect("Unable to add tapfd");

        ops.add(Events::with_data(
            &self.rx_ioevent,
            RX_IOEVENT_DATA,
            EventSet::IN,
        ))
        .expect("Unable to add rxfd");

        ops.add(Events::with_data(
            &self.tx_ioevent,
            TX_IOEVENT_DATA,
            EventSet::IN,
        ))
        .expect("Unable to add txfd");
    }
}
