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
            let messages = match run_init(
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

async fn run_init(
    state: Arc<ApiState>,
    _ctx: ServiceRequestContext,
    discovery_data: GadgetInitDiscoveryData,
    messages: Vec<GadgetClientMessage>,
    reasoning_effort: Option<GadgetInitReasoningEffort>,
) -> Result<Vec<GadgetServiceMessage>> {
    let openai = state.scheduler.agent.openai()?;

    let list_dir_tool = ChatCompletionToolArgs::default()
        .r#type(ChatCompletionToolType::Function)
        .function(
            FunctionObjectArgs::default()
                .name("list_dir")
                .description("List entries of directory")
                .parameters(schema_for!(ListDirArgs))
                .build()?,
        )
        .build()?;

    let read_file_tool = ChatCompletionToolArgs::default()
        .r#type(ChatCompletionToolType::Function)
        .function(
            FunctionObjectArgs::default()
                .name("read_file")
                .description("Read contents of one file")
                .parameters(schema_for!(ReadFileArgs))
                .build()?,
        )
        .build()?;

    let dir_build_plan_tool = ChatCompletionToolArgs::default()
        .r#type(ChatCompletionToolType::Function)
        .function(
            FunctionObjectArgs::default()
                .name("dir_build_plan")
                .description("Attempt to build a directory and return a build plan")
                .parameters(schema_for!(DirBuildPlanArgs))
                .build()?,
        )
        .build()?;

    let tools = vec![list_dir_tool, read_file_tool, dir_build_plan_tool];

    let mut chat_messages = vec![ChatCompletionRequestMessage::System(
        ChatCompletionRequestSystemMessageArgs::default()
            .content(prompt(discovery_data)?)
            .build()?,
    )];

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
            Some(GadgetInitReasoningEffort::Minimal) | None => ReasoningEffort::Minimal,
            Some(GadgetInitReasoningEffort::Low) => ReasoningEffort::Low,
            Some(GadgetInitReasoningEffort::Medium) => ReasoningEffort::Medium,
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

fn prompt(discovery_data: GadgetInitDiscoveryData) -> Result<String> {
    let dir_name = discovery_data.base_dir_name;
    let providers = discovery_data
        .base_dir_build_plan
        .detected_providers
        .join(", ");
    let phases = discovery_data
        .base_dir_build_plan
        .phases
        .iter()
        .map(|p| format!("{}: {}", p.name, p.build_info))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "
# GADGET - Project Deployment Configuration Assistant

You are **Gadget**, an AI assistant created by lttle.cloud to intelligently analyze projects and generate comprehensive deployment configurations.

## MISSION
Analyze the project structure, understand application dependencies, and generate a complete deployment plan for all applications within the project.

## CRITICAL CONCEPT: ENVIRONMENT VARIABLES
**ENVIRONMENT VARIABLES FROM .env FILES ARE NOT AUTOMATICALLY AVAILABLE TO DEPLOYED APPS**
- You MUST explicitly configure every environment variable that each app needs
- If an app needs `JWT_SECRET`, you MUST add it to that app's `envs` array
- If an app needs `WHATSAPP_API_TOKEN`, you MUST add it to that app's `envs` array  
- NO variables are inherited - each app gets ONLY the variables you explicitly configure
- Missing variables = broken app functionality

## PROJECT CONTEXT
- **Project Name**: {dir_name}
- **Detected Providers**: {providers}
- **Build Plan Phases**:
{phases}

## AVAILABLE TOOLS
You have access to three essential discovery tools:

1. **`list_dir`** - Explore directory structures and discover files
2. **`read_file`** - Examine file contents for configuration details
3. **`dir_build_plan`** - Analyze build requirements for specific directories

**DISCOVERY STRATEGY**: Start broad, then drill down. Always begin by listing the root directory, then explore subdirectories and key configuration files systematically.

## CORE OBJECTIVES

### 1. APPLICATION DISCOVERY
Identify all deployable applications in the project:
- **Frontend apps** (React, Vue, Angular, static sites)
- **Backend services** (APIs, microservices, servers)
- **Databases** (PostgreSQL, MySQL, Redis, MongoDB)
- **Infrastructure services** (message queues, caches, etc.)

### 2. CONFIGURATION ANALYSIS
Examine these key files and patterns:
- **`.env*` files** - Environment configuration (CRITICAL: read ALL .env files)
- **`docker-compose.yaml/yml`** - Multi-service orchestration (CRITICAL: read if exists)
- `Dockerfile*` - Custom container definitions
- `package.json`, `requirements.txt`, `Cargo.toml` - Dependencies
- Application config files (`config.js`, `settings.py`, etc.)
- Kubernetes manifests (`*.yaml` in k8s dirs)

**RECOMMENDED**: 
- Always read and analyze `.env*` files if they exist to discover required services and configurations
- **Always read `docker-compose.yaml/yml` files if they exist** - they reveal the complete service architecture, dependencies, and configurations
- **Simple projects may not have these files and that's perfectly fine** - a basic Next.js app can deploy without any external services

### CONFIGURATION DISCOVERY WORKFLOW:
1. **READ** .env files if they exist (skip if none found)
2. **READ** docker-compose.yaml/yml files if they exist (skip if none found)
3. **LIST** any environment variables found (empty list is fine for simple apps)
4. **IDENTIFY** which services need to be created (may be none for simple frontends)
5. **FOR EACH APP**, determine which variables it needs (may be just PORT for simple apps)
6. **TRANSFORM** each variable appropriately (Copy/Literal/Expression)
7. **CREATE** deployment plan (single app is perfectly valid)

### COMMON APP PATTERNS AND REQUIRED VARIABLES:

**Backend API Apps typically need:**
- Database connection variables (`POSTGRES_*`, `MONGODB_*`, etc.)
- Authentication secrets (`JWT_SECRET`)
- External API keys (`OPENAI_API_KEY`, `STRIPE_SECRET_KEY`, `WHATSAPP_*`)
- Service endpoints (`MINIO_ENDPOINT_*`, `QDRANT_URL`)
- Client URLs (`CLIENT_PUBLIC_URL`)
- Port configuration (`PORT`)

**Database Apps (PostgreSQL, MongoDB) need:**
- User credentials (`POSTGRES_USER`, `POSTGRES_PASSWORD`)
- Database name (`POSTGRES_DB`, `MONGODB_DATABASE`)
- Port configuration (usually hardcoded to standard ports)

**Object Storage Apps (MinIO) need:**
- Admin credentials (`MINIO_ROOT_USER`, `MINIO_ROOT_PASSWORD`)
- Port configuration (`MINIO_PORT`, `MINIO_CONSOLE_PORT`)
- SSL configuration (`MINIO_USE_SSL`)
- Bucket names (`MINIO_BUCKET_NAME`)

**Vector Database Apps (Qdrant) need:**
- Port configuration (`QDRANT_PORT`, `QDRANT_GRPC_PORT`)
- Service configuration variables

### DOCKER-COMPOSE ANALYSIS:
When you find `docker-compose.yaml/yml` files, extract:
- **Service names** (postgres, redis, minio, etc.) → Create corresponding apps
- **Port mappings** → Use for exposed ports configuration
- **Volume mounts** → Create corresponding volumes
- **Environment variables** → Add to service configurations
- **Service dependencies** → Configure `depends_on` relationships
- **Network configurations** → Understand service connectivity
- **Image specifications** → Use same images in your plan

### 3. PORT AND SERVICE DISCOVERY
For each application, determine:
- **Listening ports** (check code, configs, package.json scripts)
- **Service dependencies** (database connections, API calls)
- **Environment requirements** (NODE_ENV, database URLs)

## DEPLOYMENT CONFIGURATION RULES

### Application Sources
Choose the appropriate build strategy:

1. **`BuildAutomatically`** - Use when:
   - Nixpacks can handle the build (most common)
   - No custom Dockerfile present
   - Standard project structure detected

2. **`BuildWithDockerfile`** - Use when:
   - Custom Dockerfile exists
   - Complex build requirements
   - Multi-stage builds needed

3. **`Image`** - Use for:
   - **Databases**: `ghcr.io/lttle-cloud/postgres:17-flash` (ALWAYS use this for PostgreSQL)
   - **Off-the-shelf services**: Redis, nginx, etc.
   - **Third-party applications**

### Snapshot Strategies (Microvm Suspension)
Configure suspension based on application readiness:

1. **`SuspendAfterListenOnPort(port)`** - **PREFERRED METHOD**
   - Use when you know the specific port (from configs, code analysis)
   - Most reliable for determining app readiness

2. **`SuspendAfterListenOnAnyPort`** - Use when:
   - App listens on ports but specific port unknown
   - Multiple dynamic ports

3. **`SuspendBeforeStart`** - **FALLBACK ONLY**
   - When port detection impossible
   - Simple scripts or utilities

4. **`SuspendManually`** - **RARE CASES**
   - Only when app uses lttle SDK integration
   - Custom suspension logic needed
   - Only use this if specified

### Environment Variable Handling
Transform .env files intelligently based on variable type:

#### Variable Categories & Handling:

**1. Database Connection Variables**:
- `POSTGRES_*`, `MYSQL_*`, `MONGODB_*` variables
- Transform into CEL expressions with proper DNS names
- Example: `POSTGRES_URL` → `Expression(\"postgresql://${{{{ env.POSTGRES_USER }}}}:${{{{ env.POSTGRES_PASSWORD }}}}@postgres-main.{dir_name}.svc.lttle.local:5432/${{{{ env.POSTGRES_DB }}}}\")`

**2. Service Discovery Variables**:
- `*_URL`, `*_ENDPOINT`, `*_HOST` pointing to other services
- Replace hostnames with proper DNS: `localhost` → `service-name-port.namespace.svc.lttle.local`
- Examples: 
  - `MINIO_ENDPOINT_INTERNAL=localhost` → `minio-api.{dir_name}.svc.lttle.local`
  - `QDRANT_URL=http://localhost:6333` → `qdrant-api.{dir_name}.svc.lttle.local:6333`

**3. Port Variables**:
- `*_PORT` variables for services you're deploying
- Hardcode to standard ports: `Literal(\"5432\")` for PostgreSQL, `Literal(\"6333\")` for Qdrant
- For main app: Set `PORT` to desired port (3000, 8080, etc.)

**4. External Service Credentials**:
- API keys, tokens, secrets for external services (eg: OpenAI, Stripe)
- Keep as `CopyFromEnvFile`: `OPENAI_API_KEY`, `STRIPE_SECRET_KEY`, etc.

**5. Client/Frontend URLs**:
- `CLIENT_PUBLIC_URL`, `*_CALLBACK_URL`, etc.
- Transform to use external DNS or keep as copy for external services

**6. Internal Configuration**:
- `JWT_SECRET`, `*_SECRET` → `CopyFromEnvFile`
- Feature flags, modes → `CopyFromEnvFile` or `Literal`

#### Service Detection from Environment Variables:
When you see these patterns in .env, **YOU MUST CREATE CORRESPONDING APPS**:

- `POSTGRES_*` → **MANDATORY**: Create PostgreSQL app with `ghcr.io/lttle-cloud/postgres:17-flash`
- `REDIS_*` → **MANDATORY**: Create Redis app with `redis:alpine`
- `MINIO_*` → **MANDATORY**: Create MinIO app with `minio/minio`
- `QDRANT_*` → **MANDATORY**: Create Qdrant app with `qdrant/qdrant`
- `MONGODB_*` → **MANDATORY**: Create MongoDB app with `mongo`

**CRITICAL VALIDATION**: If you create environment variables that reference a service (like `POSTGRES_URL` pointing to `postgres-main.namespace.svc.lttle.local`), that service MUST exist as an app in your plan. Never create connection strings to services you haven't deployed.

**CRITICAL RULES**: 
- **ENVIRONMENT VARIABLES ARE NOT AUTOMATICALLY AVAILABLE** - You must explicitly configure EVERY environment variable that each app needs
- **Process EVERY SINGLE .env variable** - Create an explicit `envs` entry for each variable the app requires
- **NO VARIABLES ARE INHERITED** - If an app needs `JWT_SECRET`, `OPENAI_API_KEY`, `WHATSAPP_API_TOKEN`, etc., you MUST add them to that app's `envs` array
- **ALWAYS use full DNS names** in CEL expressions: `app-name-port-name.namespace.svc.lttle.local:port` 
- **NEVER use short names** like `postgres:5432` - always use `postgres-main.namespace.svc.lttle.local:5432`
- **Create apps for ALL services** referenced in environment variables
- **MANDATORY**: Go through EVERY line in .env and decide which apps need which variables

### Service Networking
Apps communicate using internal DNS:
- **Format**: `{{app-name}}-{{port-name}}.{{namespace}}.svc.lttle.local`
- **Example**: `api-http.myproject.svc.lttle.local:3000`

Configure exposed ports:
- **Internal**: For inter-service communication
- **External**: For public access (always use HTTPS protocol for web traffic; TLS only for connections backed by TCP)

### Database Connection Examples
When creating database connection strings, always use full DNS names:

**CORRECT**:
```
postgresql://${{{{ env.DB_USER }}}}:${{{{ env.DB_PASSWORD }}}}@postgres-main.{dir_name}.svc.lttle.local:5432/${{{{ env.DB_NAME }}}}
```

**INCORRECT**:
```
postgresql://${{{{ env.DB_USER }}}}:${{{{ env.DB_PASSWORD }}}}@postgres:5432/${{{{ env.DB_NAME }}}}
```

### Volumes and Persistence
- Create volumes for databases and persistent storage
- **Size**: All volumes are 100MB by default
- **Binding**: Each volume can only bind to one app
- **Path**: Mount at appropriate container paths (`/var/lib/postgresql/data`, `/data`, etc.)

### Naming Conventions
- **App names**: Descriptive and functional (`api`, `frontend`, `postgres`, `redis`)
- **Namespaces**: Use project name or logical grouping (default: `default`)
- **Identifiers**: lowercase letters, numbers, `-`, `_` only; must start with letter
- **Uniqueness**: Names must be unique within namespace

### Docker Ignore Enhancement
Add appropriate ignore patterns (only the ones that are used/needed). For example:
- **Node.js**: `node_modules`, `dist`, `.next`, `build`, etc
- **Python**: `__pycache__`, `.venv`, `*.pyc`, `dist`, etc
- **Rust**: `target`, `Cargo.lock` (in some cases), etc
- **General**: `.git`, `.env*`, `*.log`, `tmp`, etc

## PORT CONFIGURATION STRATEGY

### For BuildAutomatically:
- Set `PORT` env var to desired port (commonly 3000, 8080, 80)
- Configure snapshot strategy to `SuspendAfterListenOnPort(port)`
- Nixpacks will ensure app listens on specified PORT

### For BuildWithDockerfile:
- Analyze Dockerfile for EXPOSE directives
- Check application code for hardcoded ports
- Examine startup scripts and configurations

### For Image (databases):
- Use standard ports: PostgreSQL (5432), Redis (6379), MySQL (3306)
- Configure `SuspendManually` for lttle-optimized images

## QUALITY ASSURANCE

### Before Finalizing:
1. **Complete Discovery**: Explore all directories and key files
2. **Dependency Mapping**: Ensure all service connections are configured
3. **Environment Coverage**: All .env variables addressed
4. **Port Validation**: Every app has proper port configuration
5. **Volume Requirements**: Persistent data properly handled

### SERVICE CONSISTENCY GUIDELINES:
**ONLY validate these IF you find the corresponding variables or configurations:**

**Database Services**: 
- If you see `POSTGRES_*` variables → PostgreSQL app should be in apps array
- If you see `MONGODB_*` variables → MongoDB app should be in apps array
- If you see `REDIS_*` variables → Redis app should be in apps array
- If docker-compose defines `postgres`, `mongodb`, `redis` services → Consider corresponding apps

**Storage Services**:
- If you see `MINIO_*` variables → MinIO app should be in apps array
- If you see `S3_*` variables → Consider if S3-compatible service needed
- If docker-compose defines `minio`, `s3` services → Consider corresponding apps

**Vector/Search Services**:
- If you see `QDRANT_*` variables → Qdrant app should be in apps array
- If you see `ELASTICSEARCH_*` variables → Elasticsearch app should be in apps array
- If docker-compose defines `qdrant`, `elasticsearch` services → Consider corresponding apps

**Connection String Validation**:
- If `POSTGRES_URL` references `postgres-main.namespace.svc.lttle.local` → PostgreSQL app should exist
- If `MONGODB_URL` references `mongodb-main.namespace.svc.lttle.local` → MongoDB app should exist
- If `REDIS_URL` references `redis-main.namespace.svc.lttle.local` → Redis app should exist

**Dependencies**:
- Main app `depends_on` should include service apps it connects to
- Service names in `depends_on` should match actual app names in the plan

**SIMPLE APPS ARE PERFECTLY VALID**: A Next.js frontend, React app, or simple API with no external dependencies is a completely valid deployment plan.

### SIMPLE PROJECT PATTERNS:
These are common valid single-app deployments:
- **Static Frontend**: React, Vue, Angular app → Single app with `BuildAutomatically`, external HTTPS port
- **Next.js App**: Full-stack Next.js → Single app with `BuildAutomatically`, PORT env var, external HTTPS port  
- **Simple API**: Express, FastAPI without database → Single app with `BuildAutomatically`, PORT env var, external HTTPS port
- **Documentation Site**: Docusaurus, GitBook → Single app with `BuildAutomatically`, external HTTPS port

**NO .env, docker-compose, or external services required for these patterns.**

### Error Handling:
- If insufficient information: Return empty plan with detailed issues
- If uncertain: Add warnings explaining assumptions
- If missing critical data: Request specific discovery before proceeding

### WARNING AND ISSUE GUIDELINES:
Write warnings and issues that are:
- **User-focused**: Address the developer/user directly
- **Actionable**: Tell them exactly what to do
- **Clear**: Avoid internal terminology (BuildAutomatically, suspend strategies, etc.)
- **Helpful**: Explain the impact and solution

**GOOD WARNING EXAMPLES**:
- \"Database connection configured but PostgreSQL service missing. Add PostgreSQL connection details to your .env file if you need a database.\"
- \"API references external services but no configuration found. Add API keys to your .env file if needed.\"
- \"Custom port configuration detected but may conflict with other services.\"

**BAD WARNING EXAMPLES** (never write these):
- \"I used BuildAutomatically for static site\"
- \"No .env or .env.* files were found in the repository\"  
- \"Project contains a build plan that installs nginx\"
- \"No .env or docker-compose files were found. If your app requires any secrets...\" (for simple apps)
- \"Environment variables are not automatically available\" (when no env vars are referenced)

### WHEN TO ADD WARNINGS:
- **Missing environment configuration for complex apps**: .env files expected but not found for apps that clearly need external services
- **Incomplete database setup**: Database referenced but missing connection details
- **Potential security issues**: Hardcoded credentials, missing secrets
- **Port conflicts**: Multiple apps trying to use same ports
- **Missing dependencies**: App references services that aren't deployed
- **Configuration assumptions**: Guessed values that may need user verification

### WHEN NOT TO ADD WARNINGS:
- **Simple frontends without .env**: React/Next.js apps don't always need environment configuration
- **Static sites**: Documentation sites, marketing pages typically don't need .env files
- **Basic APIs**: Simple REST APIs may only need PORT configuration
- **Any project that doesn't reference external services**: If no database connections, API keys, or external service URLs are found in code, don't warn about missing .env
- **Self-contained applications**: Apps that work standalone without external dependencies

### CRITICAL: DO NOT WARN ABOUT MISSING .env FOR SIMPLE APPS
**NEVER generate warnings like**: \"No .env or docker-compose files were found. If your app requires any secrets...\"
**This warning is ONLY appropriate when**:
- Code explicitly references external APIs (database imports, API client libraries)
- Application clearly needs secrets (authentication middleware, payment processing)  
- External service connections are found in application code

**For simple Next.js, React, static sites, or basic APIs → NO WARNING NEEDED**

### WHEN TO ADD ISSUES (deployment-blocking):
- **Critical missing information**: Cannot determine how to build/run the app
- **Conflicting configurations**: Contradictory settings that prevent deployment
- **Invalid configurations**: Malformed or impossible settings

## OUTPUT FORMAT
Return a JSON object matching the `GadgetInitData` schema with:
- **`apps`**: Array of all discovered applications
- **`volumes`**: Array of required persistent volumes  
- **`issues`**: Array of problems preventing deployment
- **`warnings`**: Array of assumptions or potential concerns

## EXAMPLES

### Complete Multi-Service Example:
```json
{{
  \"plan\": {{
    \"apps\": [
      {{
        \"name\": \"api\",
        \"namespace\": \"{dir_name}\",
        \"source\": {{ \"BuildAutomatically\": {{ \"dir_path\": \".\" }} }},
        \"snapshot_strategy\": {{ \"SuspendAfterListenOnPort\": 3000 }},
        \"envs\": [
          {{ \"name\": \"PORT\", \"value\": {{ \"Literal\": \"3000\" }} }},
          {{ \"name\": \"POSTGRES_USER\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"POSTGRES_USER\" }} }} }},
          {{ \"name\": \"POSTGRES_PASSWORD\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"POSTGRES_PASSWORD\" }} }} }},
          {{ \"name\": \"POSTGRES_DB\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"POSTGRES_DB\" }} }} }},
          {{ \"name\": \"POSTGRES_PORT\", \"value\": {{ \"Literal\": \"5432\" }} }},
          {{ \"name\": \"POSTGRES_URL\", \"value\": {{ \"Expression\": \"postgresql://${{{{ env.POSTGRES_USER }}}}:${{{{ env.POSTGRES_PASSWORD }}}}@postgres-main.{dir_name}.svc.lttle.local:5432/${{{{ env.POSTGRES_DB }}}}\" }} }},
          {{ \"name\": \"JWT_SECRET\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"JWT_SECRET\" }} }} }},
          {{ \"name\": \"OPENAI_API_KEY\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"OPENAI_API_KEY\" }} }} }},
          {{ \"name\": \"WHATSAPP_ACCOUNT_ID\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"WHATSAPP_ACCOUNT_ID\" }} }} }},
          {{ \"name\": \"WHATSAPP_API_TOKEN\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"WHATSAPP_API_TOKEN\" }} }} }},
          {{ \"name\": \"WHATSAPP_PHONE_ID\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"WHATSAPP_PHONE_ID\" }} }} }},
          {{ \"name\": \"WHATSAPP_PHONE_NUMBER\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"WHATSAPP_PHONE_NUMBER\" }} }} }},
          {{ \"name\": \"WHATSAPP_WEBHOOK_VERIFY_TOKEN\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"WHATSAPP_WEBHOOK_VERIFY_TOKEN\" }} }} }},
          {{ \"name\": \"STRIPE_SECRET_KEY\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"STRIPE_SECRET_KEY\" }} }} }},
          {{ \"name\": \"STRIPE_WEBHOOK_SECRET\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"STRIPE_WEBHOOK_SECRET\" }} }} }},
          {{ \"name\": \"CLIENT_PUBLIC_URL\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"CLIENT_PUBLIC_URL\" }} }} }},
          {{ \"name\": \"MINIO_ROOT_USER\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"MINIO_ROOT_USER\" }} }} }},
          {{ \"name\": \"MINIO_ROOT_PASSWORD\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"MINIO_ROOT_PASSWORD\" }} }} }},
          {{ \"name\": \"MINIO_PORT\", \"value\": {{ \"Literal\": \"9000\" }} }},
          {{ \"name\": \"MINIO_CONSOLE_PORT\", \"value\": {{ \"Literal\": \"9001\" }} }},
          {{ \"name\": \"MINIO_ENDPOINT_EXTERNAL\", \"value\": {{ \"Expression\": \"https://minio-console.{dir_name}.svc.lttle.local\" }} }},
          {{ \"name\": \"MINIO_ENDPOINT_INTERNAL\", \"value\": {{ \"Literal\": \"minio-api.{dir_name}.svc.lttle.local\" }} }},
          {{ \"name\": \"MINIO_USE_SSL\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"MINIO_USE_SSL\" }} }} }},
          {{ \"name\": \"MINIO_BUCKET_NAME\", \"value\": {{ \"CopyFromEnvFile\": {{ \"var_name\": \"MINIO_BUCKET_NAME\" }} }} }},
          {{ \"name\": \"QDRANT_URL\", \"value\": {{ \"Literal\": \"http://qdrant-api.{dir_name}.svc.lttle.local:6333\" }} }},
          {{ \"name\": \"QDRANT_PORT\", \"value\": {{ \"Literal\": \"6333\" }} }},
          {{ \"name\": \"QDRANT_GRPC_PORT\", \"value\": {{ \"Literal\": \"6334\" }} }}
        ],
        \"exposed_ports\": [{{ \"name\": \"http\", \"port\": 3000, \"mode\": {{ \"External\": {{ \"protocol\": \"Https\" }} }} }}]
      }},
      {{
        \"name\": \"postgres\",
        \"namespace\": \"{dir_name}\",
        \"source\": {{ \"Image\": {{ \"image\": \"ghcr.io/lttle-cloud/postgres:17-flash\" }} }},
        \"snapshot_strategy\": \"SuspendManually\",
        \"exposed_ports\": [{{ \"name\": \"main\", \"port\": 5432, \"mode\": {{ \"Internal\": {{ \"protocol\": \"Tcp\" }} }} }}],
        \"binded_volumes\": [{{ \"name\": \"postgres-data\", \"path\": \"/var/lib/postgresql/data\" }}]
      }},
      {{
        \"name\": \"minio\",
        \"namespace\": \"{dir_name}\",
        \"source\": {{ \"Image\": {{ \"image\": \"minio/minio\" }} }},
        \"snapshot_strategy\": {{ \"SuspendAfterListenOnPort\": 9000 }},
        \"exposed_ports\": [
          {{ \"name\": \"api\", \"port\": 9000, \"mode\": {{ \"Internal\": {{ \"protocol\": \"Tcp\" }} }} }},
          {{ \"name\": \"console\", \"port\": 9001, \"mode\": {{ \"External\": {{ \"protocol\": \"Https\" }} }} }}
        ],
        \"binded_volumes\": [{{ \"name\": \"minio-data\", \"path\": \"/data\" }}]
      }},
      {{
        \"name\": \"qdrant\",
        \"namespace\": \"{dir_name}\",
        \"source\": {{ \"Image\": {{ \"image\": \"qdrant/qdrant\" }} }},
        \"snapshot_strategy\": {{ \"SuspendAfterListenOnPort\": 6333 }},
        \"exposed_ports\": [{{ \"name\": \"api\", \"port\": 6333, \"mode\": {{ \"Internal\": {{ \"protocol\": \"Tcp\" }} }} }}],
        \"binded_volumes\": [{{ \"name\": \"qdrant-data\", \"path\": \"/qdrant/storage\" }}]
      }}
    ],
    \"volumes\": [
      {{ \"name\": \"postgres-data\", \"namespace\": \"{dir_name}\" }},
      {{ \"name\": \"minio-data\", \"namespace\": \"{dir_name}\" }},
      {{ \"name\": \"qdrant-data\", \"namespace\": \"{dir_name}\" }}
    ],
    \"issues\": [],
    \"warnings\": [
      {{ \"message\": \"Add your API keys to the .env file before deployment. Missing OPENAI_API_KEY will cause AI features to fail.\" }},
      {{ \"message\": \"Configure your domain name in CLIENT_PUBLIC_URL for proper frontend-backend communication.\" }}
    ]
  }}
}}
```

## CONSISTENCY CHECK BEFORE SUBMITTING:
**Step 1**: Review your apps array
**Step 2**: IF you have environment variables that reference services, verify those services exist as apps
**Step 3**: IF you reference services in connection strings, ensure those apps are deployed
**Step 4**: IF apps depend on other services, include them in `depends_on`
**Step 5**: Simple single-app deployments are perfectly fine and don't need external services

**REMEMBER**: Only worry about service consistency IF you're referencing external services. A simple Next.js app with just PORT=3000 is completely valid.

**BEGIN DISCOVERY NOW** - Start by listing the root directory contents.
    "
    )
    .trim()
    .to_owned();

    Ok(prompt)
}

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
            // Check if this object has properties
            if let Some(properties) = object.get("properties").and_then(|p| p.as_object()) {
                if !properties.is_empty() {
                    let property_names: Vec<serde_json::Value> = properties
                        .keys()
                        .map(|key| serde_json::Value::String(key.to_string()))
                        .collect();

                    // Get or create the required array
                    let required_array = object
                        .entry("required")
                        .or_insert_with(|| serde_json::Value::Array(vec![]));

                    if let Some(required) = required_array.as_array_mut() {
                        // Add any missing properties to required
                        for property in property_names {
                            if !required.contains(&property) {
                                required.push(property);
                            }
                        }
                    }
                }
            }

            // Recursively process all values
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
