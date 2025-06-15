use std::path::PathBuf;

use util::{async_runtime::fs::create_dir_all, result::Result};

pub struct LogsPoolConfig {
    pub base_path: String,
}

pub struct LogsPool {
    base_path: PathBuf,
    machine_logs_path: PathBuf,
}

impl LogsPool {
    pub async fn new(config: LogsPoolConfig) -> Result<Self> {
        let base_path = PathBuf::from(config.base_path);
        let machine_logs_path = base_path.join("machines");

        if !base_path.exists() {
            create_dir_all(&base_path).await?;
        }

        if !machine_logs_path.exists() {
            create_dir_all(&machine_logs_path).await?;
        }

        Ok(Self {
            base_path,
            machine_logs_path,
        })
    }

    pub fn get_machine_log_path(&self, machine_id: impl AsRef<str>) -> String {
        self.machine_logs_path
            .join(machine_id.as_ref())
            .to_string_lossy()
            .to_string()
    }
}
