//! Local HTTP API for the desktop-first runtime.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;
use uuid::Uuid;

use crate::agent::SessionManager;
use crate::agent::submission::Submission;
use crate::channels::IncomingMessage;
use crate::config::{EmbeddingsConfig, LlmConfig, hydrate_llm_keys_from_secrets};
use crate::db::Database;
use crate::history::{ConversationMessage, ConversationSummary};
use crate::llm::{RuntimeLlmReloader, registry::ProviderRegistry};
use crate::runtime_events::SseManager;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::settings::Settings;
use crate::task_runtime::{TaskDetail, TaskMode, TaskRecord, TaskRuntime, TaskStatus};
use crate::tools::mcp::config::{EffectiveTransport, load_mcp_servers_from_db};
use crate::workspace::{SearchResult, Workspace, WorkspaceEntry};

pub const DEFAULT_API_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const DEFAULT_API_PORT: u16 = 8765;

#[derive(Clone)]
pub struct ApiState {
    owner_id: String,
    bind_addr: SocketAddr,
    store: Arc<dyn Database>,
    sse_manager: Arc<SseManager>,
    task_runtime: Option<Arc<TaskRuntime>>,
    inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    session_manager: Option<Arc<SessionManager>>,
    workspace: Option<Arc<Workspace>>,
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    llm_reloader: Option<Arc<RuntimeLlmReloader>>,
    tool_count: usize,
    dev_loaded_tool_names: Vec<String>,
    workspace_index_jobs: Arc<tokio::sync::RwLock<HashMap<Uuid, WorkspaceIndexJobResponse>>>,
}

