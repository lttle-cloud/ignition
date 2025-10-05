use anyhow::Result;
use schemars::{JsonSchema, SchemaGenerator, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::machinery::api_schema::{ApiMethod, ApiPathSegment, ApiService, ApiVerb};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GadgetInitRunParams {
    pub discovery_data: GadgetInitDiscoveryData,
    pub reasoning_effort: Option<GadgetInitReasoningEffort>,
    pub messages: Vec<GadgetClientMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum GadgetInitReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GadgetInitRunResponse {
    pub messages: Vec<GadgetServiceMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum GadgetServiceMessage {
    ReadFile(ReadFileArgs),
    ListDir(ListDirArgs),
    DirBuildPlan(DirBuildPlanArgs),
    Finish(GadgetInitData),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GadgetInitDiscoveryData {
    pub base_dir_name: String,
    pub base_dir_build_plan: DirBuildPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadFileArgs {
    /// Paths of the file to read (relative to root directory)
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadFileResult {
    /// Path of the file (relative to root directory)
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDirArgs {
    /// Path of the file or directory to list (relative to root directory)
    pub path: String,
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDirResult {
    pub items: Vec<ListDirItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDirItem {
    /// Path of the file or directory (relative to root directory)
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirBuildPlan {
    pub detected_providers: Vec<String>,
    pub phases: Vec<BuildPlanPhase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildPlanPhase {
    pub name: String,
    pub build_info: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirBuildPlanArgs {
    /// Path of the directory to attempt to build (relative to root directory)
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GadgetClientMessage {
    pub service_message: GadgetServiceMessage,
    pub client_reply: Option<GadgetClientReply>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum GadgetClientReply {
    ReadFile(ReadFileResult),
    ListDir(ListDirResult),
    DirBuildPlan(DirBuildPlan),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GadgetInitData {
    pub plan: InitPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitPlan {
    pub apps: Vec<InitApp>,
    pub volumes: Vec<InitVolume>,
    pub issues: Vec<InitIssue>,
    pub warnings: Vec<InitWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitIssue {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitWarning {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitVolume {
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitApp {
    pub name: String,
    pub namespace: Option<String>,
    pub source: InitAppSource,
    pub snapshot_strategy: Option<InitAppSnapshotStrategy>,
    pub envs: Option<Vec<InitAppEnv>>,
    /// Services that this app depends on
    pub depends_on: Option<Vec<InitAppDependsOn>>,
    pub exposed_ports: Option<Vec<InitAppExposedPort>>,
    pub binded_volumes: Option<Vec<InitAppBindedVolume>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitAppBindedVolume {
    pub name: String,
    pub namespace: Option<String>,
    /// Path of the volume inside the container
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitAppDependsOn {
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitAppEnv {
    pub name: String,
    pub value: InitAppEnvValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum InitAppEnvValue {
    /// A literal string value
    Literal(String),

    /// CEL expression (or interpolation of CEL expressions). You must include the ${{ }} in the expression (with DOUBLE curly braces).
    /// ex: http://${{ env.VAR_NAME }}-${{ env.VAR_NAME2 }}.com/${{ env.VAR_NAME3 }}
    Expression(String),

    /// The value of this variable will be copied from the .env file when the app is deployed
    CopyFromEnvFile { var_name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum InitAppSnapshotStrategy {
    SuspendBeforeStart,
    SuspendAfterListenOnAnyPort,
    SuspendAfterListenOnPort(u16),
    SuspendManually,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum InitAppSource {
    BuildAutomatically(InitAppBuildAuto),
    BuildWithDockerfile(InitAppBuildDockerfile),
    Image(InitAppImage),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitAppBuildAuto {
    /// Path of the directory to build (relative to root directory)
    pub dir_path: Option<String>,

    /// Extra files to ignore (relative to root directory)
    pub append_docker_ignore_extra: Option<InitAppendDockerIgnoreExtra>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitAppBuildDockerfile {
    /// Path of the Dockerfile to build (relative to root directory)
    pub dir_path: String,

    /// Name of the Dockerfile to use (relative to root directory) (default: Dockerfile)
    pub dockerfile_name: Option<String>,

    /// Extra files to ignore (relative to root directory)
    pub append_docker_ignore_extra: Option<InitAppendDockerIgnoreExtra>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitAppendDockerIgnoreExtra {
    /// Path of the file to append to (relative to root directory)
    pub path: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitAppImage {
    pub image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitAppExposedPort {
    pub name: String,
    /// Port inside the container
    pub port: u16,
    pub mode: InitAppExposedPortMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum InitAppExposedPortMode {
    Internal {
        protocol: InitAppExposedPortProtocolInternal,
    },
    External {
        protocol: InitAppExposedPortProtocolExternal,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum InitAppExposedPortProtocolExternal {
    Tls,
    Https,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub enum InitAppExposedPortProtocolInternal {
    Tcp,
}

pub fn gadget_api_service() -> ApiService {
    ApiService {
        name: "Gadget".to_string(),
        tag: "gadget".to_string(),
        crate_path: "resources::gadget".to_string(),
        namespaced: false,
        methods: vec![ApiMethod {
            name: "run_init".to_string(),
            path: vec![
                ApiPathSegment::Static {
                    value: "gadget".to_string(),
                },
                ApiPathSegment::Static {
                    value: "run".to_string(),
                },
                ApiPathSegment::Static {
                    value: "init".to_string(),
                },
            ],
            namespaced: false,
            verb: ApiVerb::Put,
            request: Some(crate::machinery::api_schema::ApiRequest::SchemaDefinition {
                name: "GadgetInitRunParams".to_string(),
            }),
            response: Some(
                crate::machinery::api_schema::ApiResponse::SchemaDefinition {
                    list: false,
                    optional: false,
                    name: "GadgetInitRunResponse".to_string(),
                },
            ),
        }],
    }
}

pub fn add_gadget_service_schema_defs(
    _schema_generator: &mut SchemaGenerator,
    defs: &mut Map<String, Value>,
) -> Result<()> {
    defs.insert(
        "GadgetInitRunParams".to_string(),
        schema_for!(GadgetInitRunParams).into(),
    );
    defs.insert(
        "GadgetInitRunResponse".to_string(),
        schema_for!(GadgetInitRunResponse).into(),
    );

    Ok(())
}
