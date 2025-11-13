use std::sync::Arc;

use anyhow::{Result, bail};
use async_openai::types::{
    ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestToolMessageArgs, ChatCompletionToolArgs, ChatCompletionToolType,
    CreateChatCompletionRequestArgs, FunctionCall, FunctionObjectArgs, ReasoningEffort,
    ResponseFormat, ResponseFormatJsonSchema,
};
use axum::{Json, Router, extract::State, response::IntoResponse, routing::put};
use hyper::StatusCode;
use schemars::{SchemaGenerator, generate::SchemaSettings, schema_for};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{
    api::{
        ApiState,
        context::ServiceRequestContext,
        resource_service::{ResourceService, ResourceServiceRouter},
    },
    resources::gadget::{
        DirBuildPlanArgs, GadgetClientMessage, GadgetClientReply, GadgetInitData,
        GadgetInitDiscoveryData, GadgetInitReasoningEffort, GadgetInitRunParams,
        GadgetInitRunResponse, GadgetServiceMessage, ListDirArgs, ReadFileArgs,
    },
};

pub struct GadgetService {}

impl ResourceService for GadgetService {
    fn create_router(_state: Arc<ApiState>) -> ResourceServiceRouter {
        async fn init_run(
            State(state): State<Arc<ApiState>>,
            ctx: ServiceRequestContext,
            Json(params): Json<GadgetInitRunParams>,
        ) -> impl IntoResponse {
            let messages = match run_init_verified(
                state,
                ctx,
                params.discovery_data,
                params.messages,
                params.reasoning_effort,
            )
            .await
            {
                Ok(messages) => messages,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            };

            (StatusCode::OK, Json(GadgetInitRunResponse { messages })).into_response()
        }

        let mut router = Router::new();
        router = router.route("/run/init", put(init_run));

        ResourceServiceRouter {
            name: "Gadget".to_string(),
            base_path: "/gadget".to_string(),
            router,
        }
    }
}

