use std::collections::HashMap;

use util::async_runtime::task::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstanceStatus {
    Pending,
    ImagePulling,
    ImageReady,
    NetworkSetup,
    Booting,
    Ready,
    Stopping,
    Stopped,
}

impl InstanceStatus {
    pub fn next(&self) -> Option<InstanceStatus> {
        match self {
            InstanceStatus::Pending => Some(InstanceStatus::ImagePulling),
            InstanceStatus::ImagePulling => Some(InstanceStatus::ImageReady),
            InstanceStatus::ImageReady => Some(InstanceStatus::NetworkSetup),
            InstanceStatus::NetworkSetup => Some(InstanceStatus::Booting),
            InstanceStatus::Booting => Some(InstanceStatus::Ready),
            InstanceStatus::Ready => None,
            InstanceStatus::Stopping => Some(InstanceStatus::Stopped),
            InstanceStatus::Stopped => None,
        }
    }
}

#[derive(Debug)]
pub struct InstancePipeline {
    status: InstanceStatus,
    current_progress_handles: HashMap<InstanceStatus, JoinHandle<()>>,
    cancelled: bool,
}

impl InstancePipeline {
    pub fn new() -> Self {
        Self {
            status: InstanceStatus::Pending,
            current_progress_handles: HashMap::new(),
            cancelled: false,
        }
    }

    pub fn get_status(&self) -> InstanceStatus {
        self.status
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn can_progress(&mut self) -> Option<InstanceStatus> {
        // if we don't have a handle for the current status, that's what we need to progress
        // if we do have a handle for the current status, we need to check if it's finished
        // if it is finished, we need to remove it, advance the status and return the new status
        if self.cancelled {
            return None;
        }

        let Some(current_handle) = self.current_progress_handles.get_mut(&self.status) else {
            return Some(self.status);
        };

        match current_handle.is_finished() {
            true => {
                let Some(next_status) = self.status.next() else {
                    return None;
                };

                self.current_progress_handles.remove(&self.status);
                self.status = next_status;
                Some(next_status)
            }
            false => None,
        }
    }

    pub fn start_progress(&mut self, progress_handle: JoinHandle<()>) {
        if self.cancelled {
            return;
        }

        self.current_progress_handles
            .insert(self.status, progress_handle);
    }

    pub fn go_to_next_status(&mut self) {
        if self.cancelled {
            return;
        }

        let Some(next_status) = self.status.next() else {
            return;
        };

        self.status = next_status;
    }

    pub fn go_to_status(&mut self, status: InstanceStatus) {
        if self.cancelled {
            return;
        }

        self.status = status;
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;

        for handle in self.current_progress_handles.values_mut() {
            if !handle.is_finished() {
                handle.abort();
            }
        }
    }

    pub async fn cancel_and_wait(&mut self) {
        self.cancel();

        for handle in self.current_progress_handles.values_mut() {
            if handle.is_finished() {
                continue;
            }

            if let Err(e) = handle.await {
                println!("error: {:?}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use util::async_runtime::{runtime::Runtime, task, time::sleep};

    use super::*;

    #[test]
    fn test_can_progress() {
        let rt = Runtime::new().unwrap();

        let mut pipeline = InstancePipeline::new();

        rt.block_on(async move {
            for i in 0..200 {
                if let Some(status) = pipeline.can_progress() {
                    println!("progressed to: {:?}", status);

                    if i == 20 {
                        pipeline.cancel();
                    }

                    pipeline.start_progress(task::spawn(async move {
                        let current_time = Instant::now();
                        println!("progressing {:?}", current_time);
                        sleep(Duration::from_secs(1)).await;
                    }));
                }
                sleep(Duration::from_millis(100)).await;
            }
        });

        panic!("shit");
    }
}
