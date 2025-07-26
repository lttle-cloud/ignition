mod build_utils;
mod machinery;
mod resources;

use build_utils::resource;

use crate::resources::AdmissionRule;

#[tokio::main]
pub async fn main() {
    resource::ResourcesBuilder::new()
        .resource_with_config::<resources::machine::Machine>(|cfg| {
            // TODO: disable the setter for service after the controller is implemented
            // cfg.disable_generate_service_set()
            cfg.add_admission_rule(AdmissionRule::DissalowPatchUpdate)
        })
        .build()
        .await
        .expect("failed to build resources repository");
}
