//! Local HTTP API for the desktop-first runtime.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderValue, Method, StatusCode},
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
use crate::db::Database;
use crate::history::{ConversationMessage, ConversationSummary};
use crate::runtime_events::SseManager;
use crate::settings::Settings;
use crate::task_runtime::{TaskMode, TaskRecord, TaskRuntime, TaskStatus};
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
        }
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
}

impl From<Settings> for SettingsResponse {
    fn from(value: Settings) -> Self {
        Self {
            llm_backend: value.llm_backend,
            selected_model: value.selected_model,
            ollama_base_url: value.ollama_base_url,
            openai_compatible_base_url: value.openai_compatible_base_url,
            llm_custom_providers: value.llm_custom_providers,
            llm_builtin_overrides: value.llm_builtin_overrides,
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
}

#[derive(Debug, Serialize)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskRecord>,
}

#[derive(Debug, Serialize)]
struct StreamEnvelope {
    event: String,
    thread_id: String,
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
}

#[derive(Debug, Serialize)]
pub struct SendSessionMessageResponse {
    pub accepted: bool,
    pub session_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceIndexRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceIndexResponse {
    pub path: String,
    pub document_path: String,
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
    pub chunk_id: Uuid,
    pub content: String,
    pub score: f32,
    pub fts_rank: Option<u32>,
    pub vector_rank: Option<u32>,
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
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn unprocessable_entity(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ApiErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

pub fn router(state: ApiState) -> Router {
    let cors = CorsLayer::new()
        .allow_methods([Method::DELETE, Method::GET, Method::PATCH, Method::POST])
        .allow_origin(AllowOrigin::predicate(
            |origin: &HeaderValue, _| match origin.to_str() {
                Ok(origin) => {
                    origin.starts_with("http://127.0.0.1:")
                        || origin.starts_with("http://localhost:")
                        || origin == "http://127.0.0.1"
                        || origin == "http://localhost"
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
        .route("/api/v0/tasks/{id}", get(get_task))
        .route("/api/v0/tasks/{id}/stream", get(stream_task))
        .route("/api/v0/tasks/{id}/approve", post(approve_task))
        .route("/api/v0/tasks/{id}/reject", post(reject_task))
        .route("/api/v0/tasks/{id}/mode", patch(patch_task_mode))
        .route("/api/v0/workspace/index", post(index_workspace))
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
    Ok(Json(SettingsResponse::from(settings)))
}

async fn patch_settings(
    State(state): State<ApiState>,
    Json(payload): Json<PatchSettingsRequest>,
) -> ApiResult<Json<SettingsResponse>> {
    validate_settings_patch(&payload)?;

    let mut settings = load_settings(&state).await?;
    apply_settings_patch(&mut settings, payload);

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

    Ok(Json(SettingsResponse::from(settings)))
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

    let message = IncomingMessage::new("api", &state.owner_id, payload.content)
        .with_owner_id(state.owner_id.clone())
        .with_sender_id(state.owner_id.clone())
        .with_thread(session_id.to_string())
        .with_metadata(serde_json::json!({
            "source": "api",
            "surface": "web"
        }))
        .with_timezone("UTC");
    inject_tx
        .send(message)
        .await
        .map_err(|e| ApiError::internal(format!("failed to enqueue session message: {e}")))?;

    Ok(Json(SendSessionMessageResponse {
        accepted: true,
        session_id,
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
        .filter_map(
            |(idx, (thread_id, normalized))| async move {
                serialize_stream_envelope(thread_id, idx as u64 + 1, normalized)
            },
        );

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

    let stream = initial
        .chain(live)
        .enumerate()
        .filter_map(
            |(idx, (thread_id, normalized))| async move {
                serialize_stream_envelope(thread_id, idx as u64 + 1, normalized)
            },
        );

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
) -> ApiResult<Json<TaskRecord>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    let task = runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;
    Ok(Json(task))
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
    let inject_tx = state
        .inject_tx
        .as_ref()
        .ok_or_else(|| ApiError::conflict("approval injection is not available"))?;
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
    let metadata = std::fs::metadata(path)
        .map_err(|e| ApiError::bad_request(format!("path is not accessible: {e}")))?;
    if !metadata.is_dir() {
        return Err(ApiError::bad_request("path must be a directory"));
    }

    let document_path = format!("indexes/{}.md", Uuid::new_v4());
    workspace
        .write(
            &document_path,
            &format!(
                "# Indexed Folder\n\n- source_path: {}\n- indexed_at: {}\n",
                path,
                chrono::Utc::now().to_rfc3339(),
            ),
        )
        .await
        .map_err(|e| {
            ApiError::internal(format!("failed to record workspace index request: {e}"))
        })?;

    Ok(Json(WorkspaceIndexResponse {
        path: path.to_string(),
        document_path,
    }))
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
    Ok(Json(WorkspaceSearchResponse {
        results: results
            .into_iter()
            .map(WorkspaceSearchResultResponse::from)
            .collect(),
    }))
}

async fn load_settings(state: &ApiState) -> ApiResult<Settings> {
    let map = state
        .store
        .get_all_settings(&state.owner_id)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load settings: {e}")))?;
    Ok(Settings::from_db_map(&map))
}

fn validate_settings_patch(payload: &PatchSettingsRequest) -> ApiResult<()> {
    if let Some(backend) = &payload.llm_backend
        && backend.trim().is_empty()
    {
        return Err(ApiError::bad_request("llm_backend cannot be empty"));
    }

    if let Some(model) = &payload.selected_model
        && model.trim().is_empty()
    {
        return Err(ApiError::bad_request("selected_model cannot be empty"));
    }

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

fn normalize_session_event(
    thread_id: String,
    event: ironclaw_common::AppEvent,
) -> Option<(String, (String, Value))> {
    let normalized = match event {
        ironclaw_common::AppEvent::Response { content, .. } => {
            ("session.response".to_string(), json!({ "content": content }))
        }
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

impl From<SearchResult> for WorkspaceSearchResultResponse {
    fn from(value: SearchResult) -> Self {
        Self {
            document_id: value.document_id,
            document_path: value.document_path,
            chunk_id: value.chunk_id,
            content: value.content,
            score: value.score,
            fts_rank: value.fts_rank,
            vector_rank: value.vector_rank,
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
    }

    #[tokio::test]
    async fn patch_settings_persists_updates() {
        let state = test_state().await;
        let payload = PatchSettingsRequest {
            llm_backend: Some("openai".to_string()),
            selected_model: Some("gpt-4.1".to_string()),
            ollama_base_url: None,
            openai_compatible_base_url: Some("http://127.0.0.1:11434/v1".to_string()),
            llm_custom_providers: None,
            llm_builtin_overrides: None,
        };

        let Json(updated) = patch_settings(State(state.clone()), Json(payload))
            .await
            .expect("patch");
        assert_eq!(updated.llm_backend.as_deref(), Some("openai"));

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
