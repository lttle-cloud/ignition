mod build_utils;
mod constants;
mod machinery;
mod resources;
mod utils;

use build_utils::resource;

use crate::resources::AdmissionRule;

#[tokio::main]
pub async fn main() {
    resource::ResourcesBuilder::new()
        .resource::<resources::certificate::Certificate>()
        .resource_with_config::<resources::machine::Machine>(|cfg| {
            cfg.add_admission_rule(AdmissionRule::StatusCheck)
        })
        .resource::<resources::service::Service>()
        .resource_with_config::<resources::volume::Volume>(|cfg| {
            cfg.add_admission_rule(AdmissionRule::BeforeDelete)
                .add_admission_rule(AdmissionRule::StatusCheck)
        })
        .build()
        .await
        .expect("failed to build resources repository");
}