async fn run_init_verified(
    state: Arc<ApiState>,
    _ctx: ServiceRequestContext,
    discovery_data: GadgetInitDiscoveryData,
    messages: Vec<GadgetClientMessage>,
    reasoning_effort: Option<GadgetInitReasoningEffort>,
) -> Result<Vec<GadgetServiceMessage>> {
    let openai = state.scheduler.agent.openai()?;

    // Create all tools
    let list_dir_tool = ChatCompletionToolArgs::default()
        .r#type(ChatCompletionToolType::Function)
        .function(
            FunctionObjectArgs::default()
                .name("list_dir")
                .description("List directory contents")
                .parameters(schema_for!(ListDirArgs))
                .build()?
        )
        .build()?;

    let read_file_tool = ChatCompletionToolArgs::default()
        .r#type(ChatCompletionToolType::Function)
        .function(
            FunctionObjectArgs::default()
                .name("read_file")
                .description("Read file contents")
                .parameters(schema_for!(ReadFileArgs))
                .build()?
        )
        .build()?;

    let dir_build_plan_tool = ChatCompletionToolArgs::default()
        .r#type(ChatCompletionToolType::Function)
        .function(
            FunctionObjectArgs::default()
                .name("dir_build_plan")
                .description("Get build plan for directory")
                .parameters(schema_for!(DirBuildPlanArgs))
                .build()?
        )
        .build()?;

    let tools = vec![list_dir_tool, read_file_tool, dir_build_plan_tool];

    let prompt = create_verification_prompt(discovery_data)?;

    let mut chat_messages = vec![ChatCompletionRequestMessage::System(
        ChatCompletionRequestSystemMessageArgs::default()
            .content(prompt)
            .build()?,
    )];

    // Add existing messages if continuation
    for (i, message) in messages.iter().enumerate() {
        let (tool_call_name, tool_call_args) = match &message.service_message {
            GadgetServiceMessage::Error(_) | GadgetServiceMessage::Finish(_) => {
                bail!("init run is already finished");
            }
            GadgetServiceMessage::ListDir(args) => ("list_dir", serde_json::to_string(args)?),
            GadgetServiceMessage::ReadFile(args) => ("read_file", serde_json::to_string(args)?),
            GadgetServiceMessage::DirBuildPlan(args) => {
                ("dir_build_plan", serde_json::to_string(args)?)
            }
        };
        
        let tool_call_id = format!("tool_call_{}", i);
        let tool_call_message = ChatCompletionMessageToolCall {
            id: tool_call_id.clone(),
            r#type: ChatCompletionToolType::Function,
            function: FunctionCall {
                name: tool_call_name.to_string(),
                arguments: tool_call_args,
            },
        };
        
        let assistant_message = ChatCompletionRequestAssistantMessageArgs::default()
            .tool_calls(vec![tool_call_message])
            .build()?;

        chat_messages.push(ChatCompletionRequestMessage::Assistant(assistant_message));

        if let Some(client_reply) = &message.client_reply {
            let content = match &client_reply {
                GadgetClientReply::ListDir(args) => serde_json::to_string(args)?,
                GadgetClientReply::ReadFile(args) => serde_json::to_string(args)?,
                GadgetClientReply::DirBuildPlan(args) => serde_json::to_string(args)?,
                GadgetClientReply::Error(e) => format!("error: {}", e),
            };

            let tool_message = ChatCompletionRequestToolMessageArgs::default()
                .content(content)
                .tool_call_id(tool_call_id)
                .build()?;

            chat_messages.push(ChatCompletionRequestMessage::Tool(tool_message));
        }
    }

    let mut settings = SchemaSettings::default();
    settings.inline_subschemas = true;

    let schema_generator = SchemaGenerator::new(settings);
    let schema = schema_generator.into_root_schema_for::<GadgetInitData>();
    let mut schema = serde_json::to_value(schema)?;
    enforce_openai_subset(&mut schema);

    let response_format = ResponseFormat::JsonSchema {
        json_schema: ResponseFormatJsonSchema {
            description: None,
            name: "gadget_init_data".into(),
            schema: Some(schema),
            strict: Some(true),
        },
    };

    let request = CreateChatCompletionRequestArgs::default()
        .model(openai.get_default_model())
        .messages(chat_messages)
        .response_format(response_format)
        .tools(tools)
        .parallel_tool_calls(true)
        .reasoning_effort(match reasoning_effort {
            Some(GadgetInitReasoningEffort::Minimal) | None => ReasoningEffort::Medium,
            Some(GadgetInitReasoningEffort::Low) => ReasoningEffort::Medium,
            Some(GadgetInitReasoningEffort::Medium) => ReasoningEffort::High,
            Some(GadgetInitReasoningEffort::High) => ReasoningEffort::High,
        })
        .n(1)
        .build()?;

    let response = openai.get_api_client().chat().create(request).await?;
    let Some(message) = response.choices.first().map(|c| c.message.clone()) else {
        bail!("no response from openai");
    };

    if let Some(content) = message.content {
        let response = serde_json::from_str::<GadgetInitData>(&content)?;
        return Ok(vec![GadgetServiceMessage::Finish(response)]);
    }

    let mut service_messages = vec![];
    for tool_call in message.tool_calls.unwrap_or_default() {
        match tool_call.function.name.as_str() {
            "list_dir" => {
                let args = serde_json::from_str::<ListDirArgs>(&tool_call.function.arguments)?;
                service_messages.push(GadgetServiceMessage::ListDir(args));
            }
            "read_file" => {
                let args = serde_json::from_str::<ReadFileArgs>(&tool_call.function.arguments)?;
                service_messages.push(GadgetServiceMessage::ReadFile(args));
            }
            "dir_build_plan" => {
                let args = serde_json::from_str::<DirBuildPlanArgs>(&tool_call.function.arguments)?;
                service_messages.push(GadgetServiceMessage::DirBuildPlan(args));
            }
            _ => {
                warn!("unknown tool call: {}", tool_call.function.name);
                continue;
            }
        }
    }

    if service_messages.is_empty() {
        bail!("no messages returned");
    }

    Ok(service_messages)
}

fn create_verification_prompt(discovery_data: GadgetInitDiscoveryData) -> Result<String> {
    let dir_name = discovery_data.base_dir_name;
    let providers = discovery_data
        .base_dir_build_plan
        .detected_providers
        .join(", ");

    let prompt = format!(
        r#"
# GADGET - Evidence-Based Deployment Configuration

Project: {dir_name}
Detected build providers: {providers}

## TOOL USAGE WORKFLOW

Execute these steps sequentially. Each step depends on the previous one.

### Step 1: Map Structure
Use list_dir to explore:
- Start with list_dir("/")
- List any subdirectories that might contain apps (src/, backend/, frontend/, api/, web/)
- Identify configuration files: .env*, docker-compose.*, Dockerfile*, package.json, requirements.txt, go.mod, Cargo.toml

### Step 2: Verify Technologies
For each potential technology, read the file that proves it exists:

| If you suspect | Must read file | Look for |
|---------------|----------------|-----------|
| Node.js app | package.json | dependencies object |
| Python app | requirements.txt or setup.py | package names |
| Go app | go.mod | module declaration |
| Rust app | Cargo.toml | dependencies section |
| PostgreSQL | package.json or .env | "pg" package or POSTGRES_* vars |
| MongoDB | package.json or .env | "mongodb" package or MONGO_* vars |
| Redis | package.json or .env | "redis" package or REDIS_* vars |
| Docker setup | docker-compose.yml | services section |

### Step 3: Extract Configuration
Read all configuration files completely:
- Every .env* file (extract all variables with values)
- docker-compose.yml if present (extract all service definitions)
- Dockerfiles if present (understand build process)

### Step 4: Verify Build Requirements
Use dir_build_plan on main application directories to understand build needs.

## DEPLOYMENT CONFIGURATION RULES

### Service Creation
Only create services that Step 2 verified:
- Verified PostgreSQL → create postgres service with image: ghcr.io/lttle-cloud/postgres:17-flash
- Verified Redis → create redis service with image: redis:alpine
- Verified MongoDB → create mongo service with image: mongo

### Environment Variables
From Step 3's extraction, transform variables:

| Variable Type | Transformation |
|--------------|---------------|
| Secrets (*_SECRET, *_KEY, *_TOKEN) | CopyFromEnvFile |
| Ports (PORT, *_PORT) | Literal with value |
| localhost URLs | Replace with service-port.{dir_name}.svc.lttle.local |
| Database URLs | Expression with interpolated connection string |

Each app needs its variables explicitly configured - nothing is automatic.

### Build Strategy Selection
- Has Dockerfile → BuildWithDockerfile
- No Dockerfile + standard structure → BuildAutomatically  
- Database/infrastructure → Image (use provided images)

### Snapshot Strategy
- Known port → SuspendAfterListenOnPort(port)
- Unknown port → SuspendAfterListenOnAnyPort
- lttle-optimized images → SuspendManually

## OUTPUT REQUIREMENTS

Generate GadgetInitData containing:
- apps: Array of services (only those verified in Step 2)
- volumes: Storage for stateful services
- issues: Blocking problems if any
- warnings: Missing expected configuration

## VALIDATION

Before generating final output, ensure:
- Every service was verified by reading a file
- Every environment variable was extracted from actual files
- Every service reference points to a deployed service
- Every app has all needed environment variables configured

Start with list_dir("/") now.
"#,
        dir_name = dir_name,
        providers = providers
    );

    Ok(prompt.trim().to_owned())
}