impl ApiState {
    pub fn new(
        owner_id: String,
        bind_addr: SocketAddr,
        store: Arc<dyn Database>,
        sse_manager: Arc<SseManager>,
        task_runtime: Option<Arc<TaskRuntime>>,
        inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
        session_manager: Option<Arc<SessionManager>>,
        workspace: Option<Arc<Workspace>>,
    ) -> Self {
        Self {
            owner_id,
            bind_addr,
            store,
            sse_manager,
            task_runtime,
            inject_tx,
            session_manager,
            workspace,
            secrets_store: None,
            llm_reloader: None,
            tool_count: 0,
            dev_loaded_tool_names: Vec::new(),
            workspace_index_jobs: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    pub fn with_secrets_store(
        mut self,
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    ) -> Self {
        self.secrets_store = Some(secrets_store);
        self
    }

    pub fn with_llm_reloader(mut self, llm_reloader: Arc<RuntimeLlmReloader>) -> Self {
        self.llm_reloader = Some(llm_reloader);
        self
    }

    pub fn with_workbench_metadata(
        mut self,
        tool_count: usize,
        dev_loaded_tool_names: Vec<String>,
    ) -> Self {
        self.tool_count = tool_count;
        self.dev_loaded_tool_names = dev_loaded_tool_names;
        self
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    bind: String,
    owner_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsResponse {
    pub llm_backend: Option<String>,
    pub selected_model: Option<String>,
    pub ollama_base_url: Option<String>,
    pub openai_compatible_base_url: Option<String>,
    pub llm_custom_providers: Vec<crate::settings::CustomLlmProviderSettings>,
    pub llm_builtin_overrides:
        std::collections::HashMap<String, crate::settings::LlmBuiltinOverride>,
    pub llm_ready: bool,
    pub llm_onboarding_required: bool,
    pub llm_readiness_error: Option<String>,
}

impl SettingsResponse {
    fn from_settings(value: Settings) -> Self {
        let readiness = llm_readiness(&value);
        Self {
            llm_backend: value.llm_backend,
            selected_model: value.selected_model,
            ollama_base_url: value.ollama_base_url,
            openai_compatible_base_url: value.openai_compatible_base_url,
            llm_custom_providers: value.llm_custom_providers,
            llm_builtin_overrides: value.llm_builtin_overrides,
            llm_ready: readiness.is_ok(),
            llm_onboarding_required: readiness.is_err(),
            llm_readiness_error: readiness.err(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct PatchSettingsRequest {
    pub llm_backend: Option<String>,
    pub selected_model: Option<String>,
    pub ollama_base_url: Option<String>,
    pub openai_compatible_base_url: Option<String>,
    pub llm_custom_providers: Option<Vec<crate::settings::CustomLlmProviderSettings>>,
    pub llm_builtin_overrides:
        Option<std::collections::HashMap<String, crate::settings::LlmBuiltinOverride>>,
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    field_errors: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskRecord>,
}

#[derive(Debug, Serialize)]
pub struct TaskDetailResponse {
    pub task: TaskRecord,
    pub timeline: Vec<crate::task_runtime::TaskTimelineEntry>,
}

#[derive(Debug, Serialize)]
struct StreamEnvelope {
    event: String,
    thread_id: String,
    correlation_id: String,
    sequence: u64,
    timestamp: String,
    payload: Value,
}

#[derive(Debug, Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionSummaryResponse>,
}

#[derive(Debug, Serialize)]
pub struct SessionSummaryResponse {
    pub id: Uuid,
    pub title: String,
    pub message_count: i64,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_activity: chrono::DateTime<chrono::Utc>,
    pub thread_type: Option<String>,
    pub channel: String,
}

#[derive(Debug, Serialize)]
pub struct SessionMessageResponse {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct SessionDetailResponse {
    pub session: SessionSummaryResponse,
    pub messages: Vec<SessionMessageResponse>,
    pub current_task: Option<TaskRecord>,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SendSessionMessageRequest {
    pub content: String,
    pub mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SendSessionMessageResponse {
    pub accepted: bool,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub task: Option<TaskRecord>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceIndexRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceIndexResponse {
    pub job: WorkspaceIndexJobResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceIndexJobResponse {
    pub id: Uuid,
    pub path: String,
    pub import_root: String,
    pub manifest_path: String,
    pub status: String,
    pub phase: String,
    pub total_files: usize,
    pub processed_files: usize,
    pub indexed_files: usize,
    pub skipped_files: usize,
    pub error: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WorkspaceTreeQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceTreeResponse {
    pub path: String,
    pub entries: Vec<WorkspaceEntry>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceSearchRequest {
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceSearchResponse {
    pub results: Vec<WorkspaceSearchResultResponse>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceSearchResultResponse {
    pub document_id: Uuid,
    pub document_path: String,
    pub source_path: Option<String>,
    pub chunk_id: Uuid,
    pub content: String,
    pub score: f32,
    pub fts_rank: Option<u32>,
    pub vector_rank: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct WorkbenchCapabilitiesResponse {
    pub workspace_available: bool,
    pub tool_count: usize,
    pub dev_loaded_tools: Vec<String>,
    pub mcp_servers: Vec<WorkbenchMcpServerResponse>,
}

#[derive(Debug, Serialize)]
pub struct WorkbenchMcpServerResponse {
    pub name: String,
    pub transport: String,
    pub enabled: bool,
    pub auth_mode: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApproveTaskRequest {
    pub approval_id: Option<Uuid>,
    #[serde(default)]
    pub always: bool,
}

#[derive(Debug, Deserialize)]
pub struct PatchTaskModeRequest {
    pub mode: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct RejectTaskRequest {
    pub approval_id: Option<Uuid>,
    pub reason: Option<String>,
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    field_errors: Option<HashMap<String, String>>,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            field_errors: None,
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            field_errors: None,
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
            field_errors: None,
        }
    }

    fn unprocessable_entity(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: message.into(),
            field_errors: None,
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            field_errors: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ApiErrorBody {
                error: self.message,
                field_errors: self.field_errors,
            }),
        )
            .into_response()
    }
}

pub fn router(state: ApiState) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::DELETE, Method::GET, Method::PATCH, Method::POST])
        .allow_headers([header::CONTENT_TYPE])
        .allow_origin(AllowOrigin::predicate(
            |origin: &HeaderValue, _| match origin.to_str() {
                Ok(origin) => {
                    origin.starts_with("http://127.0.0.1:")
                        || origin.starts_with("http://localhost:")
                        || origin == "http://127.0.0.1"
                        || origin == "http://localhost"
                        || origin.starts_with("tauri://")
                        || origin.starts_with("app://")
                        || origin.starts_with("http://tauri.localhost")
                        || origin.starts_with("https://tauri.localhost")
                }
                Err(_) => false,
            },
        ));

    Router::new()
        .route("/api/v0/health", get(get_health))
        .route("/api/v0/settings", get(get_settings).patch(patch_settings))
        .route("/api/v0/events", get(get_events))
        .route("/api/v0/sessions", get(list_sessions).post(create_session))
        .route("/api/v0/sessions/{id}", get(get_session))
        .route("/api/v0/sessions/{id}/messages", post(post_session_message))
        .route("/api/v0/sessions/{id}/stream", get(stream_session))
        .route("/api/v0/tasks", get(list_tasks))
        .route("/api/v0/tasks/{id}", get(get_task).delete(delete_task))
        .route("/api/v0/tasks/{id}/stream", get(stream_task))
        .route("/api/v0/tasks/{id}/approve", post(approve_task))
        .route("/api/v0/tasks/{id}/reject", post(reject_task))
        .route("/api/v0/tasks/{id}/mode", patch(patch_task_mode))
        .route("/api/v0/runs", get(list_tasks))
        .route("/api/v0/runs/{id}", get(get_task).delete(delete_task))
        .route("/api/v0/runs/{id}/stream", get(stream_task))
        .route("/api/v0/runs/{id}/approve", post(approve_task))
        .route("/api/v0/runs/{id}/reject", post(reject_task))
        .route("/api/v0/runs/{id}/mode", patch(patch_task_mode))
        .route(
            "/api/v0/workbench/capabilities",
            get(get_workbench_capabilities),
        )
        .route("/api/v0/workspace/index", post(index_workspace))
        .route("/api/v0/workspace/index/{id}", get(get_workspace_index_job))
        .route("/api/v0/workspace/tree", get(get_workspace_tree))
        .route("/api/v0/workspace/search", post(search_workspace))
        .fallback_service(ServeDir::new("static").append_index_html_on_directories(true))
        .with_state(state)
        .layer(cors)
}

pub fn local_api_addr(port: u16) -> SocketAddr {
    SocketAddr::new(DEFAULT_API_HOST, port)
}

pub async fn run_api(bind_addr: SocketAddr, state: ApiState) -> anyhow::Result<()> {
    if bind_addr.ip() != DEFAULT_API_HOST {
        anyhow::bail!(
            "refusing to bind API to {}; Phase 1 only allows 127.0.0.1",
            bind_addr
        );
    }

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    tracing::info!(bind = %bind_addr, "starting local api service");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

async fn get_health(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        bind: state.bind_addr.to_string(),
        owner_id: state.owner_id,
    })
}

async fn get_settings(State(state): State<ApiState>) -> ApiResult<Json<SettingsResponse>> {
    let settings = load_settings(&state).await?;
    Ok(Json(SettingsResponse::from_settings(settings)))
}

async fn patch_settings(
    State(state): State<ApiState>,
    Json(payload): Json<PatchSettingsRequest>,
) -> ApiResult<Json<SettingsResponse>> {
    validate_settings_patch(&payload)?;

    let mut settings = load_settings(&state).await?;
    apply_settings_patch(&mut settings, payload);
    validate_resolved_settings(&settings)?;
    persist_llm_provider_secrets(&state, &mut settings).await?;

    state
        .store
        .set_all_settings(&state.owner_id, &settings.to_db_map())
        .await
        .map_err(|e| ApiError::internal(format!("failed to persist settings: {e}")))?;

    state.sse_manager.broadcast_for_user(
        &state.owner_id,
        ironclaw_common::AppEvent::Status {
            message: "settings.updated".to_string(),
            thread_id: None,
        },
    );

    if let Some(secrets) = state.secrets_store.as_ref() {
        hydrate_llm_keys_from_secrets(&mut settings, secrets.as_ref(), &state.owner_id).await;
    }

    if let Some(reloader) = state.llm_reloader.as_ref() {
        reloader.reload_from_settings(&settings).await.map_err(|error| {
            ApiError::internal(format!("failed to reload runtime model provider: {error}"))
        })?;
    }

    Ok(Json(SettingsResponse::from_settings(settings)))
}

async fn get_events(
    State(state): State<ApiState>,
) -> Result<
    axum::response::sse::Sse<
        impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>
        + Send
        + 'static,
    >,
    StatusCode,
> {
    state
        .sse_manager
        .subscribe(Some(state.owner_id))
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)
}

async fn list_sessions(State(state): State<ApiState>) -> ApiResult<Json<SessionListResponse>> {
    let sessions = state
        .store
        .list_conversations_all_channels(&state.owner_id, 100)
        .await
        .map_err(|e| ApiError::internal(format!("failed to list sessions: {e}")))?;
    Ok(Json(SessionListResponse {
        sessions: sessions
            .into_iter()
            .map(SessionSummaryResponse::from)
            .collect(),
    }))
}

async fn create_session(
    State(state): State<ApiState>,
    payload: Option<Json<CreateSessionRequest>>,
) -> ApiResult<Json<CreateSessionResponse>> {
    let session_id = Uuid::new_v4();
    let session_manager = state
        .session_manager
        .as_ref()
        .ok_or_else(|| ApiError::conflict("session manager is not available"))?;
    let external_id = session_id.to_string();
    session_manager
        .create_bound_thread(&state.owner_id, "api", &external_id, session_id)
        .await;
    state
        .store
        .ensure_conversation(session_id, "api", &state.owner_id, Some(&external_id))
        .await
        .map_err(|e| ApiError::internal(format!("failed to create session: {e}")))?;
    if let Some(Json(body)) = payload
        && let Some(title) = body.title
        && !title.trim().is_empty()
    {
        state
            .store
            .update_conversation_metadata_field(
                session_id,
                "title",
                &serde_json::Value::String(title.trim().to_string()),
            )
            .await
            .map_err(|e| ApiError::internal(format!("failed to persist session title: {e}")))?;
    }
    Ok(Json(CreateSessionResponse { id: session_id }))
}

async fn get_session(
    State(state): State<ApiState>,
    Path(session_id): Path<Uuid>,
) -> ApiResult<Json<SessionDetailResponse>> {
    ensure_session_belongs_to_user(&state, session_id).await?;
    let summary = load_session_summary(&state, session_id).await?;
    let messages = state
        .store
        .list_conversation_messages(session_id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load session messages: {e}")))?;
    Ok(Json(SessionDetailResponse {
        session: summary,
        messages: messages
            .into_iter()
            .map(SessionMessageResponse::from)
            .collect(),
        current_task: load_session_task(&state, session_id).await,
    }))
}

async fn post_session_message(
    State(state): State<ApiState>,
    Path(session_id): Path<Uuid>,
    Json(payload): Json<SendSessionMessageRequest>,
) -> ApiResult<Json<SendSessionMessageResponse>> {
    ensure_session_belongs_to_user(&state, session_id).await?;
    let inject_tx = state
        .inject_tx
        .as_ref()
        .ok_or_else(|| ApiError::conflict("session injection is not available"))?;
    if payload.content.trim().is_empty() {
        return Err(ApiError::bad_request("message content cannot be empty"));
    }
    let requested_mode = parse_requested_task_mode(payload.mode.as_deref())?;

    let message = IncomingMessage::new("api", &state.owner_id, payload.content)
        .with_owner_id(state.owner_id.clone())
        .with_sender_id(state.owner_id.clone())
        .with_thread(session_id.to_string())
        .with_metadata(serde_json::json!({
            "source": "api",
            "surface": "web"
        }))
        .with_timezone("UTC");

    let task = if let Some(runtime) = state.task_runtime.as_ref() {
        let mut task = runtime.ensure_task(&message, session_id).await;
        if let Some(mode) = requested_mode
            && task.mode != mode
            && let Some(updated) = runtime.toggle_mode(session_id, mode).await
        {
            task = updated;
        }
        Some(task)
    } else {
        None
    };

    inject_tx
        .send(message)
        .await
        .map_err(|e| ApiError::internal(format!("failed to enqueue session message: {e}")))?;

    Ok(Json(SendSessionMessageResponse {
        accepted: true,
        session_id,
        task_id: task.as_ref().map(|task| task.id),
        task,
    }))
}

async fn stream_session(
    State(state): State<ApiState>,
    Path(session_id): Path<Uuid>,
) -> ApiResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>> + Send + 'static>> {
    ensure_session_belongs_to_user(&state, session_id).await?;
    let thread_id = session_id.to_string();
    let filter_thread_id = thread_id.clone();
    let stream = state
        .sse_manager
        .subscribe_raw(Some(state.owner_id.clone()))
        .ok_or_else(|| ApiError::conflict("sse capacity reached"))?
        .filter(move |event| {
            futures::future::ready(event_thread_id(event) == Some(filter_thread_id.as_str()))
        })
        .filter_map(move |event| {
            let thread_id = thread_id.clone();
            async move { normalize_session_event(thread_id, event) }
        })
        .enumerate()
        .filter_map(|(idx, (thread_id, normalized))| async move {
            serialize_stream_envelope(thread_id, idx as u64 + 1, normalized)
        });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(30)).text("")))
}

async fn stream_task(
    State(state): State<ApiState>,
    Path(task_id): Path<Uuid>,
) -> ApiResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>> + Send + 'static>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    let current_task = runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;

    let runtime = Arc::clone(runtime);
    let thread_id = task_id.to_string();
    let initial_thread_id = thread_id.clone();
    let filter_thread_id = thread_id.clone();
    let initial = futures::stream::once(async move {
        (
            initial_thread_id,
            ("task.created".to_string(), json!({ "task": current_task })),
        )
    });

    let live = state
        .sse_manager
        .subscribe_raw(Some(state.owner_id.clone()))
        .ok_or_else(|| ApiError::conflict("sse capacity reached"))?
        .filter(move |event| {
            futures::future::ready(event_thread_id(event) == Some(filter_thread_id.as_str()))
        })
        .filter_map(move |event| {
            let runtime = Arc::clone(&runtime);
            async move { normalize_task_event(task_id, runtime, event).await }
        });

    let stream =
        initial
            .chain(live)
            .enumerate()
            .filter_map(|(idx, (thread_id, normalized))| async move {
                serialize_stream_envelope(thread_id, idx as u64 + 1, normalized)
            });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(30)).text("")))
}

async fn list_tasks(State(state): State<ApiState>) -> ApiResult<Json<TaskListResponse>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    Ok(Json(TaskListResponse {
        tasks: runtime.list_tasks().await,
    }))
}

async fn get_task(
    State(state): State<ApiState>,
    Path(task_id): Path<Uuid>,
) -> ApiResult<Json<TaskDetailResponse>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    let detail = runtime
        .get_task_detail(task_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;
    Ok(Json(TaskDetailResponse::from(detail)))
}

async fn approve_task(
    State(state): State<ApiState>,
    Path(task_id): Path<Uuid>,
    Json(payload): Json<ApproveTaskRequest>,
) -> ApiResult<Json<TaskRecord>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    let task = runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;
    if task.status != TaskStatus::WaitingApproval {
        return Err(ApiError::conflict(format!(
            "task {task_id} is not waiting for approval"
        )));
    }

    let pending = task
        .pending_approval
        .as_ref()
        .ok_or_else(|| ApiError::conflict(format!("task {task_id} has no pending approval")))?;
    let approval_id = payload.approval_id.unwrap_or(pending.id);
    if approval_id != pending.id {
        return Err(ApiError::conflict(
            "approval_id does not match current checkpoint",
        ));
    }
    tracing::info!(
        task_id = %task_id,
        correlation_id = %task.correlation_id,
        approval_id = %approval_id,
        always = payload.always,
        "task approval accepted through api"
    );

    let inject_tx = state
        .inject_tx
        .as_ref()
        .ok_or_else(|| ApiError::conflict("approval injection is not available"))?;

    inject_approval(
        inject_tx,
        &task,
        Submission::ExecApproval {
            request_id: approval_id,
            approved: true,
            always: payload.always,
        },
    )
    .await?;

    Ok(Json(runtime.get_task(task_id).await.ok_or_else(|| {
        ApiError::not_found(format!("task {task_id} not found"))
    })?))
}

async fn reject_task(
    State(state): State<ApiState>,
    Path(task_id): Path<Uuid>,
    Json(payload): Json<RejectTaskRequest>,
) -> ApiResult<Json<TaskRecord>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    let task = runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;

