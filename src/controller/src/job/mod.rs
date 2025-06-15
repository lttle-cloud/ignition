pub mod machine;

use std::time::Duration;

use util::{
    async_runtime::{select, time::sleep},
    result::{Result, bail},
};

#[allow(async_fn_in_trait)]
pub trait Job: Send + Sync + Sized {
    type Output: Send + Sync + Sized;

    async fn run(&mut self) -> Result<Self::Output>;
    fn cancel(&mut self);

    fn task(self) -> JobRun<Self> {
        JobRun::new(self)
    }
}

pub struct JobRun<J: Job> {
    job: J,
    running: bool,
}

impl<J: Job> JobRun<J> {
    pub fn new(job: J) -> Self {
        Self {
            job,
            running: false,
        }
    }

    pub async fn start(mut self) -> Result<J::Output> {
        self.running = true;
        let result = self.job.run().await;
        self.running = false;

        if result.is_err() {
            self.job.cancel();
        }

        result
    }

    pub async fn start_with_timeout(mut self, timeout: Duration) -> Result<J::Output> {
        self.running = true;
        select! {
            result = self.job.run() => {
                self.running = false;

                if result.is_err() {
                    self.job.cancel();
                }

                result
            }
            _ = sleep(timeout) => {
                self.running = false;

                self.job.cancel();
                bail!("Job cancelled by timeout");
            }
        }
    }
}

impl<J: Job> Drop for JobRun<J> {
    fn drop(&mut self) {
        if self.running {
            self.job.cancel();
        }
    }
}
