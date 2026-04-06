//! IPC types module.
//!
//! These types are shared between the main crate and the Tauri commands wrapper.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Re-export AppState for use in commands.rs
pub use crate::desktop_runtime::AppState;

// =============================================================================
// Settings (2 commands)
// =============================================================================

#[derive(Debug, Serialize)]
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

#[derive(Debug, Deserialize)]
pub struct PatchSettingsRequest {
    pub llm_backend: Option<String>,
    pub selected_model: Option<String>,
    pub ollama_base_url: Option<String>,
    pub openai_compatible_base_url: Option<String>,
    pub llm_custom_providers: Option<Vec<crate::settings::CustomLlmProviderSettings>>,
    pub llm_builtin_overrides:
        Option<std::collections::HashMap<String, crate::settings::LlmBuiltinOverride>>,
}

// =============================================================================
// Sessions (5 commands)
// =============================================================================

#[derive(Debug, Serialize)]
pub struct SessionSummaryResponse {
    pub id: Uuid,
    pub title: String,
    pub turn_count: i64,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub last_activity: chrono::DateTime<chrono::Utc>,
    pub active_thread_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionSummaryResponse>,
}

#[derive(Debug, Serialize)]
pub struct ThreadMessageResponse {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub turn_number: usize,
}

#[derive(Debug, Serialize)]
pub struct SessionDetailResponse {
    pub session: SessionSummaryResponse,
    pub active_thread_id: Uuid,
    pub thread_messages: Vec<ThreadMessageResponse>,
    pub active_thread_task: Option<crate::task_runtime::TaskRecord>,
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
    pub active_thread_id: Uuid,
    pub active_thread_task_id: Option<Uuid>,
    pub active_thread_task: Option<crate::task_runtime::TaskRecord>,
}

// =============================================================================
// Tasks (6 commands)
// =============================================================================

#[derive(Debug, Serialize)]
pub struct TaskListResponse {
    pub tasks: Vec<crate::task_runtime::TaskRecord>,
}

#[derive(Debug, Serialize)]
pub struct TaskDetailResponse {
    pub task: crate::task_runtime::TaskRecord,
    pub timeline: Vec<crate::task_runtime::TaskTimelineEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ApproveTaskRequest {
    pub approval_id: Option<Uuid>,
    #[serde(default)]
    pub always: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct RejectTaskRequest {
    pub approval_id: Option<Uuid>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PatchTaskModeRequest {
    pub mode: String,
}

pub use crate::task_runtime::TaskRecord;

// =============================================================================
// Workspace (4 commands)
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct WorkspaceIndexRequest {
    pub path: String,
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

#[derive(Debug, Serialize)]
pub struct WorkspaceIndexResponse {
    pub job: WorkspaceIndexJobResponse,
}

#[derive(Debug, Default, Deserialize)]
pub struct WorkspaceTreeQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceTreeResponse {
    pub path: String,
    pub entries: Vec<crate::workspace::WorkspaceTreeEntry>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceSearchRequest {
    pub query: String,
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

#[derive(Debug, Serialize)]
pub struct WorkspaceSearchResponse {
    pub results: Vec<WorkspaceSearchResultResponse>,
}

// =============================================================================
// Workspace Mounts (8 commands)
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceMountRequest {
    pub path: String,
    pub display_name: Option<String>,
    #[serde(default = "default_true")]
    pub bypass_write: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceDiffQuery {
    pub scope_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceCheckpointRequest {
    pub label: Option<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub is_auto: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceActionRequest {
    pub scope_path: Option<String>,
    pub checkpoint_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ResolveWorkspaceConflictRequest {
    pub path: String,
    pub resolution: String,
    pub renamed_copy_path: Option<String>,
    pub merged_content: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceMountListResponse {
    pub mounts: Vec<crate::workspace::WorkspaceMountSummary>,
}

// =============================================================================
// Workbench (1 command)
// =============================================================================

#[derive(Debug, Serialize)]
pub struct WorkbenchMcpServerResponse {
    pub name: String,
    pub transport: String,
    pub enabled: bool,
    pub auth_mode: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkbenchCapabilitiesResponse {
    pub workspace_available: bool,
    pub tool_count: usize,
    pub dev_loaded_tools: Vec<String>,
    pub mcp_servers: Vec<WorkbenchMcpServerResponse>,
}