    if task.status != TaskStatus::WaitingApproval {
        return Err(ApiError::conflict(format!(
            "task {task_id} is not waiting for approval"
        )));
    }

    let pending = task
        .pending_approval
        .as_ref()
        .ok_or_else(|| ApiError::conflict(format!("task {task_id} has no pending approval")))?;
    if let Some(approval_id) = payload.approval_id {
        if approval_id != pending.id {
            return Err(ApiError::conflict(
                "approval_id does not match current checkpoint",
            ));
        }
    }

    let reason = payload
        .reason
        .unwrap_or_else(|| "rejected by user".to_string());
    tracing::info!(
        task_id = %task_id,
        correlation_id = %task.correlation_id,
        approval_id = %pending.id,
        reason,
        "task approval rejected through api"
    );
    runtime.mark_rejected(task_id, &reason).await;

    state.sse_manager.broadcast_for_user(
        &state.owner_id,
        ironclaw_common::AppEvent::Status {
            message: "task.rejected".to_string(),
            thread_id: Some(task_id.to_string()),
        },
    );

    Ok(Json(runtime.get_task(task_id).await.ok_or_else(|| {
        ApiError::not_found(format!("task {task_id} not found"))
    })?))
}

async fn patch_task_mode(
    State(state): State<ApiState>,
    Path(task_id): Path<Uuid>,
    Json(payload): Json<PatchTaskModeRequest>,
) -> ApiResult<Json<TaskRecord>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    let _ = runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;

    let target_mode = match payload.mode.as_str() {
        "ask" => TaskMode::Ask,
        "yolo" => TaskMode::Yolo,
        other => {
            return Err(ApiError::unprocessable_entity(format!(
                "invalid mode: {other}, must be \"ask\" or \"yolo\""
            )));
        }
    };

    let task = runtime
        .toggle_mode(task_id, target_mode)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;
    tracing::info!(
        task_id = %task_id,
        correlation_id = %task.correlation_id,
        mode = target_mode.as_str(),
        status = task.status.as_str(),
        "task mode updated through api"
    );

    state.sse_manager.broadcast_for_user(
        &state.owner_id,
        ironclaw_common::AppEvent::Status {
            message: format!("task.mode_changed:{}", payload.mode),
            thread_id: Some(task_id.to_string()),
        },
    );

    if matches!(target_mode, TaskMode::Yolo)
        && task.status == TaskStatus::WaitingApproval
        && let Some(inject_tx) = state.inject_tx.as_ref()
        && let Some(pending) = task.pending_approval.as_ref()
    {
        inject_approval(
            inject_tx,
            &task,
            Submission::ExecApproval {
                request_id: pending.id,
                approved: true,
                always: false,
            },
        )
        .await?;
    }

    Ok(Json(runtime.get_task(task_id).await.ok_or_else(|| {
        ApiError::not_found(format!("task {task_id} not found"))
    })?))
}

