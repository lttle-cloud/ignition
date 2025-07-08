mod build_utils;
mod machinery;
mod resources;

use build_utils::resource;

#[tokio::main]
pub async fn main() {
    resource::ResourcesBuilder::new()
        .resource_with_config::<resources::machine::Machine>(|cfg| {
            // TODO: disable the setter for service after the controller is implemented
            // cfg.disable_generate_service_set()
            cfg
        })
        .build()
        .await
        .expect("failed to build resources repository");
}
