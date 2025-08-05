pub mod id;
pub mod time;
pub mod tracing;

use crate::controller::context::ControllerKey;

pub fn machine_name_from_key(key: &ControllerKey) -> String {
    format!("{}-{}", key.tenant, key.metadata().to_string())
}