async fn delete_task(
    State(state): State<ApiState>,
    Path(task_id): Path<Uuid>,
) -> ApiResult<Json<TaskRecord>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    let task = runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;

    if matches!(
        task.status,
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Rejected | TaskStatus::Cancelled
    ) {
        return Err(ApiError::conflict(format!(
            "task {task_id} is already terminal"
        )));
    }

    tracing::info!(
        task_id = %task_id,
        correlation_id = %task.correlation_id,
        status = task.status.as_str(),
        "task cancelled through api"
    );
    runtime.mark_cancelled(task_id, "cancelled by user").await;

    state.sse_manager.broadcast_for_user(
        &state.owner_id,
        ironclaw_common::AppEvent::Status {
            message: "task.cancelled".to_string(),
            thread_id: Some(task_id.to_string()),
        },
    );

    Ok(Json(runtime.get_task(task_id).await.ok_or_else(|| {
        ApiError::not_found(format!("task {task_id} not found"))
    })?))
}

async fn get_workbench_capabilities(
    State(state): State<ApiState>,
) -> ApiResult<Json<WorkbenchCapabilitiesResponse>> {
    let mcp_servers = load_mcp_servers_from_db(state.store.as_ref(), &state.owner_id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load MCP server config: {e}")))?
        .servers
        .into_iter()
        .map(|server| {
            let transport = match server.effective_transport() {
                EffectiveTransport::Http => "http".to_string(),
                EffectiveTransport::Stdio { .. } => "stdio".to_string(),
                EffectiveTransport::Unix { .. } => "unix".to_string(),
            };
            let auth_mode = if server.has_custom_auth_header() {
                "custom_header".to_string()
            } else if server.requires_auth() {
                "oauth".to_string()
            } else {
                "none".to_string()
            };

            WorkbenchMcpServerResponse {
                name: server.name,
                transport,
                enabled: server.enabled,
                auth_mode,
                description: server.description,
            }
        })
        .collect();

    Ok(Json(WorkbenchCapabilitiesResponse {
        workspace_available: state.workspace.is_some(),
        tool_count: state.tool_count,
        dev_loaded_tools: state.dev_loaded_tool_names.clone(),
        mcp_servers,
    }))
}

async fn index_workspace(
    State(state): State<ApiState>,
    Json(payload): Json<WorkspaceIndexRequest>,
) -> ApiResult<Json<WorkspaceIndexResponse>> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| ApiError::conflict("workspace is not available"))?;
    let path = payload.path.trim();
    if path.is_empty() {
        return Err(ApiError::bad_request("path cannot be empty"));
    }
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|e| ApiError::bad_request(format!("path is not accessible: {e}")))?;
    let metadata = std::fs::metadata(&canonical_path)
        .map_err(|e| ApiError::bad_request(format!("path is not accessible: {e}")))?;
    if !metadata.is_dir() {
        return Err(ApiError::bad_request("path must be a directory"));
    }
    let canonical_path_string = canonical_path.display().to_string();
    let slug = workspace_index_slug(&canonical_path);
    let import_root = format!("imports/fs/{slug}");
    let manifest_path = format!("indexes/fs/{slug}.json");
    let now = chrono::Utc::now();
    let job = WorkspaceIndexJobResponse {
        id: Uuid::new_v4(),
        path: canonical_path_string.clone(),
        import_root: import_root.clone(),
        manifest_path: manifest_path.clone(),
        status: "queued".to_string(),
        phase: "queued".to_string(),
        total_files: 0,
        processed_files: 0,
        indexed_files: 0,
        skipped_files: 0,
        error: None,
        started_at: now,
        updated_at: now,
        completed_at: None,
    };

    {
        let mut jobs = state.workspace_index_jobs.write().await;
        jobs.insert(job.id, job.clone());
    }

    let jobs = Arc::clone(&state.workspace_index_jobs);
    let workspace = Arc::clone(workspace);
    tokio::spawn(async move {
        if let Err(error) = run_workspace_index_job(
            Arc::clone(&jobs),
            workspace,
            job.id,
            canonical_path,
            canonical_path_string,
            import_root,
            manifest_path,
        )
        .await
        {
            let _ = update_workspace_index_job(&jobs, job.id, |job| {
                let now = chrono::Utc::now();
                job.status = "failed".to_string();
                job.phase = "failed".to_string();
                job.error = Some(error.clone());
                job.updated_at = now;
                job.completed_at = Some(now);
            })
            .await;
            tracing::error!(job_id = %job.id, %error, "workspace index job failed");
        }
    });

    Ok(Json(WorkspaceIndexResponse { job }))
}

async fn get_workspace_index_job(
    State(state): State<ApiState>,
    Path(job_id): Path<Uuid>,
) -> ApiResult<Json<WorkspaceIndexJobResponse>> {
    let jobs = state.workspace_index_jobs.read().await;
    let job = jobs
        .get(&job_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("workspace index job {job_id} not found")))?;
    Ok(Json(job))
}

async fn get_workspace_tree(
    State(state): State<ApiState>,
    Query(query): Query<WorkspaceTreeQuery>,
) -> ApiResult<Json<WorkspaceTreeResponse>> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| ApiError::conflict("workspace is not available"))?;
    let path = query.path.unwrap_or_default();
    let entries = workspace
        .list(&path)
        .await
        .map_err(|e| ApiError::internal(format!("failed to list workspace tree: {e}")))?;
    Ok(Json(WorkspaceTreeResponse { path, entries }))
}

async fn search_workspace(
    State(state): State<ApiState>,
    Json(payload): Json<WorkspaceSearchRequest>,
) -> ApiResult<Json<WorkspaceSearchResponse>> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| ApiError::conflict("workspace is not available"))?;
    if payload.query.trim().is_empty() {
        return Err(ApiError::bad_request("query cannot be empty"));
    }
    let results = workspace
        .search(payload.query.trim(), 20)
        .await
        .map_err(|e| ApiError::internal(format!("failed to search workspace: {e}")))?;
    let filtered: Vec<SearchResult> = results
        .into_iter()
        .filter(|result| !result.document_path.starts_with("indexes/fs/"))
        .collect();
    let source_roots = load_index_source_roots(workspace.as_ref(), &filtered).await;

    Ok(Json(WorkspaceSearchResponse {
        results: filtered
            .into_iter()
            .map(|result| {
                let slug = workspace_index_slug_from_document_path(&result.document_path);
                let source_root = slug.as_ref().and_then(|value| source_roots.get(value));
                WorkspaceSearchResultResponse::from_with_source_root(result, source_root)
            })
            .collect(),
    }))
}

