mod build_utils;
mod machinery;
mod resources;

use build_utils::{cargo, resource};

#[tokio::main]
pub async fn main() {
    cargo::warn("hello from build.rs");

    resource::ResourcesRepositoryBuilder::new()
        .resource::<resources::machine::Machine>()
        .build()
        .await
        .expect("failed to build resources repository");
}
