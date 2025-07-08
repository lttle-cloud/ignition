mod build_utils;
mod machinery;
mod resources;

use build_utils::{cargo, resource};

#[tokio::main]
pub async fn main() {
    cargo::warn("hello from build.rs");

    resource::ResourcesBuilder::new()
        .resource_with_config::<resources::machine::Machine>(|cfg| {
            cfg.disable_generate_service_set()
        })
        .build()
        .await
        .expect("failed to build resources repository");
}
