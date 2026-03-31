//! Local HTTP API for the desktop-first runtime.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

use crate::agent::submission::Submission;
use crate::channels::IncomingMessage;
use crate::db::SettingsStore;
use crate::runtime_events::SseManager;
use crate::settings::Settings;
use crate::task_runtime::{TaskMode, TaskRecord, TaskRuntime, TaskStatus};

pub const DEFAULT_API_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const DEFAULT_API_PORT: u16 = 8765;

#[derive(Clone)]
pub struct ApiState {
    owner_id: String,
    bind_addr: SocketAddr,
    settings_store: Arc<dyn SettingsStore>,
    sse_manager: Arc<SseManager>,
    task_runtime: Option<Arc<TaskRuntime>>,
    inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
}

impl ApiState {
    pub fn new(
        owner_id: String,
        bind_addr: SocketAddr,
        settings_store: Arc<dyn SettingsStore>,
        sse_manager: Arc<SseManager>,
        task_runtime: Option<Arc<TaskRuntime>>,
        inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
    ) -> Self {
        Self {
            owner_id,
            bind_addr,
            settings_store,
            sse_manager,
            task_runtime,
            inject_tx,
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

#[derive(Debug, Deserialize)]
pub struct ApproveTaskRequest {
    pub request_id: Option<Uuid>,
    #[serde(default)]
    pub always: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct ToggleYoloRequest {
    pub enabled: Option<bool>,
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
        .allow_methods([Method::GET, Method::PATCH, Method::POST])
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
        .route("/api/v0/tasks", get(list_tasks))
        .route("/api/v0/tasks/{id}", get(get_task))
        .route("/api/v0/tasks/{id}/approve", post(approve_task))
        .route("/api/v0/tasks/{id}/yolo-toggle", post(toggle_task_yolo))
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
        .settings_store
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
        .pending_operation
        .as_ref()
        .ok_or_else(|| ApiError::conflict(format!("task {task_id} has no pending operation")))?;
    let request_id = payload.request_id.unwrap_or(pending.request_id);
    if request_id != pending.request_id {
        return Err(ApiError::conflict(
            "approval request_id does not match current checkpoint",
        ));
    }

    inject_approval(
        inject_tx,
        &task,
        Submission::ExecApproval {
            request_id,
            approved: true,
            always: payload.always,
        },
    )
    .await?;

    Ok(Json(runtime.get_task(task_id).await.ok_or_else(|| {
        ApiError::not_found(format!("task {task_id} not found"))
    })?))
}

async fn toggle_task_yolo(
    State(state): State<ApiState>,
    Path(task_id): Path<Uuid>,
    payload: Option<Json<ToggleYoloRequest>>,
) -> ApiResult<Json<TaskRecord>> {
    let runtime = state
        .task_runtime
        .as_ref()
        .ok_or_else(|| ApiError::not_found("task runtime is not available"))?;
    let current = runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;
    let enabled = payload
        .and_then(|Json(body)| body.enabled)
        .unwrap_or(!matches!(current.mode, TaskMode::Yolo));
    let target_mode = if enabled {
        TaskMode::Yolo
    } else {
        TaskMode::Ask
    };

    let task = runtime
        .toggle_mode(task_id, target_mode)
        .await
        .ok_or_else(|| ApiError::not_found(format!("task {task_id} not found")))?;

    if enabled
        && task.status == TaskStatus::WaitingApproval
        && let Some(inject_tx) = state.inject_tx.as_ref()
        && let Some(pending) = task.pending_operation.as_ref()
    {
        inject_approval(
            inject_tx,
            &task,
            Submission::ExecApproval {
                request_id: pending.request_id,
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

async fn load_settings(state: &ApiState) -> ApiResult<Settings> {
    let map = state
        .settings_store
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

    async fn test_state() -> ApiState {
        let db_path =
            std::env::temp_dir().join(format!("ironcowork-api-test-{}.db", uuid::Uuid::new_v4()));
        let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
        db.run_migrations().await.expect("migrations");
        ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            db,
            Arc::new(SseManager::new()),
            Some(Arc::new(TaskRuntime::new())),
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
            .settings_store
            .get_all_settings("test-user")
            .await
            .expect("stored");
        let restored = Settings::from_db_map(&stored);
        assert_eq!(restored.llm_backend.as_deref(), Some("openai"));
        assert_eq!(restored.selected_model.as_deref(), Some("gpt-4.1"));
    }

    #[tokio::test]
    async fn toggle_task_yolo_updates_runtime_mode() {
        let runtime = Arc::new(TaskRuntime::new());
        let task_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "test-user", "organize files")
            .with_thread(task_id.to_string())
            .with_timezone("Asia/Shanghai");
        runtime.ensure_task(&message, task_id).await;

        let state = ApiState::new(
            "test-user".to_string(),
            SocketAddr::from(([127, 0, 0, 1], 8765)),
            test_state().await.settings_store,
            Arc::new(SseManager::new()),
            Some(runtime.clone()),
            None,
        );

        let Json(task) = toggle_task_yolo(
            State(state),
            Path(task_id),
            Some(Json(ToggleYoloRequest {
                enabled: Some(true),
            })),
        )
        .await
        .expect("toggle yolo");

        assert_eq!(task.mode, TaskMode::Yolo);
        assert_eq!(runtime.mode_for_task(task_id).await, TaskMode::Yolo);
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
            test_state().await.settings_store,
            Arc::new(SseManager::new()),
            Some(runtime),
            Some(inject_tx),
        );

        let Json(task) = approve_task(
            State(state),
            Path(task_id),
            Json(ApproveTaskRequest {
                request_id: Some(pending.request_id),
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
