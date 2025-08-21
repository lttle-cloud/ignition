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
        .resource_with_config::<resources::machine::Machine>(|cfg| {
            cfg.add_admission_rule(AdmissionRule::BeforeSet)
        })
        .resource_with_config::<resources::service::Service>(|cfg| {
            cfg.add_admission_rule(AdmissionRule::BeforeSet)
                .add_admission_rule(AdmissionRule::BeforeDelete)
        })
        .resource_with_config::<resources::certificate::Certificate>(|cfg| {
            cfg.add_admission_rule(AdmissionRule::BeforeSet)
                .add_admission_rule(AdmissionRule::BeforeDelete)
        })
        .resource_with_config::<resources::volume::Volume>(|cfg| {
            cfg.add_admission_rule(AdmissionRule::BeforeDelete)
                .add_admission_rule(AdmissionRule::StatusCheck)
        })
        .build()
        .await
        .expect("failed to build resources repository");
}
