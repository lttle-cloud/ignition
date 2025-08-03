use tracing::Level;
use tracing_subscriber::fmt::Subscriber;

pub fn init_tracing() {
    let subscriber = Subscriber::builder().with_max_level(Level::INFO).finish();
    tracing::subscriber::set_global_default(subscriber).expect("failed to set subscriber");
}