async fn load_settings(state: &ApiState) -> ApiResult<Settings> {
    let map = state
        .store
        .get_all_settings(&state.owner_id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load settings: {e}")))?;
    let mut settings = Settings::from_db_map(&map);
    if let Some(secrets) = state.secrets_store.as_ref() {
        hydrate_llm_keys_from_secrets(&mut settings, secrets.as_ref(), &state.owner_id).await;
    }
    Ok(settings)
}

fn validate_settings_patch(payload: &PatchSettingsRequest) -> ApiResult<()> {
    if let Some(backend) = &payload.llm_backend
        && backend.trim().is_empty()
    {
        return Err(ApiError::bad_request("llm_backend cannot be empty"));
    }

    if let Some(backend) = &payload.llm_backend {
        let normalized = backend.trim().to_ascii_lowercase();
        let supported = [
            "openai",
            "openai_codex",
            "anthropic",
            "groq",
            "openrouter",
            "ollama",
        ];
        if !supported.contains(&normalized.as_str()) {
            return Err(ApiError::unprocessable_entity(format!(
                "unsupported llm_backend '{backend}'"
            )));
        }
    }

    if let Some(model) = &payload.selected_model
        && model.trim().is_empty()
    {
        return Err(ApiError::bad_request("selected_model cannot be empty"));
    }

    Ok(())
}

fn validate_resolved_settings(settings: &Settings) -> ApiResult<()> {
    LlmConfig::resolve(settings)
        .map_err(|e| ApiError::unprocessable_entity(format!("invalid LLM settings: {e}")))?;
    EmbeddingsConfig::resolve(settings)
        .map_err(|e| ApiError::unprocessable_entity(format!("invalid embeddings settings: {e}")))?;
    llm_readiness(settings).map_err(ApiError::unprocessable_entity)?;
    Ok(())
}

fn llm_readiness(settings: &Settings) -> Result<(), String> {
    let Some(raw_backend) = settings.llm_backend.as_deref().map(str::trim) else {
        return Err("Choose a model provider to continue.".to_string());
    };
    if raw_backend.is_empty() {
        return Err("Choose a model provider to continue.".to_string());
    }

    let backend = raw_backend.to_lowercase();
    let registry = ProviderRegistry::load();
    let supported = [
        "openai",
        "openai_codex",
        "anthropic",
        "groq",
        "openrouter",
        "ollama",
    ];
    if !supported.contains(&backend.as_str()) {
        return Err(format!(
            "Provider '{raw_backend}' is no longer supported. Use OpenAI, Codex, Anthropic, Groq, OpenRouter, or Ollama."
        ));
    }

    if backend == "openai_codex" {
        let session_path = crate::llm::OpenAiCodexConfig::default().session_path;
        if !session_path.exists() {
            return Err("Codex requires ChatGPT OAuth login before it can be used.".to_string());
        }
    } else if let Some(definition) = registry.find(&backend) {
        let builtin_override = settings.llm_builtin_overrides.get(definition.id.as_str());
        if definition.api_key_required {
            let has_api_key = builtin_override
                .and_then(|entry| entry.api_key.as_deref())
                .is_some_and(|value| !value.trim().is_empty());
            if !has_api_key {
                return Err(format!("{} requires an API key.", definition.id));
            }
        }

        let base_url = builtin_override
            .and_then(|entry| entry.base_url.as_deref())
            .or(match definition.id.as_str() {
                "ollama" => settings.ollama_base_url.as_deref(),
                _ => None,
            })
            .or(definition.default_base_url.as_deref());
        if definition.base_url_required && !base_url.is_some_and(|value| !value.trim().is_empty()) {
            return Err(format!("{} requires a base URL.", definition.id));
        }

        let has_model = settings
            .selected_model
            .as_deref()
            .or(builtin_override.and_then(|entry| entry.model.as_deref()))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            || !definition.default_model.trim().is_empty();
        if !has_model {
            return Err(format!("{} requires a model selection.", definition.id));
        }
    } else {
        return Err(format!("Unknown model provider '{raw_backend}'."));
    }

    LlmConfig::resolve(settings)
        .map_err(|error| format!("LLM configuration is incomplete: {error}"))?;
    Ok(())
}

fn apply_settings_patch(settings: &mut Settings, payload: PatchSettingsRequest) {
    if let Some(value) = payload.llm_backend {
        settings.llm_backend = Some(value);
    }
    if let Some(value) = payload.selected_model {
        settings.selected_model = Some(value);
    }
    if let Some(value) = payload.ollama_base_url {
        settings.ollama_base_url = Some(value);
    }
    if let Some(value) = payload.openai_compatible_base_url {
        settings.openai_compatible_base_url = Some(value);
    }
    if let Some(value) = payload.llm_custom_providers {
        settings.llm_custom_providers = value;
    }
    if let Some(value) = payload.llm_builtin_overrides {
        settings.llm_builtin_overrides = value;
    }
}

async fn persist_llm_provider_secrets(state: &ApiState, settings: &mut Settings) -> ApiResult<()> {
    let Some(secrets) = state.secrets_store.as_ref() else {
        return Ok(());
    };

    for (provider_id, override_value) in settings.llm_builtin_overrides.iter_mut() {
        if let Some(api_key) = override_value.api_key.take()
            && !api_key.trim().is_empty()
        {
            let secret_name = crate::settings::builtin_secret_name(provider_id);
            secrets
                .create(
                    &state.owner_id,
                    CreateSecretParams::new(secret_name, api_key).with_provider(provider_id),
                )
                .await
                .map_err(|e| {
                    ApiError::internal(format!(
                        "failed to persist builtin provider secret for {provider_id}: {e}"
                    ))
                })?;
        }
    }

    for provider in &mut settings.llm_custom_providers {
        if let Some(api_key) = provider.api_key.take()
            && !api_key.trim().is_empty()
        {
            let secret_name = crate::settings::custom_secret_name(&provider.id);
            secrets
                .create(
                    &state.owner_id,
                    CreateSecretParams::new(secret_name, api_key).with_provider(&provider.id),
                )
                .await
                .map_err(|e| {
                    ApiError::internal(format!(
                        "failed to persist custom provider secret for {}: {e}",
                        provider.id
                    ))
                })?;
        }
    }

    Ok(())
}

async fn inject_approval(
    inject_tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    task: &TaskRecord,
    submission: Submission,
) -> ApiResult<()> {
    let content = serde_json::to_string(&submission)
        .map_err(|e| ApiError::internal(format!("failed to serialize approval command: {e}")))?;
    inject_tx
        .send(task.route.to_incoming_message(content))
        .await
        .map_err(|e| ApiError::internal(format!("failed to inject approval command: {e}")))?;
    Ok(())
}

async fn ensure_session_belongs_to_user(state: &ApiState, session_id: Uuid) -> ApiResult<()> {
    let belongs = state
        .store
        .conversation_belongs_to_user(session_id, &state.owner_id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to verify session ownership: {e}")))?;
    if belongs {
        Ok(())
    } else {
        Err(ApiError::not_found(format!(
            "session {session_id} not found"
        )))
    }
}

async fn load_session_summary(
    state: &ApiState,
    session_id: Uuid,
) -> ApiResult<SessionSummaryResponse> {
    let session = state
        .store
        .list_conversations_all_channels(&state.owner_id, 200)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load session summaries: {e}")))?
        .into_iter()
        .find(|conversation| conversation.id == session_id)
        .ok_or_else(|| ApiError::not_found(format!("session {session_id} not found")))?;
    Ok(SessionSummaryResponse::from(session))
}

async fn load_session_task(state: &ApiState, session_id: Uuid) -> Option<TaskRecord> {
    let runtime = state.task_runtime.as_ref()?;
    runtime.get_task(session_id).await
}

fn parse_requested_task_mode(mode: Option<&str>) -> ApiResult<Option<TaskMode>> {
    match mode {
        None => Ok(None),
        Some("ask") => Ok(Some(TaskMode::Ask)),
        Some("yolo") => Ok(Some(TaskMode::Yolo)),
        Some(_) => Err(ApiError::unprocessable_entity(
            "invalid mode: expected 'ask' or 'yolo'",
        )),
    }
}

fn event_thread_id(event: &ironclaw_common::AppEvent) -> Option<&str> {
    match event {
        ironclaw_common::AppEvent::Response { thread_id, .. } => Some(thread_id.as_str()),
        ironclaw_common::AppEvent::Thinking { thread_id, .. }
        | ironclaw_common::AppEvent::ToolStarted { thread_id, .. }
        | ironclaw_common::AppEvent::ToolCompleted { thread_id, .. }
        | ironclaw_common::AppEvent::ToolResult { thread_id, .. }
        | ironclaw_common::AppEvent::StreamChunk { thread_id, .. }
        | ironclaw_common::AppEvent::Status { thread_id, .. }
        | ironclaw_common::AppEvent::ApprovalNeeded { thread_id, .. }
        | ironclaw_common::AppEvent::Error { thread_id, .. }
        | ironclaw_common::AppEvent::ImageGenerated { thread_id, .. }
        | ironclaw_common::AppEvent::Suggestions { thread_id, .. }
        | ironclaw_common::AppEvent::TurnCost { thread_id, .. }
        | ironclaw_common::AppEvent::ReasoningUpdate { thread_id, .. } => thread_id.as_deref(),
        _ => None,
    }
}

impl From<TaskDetail> for TaskDetailResponse {
    fn from(value: TaskDetail) -> Self {
        Self {
            task: value.task,
            timeline: value.timeline,
        }
    }
}

impl WorkspaceSearchResultResponse {
    fn from_with_source_root(result: SearchResult, source_root: Option<&String>) -> Self {
        let slug = workspace_index_slug_from_document_path(&result.document_path);
        let source_path = match (slug.as_deref(), source_root) {
            (Some(_), Some(root)) => source_relative_from_document_path(&result.document_path)
                .map(|relative| FsPath::new(root).join(relative).display().to_string()),
            _ => None,
        };

        Self {
            document_id: result.document_id,
            document_path: result.document_path,
            source_path,
            chunk_id: result.chunk_id,
            content: result.content,
            score: result.score,
            fts_rank: result.fts_rank,
            vector_rank: result.vector_rank,
        }
    }
}

fn normalize_session_event(
    thread_id: String,
    event: ironclaw_common::AppEvent,
) -> Option<(String, (String, Value))> {
    let normalized = match event {
        ironclaw_common::AppEvent::Response { content, .. } => (
            "session.response".to_string(),
            json!({ "content": content }),
        ),
        ironclaw_common::AppEvent::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
            allow_always,
            ..
        } => (
            "session.approval_needed".to_string(),
            json!({
                "approval_id": request_id,
                "tool_name": tool_name,
                "summary": description,
                "parameters": parameters,
                "allow_always": allow_always,
            }),
        ),
        ironclaw_common::AppEvent::Status { message, .. } => {
            ("session.status".to_string(), json!({ "message": message }))
        }
        ironclaw_common::AppEvent::Error { message, .. } => {
            ("session.error".to_string(), json!({ "message": message }))
        }
        other => (
            format!("session.{}", other.event_type()),
            strip_event_type_field(event_to_value(other)),
        ),
    };

    Some((thread_id, normalized))
}

async fn load_index_source_roots(
    workspace: &Workspace,
    results: &[SearchResult],
) -> HashMap<String, String> {
    let mut roots = HashMap::new();
    for result in results {
        let Some(slug) = workspace_index_slug_from_document_path(&result.document_path) else {
            continue;
        };
        if roots.contains_key(&slug) {
            continue;
        }
        let manifest_path = format!("indexes/fs/{slug}.json");
        let Ok(doc) = workspace.read(&manifest_path).await else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&doc.content) else {
            continue;
        };
        if let Some(source_root) = value.get("source_root").and_then(Value::as_str) {
            roots.insert(slug, source_root.to_string());
        }
    }
    roots
}

