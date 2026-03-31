//! Local HTTP API for the desktop-first runtime.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::db::SettingsStore;
use crate::runtime_events::SseManager;
use crate::settings::Settings;

pub const DEFAULT_API_HOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const DEFAULT_API_PORT: u16 = 8765;

#[derive(Clone)]
pub struct ApiState {
    owner_id: String,
    bind_addr: SocketAddr,
    settings_store: Arc<dyn SettingsStore>,
    sse_manager: Arc<SseManager>,
}

impl ApiState {
    pub fn new(
        owner_id: String,
        bind_addr: SocketAddr,
        settings_store: Arc<dyn SettingsStore>,
        sse_manager: Arc<SseManager>,
    ) -> Self {
        Self {
            owner_id,
            bind_addr,
            settings_store,
            sse_manager,
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
        .allow_methods([Method::GET, Method::PATCH])
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use crate::db::Database;
    use crate::db::libsql::LibSqlBackend;

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
}
