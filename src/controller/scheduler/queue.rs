use std::{sync::Arc, time::Duration};

use async_channel::{Receiver, Sender};
use papaya::HashMap;
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

        match keys.get(key) {
            Some(KeyStatus::InFlight) => {
                keys.insert(key.clone(), KeyStatus::Pending);
            }
            Some(KeyStatus::Pending) => {
                warn!("key {} is already pending", key.to_string());
            }
            None => {
                keys.insert(key.clone(), KeyStatus::InFlight);
                self.tx
                    .send(key.clone())
                    .await
                    .expect("failed to send key to queue");
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

        if let Some(KeyStatus::Pending) = keys.remove(key) {
            self.push(key).await;
        }
    }
}
