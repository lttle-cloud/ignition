use anyhow::Result;
use meta::resource;
use std::collections::BTreeMap;

use crate::resources::{
    Convert, FromResource,
    machine::{
        MachineDependency, MachineMode, MachineResources, MachineRestartPolicy,
        MachineVolumeBinding,
    },
};

#[resource(name = "App", tag = "app")]
mod app {

    #[version(stored + served + latest)]
    struct V1 {
        build: Option<AppBuild>,
        image: Option<String>,
        resources: MachineResources,
        #[serde(rename = "restart-policy")]
        restart_policy: Option<MachineRestartPolicy>,
        mode: Option<MachineMode>,
        volumes: Option<Vec<MachineVolumeBinding>>,
        command: Option<Vec<String>>,
        environment: Option<BTreeMap<String, String>>,
        #[serde(rename = "depends-on")]
        depends_on: Option<Vec<MachineDependency>>,
        expose: Option<Vec<AppExpose>>,
    }

    #[schema]
    struct AppBuild {
        context: String,
        #[serde(rename = "image-name")]
        image_name: Option<String>,
        tag: Option<String>,
    }

    #[schema]
    struct AppExpose {
        port: u16,
        #[serde(rename = "external-port")]
        external_port: Option<u16>,
        protocol: AppExposeProtocol,
        host: Option<String>,
    }

    #[schema]
    enum AppExposeProtocol {
        #[serde(rename = "http")]
        Http,
        #[serde(rename = "https")]
        Https,
        #[serde(rename = "tls")]
        Tls,
    }

    #[status]
    struct Status {}
}

impl FromResource<App> for AppStatus {
    fn from_resource(app: App) -> Result<Self> {
        let _app = app.latest();

        Ok(AppStatus {})
    }
}
