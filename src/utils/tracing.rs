use std::str::FromStr;

use anyhow::anyhow;
use tracing::Level;
use tracing_subscriber::fmt::Subscriber;

pub fn init_tracing() {
    let log_level = std::env::var("LOG_LEVEL")
        .map_err(|e| anyhow!("LOG_LEVEL environment variable is not set: {}", e))
        .and_then(|l| Level::from_str(&l).map_err(|e| anyhow!("Invalid log level: {}", e)))
        .unwrap_or(Level::INFO);

    let subscriber = Subscriber::builder().with_max_level(log_level).finish();
    tracing::subscriber::set_global_default(subscriber).expect("failed to set subscriber");
}
