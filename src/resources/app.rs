use anyhow::Result;
use meta::resource;
use std::collections::BTreeMap;

use crate::resources::{
    Convert, FromResource,
    machine::{
        MachineBuild, MachineDependency, MachineMode, MachineResources, MachineRestartPolicy,
        MachineVolumeBinding,
    },
    service::{ServiceBindExternalProtocol, ServiceTargetConnectionTracking},
};

#[resource(name = "App", tag = "app")]
mod app {
    #[version(stored + served + latest)]
    struct V1 {
        image: Option<String>,
        build: Option<MachineBuild>,
        resources: MachineResources,
        #[serde(rename = "restart-policy")]
        restart_policy: Option<MachineRestartPolicy>,
        mode: Option<MachineMode>,
        volumes: Option<Vec<MachineVolumeBinding>>,
        command: Option<Vec<String>>,
        environment: Option<BTreeMap<String, String>>,
        #[serde(rename = "depends-on")]
        depends_on: Option<Vec<MachineDependency>>,
        expose: Option<BTreeMap<String, AppExpose>>,
    }

    #[schema]
    struct AppExpose {
        port: u16,
        #[serde(rename = "connection-tracking")]
        connection_tracking: Option<ServiceTargetConnectionTracking>,
        external: Option<AppExposeExternal>,
        internal: Option<AppExposeInternal>,
    }

    #[schema]
    struct AppExposeInternal {
        port: Option<u16>,
    }

    #[schema]
    struct AppExposeExternal {
        host: Option<String>,
        port: Option<u16>,
        protocol: ServiceBindExternalProtocol,
    }

    #[status]
    struct Status {
        machine_hash: u64,
        machine_name: Option<String>,
        allocated_services: BTreeMap<String, AppAllocatedService>,
    }

    #[schema]
    struct AppAllocatedService {
        name: String,
        hash: u64,
        domain: Option<String>,
    }
}

impl FromResource<App> for AppStatus {
    fn from_resource(app: App) -> Result<Self> {
        let _app = app.latest();

        Ok(AppStatus {
            machine_hash: 0,
            machine_name: None,
            allocated_services: BTreeMap::new(),
        })
    }
}