fn workspace_index_slug(path: &FsPath) -> String {
    use std::hash::{Hash, Hasher};

    let source = path.display().to_string();
    let base = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("workspace");
    let sanitized: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!(
        "{}-{:08x}",
        sanitized.trim_matches('-'),
        (hasher.finish() & 0xffff_ffff) as u32
    )
}

fn workspace_index_slug_from_document_path(document_path: &str) -> Option<String> {
    let rest = document_path.strip_prefix("imports/fs/")?;
    rest.split('/').next().map(str::to_string)
}

fn source_relative_from_document_path(document_path: &str) -> Option<PathBuf> {
    let rest = document_path.strip_prefix("imports/fs/")?;
    let (_, relative) = rest.split_once('/')?;
    Some(PathBuf::from(relative))
}

#[derive(Debug, Clone)]
struct IndexCandidate {
    absolute_path: PathBuf,
    relative_path: String,
}

async fn run_workspace_index_job(
    jobs: Arc<tokio::sync::RwLock<HashMap<Uuid, WorkspaceIndexJobResponse>>>,
    workspace: Arc<Workspace>,
    job_id: Uuid,
    source_root: PathBuf,
    source_root_string: String,
    import_root: String,
    manifest_path: String,
) -> Result<(), String> {
    update_workspace_index_job(&jobs, job_id, |job| {
        job.status = "running".to_string();
        job.phase = "scanning".to_string();
        job.updated_at = chrono::Utc::now();
    })
    .await?;

    let source_root_for_scan = source_root.clone();
    let candidates =
        tokio::task::spawn_blocking(move || discover_index_candidates(&source_root_for_scan))
            .await
            .map_err(|e| format!("failed to join index discovery task: {e}"))??;

    update_workspace_index_job(&jobs, job_id, |job| {
        job.phase = "indexing".to_string();
        job.total_files = candidates.len();
        job.updated_at = chrono::Utc::now();
    })
    .await?;

    let stale_paths: Vec<String> = workspace
        .list_all()
        .await
        .map_err(|e| format!("failed to list workspace paths before reindex: {e}"))?
        .into_iter()
        .filter(|path| path.starts_with(&import_root))
        .collect();

    for stale_path in stale_paths {
        workspace
            .delete(&stale_path)
            .await
            .map_err(|e| format!("failed to delete stale indexed path {stale_path}: {e}"))?;
    }

    for candidate in &candidates {
        match ingest_index_candidate(&workspace, &import_root, candidate).await {
            Ok(true) => {
                update_workspace_index_job(&jobs, job_id, |job| {
                    job.processed_files += 1;
                    job.indexed_files += 1;
                    job.updated_at = chrono::Utc::now();
                })
                .await?;
            }
            Ok(false) => {
                update_workspace_index_job(&jobs, job_id, |job| {
                    job.processed_files += 1;
                    job.skipped_files += 1;
                    job.updated_at = chrono::Utc::now();
                })
                .await?;
            }
            Err(error) => {
                update_workspace_index_job(&jobs, job_id, |job| {
                    job.processed_files += 1;
                    job.skipped_files += 1;
                    job.updated_at = chrono::Utc::now();
                })
                .await?;
                tracing::warn!(
                    job_id = %job_id,
                    path = %candidate.absolute_path.display(),
                    %error,
                    "skipped workspace file during indexing"
                );
            }
        }
    }

    update_workspace_index_job(&jobs, job_id, |job| {
        job.phase = "finalizing".to_string();
        job.updated_at = chrono::Utc::now();
    })
    .await?;

    let snapshot = {
        let jobs_guard = jobs.read().await;
        jobs_guard
            .get(&job_id)
            .cloned()
            .ok_or_else(|| format!("workspace index job {job_id} disappeared"))?
    };
    let manifest = serde_json::to_string_pretty(&json!({
        "job_id": job_id,
        "source_root": source_root_string,
        "import_root": import_root,
        "manifest_path": manifest_path,
        "indexed_at": chrono::Utc::now().to_rfc3339(),
        "total_files": snapshot.total_files,
        "indexed_files": snapshot.indexed_files,
        "skipped_files": snapshot.skipped_files
    }))
    .map_err(|e| format!("failed to serialize workspace index manifest: {e}"))?;
    workspace
        .write(&manifest_path, &manifest)
        .await
        .map_err(|e| format!("failed to write workspace index manifest: {e}"))?;

    update_workspace_index_job(&jobs, job_id, |job| {
        let now = chrono::Utc::now();
        job.status = "completed".to_string();
        job.phase = "completed".to_string();
        job.updated_at = now;
        job.completed_at = Some(now);
    })
    .await?;

    Ok(())
}