// Schema enforcement functions
fn enforce_openai_subset(schema: &mut serde_json::Value) {
    remove_property_format_value_from_json(schema);
    replace_one_of_by_any_of(schema);
    set_additional_properties_to_false(schema);
    enforce_all_required_properties(schema);
}

fn set_additional_properties_to_false(object: &mut serde_json::Value) {
    match object {
        serde_json::Value::Object(object) => {
            if object.get("type") == Some(&serde_json::Value::String("object".into())) {
                object.insert(
                    "additionalProperties".into(),
                    serde_json::Value::Bool(false),
                );
            }
            for value in object.values_mut() {
                set_additional_properties_to_false(value);
            }
        }
        serde_json::Value::Array(array) => {
            for value in array.iter_mut() {
                set_additional_properties_to_false(value);
            }
        }
        _ => {}
    }
}

fn enforce_all_required_properties(object: &mut serde_json::Value) {
    match object {
        serde_json::Value::Object(object) => {
            if let Some(properties) = object.get("properties").and_then(|p| p.as_object()) {
                if !properties.is_empty() {
                    let property_names: Vec<serde_json::Value> = properties
                        .keys()
                        .map(|key| serde_json::Value::String(key.to_string()))
                        .collect();

                    let required_array = object
                        .entry("required")
                        .or_insert_with(|| serde_json::Value::Array(vec![]));

                    if let Some(required) = required_array.as_array_mut() {
                        for property in property_names {
                            if !required.contains(&property) {
                                required.push(property);
                            }
                        }
                    }
                }
            }

            for value in object.values_mut() {
                enforce_all_required_properties(value);
            }
        }
        serde_json::Value::Array(array) => {
            for value in array.iter_mut() {
                enforce_all_required_properties(value);
            }
        }
        _ => {}
    }
}

fn replace_one_of_by_any_of(object: &mut serde_json::Value) {
    match object {
        serde_json::Value::Object(object) => {
            for key in ["oneOf", "allOf"] {
                if object.contains_key(key) {
                    if let Some(value) = object.remove(key) {
                        object.insert("anyOf".into(), value);
                    }
                }
            }
            for value in object.values_mut() {
                replace_one_of_by_any_of(value);
            }
        }
        serde_json::Value::Array(array) => {
            for value in array.iter_mut() {
                replace_one_of_by_any_of(value);
            }
        }
        _ => {}
    }
}

fn remove_property_format_value_from_json(object: &mut serde_json::Value) {
    match object {
        serde_json::Value::Object(object) => {
            for key in [
                "minLength",
                "maxLength", 
                "pattern",
                "format",
                "minimum",
                "maximum",
                "multipleOf",
                "patternProperties",
                "unevaluatedProperties",
                "propertyNames",
                "minProperties",
                "maxProperties",
                "unevaluatedItems",
                "contains",
                "minContains",
                "maxContains",
                "minItems",
                "maxItems",
                "uniqueItems",
            ] {
                object.remove(key);
            }
            for value in object.values_mut() {
                remove_property_format_value_from_json(value);
            }
        }
        serde_json::Value::Array(array) => {
            for value in array.iter_mut() {
                remove_property_format_value_from_json(value);
            }
        }
        _ => {}
    }
}