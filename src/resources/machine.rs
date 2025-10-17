use anyhow::Result;
use meta::resource;
use std::collections::BTreeMap;

use crate::resources::{Convert, FromResource, ProvideMetadata};

#[resource(name = "Machine", tag = "machine")]
mod machine {

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
    }

    #[schema]
    enum MachineBuild {
        #[serde(rename = "auto")]
        NixpacksAuto,
        #[serde(rename = "options")]
        Nixpacks(MachineBuildOptions),
        #[serde(rename = "docker")]
        Docker(MachineDockerOptions),
    }

    #[schema]
    struct MachineBuildOptions {
        name: Option<String>,
        tag: Option<String>,
        image: Option<String>,
        dir: Option<String>,
        envs: Option<BTreeMap<String, String>>,
        providers: Option<Vec<String>>,
        #[serde(rename = "build-image")]
        build_image: Option<String>,
        variables: Option<BTreeMap<String, String>>,
        #[serde(rename = "static-assets")]
        static_assets: Option<BTreeMap<String, String>>,
        phases: Option<BTreeMap<String, MachineBuildPlanPhase>>,
        #[serde(rename = "start")]
        start_phase: Option<MachineBuildPlanStartPhase>,
    }

    #[schema]
    struct MachineBuildPlanPhase {
        name: Option<String>,
        #[serde(rename = "depends-on")]
        depends_on: Option<Vec<String>>,
        #[serde(rename = "nix-pkgs")]
        nix_pkgs: Option<Vec<String>>,
        #[serde(rename = "nix-libs")]
        nix_libs: Option<Vec<String>>,
        #[serde(rename = "nix-overlays")]
        nix_overlays: Option<Vec<String>>,
        #[serde(rename = "nixpkgs-archive")]
        nixpkgs_archive: Option<String>,
        #[serde(rename = "apt-pkgs")]
        apt_pkgs: Option<Vec<String>>,
        #[serde(rename = "cmds")]
        cmds: Option<Vec<String>>,
        #[serde(rename = "only-include-files")]
        only_include_files: Option<Vec<String>>,
        #[serde(rename = "cache-directories")]
        cache_directories: Option<Vec<String>>,
        #[serde(rename = "paths")]
        paths: Option<Vec<String>>,
    }

    #[schema]
    struct MachineBuildPlanStartPhase {
        cmd: Option<String>,
        #[serde(rename = "run-image")]
        run_image: Option<String>,
        #[serde(rename = "only-include-files")]
        only_include_files: Option<Vec<String>>,
        user: Option<String>,
    }

    #[schema]
    struct MachineDockerOptions {
        name: Option<String>,
        tag: Option<String>,
        image: Option<String>,
        context: Option<String>,
        dockerfile: Option<String>,
        #[serde(rename = "args")]
        args: Option<BTreeMap<String, String>>,
    }

    #[schema]
    enum MachineRestartPolicy {
        #[serde(rename = "never")]
        Never,
        #[serde(rename = "always")]
        Always,
        #[serde(rename = "on-failure")]
        OnFailure,
        #[serde(rename = "remove")]
        Remove,
    }

    #[schema]
    struct MachineResources {
        cpu: u8,
        memory: u64,
    }

    #[schema]
    enum MachineMode {
        #[serde(rename = "regular")]
        Regular,
        #[serde(rename = "flash")]
        Flash {
            strategy: MachineSnapshotStrategy,
            timeout: Option<u64>,
        },
    }

    #[schema]
    enum MachineSnapshotStrategy {
        #[serde(rename = "first-listen")]
        WaitForFirstListen,
        #[serde(rename = "nth-listen")]
        WaitForNthListen(u32),
        #[serde(rename = "listen-on-port")]
        WaitForListenOnPort(u16),
        #[serde(rename = "user-space-ready")]
        WaitForUserSpaceReady,
        #[serde(rename = "manual")]
        Manual,
    }

    #[schema]
    struct MachineVolumeBinding {
        #[serde(deserialize_with = "super::de_trim_non_empty_string")]
        name: String,
        #[serde(default, deserialize_with = "super::de_opt_trim_non_empty_string")]
        namespace: Option<String>,
        #[serde(deserialize_with = "super::de_trim_non_empty_string")]
        path: String,
    }

    #[schema]
    struct MachineDependency {
        #[serde(deserialize_with = "super::de_trim_non_empty_string")]
        name: String,
        #[serde(default, deserialize_with = "super::de_opt_trim_non_empty_string")]
        namespace: Option<String>,
    }

    #[status]
    struct Status {
        hash: u64,
        phase: MachinePhase,
        image_id: Option<String>,
        image_resolved_reference: Option<String>,
        machine_id: Option<String>,
        machine_ip: Option<String>,
        machine_tap: Option<String>,
        machine_image_volume_id: Option<String>,
        last_boot_time_us: Option<u64>,
        first_boot_time_us: Option<u64>,
        last_restarting_time_us: Option<u64>,
        last_exit_code: Option<i32>,
    }

    #[schema]
    enum MachinePhase {
        #[serde(rename = "idle")]
        Idle,
        #[serde(rename = "pulling-image")]
        PullingImage,
        #[serde(rename = "waiting")]
        Waiting,
        #[serde(rename = "creating")]
        Creating,
        #[serde(rename = "booting")]
        Booting,
        #[serde(rename = "ready")]
        Ready,
        #[serde(rename = "suspending")]
        Suspending,
        #[serde(rename = "suspended")]
        Suspended,
        #[serde(rename = "stopping")]
        Stopping,
        #[serde(rename = "stopped")]
        Stopped,
        #[serde(rename = "restarting")]
        Restarting,
        #[serde(rename = "error")]
        Error { message: String },
    }
}

