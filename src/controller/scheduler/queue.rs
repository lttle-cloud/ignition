use std::{sync::Arc, time::Duration};

use async_channel::{Receiver, Sender};
use papaya::{Compute, HashMap, Operation};
use tracing::warn;

use crate::controller::context::ControllerKey;

#[derive(Clone, Debug)]
enum KeyStatus {
    InFlight,
    Pending,
}

#[derive(Clone)]
pub struct WorkQueue {
    keys: Arc<HashMap<ControllerKey, KeyStatus>>,
    tx: Sender<ControllerKey>,
}

impl WorkQueue {
    pub fn new() -> (Self, Receiver<ControllerKey>) {
        let (tx, rx) = async_channel::unbounded();

        (
            Self {
                keys: Arc::new(HashMap::new()),
                tx,
            },
            rx,
        )
    }

    pub async fn push(&self, key: &ControllerKey) {
        let keys = self.keys.pin_owned();

        let result = keys.compute(key.clone(), |entry| {
            match entry {
                Some((_key, KeyStatus::InFlight)) => Operation::Insert(KeyStatus::Pending),
                Some((_key, KeyStatus::Pending)) => Operation::Abort(false), // false = already pending
                None => Operation::Insert(KeyStatus::InFlight),
            }
        });

        match result {
            Compute::Inserted(_key, KeyStatus::InFlight) => {
                self.tx
                    .send(key.clone())
                    .await
                    .expect("failed to send key to queue");
            }
            Compute::Inserted(_key, KeyStatus::Pending) => {}
            Compute::Aborted(false) => {
                warn!("key {} is already pending", key.to_string());
            }
            _ => {
                warn!("key {} is already in the queue", key.to_string());
            }
        }
    }

    pub fn push_after(&self, key: &ControllerKey, delay: Duration) {
        let this = self.clone();
        let key = key.clone();

        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            this.push(&key).await;
        });
    }

    pub async fn done(&self, key: &ControllerKey) {
        let keys = self.keys.pin_owned();

        let result = keys.compute(key.clone(), |entry| match entry {
            Some((_key, KeyStatus::Pending)) => Operation::Remove,
            Some((_, KeyStatus::InFlight)) => Operation::Remove,
            None => Operation::Abort(()),
        });

        if let Compute::Removed(_key, KeyStatus::Pending) = result {
            self.push(key).await;
        }
    }
}
