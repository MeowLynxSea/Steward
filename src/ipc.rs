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
    pub backends: Vec<crate::settings::BackendInstance>,
    pub major_backend_id: Option<String>,
    pub cheap_backend_id: Option<String>,
    pub cheap_model_uses_primary: bool,
    pub embeddings: crate::settings::EmbeddingsSettings,
    pub llm_ready: bool,
    pub llm_onboarding_required: bool,
    pub llm_readiness_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PatchSettingsRequest {
    pub backends: Option<Vec<crate::settings::BackendInstance>>,
    pub major_backend_id: Option<String>,
    pub cheap_backend_id: Option<String>,
    pub cheap_model_uses_primary: Option<bool>,
    pub embeddings: Option<crate::settings::EmbeddingsSettings>,
}

// =============================================================================
// Sessions (5 commands)
// =============================================================================

#[derive(Debug, Serialize)]
pub struct SessionSummaryResponse {
    pub id: Uuid,
    pub title: String,
    pub title_emoji: Option<String>,
    pub title_pending: bool,
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
#[serde(rename_all = "camelCase")]
pub struct ThreadToolCallResponse {
    pub name: String,
    pub status: String,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub parameters: Option<String>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnCostResponse {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: String,
}

#[derive(Debug, Serialize)]
pub struct ReflectionMessageResponse {
    pub id: Uuid,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct ReflectionToolCallResponse {
    pub id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub tool_call: ThreadToolCallResponse,
}

#[derive(Debug, Serialize)]
pub struct ReflectionDetailResponse {
    pub assistant_message_id: Uuid,
    pub status: String,
    pub outcome: Option<String>,
    pub summary: Option<String>,
    pub detail: Option<String>,
    pub run_started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub run_completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub tool_calls: Vec<ReflectionToolCallResponse>,
    pub messages: Vec<ReflectionMessageResponse>,
}

#[derive(Debug, Serialize)]
pub struct ThreadMessageResponse {
    pub id: Uuid,
    pub kind: String,
    pub role: Option<String>,
    pub content: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub turn_number: usize,
    pub turn_cost: Option<TurnCostResponse>,
    pub tool_call: Option<ThreadToolCallResponse>,
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

#[derive(Debug, Serialize)]
pub struct MemorySidebarResponse {
    pub sections: Vec<crate::memory::MemorySidebarSection>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryGraphSearchRequest {
    pub query: String,
    pub limit: Option<usize>,
    pub domains: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct MemoryGraphSearchResponse {
    pub results: Vec<crate::memory::MemorySearchHit>,
}

#[derive(Debug, Serialize)]
pub struct MemoryNodeDetailResponse {
    pub detail: Option<crate::memory::MemoryNodeDetail>,
}

#[derive(Debug, Serialize)]
pub struct MemoryTimelineResponse {
    pub entries: Vec<crate::memory::MemoryTimelineEntry>,
}

#[derive(Debug, Serialize)]
pub struct MemoryReviewsResponse {
    pub reviews: Vec<crate::memory::MemoryChangeSet>,
}

#[derive(Debug, Serialize)]
pub struct MemoryVersionsResponse {
    pub versions: Vec<crate::memory::MemoryVersion>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryReviewActionRequest {
    pub action: String,
}

// =============================================================================
// Workspace Allowlists (8 commands)
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceAllowlistRequest {
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
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default = "default_true")]
    pub include_content: bool,
    pub max_files: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceCheckpointRequest {
    pub revision_id: Option<Uuid>,
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
    #[serde(default)]
    pub set_as_baseline: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceCheckpointListQuery {
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceHistoryQuery {
    pub scope_path: Option<String>,
    pub limit: Option<usize>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_true")]
    pub include_checkpoints: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceRestoreRequest {
    pub target: String,
    pub scope_path: Option<String>,
    #[serde(default)]
    pub set_as_baseline: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub create_checkpoint_before_restore: bool,
    pub created_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceBaselineSetRequest {
    pub target: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveWorkspaceConflictRequest {
    pub path: String,
    pub resolution: String,
    pub renamed_copy_path: Option<String>,
    pub merged_content: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceAllowlistListResponse {
    pub allowlists: Vec<crate::workspace::WorkspaceAllowlistSummary>,
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

// =============================================================================
// MCP (14 commands)
// =============================================================================

#[derive(Debug, Serialize)]
pub struct McpServerSummaryResponse {
    pub name: String,
    pub transport: String,
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub socket_path: Option<String>,
    pub headers: std::collections::HashMap<String, String>,
    pub enabled: bool,
    pub description: Option<String>,
    pub client_id: Option<String>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub scopes: Vec<String>,
    pub authenticated: bool,
    pub requires_auth: bool,
    pub active: bool,
    pub tool_count: usize,
    pub negotiated_protocol_version: Option<String>,
    pub negotiated_capabilities: Option<serde_json::Value>,
    pub last_health_check: Option<chrono::DateTime<chrono::Utc>>,
    pub subscribed_resource_uris: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct McpServerListResponse {
    pub servers: Vec<McpServerSummaryResponse>,
}

#[derive(Debug, Deserialize)]
pub struct McpServerUpsertRequest {
    pub name: String,
    pub transport: String,
    pub url: Option<String>,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    pub socket_path: Option<String>,
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    pub enabled: Option<bool>,
    pub description: Option<String>,
    pub client_id: Option<String>,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct McpServerUpsertResponse {
    pub server: McpServerSummaryResponse,
}

#[derive(Debug, Serialize)]
pub struct McpAuthResponse {
    pub authenticated: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct McpTestResponse {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct McpToolListResponse {
    pub tools: Vec<crate::tools::mcp::McpTool>,
}

#[derive(Debug, Serialize)]
pub struct McpResourceListResponse {
    pub resources: Vec<crate::tools::mcp::McpResource>,
}

#[derive(Debug, Serialize)]
pub struct McpResourceTemplateListResponse {
    pub templates: Vec<crate::tools::mcp::McpResourceTemplate>,
}

#[derive(Debug, Serialize)]
pub struct McpReadResourceResponse {
    pub resource: crate::tools::mcp::ReadResourceResult,
}

#[derive(Debug, Serialize)]
pub struct McpSaveResourceSnapshotResponse {
    pub snapshot_path: String,
}

#[derive(Debug, Serialize)]
pub struct McpAddResourceToThreadResponse {
    pub session_id: Uuid,
    pub active_thread_id: Uuid,
    pub active_thread_task_id: Option<Uuid>,
    pub active_thread_task: Option<crate::task_runtime::TaskRecord>,
    pub attachment_count: usize,
}

#[derive(Debug, Serialize)]
pub struct McpPromptListResponse {
    pub prompts: Vec<crate::tools::mcp::McpPrompt>,
}

#[derive(Debug, Deserialize)]
pub struct McpPromptGetRequest {
    #[serde(default)]
    pub arguments: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct McpPromptResponse {
    pub prompt: crate::tools::mcp::GetPromptResult,
}

#[derive(Debug, Deserialize)]
pub struct McpCompleteArgumentRequest {
    pub reference_type: String,
    pub reference_name: String,
    pub argument_name: String,
    pub value: String,
    #[serde(default)]
    pub context_arguments: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct McpCompleteArgumentResponse {
    pub completion: crate::tools::mcp::CompleteResult,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpRootGrantResponse {
    pub uri: String,
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct McpRootsResponse {
    pub roots: Vec<McpRootGrantResponse>,
}

#[derive(Debug, Deserialize)]
pub struct McpSetRootsRequest {
    pub roots: Vec<McpRootGrantResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpActivityItemResponse {
    pub id: String,
    pub server_name: String,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct McpActivityListResponse {
    pub items: Vec<McpActivityItemResponse>,
}

#[derive(Debug, Deserialize)]
pub struct McpRespondSamplingRequest {
    pub action: String,
    #[serde(default)]
    pub request: Option<crate::tools::mcp::McpSamplingRequest>,
    #[serde(default)]
    pub generated_text: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct McpRespondSamplingResponse {
    pub task: crate::task_runtime::TaskRecord,
}

#[derive(Debug, Deserialize)]
pub struct McpRespondElicitationRequest {
    pub action: String,
    #[serde(default)]
    pub content: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
pub struct McpRespondElicitationResponse {
    pub task: crate::task_runtime::TaskRecord,
}