impl ToString for MachinePhase {
    fn to_string(&self) -> String {
        match self {
            MachinePhase::Idle => "idle".to_string(),
            MachinePhase::PullingImage => "pulling-image".to_string(),
            MachinePhase::Creating => "creating".to_string(),
            MachinePhase::Waiting => "waiting".to_string(),
            MachinePhase::Booting => "booting".to_string(),
            MachinePhase::Ready => "ready".to_string(),
            MachinePhase::Suspending => "suspending".to_string(),
            MachinePhase::Suspended => "suspended".to_string(),
            MachinePhase::Stopping => "stopping".to_string(),
            MachinePhase::Stopped => "stopped".to_string(),
            MachinePhase::Restarting => "restarting".to_string(),
            MachinePhase::Error { message } => format!("error ({})", message),
        }
    }
}

impl ToString for MachineRestartPolicy {
    fn to_string(&self) -> String {
        match self {
            MachineRestartPolicy::Never => "never".to_string(),
            MachineRestartPolicy::Always => "always".to_string(),
            MachineRestartPolicy::OnFailure => "on-failure".to_string(),
            MachineRestartPolicy::Remove => "remove".to_string(),
        }
    }
}

impl FromResource<Machine> for MachineStatus {
    fn from_resource(_resource: Machine) -> Result<Self> {
        Ok(MachineStatus {
            hash: 0,
            phase: MachinePhase::Idle,
            image_id: None,
            image_resolved_reference: None,
            machine_id: None,
            machine_ip: None,
            machine_tap: None,
            machine_image_volume_id: None,
            last_boot_time_us: None,
            first_boot_time_us: None,
            last_restarting_time_us: None,
            last_exit_code: None,
        })
    }
}

impl Machine {
    pub fn hash_with_updated_metadata(&self) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let metadata = self.metadata();
        let mut machine = self.stored();
        machine.namespace = metadata.namespace;
        let machine: Machine = machine.into();

        let mut hasher = DefaultHasher::new();
        machine.hash(&mut hasher);
        hasher.finish()
    }
}