async fn update_workspace_index_job(
    jobs: &Arc<tokio::sync::RwLock<HashMap<Uuid, WorkspaceIndexJobResponse>>>,
    job_id: Uuid,
    apply: impl FnOnce(&mut WorkspaceIndexJobResponse),
) -> Result<(), String> {
    let mut jobs_guard = jobs.write().await;
    let Some(job) = jobs_guard.get_mut(&job_id) else {
        return Err(format!("workspace index job {job_id} not found"));
    };
    apply(job);
    Ok(())
}

fn discover_index_candidates(source_root: &FsPath) -> Result<Vec<IndexCandidate>, String> {
    fn walk(root: &FsPath, current: &FsPath, out: &mut Vec<IndexCandidate>) -> Result<(), String> {
        let entries = std::fs::read_dir(current)
            .map_err(|e| format!("failed to read directory {}: {e}", current.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("failed to read directory entry: {e}"))?;
            let path = entry.path();
            let metadata = entry
                .metadata()
                .map_err(|e| format!("failed to read metadata for {}: {e}", path.display()))?;
            if metadata.is_dir() {
                walk(root, &path, out)?;
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|e| format!("failed to relativize {}: {e}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            out.push(IndexCandidate {
                absolute_path: path,
                relative_path: relative,
            });
        }
        Ok(())
    }

    let mut candidates = Vec::new();
    walk(source_root, source_root, &mut candidates)?;
    candidates.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(candidates)
}

async fn ingest_index_candidate(
    workspace: &Workspace,
    import_root: &str,
    candidate: &IndexCandidate,
) -> Result<bool, String> {
    let bytes = tokio::fs::read(&candidate.absolute_path)
        .await
        .map_err(|e| format!("failed to read file: {e}"))?;
    let Some(content) = extract_indexable_text(&bytes) else {
        return Ok(false);
    };
    let workspace_path = format!("{import_root}/{}", candidate.relative_path);
    workspace
        .write(&workspace_path, &content)
        .await
        .map_err(|e| format!("failed to write indexed document: {e}"))?;
    Ok(true)
}

fn extract_indexable_text(bytes: &[u8]) -> Option<String> {
    const MAX_INDEX_BYTES: usize = 512 * 1024;

    if bytes.is_empty() {
        return None;
    }
    if bytes.len() > MAX_INDEX_BYTES {
        return None;
    }
    if bytes.contains(&0) {
        return None;
    }
    let text = String::from_utf8(bytes.to_vec()).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(text)
}

async fn normalize_task_event(
    task_id: Uuid,
    runtime: Arc<TaskRuntime>,
    event: ironclaw_common::AppEvent,
) -> Option<(String, (String, Value))> {
    let task = runtime.get_task(task_id).await?;
    let thread_id = task_id.to_string();

    let normalized = match event {
        ironclaw_common::AppEvent::ApprovalNeeded { .. } => {
            ("task.waiting_approval".to_string(), json!({ "task": task }))
        }
        ironclaw_common::AppEvent::Status { message, .. } if message == "task.completed" => {
            ("task.completed".to_string(), json!({ "task": task }))
        }
        ironclaw_common::AppEvent::Status { message, .. } if message == "task.rejected" => {
            ("task.rejected".to_string(), json!({ "task": task }))
        }
        ironclaw_common::AppEvent::Status { message, .. } if message == "task.cancelled" => {
            ("task.cancelled".to_string(), json!({ "task": task }))
        }
        ironclaw_common::AppEvent::Status { message, .. }
            if message.starts_with("task.mode_changed:") =>
        {
            let mode = message
                .split_once(':')
                .map(|(_, mode)| mode.to_string())
                .unwrap_or_else(|| "ask".to_string());
            (
                "task.mode_changed".to_string(),
                json!({ "mode": mode, "task": task }),
            )
        }
        ironclaw_common::AppEvent::Error { message, .. } => (
            "task.failed".to_string(),
            json!({ "message": message, "task": task }),
        ),
        other => (
            "task.updated".to_string(),
            json!({ "source_event": other.event_type(), "task": task }),
        ),
    };

    Some((thread_id, normalized))
}

fn serialize_stream_envelope(
    thread_id: String,
    sequence: u64,
    normalized: (String, Value),
) -> Option<Result<Event, Infallible>> {
    let envelope = StreamEnvelope {
        event: normalized.0.clone(),
        correlation_id: thread_id.clone(),
        thread_id,
        sequence,
        timestamp: chrono::Utc::now().to_rfc3339(),
        payload: normalized.1,
    };
    let data = serde_json::to_string(&envelope).ok()?;
    Some(Ok(Event::default().event(&normalized.0).data(data)))
}

fn event_to_value(event: ironclaw_common::AppEvent) -> Value {
    serde_json::to_value(event).unwrap_or_else(|_| json!({}))
}

fn strip_event_type_field(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("type");
    }
    value
}

impl From<ConversationSummary> for SessionSummaryResponse {
    fn from(value: ConversationSummary) -> Self {
        Self {
            id: value.id,
            title: value
                .title
                .unwrap_or_else(|| "Untitled Session".to_string()),
            message_count: value.message_count,
            started_at: value.started_at,
            last_activity: value.last_activity,
            thread_type: value.thread_type,
            channel: value.channel,
        }
    }
}

impl From<ConversationMessage> for SessionMessageResponse {
    fn from(value: ConversationMessage) -> Self {
        Self {
            id: value.id,
            role: value.role,
            content: value.content,
            created_at: value.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use crate::agent::submission::Submission;
    use crate::channels::IncomingMessage;
    use crate::db::Database;
    use crate::db::libsql::LibSqlBackend;
    use crate::task_runtime::{TaskMode, TaskRuntime, TaskStatus};
    use tokio::sync::mpsc;

    async fn test_database() -> Arc<dyn Database> {
        let db_path =
            std::env::temp_dir().join(format!("ironcowork-api-test-{}.db", uuid::Uuid::new_v4()));
        let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
        db.run_migrations().await.expect("migrations");
        db
    }

    async fn test_state() -> ApiState {
        ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            test_database().await,
            Arc::new(SseManager::new()),
            Some(Arc::new(TaskRuntime::new())),
            None,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn get_settings_returns_defaults() {
        let state = test_state().await;
        let Json(settings) = get_settings(State(state)).await.expect("settings");
        assert_eq!(settings.llm_backend, None);
        assert!(settings.llm_custom_providers.is_empty());
        assert!(!settings.llm_ready);
        assert!(settings.llm_onboarding_required);
    }

    #[tokio::test]
    async fn patch_settings_persists_updates() {
        let state = test_state().await;
        let mut llm_builtin_overrides = std::collections::HashMap::new();
        llm_builtin_overrides.insert(
            "openai".to_string(),
            crate::settings::LlmBuiltinOverride {
                api_key: Some("test-openai-key".to_string()),
                model: None,
                base_url: None,
                request_format: None,
            },
        );
        let payload = PatchSettingsRequest {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4.1".to_string()),
            ollama_base_url: None,
            openai_compatible_base_url: Some("http://127.0.0.1:11434/v1".to_string()),
            llm_custom_providers: None,
            llm_builtin_overrides: Some(llm_builtin_overrides),
        };

        let Json(updated) = patch_settings(State(state.clone()), Json(payload))
            .await
            .expect("patch");
        assert_eq!(updated.llm_backend.as_deref(), Some("openai"));
        assert!(updated.llm_ready);

        let stored = state
            .store
            .get_all_settings("test-user")
            .await
            .expect("stored");
        let restored = Settings::from_db_map(&stored);
        assert_eq!(restored.llm_backend.as_deref(), Some("openai"));
        assert_eq!(restored.selected_model.as_deref(), Some("gpt-4.1"));
    }

    #[tokio::test]
    async fn patch_task_mode_updates_runtime_mode() {
        let runtime = Arc::new(TaskRuntime::new());
        let task_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "test-user", "organize files")
            .with_thread(task_id.to_string())
            .with_timezone("Asia/Shanghai");
        runtime.ensure_task(&message, task_id).await;

        let state = ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            test_database().await,
            Arc::new(SseManager::new()),
            Some(runtime.clone()),
            None,
            None,
            None,
        );

        let Json(task) = patch_task_mode(
            State(state),
            Path(task_id),
            Json(PatchTaskModeRequest {
                mode: "yolo".to_string(),
            }),
        )
        .await
        .expect("patch mode");

        assert_eq!(task.mode, TaskMode::Yolo);
        assert_eq!(runtime.mode_for_task(task_id).await, TaskMode::Yolo);
    }

    #[tokio::test]
    async fn patch_task_mode_rejects_invalid_mode() {
        let runtime = Arc::new(TaskRuntime::new());
        let task_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "test-user", "organize files")
            .with_thread(task_id.to_string());
        runtime.ensure_task(&message, task_id).await;

        let state = ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            test_database().await,
            Arc::new(SseManager::new()),
            Some(runtime),
            None,
            None,
            None,
        );

