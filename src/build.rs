mod build_utils;
mod constants;
mod machinery;
mod resources;

use build_utils::resource;

use crate::resources::AdmissionRule;

#[tokio::main]
pub async fn main() {
    resource::ResourcesBuilder::new()
        .resource_with_config::<resources::machine::Machine>(|cfg| {
            cfg.add_admission_rule(AdmissionRule::DissalowPatchUpdate)
        })
        .resource::<resources::service::Service>()
        .build()
        .await
        .expect("failed to build resources repository");
}
