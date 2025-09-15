use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use anyhow::Result;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::controller::{
    context::{ControllerEvent, ControllerKey},
    scheduler::Scheduler,
};

struct JobState {
    job: JoinHandle<Result<()>>,
    watch_keys: Vec<ControllerKey>,
}

pub struct JobAgent {
    scheduler: Weak<Scheduler>,
    jobs: Arc<Mutex<HashMap<String, JobState>>>,
    temp_results: Arc<Mutex<HashMap<(ControllerKey, String), ControllerEvent>>>,
}

impl JobAgent {
    pub fn new(scheduler: Weak<Scheduler>) -> Self {
        Self {
            scheduler,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            temp_results: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn run_with_notify<R: Send + Clone, E: Send + Clone>(
        &self,
        key: ControllerKey,
        job_key: impl AsRef<str>,
        job: impl Future<Output = std::result::Result<R, E>> + Send + 'static,
        notifier: impl (Fn(std::result::Result<R, E>, ControllerKey) -> Option<ControllerEvent>)
        + Send
        + 'static,
    ) -> Result<()> {
        let job_key = job_key.as_ref().to_string();
        let mut jobs = self.jobs.lock().await;

        if let Some(job_state) = jobs.get_mut(&job_key) {
            if !job_state.watch_keys.contains(&key) {
                job_state.watch_keys.push(key);
            }
        } else {
            let task_scheduler = self.scheduler.clone();
            let task_jobs = self.jobs.clone();
            let task_job_key = job_key.clone();
            let task_temp_results = self.temp_results.clone();

            let job = tokio::spawn(async move {
                let result = job.await;
                let mut jobs = task_jobs.lock().await;
                let Some(job_state) = jobs.remove(&task_job_key) else {
                    return Ok(());
                };

                let mut temp_results = task_temp_results.lock().await;
                for watch_key in job_state.watch_keys {
                    let event = notifier(result.clone(), watch_key.clone());
                    let Some(task_scheduler) = task_scheduler.upgrade() else {
                        continue;
                    };

                    let Some(event) = event else {
                        continue;
                    };

                    temp_results.insert((watch_key.clone(), task_job_key.clone()), event.clone());
                    task_scheduler.push(watch_key.tenant, event).await?;
                }

                Ok(())
            });

            jobs.insert(
                job_key,
                JobState {
                    job,
                    watch_keys: vec![key],
                },
            );
        }

        Ok(())
    }

    pub async fn cancel_notify(&self, job_key: impl AsRef<str>, key: ControllerKey) {
        let mut jobs = self.jobs.lock().await;
        let Some(job_state) = jobs.get_mut(job_key.as_ref()) else {
            return;
        };

        job_state.watch_keys.retain(|k| k != &key);
        if job_state.watch_keys.is_empty() {
            let job = jobs.remove(job_key.as_ref()).unwrap();
            job.job.abort();
        }
    }

    pub async fn get_result(
        &self,
        job_key: impl AsRef<str>,
        key: ControllerKey,
    ) -> Result<Option<ControllerEvent>> {
        let temp_results = self.temp_results.lock().await;
        Ok(temp_results
            .get(&(key, job_key.as_ref().to_string()))
            .cloned())
    }

    pub async fn consume_result(&self, job_key: impl AsRef<str>, key: ControllerKey) -> Result<()> {
        let mut temp_results = self.temp_results.lock().await;
        temp_results.remove(&(key, job_key.as_ref().to_string()));
        Ok(())
    }
}