        let err = patch_task_mode(
            State(state),
            Path(task_id),
            Json(PatchTaskModeRequest {
                mode: "auto".to_string(),
            }),
        )
        .await
        .expect_err("should reject invalid mode");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn reject_task_transitions_to_rejected() {
        let runtime = Arc::new(TaskRuntime::new());
        let task_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "test-user", "archive downloads")
            .with_thread(task_id.to_string())
            .with_owner_id("owner-1")
            .with_sender_id("sender-1")
            .with_metadata(serde_json::json!({"source":"api-test"}))
            .with_timezone("UTC");
        runtime.ensure_task(&message, task_id).await;
        let pending = crate::agent::session::PendingApproval {
            request_id,
            tool_name: "write_file".to_string(),
            parameters: serde_json::json!({"path":"/tmp/report.md"}),
            display_parameters: serde_json::json!({"path":"/tmp/report.md"}),
            description: "write a file".to_string(),
            tool_call_id: "call_1".to_string(),
            context_messages: Vec::new(),
            deferred_tool_calls: Vec::new(),
            user_timezone: Some("UTC".to_string()),
            allow_always: false,
        };
        runtime
            .mark_waiting_approval(&message, task_id, &pending)
            .await;

        let state = ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            test_database().await,
            Arc::new(SseManager::new()),
            Some(runtime),
            None,
            None,
            None,
        );

        let Json(task) = reject_task(
            State(state),
            Path(task_id),
            Json(RejectTaskRequest {
                approval_id: Some(request_id),
                reason: Some("not safe".to_string()),
            }),
        )
        .await
        .expect("reject task");

        assert_eq!(task.status, TaskStatus::Rejected);
        assert_eq!(task.last_error.as_deref(), Some("not safe"));
    }

    #[tokio::test]
    async fn reject_task_returns_409_when_not_waiting_approval() {
        let runtime = Arc::new(TaskRuntime::new());
        let task_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "test-user", "archive downloads")
            .with_thread(task_id.to_string());
        runtime.ensure_task(&message, task_id).await;

        let state = ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            test_database().await,
            Arc::new(SseManager::new()),
            Some(runtime),
            None,
            None,
            None,
        );

        let err = reject_task(
            State(state),
            Path(task_id),
            Json(RejectTaskRequest {
                approval_id: None,
                reason: None,
            }),
        )
        .await
        .expect_err("should fail when not waiting approval");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn reject_task_returns_409_on_stale_approval_id() {
        let runtime = Arc::new(TaskRuntime::new());
        let task_id = Uuid::new_v4();
        let request_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "test-user", "archive downloads")
            .with_thread(task_id.to_string())
            .with_owner_id("owner-1")
            .with_sender_id("sender-1")
            .with_metadata(serde_json::json!({"source":"api-test"}))
            .with_timezone("UTC");
        runtime.ensure_task(&message, task_id).await;
        let pending = crate::agent::session::PendingApproval {
            request_id,
            tool_name: "write_file".to_string(),
            parameters: serde_json::json!({"path":"/tmp/report.md"}),
            display_parameters: serde_json::json!({"path":"/tmp/report.md"}),
            description: "write a file".to_string(),
            tool_call_id: "call_1".to_string(),
            context_messages: Vec::new(),
            deferred_tool_calls: Vec::new(),
            user_timezone: Some("UTC".to_string()),
            allow_always: false,
        };
        runtime
            .mark_waiting_approval(&message, task_id, &pending)
            .await;

        let state = ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            test_database().await,
            Arc::new(SseManager::new()),
            Some(runtime),
            None,
            None,
            None,
        );

        let stale_id = Uuid::new_v4();
        let err = reject_task(
            State(state),
            Path(task_id),
            Json(RejectTaskRequest {
                approval_id: Some(stale_id),
                reason: None,
            }),
        )
        .await
        .expect_err("should fail on stale approval_id");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn approve_task_injects_exec_approval_message() {
        let runtime = Arc::new(TaskRuntime::new());
        let task_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "test-user", "archive downloads")
            .with_thread(task_id.to_string())
            .with_owner_id("owner-1")
            .with_sender_id("sender-1")
            .with_metadata(serde_json::json!({"source":"api-test"}))
            .with_timezone("Asia/Shanghai");
        runtime.ensure_task(&message, task_id).await;
        let pending = crate::agent::session::PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "write_file".to_string(),
            parameters: serde_json::json!({"path":"/tmp/report.md"}),
            display_parameters: serde_json::json!({"path":"/tmp/report.md"}),
            description: "write a file".to_string(),
            tool_call_id: "call_1".to_string(),
            context_messages: Vec::new(),
            deferred_tool_calls: Vec::new(),
            user_timezone: Some("Asia/Shanghai".to_string()),
            allow_always: true,
        };
        runtime
            .mark_waiting_approval(&message, task_id, &pending)
            .await;
        let (inject_tx, mut inject_rx) = mpsc::channel(4);

        let state = ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            test_database().await,
            Arc::new(SseManager::new()),
            Some(runtime),
            Some(inject_tx),
            None,
            None,
        );

        let Json(task) = approve_task(
            State(state),
            Path(task_id),
            Json(ApproveTaskRequest {
                approval_id: Some(pending.request_id),
                always: true,
            }),
        )
        .await
        .expect("approve task");

        assert_eq!(task.status, TaskStatus::WaitingApproval);
        let injected = inject_rx.recv().await.expect("injected message");
        let thread_id = task_id.to_string();
        assert_eq!(injected.thread_id.as_deref(), Some(thread_id.as_str()));
        let submission: Submission =
            serde_json::from_str(&injected.content).expect("approval submission json");
        match submission {
            Submission::ExecApproval {
                request_id,
                approved,
                always,
            } => {
                assert_eq!(request_id, pending.request_id);
                assert!(approved);
                assert!(always);
            }
            other => panic!("unexpected submission: {other:?}"),
        }
    }
}
