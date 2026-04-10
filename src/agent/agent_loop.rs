//! Main agent loop.
//!
//! Contains the `Agent` struct, `AgentDeps`, and the core event loop (`run`).
//! The heavy lifting is delegated to sibling modules:
//!
//! - `dispatcher` - Tool dispatch (agentic loop, tool execution)
//! - `commands` - System commands and job handlers
//! - `thread_ops` - Thread/session operations (user input, undo, approval, persistence)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use uuid::Uuid;

use crate::agent::context_monitor::ContextMonitor;
use crate::agent::heartbeat::{spawn_heartbeat, spawn_multi_user_heartbeat};
use crate::agent::routine_engine::{RoutineEngine, spawn_cron_ticker};
use crate::agent::self_repair::{DefaultSelfRepair, RepairResult, SelfRepair};
use crate::agent::session::ThreadState;
use crate::agent::session_manager::SessionManager;
use crate::agent::submission::{Submission, SubmissionParser, SubmissionResult};
use crate::agent::{HeartbeatConfig as AgentHeartbeatConfig, Router, Scheduler, SchedulerDeps};
use crate::channels::{IncomingMessage, MessageStream, MessageTransport, OutgoingResponse};
use crate::config::{AgentConfig, HeartbeatConfig, RoutineConfig, SkillsConfig};
use crate::context::ContextManager;
use crate::db::Database;
use crate::error::{ChannelError, Error};
use crate::extensions::ExtensionManager;
use crate::hooks::HookRegistry;
use crate::llm::LlmProvider;
use crate::memory::MemoryManager;
use crate::runtime_events::RuntimeEventEmitter;
use crate::safety::SafetyLayer;
use crate::skills::SkillRegistry;
use crate::task_runtime::{TaskMode, TaskRuntime};
use crate::tools::{ApprovalRequirement, Tool, ToolRegistry};
use crate::workspace::Workspace;

/// Static greeting persisted to DB and broadcast on first launch.
///
/// Sent before the LLM is involved so the user sees something immediately.
/// The conversational onboarding (profile building, desktop setup) happens
/// organically in the subsequent turns driven by BOOTSTRAP.md.
const BOOTSTRAP_GREETING: &str = include_str!("../workspace/seeds/GREETING.md");

/// Collapse a tool output string into a single-line preview for display.
pub(crate) fn truncate_for_preview(output: &str, max_chars: usize) -> String {
    let collapsed: String = output
        .chars()
        .take(max_chars + 50)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // char_indices gives us byte offsets at char boundaries, so the slice is always valid UTF-8.
    if collapsed.chars().count() > max_chars {
        let byte_offset = collapsed
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(collapsed.len());
        format!("{}...", &collapsed[..byte_offset])
    } else {
        collapsed
    }
}

#[cfg(test)]
fn resolve_routine_notification_user(metadata: &serde_json::Value) -> Option<String> {
    resolve_owner_scope_notification_user(
        metadata.get("notify_user").and_then(|value| value.as_str()),
        metadata.get("owner_id").and_then(|value| value.as_str()),
    )
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn resolve_owner_scope_notification_user(
    explicit_user: Option<&str>,
    owner_fallback: Option<&str>,
) -> Option<String> {
    trimmed_option(explicit_user).or_else(|| trimmed_option(owner_fallback))
}

fn is_single_message_console(message: &IncomingMessage) -> bool {
    message.channel == "desktop-console"
        && message
            .metadata
            .get("single_message_mode")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
}

async fn resolve_channel_notification_user(
    extension_manager: Option<&Arc<ExtensionManager>>,
    channel: Option<&str>,
    explicit_user: Option<&str>,
    owner_fallback: Option<&str>,
) -> Option<String> {
    if let Some(user) = trimmed_option(explicit_user) {
        return Some(user);
    }

    if let Some(channel_name) = trimmed_option(channel)
        && let Some(extension_manager) = extension_manager
        && let Some(target) = extension_manager
            .notification_target_for_channel(&channel_name)
            .await
    {
        return Some(target);
    }

    resolve_owner_scope_notification_user(explicit_user, owner_fallback)
}

async fn resolve_routine_notification_target(
    extension_manager: Option<&Arc<ExtensionManager>>,
    metadata: &serde_json::Value,
) -> Option<String> {
    resolve_channel_notification_user(
        extension_manager,
        metadata
            .get("notify_channel")
            .and_then(|value| value.as_str()),
        metadata.get("notify_user").and_then(|value| value.as_str()),
        metadata.get("owner_id").and_then(|value| value.as_str()),
    )
    .await
}

pub(crate) fn chat_tool_execution_metadata(message: &IncomingMessage) -> serde_json::Value {
    serde_json::json!({
        "notify_channel": message.channel,
        "notify_user": message
            .routing_target()
            .unwrap_or_else(|| message.user_id.clone()),
        "notify_thread_id": message.thread_id,
        "notify_metadata": message.metadata,
    })
}

fn should_fallback_routine_notification(error: &ChannelError) -> bool {
    !matches!(error, ChannelError::MissingRoutingTarget { .. })
}

fn preferred_desktop_session_id(message: &IncomingMessage) -> Option<Uuid> {
    message
        .metadata
        .get("desktop_session_id")
        .and_then(|value| value.as_str())
        .and_then(|value| Uuid::parse_str(value).ok())
}

/// Core dependencies for the agent.
///
/// Bundles the shared components to reduce argument count.
pub struct AgentDeps {
    /// Resolved durable owner scope for the instance.
    pub owner_id: String,
    pub store: Option<Arc<dyn Database>>,
    pub llm: Arc<dyn LlmProvider>,
    /// Cheap/fast LLM for lightweight tasks (heartbeat, routing, evaluation).
    /// Falls back to the main `llm` if None.
    pub cheap_llm: Option<Arc<dyn LlmProvider>>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub workspace: Option<Arc<Workspace>>,
    pub memory: Option<Arc<MemoryManager>>,
    pub extension_manager: Option<Arc<ExtensionManager>>,
    pub skill_registry: Option<Arc<std::sync::RwLock<SkillRegistry>>>,
    pub skill_catalog: Option<Arc<crate::skills::catalog::SkillCatalog>>,
    pub skills_config: SkillsConfig,
    pub hooks: Arc<HookRegistry>,
    /// Cost enforcement guardrails (daily budget, hourly rate limits).
    pub cost_guard: Arc<crate::agent::cost_guard::CostGuard>,
    /// Runtime event stream manager for browser-mode compatibility.
    pub sse_tx: Option<Arc<crate::runtime_events::SseManager>>,
    /// Optional Tauri event emitter for native desktop events.
    pub emitter: Option<Arc<dyn RuntimeEventEmitter>>,
    /// HTTP interceptor for trace recording/replay.
    pub http_interceptor: Option<Arc<dyn crate::llm::recording::HttpInterceptor>>,
    /// Audio transcription middleware for voice messages.
    pub transcription: Option<Arc<crate::llm::transcription::TranscriptionMiddleware>>,
    /// Document text extraction middleware for PDF, DOCX, PPTX, etc.
    pub document_extraction: Option<Arc<crate::document_extraction::DocumentExtractionMiddleware>>,
    /// Local Claude Code execution configuration for jobs using the claude_code strategy.
    pub claude_code_config: crate::config::ClaudeCodeConfig,
    /// Software builder for self-repair tool rebuilding.
    pub builder: Option<Arc<dyn crate::tools::SoftwareBuilder>>,
    /// Resolved LLM backend identifier (e.g., "openai", "anthropic", "groq").
    /// Used by `/model` persistence to determine which env var to update.
    pub llm_backend: String,
    /// Per-tenant rate limiting registry (lazily creates rate state per user).
    pub tenant_rates: Arc<crate::tenant::TenantRateRegistry>,
    /// Task-mode runtime for Ask/Yolo approvals and task API bridging.
    pub task_runtime: Option<Arc<TaskRuntime>>,
}

#[derive(Clone, Default)]
pub(super) struct AgentChannels {
    transport: Option<Arc<dyn MessageTransport>>,
    /// Optional Tauri event emitter for native desktop events.
    emitter: Option<Arc<dyn RuntimeEventEmitter>>,
    owner_id: String,
}

impl AgentChannels {
    pub(super) fn new(
        transport: Option<Arc<dyn MessageTransport>>,
        emitter: Option<Arc<dyn RuntimeEventEmitter>>,
        owner_id: String,
    ) -> Self {
        Self {
            transport,
            emitter,
            owner_id,
        }
    }

    pub(super) async fn respond(
        &self,
        message: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(transport) = self.transport.as_ref() {
            transport.respond(message, response).await
        } else {
            Ok(())
        }
    }

    pub(super) async fn send_status(
        &self,
        channel_name: &str,
        status: crate::channels::StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Mirror status updates to runtime events so the desktop UI receives them in real-time.
        let user_id = metadata
            .get("notify_user")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.owner_id);
        let thread_id = metadata
            .get("notify_thread_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        if let Some(event) = status_update_to_app_event(&status, thread_id) {
            if let Some(emitter) = self.emitter.as_ref() {
                emitter.emit_for_user(user_id, event);
            }
        }
        if let Some(transport) = self.transport.as_ref() {
            let _ = channel_name;
            transport.send_status(status, metadata).await
        } else {
            Ok(())
        }
    }

    pub(super) async fn broadcast(
        &self,
        channel_name: &str,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(transport) = self.transport.as_ref() {
            transport.broadcast(channel_name, user_id, response).await
        } else {
            Ok(())
        }
    }

    pub(super) async fn shutdown_all(&self) -> Result<(), ChannelError> {
        if let Some(transport) = self.transport.as_ref() {
            transport.shutdown().await
        } else {
            Ok(())
        }
    }

    pub(super) fn conversation_context(
        &self,
        metadata: &serde_json::Value,
    ) -> HashMap<String, String> {
        self.transport
            .as_ref()
            .map(|transport| transport.conversation_context(metadata))
            .unwrap_or_default()
    }
}

/// The main agent that coordinates all components.
pub struct Agent {
    pub(super) config: AgentConfig,
    pub(super) deps: AgentDeps,
    pub(super) channels: AgentChannels,
    pub(super) message_stream: tokio::sync::Mutex<Option<MessageStream>>,
    pub(super) context_manager: Arc<ContextManager>,
    pub(super) scheduler: Arc<Scheduler>,
    pub(super) router: Router,
    pub(super) session_manager: Arc<SessionManager>,
    pub(super) context_monitor: ContextMonitor,
    pub(super) heartbeat_config: Option<HeartbeatConfig>,
    pub(super) hygiene_config: Option<crate::config::HygieneConfig>,
    pub(super) routine_config: Option<RoutineConfig>,
    /// Shared routine-engine slot used for internal event matching and manual
    /// runtime trigger entry points.
    pub(super) routine_engine_slot:
        Arc<tokio::sync::RwLock<Option<Arc<crate::agent::routine_engine::RoutineEngine>>>>,
}

impl Agent {
    pub(super) fn is_path_scoped_filesystem_tool(&self, tool_name: &str) -> bool {
        matches!(
            tool_name,
            "read_file" | "write_file" | "list_dir" | "apply_patch" | "move_file"
        )
    }

    pub(super) fn resolve_filesystem_access_paths(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<Vec<PathBuf>, Error> {
        let normalize = |value: &str| -> Result<PathBuf, Error> {
            if value.contains('\0') {
                return Err(Error::Tool(crate::error::ToolError::InvalidParameters {
                    name: tool_name.to_string(),
                    reason: "path contains null byte".to_string(),
                }));
            }
            let path = PathBuf::from(value);
            if path.is_absolute() {
                Ok(path.canonicalize().unwrap_or(path))
            } else {
                let joined = std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(path);
                Ok(joined.canonicalize().unwrap_or(joined))
            }
        };
        let required = |name: &str| {
            params
                .get(name)
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    Error::Tool(crate::error::ToolError::InvalidParameters {
                        name: tool_name.to_string(),
                        reason: format!("missing '{name}' parameter"),
                    })
                })
        };
        match tool_name {
            "read_file" | "write_file" | "apply_patch" => Ok(vec![normalize(required("path")?)?]),
            "list_dir" => {
                let value = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                Ok(vec![normalize(value)?])
            }
            "move_file" => Ok(vec![
                normalize(required("source_path")?)?,
                normalize(required("destination_path")?)?,
            ]),
            _ => Ok(Vec::new()),
        }
    }

    fn nearest_existing_directory(path: &Path) -> Option<PathBuf> {
        let mut current = if path.exists() && path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()?.to_path_buf()
        };

        loop {
            if current.exists() && current.is_dir() {
                return Some(current.canonicalize().unwrap_or_else(|_| current.clone()));
            }
            current = current.parent()?.to_path_buf();
        }
    }

    fn mount_scope_roots(&self, tool_name: &str, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for path in paths {
            let candidate = if tool_name == "list_dir" {
                Self::nearest_existing_directory(path)
                    .or_else(|| path.parent().map(|parent| parent.to_path_buf()))
            } else {
                Self::nearest_existing_directory(path)
            };
            if let Some(root) = candidate
                && !roots.iter().any(|existing| existing == &root)
            {
                roots.push(root);
            }
        }
        roots
    }

    fn filesystem_access_path_arguments(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<Vec<(&'static str, String)>, Error> {
        let required = |name: &'static str| {
            params
                .get(name)
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .ok_or_else(|| {
                    Error::Tool(crate::error::ToolError::InvalidParameters {
                        name: tool_name.to_string(),
                        reason: format!("missing '{name}' parameter"),
                    })
                })
        };

        match tool_name {
            "read_file" | "write_file" | "apply_patch" => Ok(vec![("path", required("path")?)]),
            "list_dir" => Ok(vec![(
                "path",
                params
                    .get("path")
                    .and_then(|value| value.as_str())
                    .unwrap_or(".")
                    .to_string(),
            )]),
            "move_file" => Ok(vec![
                ("source_path", required("source_path")?),
                ("destination_path", required("destination_path")?),
            ]),
            _ => Ok(Vec::new()),
        }
    }

    async fn filesystem_paths_are_mounted(&self, user_id: &str, paths: &[PathBuf]) -> bool {
        let Some(workspace) = self.tenant_ctx(user_id).await.workspace().cloned() else {
            return false;
        };
        let Ok(mounts) = workspace.list_mounts().await else {
            return false;
        };
        paths.iter().all(|path| {
            mounts
                .iter()
                .any(|mount| path.starts_with(Path::new(&mount.mount.source_root)))
        })
    }

    async fn filesystem_access_is_preapproved(
        &self,
        session: &Arc<tokio::sync::Mutex<crate::agent::Session>>,
        user_id: &str,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> bool {
        if !self.is_path_scoped_filesystem_tool(tool_name) {
            return false;
        }
        let Ok(paths) = self.resolve_filesystem_access_paths(tool_name, params) else {
            return false;
        };
        if paths.is_empty() {
            return false;
        }
        {
            let sess = session.lock().await;
            if paths.iter().all(|path| sess.is_path_auto_approved(path)) {
                return true;
            }
        }
        self.filesystem_paths_are_mounted(user_id, &paths).await
    }

    pub(super) async fn mounted_workspace_redirect_for_tool(
        &self,
        user_id: &str,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Option<String> {
        if !self.is_path_scoped_filesystem_tool(tool_name) {
            return None;
        }

        let path_args = self
            .filesystem_access_path_arguments(tool_name, params)
            .ok()?;
        if path_args.is_empty() {
            return None;
        }

        let paths = self
            .resolve_filesystem_access_paths(tool_name, params)
            .ok()?;
        if paths.is_empty() {
            return None;
        }

        let Some(workspace) = self.tenant_ctx(user_id).await.workspace().cloned() else {
            return None;
        };
        let mounts = workspace.list_mounts().await.ok()?;

        let mut redirects = Vec::new();
        for ((param_name, raw_path), normalized_path) in
            path_args.into_iter().zip(paths.into_iter())
        {
            if raw_path.starts_with("workspace://") {
                continue;
            }

            let mut selected: Option<(usize, String)> = None;
            for mount in &mounts {
                let root = Path::new(&mount.mount.source_root);
                if !normalized_path.starts_with(root) {
                    continue;
                }

                let rel = normalized_path
                    .strip_prefix(root)
                    .ok()
                    .map(|value| {
                        value
                            .iter()
                            .map(|segment| segment.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .unwrap_or_default();
                let uri = crate::workspace::WorkspaceUri::mount_uri(
                    mount.mount.id,
                    if rel.is_empty() {
                        None
                    } else {
                        Some(rel.as_str())
                    },
                );
                let depth = root.components().count();

                match &selected {
                    Some((best_depth, _)) if *best_depth >= depth => {}
                    _ => selected = Some((depth, uri)),
                }
            }

            if let Some((_, uri)) = selected {
                redirects.push((param_name, uri));
            }
        }

        if redirects.is_empty() {
            return None;
        }

        let mut message = format!(
            "Tool '{tool_name}' is targeting a raw filesystem path inside a mounted workspace directory. \
             Mounted workspace content must be accessed via workspace URIs, not direct disk paths."
        );
        let guidance = match tool_name {
            "read_file" => "Use workspace_read with:",
            "list_dir" => "Use workspace_tree with:",
            _ => "Use workspace tools such as workspace_read or workspace_write with:",
        };
        message.push('\n');
        message.push_str(guidance);
        for (param_name, uri) in redirects {
            message.push_str(&format!("\n- {param_name}: {uri}"));
        }
        message.push_str("\nDo not access mounted workspace files through raw local paths.");
        Some(message)
    }

    pub(super) async fn approval_decision_for_tool(
        &self,
        session: &Arc<tokio::sync::Mutex<crate::agent::Session>>,
        user_id: &str,
        tool_name: &str,
        tool: &Arc<dyn Tool>,
        params: &serde_json::Value,
        task_mode: TaskMode,
    ) -> (bool, bool) {
        if task_mode == TaskMode::Yolo || self.config.auto_approve_tools {
            return (false, true);
        }

        let requirement = tool.requires_approval(params);
        let allow_always = !matches!(requirement, ApprovalRequirement::Always);
        let needs_approval = match requirement {
            ApprovalRequirement::Never => false,
            ApprovalRequirement::UnlessAutoApproved => {
                if self
                    .filesystem_access_is_preapproved(session, user_id, tool_name, params)
                    .await
                {
                    false
                } else {
                    let sess = session.lock().await;
                    !sess.is_tool_auto_approved(tool_name)
                }
            }
            ApprovalRequirement::Always => true,
        };

        (needs_approval, allow_always)
    }

    pub(super) async fn promote_filesystem_approval(
        &self,
        session: &Arc<tokio::sync::Mutex<crate::agent::Session>>,
        user_id: &str,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<(), Error> {
        let paths = self
            .resolve_filesystem_access_paths(tool_name, params)
            .map_err(Error::from)?;
        let roots = self.mount_scope_roots(tool_name, &paths);

        {
            let mut sess = session.lock().await;
            for root in &roots {
                sess.auto_approve_path_prefix(root.display().to_string());
            }
        }

        let Some(workspace) = self.tenant_ctx(user_id).await.workspace().cloned() else {
            return Ok(());
        };
        let mut existing_mounts = workspace.list_mounts().await?;
        for root in roots {
            if existing_mounts
                .iter()
                .any(|mount| root.starts_with(Path::new(&mount.mount.source_root)))
            {
                continue;
            }
            if !root.exists() || !root.is_dir() {
                tracing::warn!(path = %root.display(), "Skipping mount promotion for non-directory path");
                continue;
            }
            let display_name = root
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .unwrap_or("mount")
                .to_string();
            let summary = workspace
                .create_mount(display_name, root.display().to_string(), true)
                .await?;
            existing_mounts.push(summary);
        }
        Ok(())
    }

    pub(super) fn task_runtime(&self) -> Option<&Arc<TaskRuntime>> {
        self.deps.task_runtime.as_ref()
    }

    pub(super) fn emitter(&self) -> Option<&Arc<dyn RuntimeEventEmitter>> {
        self.deps.emitter.as_ref()
    }

    pub(super) fn emit_runtime_event_for_message(
        &self,
        message: &IncomingMessage,
        event: steward_common::AppEvent,
    ) {
        if let Some(emitter) = self.emitter() {
            tracing::info!(message_id = %message.id, event_type = %event.event_type(), "EMITTER: emitting event");
            emitter.emit_for_user(&message.user_id, event);
        } else {
            tracing::info!(message_id = %message.id, "EMITTER: no emitter available, event dropped");
        }
    }

    pub(super) fn emit_runtime_event_for_user(
        &self,
        user_id: &str,
        event: steward_common::AppEvent,
    ) {
        if let Some(emitter) = self.emitter() {
            tracing::info!(user_id, event_type = %event.event_type(), "EMITTER: emitting user-scoped event");
            emitter.emit_for_user(user_id, event);
        } else {
            tracing::info!(
                user_id,
                "EMITTER: no emitter available for user-scoped event"
            );
        }
    }

    pub(super) async fn task_mode_for_thread(&self, thread_id: Uuid) -> TaskMode {
        match self.task_runtime() {
            Some(runtime) => runtime.mode_for_task(thread_id).await,
            None => TaskMode::Ask,
        }
    }

    pub(super) fn owner_id(&self) -> &str {
        if let Some(workspace) = self.deps.workspace.as_ref() {
            debug_assert_eq!(
                workspace.user_id(),
                self.deps.owner_id,
                "workspace.user_id() must stay aligned with deps.owner_id"
            );
        }

        &self.deps.owner_id
    }

    pub(super) async fn send_channel_response(
        &self,
        message: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.channels.respond(message, response).await
    }

    pub(super) async fn send_channel_status(
        &self,
        channel_name: &str,
        status: crate::channels::StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Runtime-event mirroring is handled inside AgentChannels::send_status().
        self.channels
            .send_status(channel_name, status, metadata)
            .await
    }

    pub(super) async fn broadcast_channel(
        &self,
        channel_name: &str,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.channels
            .broadcast(channel_name, user_id, response)
            .await
    }

    pub(super) async fn shutdown_channels(&self) -> Result<(), ChannelError> {
        self.channels.shutdown_all().await
    }

    pub(super) async fn deliver_response(
        &self,
        message: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if message.channel == "desktop" {
            self.emit_runtime_event_for_message(
                message,
                steward_common::AppEvent::Response {
                    content: response.content,
                    thread_id: response
                        .thread_id
                        .unwrap_or_else(|| message.thread_id.clone().unwrap_or_default()),
                },
            );
            Ok(())
        } else {
            self.send_channel_response(message, response).await
        }
    }

    pub(super) async fn deliver_bootstrap_greeting(
        &self,
        channel_name: &str,
        user_id: &str,
        thread_id: Uuid,
    ) -> Result<(), ChannelError> {
        if channel_name == "desktop" {
            self.emit_runtime_event_for_user(
                user_id,
                steward_common::AppEvent::Response {
                    content: BOOTSTRAP_GREETING.to_string(),
                    thread_id: thread_id.to_string(),
                },
            );
            Ok(())
        } else {
            let mut out = OutgoingResponse::text(BOOTSTRAP_GREETING.to_string());
            out.thread_id = Some(thread_id.to_string());
            self.broadcast_channel(channel_name, user_id, out).await
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_message_stream(
        config: AgentConfig,
        deps: AgentDeps,
        message_stream: MessageStream,
        transport: Option<Arc<dyn MessageTransport>>,
        heartbeat_config: Option<HeartbeatConfig>,
        hygiene_config: Option<crate::config::HygieneConfig>,
        routine_config: Option<RoutineConfig>,
        context_manager: Option<Arc<ContextManager>>,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Self {
        Self::new_inner(
            config,
            deps,
            message_stream,
            transport,
            heartbeat_config,
            hygiene_config,
            routine_config,
            context_manager,
            session_manager,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        config: AgentConfig,
        deps: AgentDeps,
        message_stream: MessageStream,
        transport: Option<Arc<dyn MessageTransport>>,
        heartbeat_config: Option<HeartbeatConfig>,
        hygiene_config: Option<crate::config::HygieneConfig>,
        routine_config: Option<RoutineConfig>,
        context_manager: Option<Arc<ContextManager>>,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Self {
        let context_manager = context_manager
            .unwrap_or_else(|| Arc::new(ContextManager::new(config.max_parallel_jobs)));

        let session_manager = session_manager.unwrap_or_else(|| Arc::new(SessionManager::new()));

        let mut scheduler = Scheduler::new(
            config.clone(),
            context_manager.clone(),
            deps.llm.clone(),
            deps.safety.clone(),
            SchedulerDeps {
                tools: deps.tools.clone(),
                extension_manager: deps.extension_manager.clone(),
                store: deps
                    .store
                    .as_ref()
                    .map(|db| crate::tenant::AdminScope::new(Arc::clone(db))),
                hooks: deps.hooks.clone(),
                claude_code: deps.claude_code_config.clone(),
            },
        );
        if let Some(ref sse) = deps.sse_tx {
            scheduler.set_sse_sender(Arc::clone(sse));
        }
        if let Some(ref interceptor) = deps.http_interceptor {
            scheduler.set_http_interceptor(Arc::clone(interceptor));
        }
        let scheduler = Arc::new(scheduler);

        let emitter_for_channels = deps.emitter.clone();
        let owner_for_channels = deps.owner_id.clone();

        Self {
            config,
            deps,
            channels: AgentChannels::new(transport, emitter_for_channels, owner_for_channels),
            message_stream: tokio::sync::Mutex::new(Some(message_stream)),
            context_manager,
            scheduler,
            router: Router::new(),
            session_manager,
            context_monitor: ContextMonitor::new(),
            heartbeat_config,
            hygiene_config,
            routine_config,
            routine_engine_slot: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Replace the routine-engine slot with a shared one so desktop/runtime
    /// entry points reference the same engine.
    pub fn set_routine_engine_slot(
        &mut self,
        slot: Arc<tokio::sync::RwLock<Option<Arc<crate::agent::routine_engine::RoutineEngine>>>>,
    ) {
        self.routine_engine_slot = slot;
    }

    async fn routine_engine(&self) -> Option<Arc<crate::agent::routine_engine::RoutineEngine>> {
        self.routine_engine_slot.read().await.clone()
    }

    // Convenience accessors

    /// Get the scheduler (for external wiring, e.g. CreateJobTool).
    pub fn scheduler(&self) -> Arc<Scheduler> {
        Arc::clone(&self.scheduler)
    }

    pub(super) fn store(&self) -> Option<&Arc<dyn Database>> {
        self.deps.store.as_ref()
    }

    pub(super) fn llm(&self) -> &Arc<dyn LlmProvider> {
        &self.deps.llm
    }

    /// Get the cheap/fast LLM provider, falling back to the main one.
    pub(super) fn cheap_llm(&self) -> &Arc<dyn LlmProvider> {
        self.deps.cheap_llm.as_ref().unwrap_or(&self.deps.llm)
    }

    pub(super) fn safety(&self) -> &Arc<SafetyLayer> {
        &self.deps.safety
    }

    pub(super) fn tools(&self) -> &Arc<ToolRegistry> {
        &self.deps.tools
    }

    pub(super) fn workspace(&self) -> Option<&Arc<Workspace>> {
        self.deps.workspace.as_ref()
    }

    pub(super) fn memory(&self) -> Option<&Arc<MemoryManager>> {
        self.deps.memory.as_ref()
    }

    pub(super) fn hooks(&self) -> &Arc<HookRegistry> {
        &self.deps.hooks
    }

    pub(super) fn cost_guard(&self) -> &Arc<crate::agent::cost_guard::CostGuard> {
        &self.deps.cost_guard
    }

    /// Build a tenant-scoped execution context for the given user.
    ///
    /// This is the standard entry point for per-user operations. The returned
    /// [`TenantCtx`] provides a [`TenantScope`] that auto-binds `user_id` on
    /// every database operation and a per-user rate limiter.
    pub(super) async fn tenant_ctx(&self, user_id: &str) -> crate::tenant::TenantCtx {
        let rate = self.deps.tenant_rates.get_or_create(user_id).await;

        let store = self
            .deps
            .store
            .as_ref()
            .map(|db| crate::tenant::TenantScope::new(user_id, Arc::clone(db)));

        // Reuse the owner workspace if user matches, otherwise create per-user.
        // Per-user workspaces are seeded on first creation so they get identity
        // files and BOOTSTRAP.md (which triggers the onboarding greeting).
        let workspace = match &self.deps.workspace {
            Some(ws) if ws.user_id() == user_id => Some(Arc::clone(ws)),
            _ => {
                if let Some(db) = self.deps.store.as_ref() {
                    let ws = Arc::new(Workspace::new_with_db(user_id, Arc::clone(db)));
                    if let Err(e) = ws.seed_if_empty().await {
                        tracing::warn!(
                            user_id = user_id,
                            "Failed to seed per-user workspace: {}",
                            e
                        );
                    }
                    Some(ws)
                } else {
                    None
                }
            }
        };

        crate::tenant::TenantCtx::new(
            user_id,
            store,
            workspace,
            Arc::clone(&self.deps.cost_guard),
            rate,
        )
    }

    /// Get an admin-scoped database accessor for cross-tenant operations.
    ///
    /// Only for system-level components (heartbeat, routine engine, self-repair,
    /// scheduler). Handler code should use [`tenant_ctx()`](Self::tenant_ctx) instead.
    pub(super) fn admin_store(&self) -> Option<crate::tenant::AdminScope> {
        self.deps
            .store
            .as_ref()
            .map(|db| crate::tenant::AdminScope::new(Arc::clone(db)))
    }

    pub(super) fn skill_registry(&self) -> Option<&Arc<std::sync::RwLock<SkillRegistry>>> {
        self.deps.skill_registry.as_ref()
    }

    pub(super) fn skill_catalog(&self) -> Option<&Arc<crate::skills::catalog::SkillCatalog>> {
        self.deps.skill_catalog.as_ref()
    }

    /// Select active skills for a message using deterministic prefiltering.
    pub(super) fn select_active_skills(
        &self,
        message_content: &str,
    ) -> Vec<crate::skills::LoadedSkill> {
        if let Some(registry) = self.skill_registry() {
            let guard = match registry.read() {
                Ok(g) => g,
                Err(e) => {
                    tracing::error!("Skill registry lock poisoned: {}", e);
                    return vec![];
                }
            };
            let available = guard.skills();
            let skills_cfg = &self.deps.skills_config;
            let selected = crate::skills::prefilter_skills(
                message_content,
                available,
                skills_cfg.max_active_skills,
                skills_cfg.max_context_tokens,
            );

            if !selected.is_empty() {
                tracing::debug!(
                    "Selected {} skill(s) for message: {}",
                    selected.len(),
                    selected
                        .iter()
                        .map(|s| s.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            selected.into_iter().cloned().collect()
        } else {
            vec![]
        }
    }

    /// Run the agent main loop.
    pub async fn run(self) -> Result<(), Error> {
        // Proactive bootstrap: persist the static greeting to DB *before*
        // starting transport consumers so the first desktop client sees it via history.
        let bootstrap_thread_id = if self
            .workspace()
            .is_some_and(|ws| ws.take_bootstrap_pending())
        {
            tracing::debug!(
                "Fresh workspace detected — persisting static bootstrap greeting to DB"
            );
            if let Some(store) = self.store() {
                let thread_id = store
                    .get_or_create_assistant_conversation("default", "desktop")
                    .await
                    .ok();
                if let Some(id) = thread_id {
                    if let Err(error) = store
                        .add_conversation_message(id, "assistant", BOOTSTRAP_GREETING)
                        .await
                    {
                        tracing::warn!(
                            thread_id = %id,
                            %error,
                            "Failed to persist bootstrap greeting"
                        );
                    }
                }
                thread_id
            } else {
                None
            }
        } else {
            None
        };

        // Start the primary message ingress.
        let mut message_stream = self
            .message_stream
            .lock()
            .await
            .take()
            .expect("agent requires a configured message stream");

        // Start self-repair task with notification forwarding
        let mut self_repair = DefaultSelfRepair::new(
            self.context_manager.clone(),
            self.config.stuck_threshold,
            self.config.max_repair_attempts,
        );
        if let Some(admin) = self.admin_store() {
            self_repair = self_repair.with_store(admin);
        }
        if let Some(ref builder) = self.deps.builder {
            self_repair = self_repair.with_builder(Arc::clone(builder), Arc::clone(self.tools()));
        }
        let repair = Arc::new(self_repair);
        let repair_interval = self.config.repair_check_interval;
        let repair_channels = self.channels.clone();
        let repair_owner_id = self.owner_id().to_string();
        let repair_emitter = self.deps.emitter.clone();
        let repair_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(repair_interval).await;

                // Check stuck jobs
                let stuck_jobs = repair.detect_stuck_jobs().await;
                for job in stuck_jobs {
                    tracing::info!("Attempting to repair stuck job {}", job.job_id);
                    let result = repair.repair_stuck_job(&job).await;
                    let notification = match &result {
                        Ok(RepairResult::Success { message }) => {
                            tracing::info!("Repair succeeded: {}", message);
                            Some(format!(
                                "Job {} was stuck for {}s, recovery succeeded: {}",
                                job.job_id,
                                job.stuck_duration.as_secs(),
                                message
                            ))
                        }
                        Ok(RepairResult::Failed { message }) => {
                            tracing::error!("Repair failed: {}", message);
                            Some(format!(
                                "Job {} was stuck for {}s, recovery failed permanently: {}",
                                job.job_id,
                                job.stuck_duration.as_secs(),
                                message
                            ))
                        }
                        Ok(RepairResult::ManualRequired { message }) => {
                            tracing::warn!("Manual intervention needed: {}", message);
                            Some(format!(
                                "Job {} needs manual intervention: {}",
                                job.job_id, message
                            ))
                        }
                        Ok(RepairResult::Retry { message }) => {
                            tracing::warn!("Repair needs retry: {}", message);
                            None // Don't spam the user on retries
                        }
                        Err(e) => {
                            tracing::error!("Repair error: {}", e);
                            None
                        }
                    };

                    if let Some(msg) = notification {
                        let response = OutgoingResponse::text(format!("Self-Repair: {}", msg));
                        if let Some(ref emitter) = repair_emitter {
                            emitter.emit_for_user(
                                &repair_owner_id,
                                steward_common::AppEvent::Response {
                                    content: response.content.clone(),
                                    thread_id: String::new(),
                                },
                            );
                        }
                        let _ = repair_channels
                            .broadcast("desktop", &repair_owner_id, response)
                            .await;
                    }
                }

                // Check broken tools
                let broken_tools = repair.detect_broken_tools().await;
                for tool in broken_tools {
                    tracing::info!("Attempting to repair broken tool: {}", tool.name);
                    match repair.repair_broken_tool(&tool).await {
                        Ok(RepairResult::Success { message }) => {
                            let response = OutgoingResponse::text(format!(
                                "Self-Repair: Tool '{}' repaired: {}",
                                tool.name, message
                            ));
                            if let Some(ref emitter) = repair_emitter {
                                emitter.emit_for_user(
                                    &repair_owner_id,
                                    steward_common::AppEvent::Response {
                                        content: response.content.clone(),
                                        thread_id: String::new(),
                                    },
                                );
                            }
                            let _ = repair_channels
                                .broadcast("desktop", &repair_owner_id, response)
                                .await;
                        }
                        Ok(result) => {
                            tracing::info!("Tool repair result: {:?}", result);
                        }
                        Err(e) => {
                            tracing::error!("Tool repair error: {}", e);
                        }
                    }
                }
            }
        });

        // Spawn session pruning task
        let session_mgr = self.session_manager.clone();
        let session_idle_timeout = self.config.session_idle_timeout;
        let pruning_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600)); // Every 10 min
            interval.tick().await; // Skip immediate first tick
            loop {
                interval.tick().await;
                session_mgr.prune_stale_sessions(session_idle_timeout).await;
            }
        });

        // Spawn heartbeat if enabled
        let heartbeat_handle = if let Some(ref hb_config) = self.heartbeat_config {
            if hb_config.enabled {
                if let Some(workspace) = self.workspace() {
                    let mut config = AgentHeartbeatConfig::default()
                        .with_interval(std::time::Duration::from_secs(hb_config.interval_secs));
                    config.quiet_hours_start = hb_config.quiet_hours_start;
                    config.quiet_hours_end = hb_config.quiet_hours_end;
                    config.multi_tenant = hb_config.multi_tenant;
                    config.timezone = hb_config
                        .timezone
                        .clone()
                        .or_else(|| Some(self.config.default_timezone.clone()));
                    let heartbeat_notify_user = resolve_owner_scope_notification_user(
                        hb_config.notify_user.as_deref(),
                        Some(self.owner_id()),
                    );
                    if let Some(channel) = &hb_config.notify_channel
                        && let Some(user) = heartbeat_notify_user.as_deref()
                    {
                        config = config.with_notify(user, channel);
                    }

                    // Set up notification channel
                    let (notify_tx, mut notify_rx) =
                        tokio::sync::mpsc::channel::<OutgoingResponse>(16);

                    // Spawn notification forwarder that routes through channel manager
                    let notify_channel = hb_config.notify_channel.clone();
                    let notify_target = resolve_channel_notification_user(
                        self.deps.extension_manager.as_ref(),
                        hb_config.notify_channel.as_deref(),
                        hb_config.notify_user.as_deref(),
                        Some(self.owner_id()),
                    )
                    .await;
                    let notify_user = heartbeat_notify_user;
                    let channels = self.channels.clone();
                    let emitter = self.deps.emitter.clone();
                    let is_multi_tenant = hb_config.multi_tenant;
                    tokio::spawn(async move {
                        while let Some(response) = notify_rx.recv().await {
                            // In multi-tenant mode, extract the owning user_id from
                            // the response metadata so notifications reach the
                            // correct user rather than the agent's owner.
                            // This intentionally overrides the configured notify_target
                            // because each user's heartbeat should notify that user.
                            let effective_user = if is_multi_tenant {
                                response
                                    .metadata
                                    .get("owner_id")
                                    .and_then(|v| v.as_str())
                                    .map(String::from)
                            } else {
                                None
                            };

                            // Prefer the configured ingress target. If no transport
                            // is attached, use Tauri emitter for native delivery.
                            let targeted_ok = if let Some(channel) = notify_channel.as_ref() {
                                let target = effective_user.as_deref().or(notify_target.as_deref());
                                if let Some(user) = target {
                                    if let Some(ref emitter) = emitter {
                                        emitter.emit_for_user(
                                            user,
                                            steward_common::AppEvent::Response {
                                                content: response.content.clone(),
                                                thread_id: response
                                                    .thread_id
                                                    .clone()
                                                    .unwrap_or_default(),
                                            },
                                        );
                                    }
                                    channels
                                        .broadcast(channel, user, response.clone())
                                        .await
                                        .is_ok()
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            if !targeted_ok
                                && let Some(user) =
                                    effective_user.as_deref().or(notify_user.as_deref())
                            {
                                if let Some(ref emitter) = emitter {
                                    emitter.emit_for_user(
                                        user,
                                        steward_common::AppEvent::Response {
                                            content: response.content.clone(),
                                            thread_id: response
                                                .thread_id
                                                .clone()
                                                .unwrap_or_default(),
                                        },
                                    );
                                }
                            } else if !targeted_ok
                                && effective_user
                                    .as_deref()
                                    .or(notify_user.as_deref())
                                    .is_none()
                            {
                                tracing::warn!(
                                    "Dropping heartbeat notification with no user target"
                                );
                            }
                        }
                    });

                    let hygiene = self
                        .hygiene_config
                        .as_ref()
                        .map(|h| h.to_workspace_config())
                        .unwrap_or_default();

                    if config.multi_tenant {
                        if let Some(admin) = self.admin_store() {
                            Some(spawn_multi_user_heartbeat(
                                config,
                                hygiene,
                                self.cheap_llm().clone(),
                                Some(notify_tx),
                                admin,
                            ))
                        } else {
                            tracing::warn!("Multi-tenant heartbeat requires a database store");
                            None
                        }
                    } else {
                        Some(spawn_heartbeat(
                            config,
                            hygiene,
                            workspace.clone(),
                            self.memory().cloned(),
                            self.cheap_llm().clone(),
                            Some(notify_tx),
                            self.admin_store(),
                        ))
                    }
                } else {
                    tracing::warn!("Heartbeat enabled but no workspace available");
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Spawn routine engine if enabled
        let routine_handle = if let Some(ref rt_config) = self.routine_config {
            if rt_config.enabled {
                if let (Some(store), Some(workspace)) = (self.store(), self.workspace()) {
                    // Set up notification channel (same pattern as heartbeat)
                    let (notify_tx, mut notify_rx) =
                        tokio::sync::mpsc::channel::<OutgoingResponse>(32);

                    let engine = Arc::new(RoutineEngine::new(
                        rt_config.clone(),
                        crate::tenant::AdminScope::new(Arc::clone(store)),
                        self.llm().clone(),
                        Arc::clone(workspace),
                        self.memory().cloned(),
                        notify_tx,
                        Some(self.scheduler.clone()),
                        self.deps.extension_manager.clone(),
                        self.tools().clone(),
                        self.safety().clone(),
                    ));

                    // Register routine tools
                    self.deps
                        .tools
                        .register_routine_tools(Arc::clone(store), Arc::clone(&engine));

                    // Load initial event cache
                    engine.refresh_event_cache().await;

                    // Spawn notification forwarder (mirrors heartbeat pattern)
                    let channels = self.channels.clone();
                    let emitter = self.deps.emitter.clone();
                    let extension_manager = self.deps.extension_manager.clone();
                    tokio::spawn(async move {
                        while let Some(response) = notify_rx.recv().await {
                            let notify_channel = response
                                .metadata
                                .get("notify_channel")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let fallback_user = resolve_owner_scope_notification_user(
                                response
                                    .metadata
                                    .get("notify_user")
                                    .and_then(|v| v.as_str()),
                                response.metadata.get("owner_id").and_then(|v| v.as_str()),
                            );
                            let Some(user) = resolve_routine_notification_target(
                                extension_manager.as_ref(),
                                &response.metadata,
                            )
                            .await
                            else {
                                tracing::warn!(
                                    notify_channel = ?notify_channel,
                                    "Skipping routine notification with no explicit target or owner scope"
                                );
                                continue;
                            };

                            // Prefer the configured ingress target. If no transport
                            // is attached, runtime events remain the desktop delivery path.
                            let targeted_ok = if let Some(channel) = notify_channel.as_ref() {
                                if let Some(ref emitter) = emitter {
                                    emitter.emit_for_user(
                                        &user,
                                        steward_common::AppEvent::Response {
                                            content: response.content.clone(),
                                            thread_id: response
                                                .thread_id
                                                .clone()
                                                .unwrap_or_default(),
                                        },
                                    );
                                }
                                match channels.broadcast(channel, &user, response.clone()).await {
                                    Ok(()) => true,
                                    Err(e) => {
                                        let should_fallback =
                                            should_fallback_routine_notification(&e);
                                        tracing::warn!(
                                            channel = %channel,
                                            user = %user,
                                            error = %e,
                                            should_fallback,
                                            "Failed to send routine notification to configured channel"
                                        );
                                        if !should_fallback {
                                            continue;
                                        }
                                        false
                                    }
                                }
                            } else {
                                false
                            };

                            if !targeted_ok && let Some(user) = fallback_user {
                                if let Some(ref emitter) = emitter {
                                    emitter.emit_for_user(
                                        &user,
                                        steward_common::AppEvent::Response {
                                            content: response.content.clone(),
                                            thread_id: response
                                                .thread_id
                                                .clone()
                                                .unwrap_or_default(),
                                        },
                                    );
                                }
                            }
                        }
                    });

                    // Spawn cron ticker
                    let cron_interval =
                        std::time::Duration::from_secs(rt_config.cron_check_interval_secs);
                    let cron_handle = spawn_cron_ticker(Arc::clone(&engine), cron_interval);

                    // Store engine reference for event trigger checking
                    // Safety: we're in run() which takes self, no other reference exists
                    let engine_ref = Arc::clone(&engine);
                    // SAFETY: self is consumed by run(), we can smuggle the engine in
                    // via a local to use in the message loop below.

                    // Expose engine for manual triggering through shared runtime paths
                    *self.routine_engine_slot.write().await = Some(Arc::clone(&engine));

                    tracing::debug!(
                        "Routines enabled: cron ticker every {}s, max {} concurrent",
                        rt_config.cron_check_interval_secs,
                        rt_config.max_concurrent_routines
                    );

                    Some((cron_handle, engine_ref))
                } else {
                    tracing::warn!("Routines enabled but store/workspace not available");
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Bootstrap phase 2: register the thread in session manager and
        // broadcast the greeting via runtime events for any clients already connected.
        // The greeting was already persisted to DB before start_all(), so
        // clients that connect after this point will see it via history.
        if let Some(id) = bootstrap_thread_id {
            // Use get_or_create_session (not resolve_thread) to avoid creating
            // an orphan thread. Then insert the DB-sourced thread directly.
            let session = self.session_manager.get_or_create_session("default").await;
            {
                use crate::agent::session::Thread;
                let mut sess = session.lock().await;
                let thread = Thread::with_id(id, sess.id);
                sess.active_thread = Some(id);
                sess.threads.entry(id).or_insert(thread);
            }
            self.session_manager
                .register_thread("default", "desktop", id, session)
                .await;

            let _ = self
                .deliver_bootstrap_greeting("desktop", "default", id)
                .await;
        }

        // Main message loop
        tracing::info!("Agent {} ready and listening", self.config.name);

        loop {
            let message = tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => {
                    tracing::debug!("Ctrl+C received, shutting down...");
                    break;
                }
                msg = message_stream.next() => {
                    match msg {
                        Some(m) => {
                            tracing::info!(message_id = %m.id, channel = %m.channel, "MAINLOOP: Received message from stream");
                            m
                        }
                        None => {
                            tracing::debug!("All channel streams ended, shutting down...");
                            break;
                        }
                    }
                }
            };

            // Apply transcription middleware to audio attachments
            let mut message = message;
            if let Some(ref transcription) = self.deps.transcription {
                transcription.process(&mut message).await;
            }

            // Apply document extraction middleware to document attachments
            if let Some(ref doc_extraction) = self.deps.document_extraction {
                doc_extraction.process(&mut message).await;
            }

            // Store successfully extracted document text in workspace for indexing
            self.store_extracted_documents(&message).await;

            // Notify the desktop UI that processing has started.
            self.emit_runtime_event_for_message(
                &message,
                steward_common::AppEvent::Thinking {
                    message: "正在处理...".to_string(),
                    message_id: None,
                    thread_id: message.thread_id.clone(),
                },
            );
            tracing::info!(message_id = %message.id, "MAINLOOP: Thinking event sent via emitter");

            match self.handle_message(&message).await {
                Ok(Some(response)) if !response.is_empty() => {
                    // Hook: BeforeOutbound — allow hooks to modify or suppress outbound
                    // Final delivery happens after hooks so desktop receives the
                    // exact outbound payload once, from a single code path.
                    let event = crate::hooks::HookEvent::Outbound {
                        user_id: message.user_id.clone(),
                        channel: message.channel.clone(),
                        content: response.clone(),
                        thread_id: message.thread_id.clone(),
                    };
                    match self.hooks().run(&event).await {
                        Err(err) => {
                            tracing::warn!("BeforeOutbound hook blocked response: {}", err);
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_content),
                        }) => {
                            // Skip transport echo for desktop-originated messages.
                            if let Err(e) = self
                                .deliver_response(&message, OutgoingResponse::text(new_content))
                                .await
                            {
                                tracing::error!(
                                    channel = %message.channel,
                                    error = %e,
                                    "Failed to send response to channel"
                                );
                            }
                        }
                        _ => {
                            // Skip transport echo for desktop-originated messages.
                            if let Err(e) = self
                                .deliver_response(&message, OutgoingResponse::text(response))
                                .await
                            {
                                tracing::error!(
                                    channel = %message.channel,
                                    error = %e,
                                    "Failed to send response to channel"
                                );
                            }
                        }
                    }
                }
                Ok(Some(empty)) => {
                    // Empty response, nothing to send (e.g. approval handled via send_status)
                    tracing::debug!(
                        channel = %message.channel,
                        user = %message.user_id,
                        empty_len = empty.len(),
                        "Suppressed empty response (not sent to channel)"
                    );
                }
                Ok(None) => {
                    // Shutdown signal only from desktop-console /quit command.
                    // For desktop-driven threads, treat as empty response — never
                    // terminate the agent loop from a normal desktop message.
                    if message.channel == "desktop-console" {
                        tracing::debug!(
                            "Shutdown command received from {}, exiting...",
                            message.channel
                        );
                        break;
                    }
                    tracing::debug!(
                        channel = %message.channel,
                        "Ignoring shutdown signal from non-interactive channel"
                    );
                }
                Err(e) => {
                    self.emit_runtime_event_for_message(
                        &message,
                        steward_common::AppEvent::Error {
                            message: e.to_string(),
                            thread_id: message.thread_id.clone(),
                        },
                    );
                    tracing::error!("Error handling message: {}", e);
                    if let Err(send_err) = self
                        .send_channel_response(
                            &message,
                            OutgoingResponse::text(format!("Error: {}", e)),
                        )
                        .await
                    {
                        tracing::error!(
                            channel = %message.channel,
                            error = %send_err,
                            "Failed to send error response to channel"
                        );
                    }
                }
            }
        }

        // Cleanup
        tracing::debug!("Agent shutting down...");
        repair_handle.abort();
        pruning_handle.abort();
        if let Some(handle) = heartbeat_handle {
            handle.abort();
        }
        if let Some((cron_handle, _)) = routine_handle {
            cron_handle.abort();
        }
        self.scheduler.stop_all().await;
        self.shutdown_channels().await?;

        Ok(())
    }

    /// Store extracted document text in workspace memory for future search/recall.
    async fn store_extracted_documents(&self, message: &IncomingMessage) {
        let workspace = match self.workspace() {
            Some(ws) => ws,
            None => return,
        };

        for attachment in &message.attachments {
            if attachment.kind != crate::channels::AttachmentKind::Document {
                continue;
            }
            let text = match &attachment.extracted_text {
                Some(t) if !t.starts_with('[') => t, // skip error messages like "[Failed to..."
                _ => continue,
            };

            // Sanitize filename: strip path separators to prevent directory traversal
            let raw_name = attachment.filename.as_deref().unwrap_or("unnamed_document");
            let filename: String = raw_name
                .chars()
                .map(|c| {
                    if c == '/' || c == '\\' || c == '\0' {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            let filename = filename.trim_start_matches('.');
            let filename = if filename.is_empty() {
                "unnamed_document"
            } else {
                filename
            };
            let date = chrono::Utc::now().format("%Y-%m-%d");
            let path = format!("documents/{date}/{filename}");

            let header = format!(
                "# {filename}\n\n\
                 > Uploaded by **{}** via **{}** on {date}\n\
                 > MIME: {} | Size: {} bytes\n\n---\n\n",
                message.user_id,
                message.channel,
                attachment.mime_type,
                attachment.size_bytes.unwrap_or(0),
            );
            let content = format!("{header}{text}");

            match workspace.write(&path, &content).await {
                Ok(_) => {
                    tracing::info!(
                        path = %path,
                        text_len = text.len(),
                        "Stored extracted document in workspace memory"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path,
                        error = %e,
                        "Failed to store extracted document in workspace"
                    );
                }
            }
        }
    }

    async fn handle_message(&self, message: &IncomingMessage) -> Result<Option<String>, Error> {
        // Log sensitive details at debug level for troubleshooting
        tracing::info!(
            message_id = %message.id,
            user_id = %message.user_id,
            channel = %message.channel,
            thread_id = ?message.thread_id,
            "==> handle_message START"
        );

        // Internal messages (e.g. job-monitor notifications) are already
        // rendered text and should be forwarded directly to the user without
        // entering the normal user-input pipeline (LLM/tool loop).
        // The `is_internal` field and `into_internal()` setter are pub(crate),
        // so external channels cannot spoof this flag.
        if message.is_internal {
            tracing::debug!(
                message_id = %message.id,
                channel = %message.channel,
                "Forwarding internal message"
            );
            return Ok(Some(message.content.clone()));
        }

        // Parse submission type first
        let mut submission = SubmissionParser::parse(&message.content);
        tracing::trace!(
            "[agent_loop] Parsed submission: {:?}",
            std::any::type_name_of_val(&submission)
        );

        // Hook: BeforeInbound — allow hooks to modify or reject user input
        if let Submission::UserInput { ref content } = submission {
            let event = crate::hooks::HookEvent::Inbound {
                user_id: message.user_id.clone(),
                channel: message.channel.clone(),
                content: content.clone(),
                thread_id: message.thread_id.clone(),
            };
            match self.hooks().run(&event).await {
                Err(crate::hooks::HookError::Rejected { reason }) => {
                    return Ok(Some(format!("[Message rejected: {}]", reason)));
                }
                Err(err) => {
                    return Ok(Some(format!("[Message blocked by hook policy: {}]", err)));
                }
                Ok(crate::hooks::HookOutcome::Continue {
                    modified: Some(new_content),
                }) => {
                    submission = Submission::UserInput {
                        content: new_content,
                    };
                }
                _ => {} // Continue, fail-open errors already logged in registry
            }
        }

        let preferred_session_id = preferred_desktop_session_id(message);

        // Hydrate thread from DB if it's a historical thread not in memory
        if let Some(external_thread_id) = message.conversation_scope() {
            let external_thread_uuid = Uuid::parse_str(external_thread_id).ok();
            let thread_loaded_in_preferred_session = if let Some(session_id) = preferred_session_id
            {
                if let Some(session) = self
                    .session_manager
                    .get_session_by_id(&message.user_id, session_id)
                    .await
                {
                    let sess = session.lock().await;
                    external_thread_uuid
                        .map(|thread_id| sess.threads.contains_key(&thread_id))
                        .unwrap_or(false)
                } else {
                    false
                }
            } else {
                false
            };
            tracing::trace!(
                message_id = %message.id,
                thread_id = %external_thread_id,
                "Hydrating thread from DB"
            );
            if !thread_loaded_in_preferred_session
                && let Some(rejection) =
                    self.maybe_hydrate_thread(message, external_thread_id).await
            {
                return Ok(Some(format!("Error: {}", rejection)));
            }
        }

        // Resolve session and thread. Approval submissions are allowed to
        // target an already-loaded owned thread by UUID across channels so the
        // desktop approval UI can approve work that originated from other
        // owner-scoped runtime paths.
        let approval_thread_uuid = if matches!(
            submission,
            Submission::ExecApproval { .. } | Submission::ApprovalResponse { .. }
        ) {
            message
                .conversation_scope()
                .and_then(|thread_id| Uuid::parse_str(thread_id).ok())
        } else {
            None
        };

        let preferred_thread_uuid = message
            .conversation_scope()
            .and_then(|thread_id| Uuid::parse_str(thread_id).ok());

        let preferred_session_thread = if let (Some(session_id), Some(target_thread_id)) =
            (preferred_session_id, preferred_thread_uuid)
        {
            if let Some(session) = self
                .session_manager
                .get_session_by_id(&message.user_id, session_id)
                .await
            {
                let mut sess = session.lock().await;
                if sess.threads.contains_key(&target_thread_id) {
                    sess.active_thread = Some(target_thread_id);
                    sess.last_active_at = chrono::Utc::now();
                    drop(sess);
                    self.session_manager
                        .register_thread(
                            &message.user_id,
                            &message.channel,
                            target_thread_id,
                            Arc::clone(&session),
                        )
                        .await;
                    Some((session, target_thread_id))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let (session, thread_id) = if let Some((session, thread_id)) = preferred_session_thread {
            (session, thread_id)
        } else if let Some(target_thread_id) = approval_thread_uuid {
            let session = self
                .session_manager
                .get_or_create_session(&message.user_id)
                .await;
            let mut sess = session.lock().await;
            if sess.threads.contains_key(&target_thread_id) {
                sess.active_thread = Some(target_thread_id);
                sess.last_active_at = chrono::Utc::now();
                drop(sess);
                self.session_manager
                    .register_thread(
                        &message.user_id,
                        &message.channel,
                        target_thread_id,
                        Arc::clone(&session),
                    )
                    .await;
                (session, target_thread_id)
            } else {
                drop(sess);
                self.session_manager
                    .resolve_thread_with_parsed_uuid(
                        &message.user_id,
                        &message.channel,
                        message.conversation_scope(),
                        approval_thread_uuid,
                    )
                    .await
            }
        } else {
            self.session_manager
                .resolve_thread(
                    &message.user_id,
                    &message.channel,
                    message.conversation_scope(),
                )
                .await
        };
        tracing::debug!(
            message_id = %message.id,
            thread_id = %thread_id,
            "Resolved session and thread"
        );

        // Auth mode interception: if the thread is awaiting a token, route
        // the message directly to the credential store. Nothing touches
        // logs, turns, history, or compaction.
        let pending_auth = {
            let sess = session.lock().await;
            sess.threads
                .get(&thread_id)
                .and_then(|t| t.pending_auth.clone())
        };

        if let Some(pending) = pending_auth {
            if pending.is_expired() {
                // TTL exceeded — clear stale auth mode
                tracing::warn!(
                    extension = %pending.extension_name,
                    "Auth mode expired after TTL, clearing"
                );
                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.pending_auth = None;
                    }
                }
                // If this was a user message (possibly a pasted token), return an
                // explicit error instead of forwarding it to the LLM/history.
                if matches!(submission, Submission::UserInput { .. }) {
                    return Ok(Some(format!(
                        "Authentication for **{}** expired. Please try again.",
                        pending.extension_name
                    )));
                }
                // Control submissions (interrupt, undo, etc.) fall through to normal handling
            } else {
                match &submission {
                    Submission::UserInput { content } => {
                        return self
                            .process_auth_token(message, &pending, content, session, thread_id)
                            .await;
                    }
                    _ => {
                        // Any control submission (interrupt, undo, etc.) cancels auth mode
                        let mut sess = session.lock().await;
                        if let Some(thread) = sess.threads.get_mut(&thread_id) {
                            thread.pending_auth = None;
                        }
                        // Fall through to normal handling
                    }
                }
            }
        }

        tracing::trace!(
            "Received message from {} on {} ({} chars)",
            message.user_id,
            message.channel,
            message.content.len()
        );

        if !message.is_internal
            && let Submission::UserInput { ref content } = submission
            && let Some(engine) = self.routine_engine().await
        {
            let single_message_console = is_single_message_console(message);
            // Use post-hook content so that BeforeInbound hooks that rewrite
            // input are respected by event trigger matching.
            let fired = if single_message_console {
                engine.check_event_triggers_and_wait(message, content).await
            } else {
                engine.check_event_triggers(message, content).await
            };
            if fired > 0 {
                tracing::debug!(
                    channel = %message.channel,
                    user = %message.user_id,
                    fired,
                    "Consumed inbound user message with matching event-triggered routine(s)"
                );
                return if single_message_console {
                    Ok(None)
                } else {
                    Ok(Some(String::new()))
                };
            }
        }

        // Build per-tenant execution context once; threaded through all handlers.
        let tenant = self.tenant_ctx(&message.user_id).await;

        // Per-user bootstrap: if this user's workspace was just seeded (fresh),
        // persist the static greeting to their assistant conversation and
        // broadcast it so the desktop client shows it immediately.
        if tenant
            .workspace()
            .is_some_and(|ws| ws.take_bootstrap_pending())
        {
            tracing::info!(
                user_id = message.user_id,
                "Fresh user workspace — persisting bootstrap greeting"
            );
            if let Some(store) = tenant.store()
                && let Ok(conv_id) = store
                    .get_or_create_assistant_conversation(&message.channel)
                    .await
            {
                let _ = store
                    .add_conversation_message(conv_id, "assistant", BOOTSTRAP_GREETING)
                    .await;
                let mut out = OutgoingResponse::text(BOOTSTRAP_GREETING.to_string());
                out.thread_id = Some(conv_id.to_string());
                let _ = self
                    .deliver_bootstrap_greeting(&message.channel, &message.user_id, conv_id)
                    .await;
            }
        }

        let session_for_empty_exit = Arc::clone(&session);

        // Process based on submission type
        let result = match submission {
            Submission::UserInput { content } => {
                let mut result = self
                    .process_user_input(
                        message,
                        tenant.clone(),
                        session.clone(),
                        thread_id,
                        &content,
                    )
                    .await;

                // Drain any messages queued during processing.
                // Messages are merged (newline-separated) so the LLM receives
                // full context from rapid consecutive inputs instead of
                // processing each as a separate turn with partial context (#259).
                //
                // Only `Response` continues the drain — the user got a normal
                // reply and there may be more queued messages to process.
                //
                // Everything else stops the loop:
                // - `NeedApproval`: thread is blocked on user approval
                // - `Interrupted`: turn was cancelled
                // - `Ok`: control-command acknowledgment (including the "queued"
                //    ack returned when a message arrives during Processing)
                // - `Error`: soft error — draining more messages after an error
                //    would produce confusing interleaved output
                // - `Err(_)`: hard error
                while let Ok(SubmissionResult::Response { content: outgoing }) = &result {
                    let merged = {
                        let mut sess = session.lock().await;
                        sess.threads
                            .get_mut(&thread_id)
                            .and_then(|t| t.drain_pending_messages())
                    };
                    let Some(next_content) = merged else {
                        break;
                    };

                    tracing::debug!(
                        thread_id = %thread_id,
                        merged_len = next_content.len(),
                        "Drain loop: processing merged queued messages"
                    );

                    // Send the completed turn's response before starting the next.
                    //
                    // Known limitations:
                    // - One-shot channels (HttpChannel) consume the response
                    //   sender on the first respond() call keyed by msg.id.
                    //   Subsequent calls (including the outer handler's final
                    //   respond) are silently dropped. For one-shot channels
                    //   only this intermediate response is delivered.
                    // - All drain-loop responses are routed via the original
                    //   `message`, so channels that key routing on message
                    //   identity will attribute every response to the first
                    //   message. This is acceptable for the current
                    //   single-user-per-thread model.
                    //
                    // Desktop channel bypasses the channel transport entirely
                    // (responses go through SSE emitter in handle_message), so
                    // skip respond() for desktop to avoid misleading "Channel not
                    // found" warnings when the drain loop runs.
                    if message.channel != "desktop" {
                        if let Err(e) = self
                            .channels
                            .respond(message, OutgoingResponse::text(outgoing.clone()))
                            .await
                        {
                            tracing::warn!(
                                thread_id = %thread_id,
                                "Failed to send intermediate drain-loop response: {e}"
                            );
                        }
                    }

                    // Process merged queued messages as a single turn.
                    // Use a message clone with cleared attachments so
                    // augment_with_attachments doesn't re-apply the original
                    // message's attachments to unrelated queued text.
                    let mut queued_msg = message.clone();
                    queued_msg.id = Uuid::new_v4();
                    queued_msg.attachments.clear();
                    result = self
                        .process_user_input(
                            &queued_msg,
                            tenant.clone(),
                            session.clone(),
                            thread_id,
                            &next_content,
                        )
                        .await;

                    // If processing failed, re-queue the drained content so it
                    // isn't lost. It will be picked up on the next successful turn.
                    if !matches!(&result, Ok(SubmissionResult::Response { .. })) {
                        let mut sess = session.lock().await;
                        if let Some(thread) = sess.threads.get_mut(&thread_id) {
                            thread.requeue_drained(next_content);
                            tracing::debug!(
                                thread_id = %thread_id,
                                "Re-queued drained content after non-Response result"
                            );
                        }
                    }
                }

                result
            }
            Submission::SystemCommand { command, args } => {
                tracing::debug!(
                    "[agent_loop] SystemCommand: command={}, channel={}",
                    command,
                    message.channel
                );
                // /reasoning is special-cased here (not in handle_system_command)
                // because it needs the session + thread_id to read turn reasoning
                // data, which handle_system_command's signature doesn't provide.
                if command == "reasoning" {
                    let result = self
                        .handle_reasoning_command(&args, &session, thread_id)
                        .await;
                    return match result {
                        SubmissionResult::Response { content } => Ok(Some(content)),
                        SubmissionResult::Ok { message } => Ok(message),
                        SubmissionResult::Error { message } => {
                            Ok(Some(format!("Error: {}", message)))
                        }
                        _ => {
                            if is_single_message_console(message) {
                                Ok(None)
                            } else {
                                Ok(Some(String::new()))
                            }
                        }
                    };
                }
                // Authorization checks (including restart channel check) are enforced in handle_system_command
                self.handle_system_command(&command, &args, &message.channel, &tenant)
                    .await
            }
            Submission::Undo => self.process_undo(session, thread_id).await,
            Submission::Redo => self.process_redo(session, thread_id).await,
            Submission::Interrupt => self.process_interrupt(session, thread_id).await,
            Submission::Compact => self.process_compact(session, thread_id).await,
            Submission::Clear => self.process_clear(session, thread_id).await,
            Submission::NewThread => self.process_new_thread(message).await,
            Submission::Heartbeat => self.process_heartbeat().await,
            Submission::Summarize => self.process_summarize(session, thread_id).await,
            Submission::Suggest => self.process_suggest(session, thread_id).await,
            Submission::JobStatus { job_id } => {
                self.process_job_status(&tenant, job_id.as_deref()).await
            }
            Submission::JobCancel { job_id } => self.process_job_cancel(&tenant, &job_id).await,
            Submission::Quit => return Ok(None),
            Submission::SwitchThread { thread_id: target } => {
                self.process_switch_thread(message, target).await
            }
            Submission::Resume { checkpoint_id } => {
                self.process_resume(session, thread_id, checkpoint_id).await
            }
            Submission::ExecApproval {
                request_id,
                approved,
                always,
            } => {
                self.process_approval(
                    message,
                    session,
                    thread_id,
                    Some(request_id),
                    approved,
                    always,
                )
                .await
            }
            Submission::ApprovalResponse { approved, always } => {
                self.process_approval(message, session, thread_id, None, approved, always)
                    .await
            }
        };

        // Convert SubmissionResult to response string
        match result? {
            SubmissionResult::Response { content } => {
                // Suppress silent replies (e.g. from group chat "nothing to say" responses)
                if crate::llm::is_silent_reply(&content) {
                    tracing::debug!("Suppressing silent reply token");
                    Ok(Some(String::new()))
                } else {
                    Ok(Some(content))
                }
            }
            SubmissionResult::Ok {
                message: output_message,
            } => {
                let should_exit = if output_message.as_deref() == Some("")
                    && is_single_message_console(message)
                {
                    let sess = session_for_empty_exit.lock().await;
                    sess.threads
                        .get(&thread_id)
                        .map(|thread| thread.state != ThreadState::AwaitingApproval)
                        .unwrap_or(true)
                } else {
                    false
                };

                if should_exit {
                    Ok(None)
                } else {
                    // Treat None as empty string so the main loop does not
                    // mis-interpret it as a shutdown signal.
                    Ok(Some(output_message.unwrap_or_default()))
                }
            }
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            SubmissionResult::Interrupted => Ok(Some("Interrupted.".into())),
            SubmissionResult::NeedApproval { .. } => {
                // ApprovalNeeded status was already sent by thread_ops.rs before
                // returning this result. Empty string signals the caller to skip
                // respond() (no duplicate text).
                Ok(Some(String::new()))
            }
        }
    }
}

/// Split a response string into UTF-8-safe chunks suitable for progressive
/// delivery to the desktop UI. Only used by tests now — real streaming replaces this.
#[cfg(test)]
fn split_into_stream_chunks(text: &str) -> Vec<String> {
    const MIN_CHARS: usize = 12;
    const MAX_CHARS: usize = 28;

    if text.chars().count() <= MAX_CHARS {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        let mut chars_seen = 0usize;
        let mut preferred_split = None;
        let mut split_at = remaining.len();

        for (idx, ch) in remaining.char_indices() {
            chars_seen += 1;
            let next_idx = idx + ch.len_utf8();

            if chars_seen >= MIN_CHARS && is_stream_chunk_boundary(ch) {
                preferred_split = Some(next_idx);
            }

            if chars_seen >= MAX_CHARS {
                split_at = preferred_split.unwrap_or(next_idx);
                break;
            }
        }

        if chars_seen < MAX_CHARS {
            split_at = remaining.len();
        }

        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
fn is_stream_chunk_boundary(ch: char) -> bool {
    matches!(
        ch,
        '\n' | ' '
            | '\t'
            | '.'
            | ','
            | '!'
            | '?'
            | ';'
            | ':'
            | '。'
            | '，'
            | '、'
            | '！'
            | '？'
            | '；'
            | '：'
    )
}

/// Convert a channel [`StatusUpdate`] into an [`AppEvent`] suitable for desktop
/// runtime emission. Returns `None` for status variants that have no meaningful
/// UI counterpart.
fn status_update_to_app_event(
    status: &crate::channels::StatusUpdate,
    thread_id: Option<String>,
) -> Option<steward_common::AppEvent> {
    use crate::channels::StatusUpdate;

    match status {
        StatusUpdate::Thinking(message) => Some(steward_common::AppEvent::Thinking {
            message: message.clone(),
            message_id: None,
            thread_id,
        }),
        StatusUpdate::ToolStarted {
            name,
            tool_call_id,
            parameters,
        } => Some(steward_common::AppEvent::ToolStarted {
            name: name.clone(),
            tool_call_id: tool_call_id.clone(),
            parameters: parameters.clone(),
            thread_id,
        }),
        StatusUpdate::ToolCompleted {
            name,
            tool_call_id,
            success,
            error,
            parameters,
        } => Some(steward_common::AppEvent::ToolCompleted {
            name: name.clone(),
            tool_call_id: tool_call_id.clone(),
            success: *success,
            error: error.clone(),
            parameters: parameters.clone(),
            thread_id,
        }),
        StatusUpdate::ToolResult {
            name,
            tool_call_id,
            preview,
        } => Some(steward_common::AppEvent::ToolResult {
            name: name.clone(),
            tool_call_id: tool_call_id.clone(),
            preview: preview.clone(),
            thread_id,
        }),
        StatusUpdate::StreamChunk(content) => Some(steward_common::AppEvent::StreamChunk {
            content: content.clone(),
            thread_id,
        }),
        StatusUpdate::Status(message) => Some(steward_common::AppEvent::Status {
            message: message.clone(),
            thread_id,
        }),
        StatusUpdate::ImageGenerated { data_url, path } => {
            Some(steward_common::AppEvent::ImageGenerated {
                data_url: data_url.clone(),
                path: path.clone(),
                thread_id,
            })
        }
        StatusUpdate::Suggestions { suggestions } => Some(steward_common::AppEvent::Suggestions {
            suggestions: suggestions.clone(),
            thread_id,
        }),
        StatusUpdate::TurnCost {
            input_tokens,
            output_tokens,
            cost_usd,
        } => Some(steward_common::AppEvent::TurnCost {
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
            cost_usd: cost_usd.clone(),
            thread_id,
        }),
        StatusUpdate::ReasoningUpdate {
            narrative,
            decisions,
        } => Some(steward_common::AppEvent::ReasoningUpdate {
            narrative: narrative.clone(),
            decisions: decisions
                .iter()
                .map(|d| steward_common::ToolDecisionDto {
                    tool_name: d.tool_name.clone(),
                    rationale: d.rationale.clone(),
                })
                .collect(),
            thread_id,
        }),
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            parameters,
            allow_always,
        } => Some(steward_common::AppEvent::ApprovalNeeded {
            request_id: request_id.clone(),
            tool_name: tool_name.clone(),
            description: description.clone(),
            parameters: serde_json::to_string(parameters).unwrap_or_default(),
            thread_id,
            allow_always: *allow_always,
        }),
        StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
        } => Some(steward_common::AppEvent::AuthRequired {
            extension_name: extension_name.clone(),
            instructions: instructions.clone(),
            auth_url: auth_url.clone(),
            setup_url: setup_url.clone(),
        }),
        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
        } => Some(steward_common::AppEvent::AuthCompleted {
            extension_name: extension_name.clone(),
            success: *success,
            message: message.clone(),
        }),
        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => Some(steward_common::AppEvent::JobStarted {
            job_id: job_id.clone(),
            title: title.clone(),
            browse_url: browse_url.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        chat_tool_execution_metadata, is_single_message_console, resolve_routine_notification_user,
        should_fallback_routine_notification, split_into_stream_chunks, truncate_for_preview,
    };
    #[cfg(feature = "libsql")]
    use crate::agent::{Agent, AgentDeps};
    use crate::channels::IncomingMessage;
    #[cfg(feature = "libsql")]
    use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
    use crate::error::ChannelError;
    #[cfg(feature = "libsql")]
    use crate::hooks::HookRegistry;
    #[cfg(feature = "libsql")]
    use crate::safety::SafetyLayer;
    #[cfg(feature = "libsql")]
    use crate::testing::{StubLlm, test_db};
    #[cfg(feature = "libsql")]
    use crate::tools::ToolRegistry;
    #[cfg(feature = "libsql")]
    use crate::workspace::WorkspaceUri;
    #[cfg(feature = "libsql")]
    use std::sync::Arc;
    #[cfg(feature = "libsql")]
    use std::time::Duration;
    #[cfg(feature = "libsql")]
    use tokio::sync::mpsc;
    #[cfg(feature = "libsql")]
    use tokio_stream::wrappers::ReceiverStream;

    #[cfg(feature = "libsql")]
    fn make_empty_message_stream() -> crate::channels::MessageStream {
        let (_tx, rx) = mpsc::channel(1);
        Box::pin(ReceiverStream::new(rx))
    }

    #[cfg(feature = "libsql")]
    fn make_test_agent_with_db(db: Arc<dyn crate::db::Database>) -> Agent {
        let deps = AgentDeps {
            owner_id: "default".to_string(),
            store: Some(db),
            llm: Arc::new(StubLlm::default()),
            cheap_llm: None,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            })),
            tools: Arc::new(ToolRegistry::new()),
            workspace: None,
            memory: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks: Arc::new(HookRegistry::new()),
            cost_guard: Arc::new(crate::agent::cost_guard::CostGuard::new(
                crate::agent::cost_guard::CostGuardConfig::default(),
            )),
            sse_tx: None,
            emitter: None,
            http_interceptor: None,
            transcription: None,
            document_extraction: None,
            claude_code_config: crate::config::ClaudeCodeConfig::default(),
            builder: None,
            llm_backend: "openai".to_string(),
            tenant_rates: Arc::new(crate::tenant::TenantRateRegistry::new(4, 3)),
            task_runtime: None,
        };

        Agent::new_with_message_stream(
            AgentConfig {
                name: "test-agent".to_string(),
                max_parallel_jobs: 1,
                job_timeout: Duration::from_secs(60),
                stuck_threshold: Duration::from_secs(60),
                repair_check_interval: Duration::from_secs(30),
                max_repair_attempts: 1,
                use_planning: false,
                session_idle_timeout: Duration::from_secs(300),
                allow_local_tools: false,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_cost_per_user_per_day_cents: None,
                max_tool_iterations: 8,
                auto_approve_tools: false,
                default_timezone: "UTC".to_string(),
                max_jobs_per_user: None,
                max_tokens_per_job: 0,
                multi_tenant: false,
                max_llm_concurrent_per_user: None,
                max_jobs_concurrent_per_user: None,
            },
            deps,
            make_empty_message_stream(),
            None,
            None,
            None,
            None,
            Some(Arc::new(crate::context::ContextManager::new(1))),
            None,
        )
    }

    #[test]
    fn test_truncate_short_input() {
        assert_eq!(truncate_for_preview("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_empty_input() {
        assert_eq!(truncate_for_preview("", 10), "");
    }

    #[test]
    fn test_truncate_exact_length() {
        assert_eq!(truncate_for_preview("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_over_limit() {
        let result = truncate_for_preview("hello world, this is long", 10);
        assert!(result.ends_with("..."));
        // "hello worl" = 10 chars + "..."
        assert_eq!(result, "hello worl...");
    }

    #[test]
    fn test_truncate_collapses_newlines() {
        let result = truncate_for_preview("line1\nline2\nline3", 100);
        assert!(!result.contains('\n'));
        assert_eq!(result, "line1 line2 line3");
    }

    #[test]
    fn test_truncate_collapses_whitespace() {
        let result = truncate_for_preview("hello   world", 100);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_truncate_multibyte_utf8() {
        // Each emoji is 4 bytes. Truncating at char boundary must not panic.
        let input = "😀😁😂🤣😃😄😅😆😉😊";
        let result = truncate_for_preview(input, 5);
        assert!(result.ends_with("..."));
        // First 5 chars = 5 emoji
        assert_eq!(result, "😀😁😂🤣😃...");
    }

    #[test]
    fn test_truncate_cjk_characters() {
        // CJK chars are 3 bytes each in UTF-8.
        let input = "你好世界测试数据很长的字符串";
        let result = truncate_for_preview(input, 4);
        assert_eq!(result, "你好世界...");
    }

    #[test]
    fn test_truncate_mixed_multibyte_and_ascii() {
        let input = "hello 世界 foo";
        let result = truncate_for_preview(input, 8);
        // 'h','e','l','l','o',' ','世','界' = 8 chars
        assert_eq!(result, "hello 世界...");
    }

    #[test]
    fn resolve_routine_notification_user_prefers_explicit_target() {
        let metadata = serde_json::json!({
            "notify_user": "12345",
            "owner_id": "owner-scope",
        });

        let resolved = resolve_routine_notification_user(&metadata);
        assert_eq!(resolved.as_deref(), Some("12345")); // safety: test-only assertion
    }

    #[test]
    fn resolve_routine_notification_user_falls_back_to_owner_scope() {
        let metadata = serde_json::json!({
            "notify_user": null,
            "owner_id": "owner-scope",
        });

        let resolved = resolve_routine_notification_user(&metadata);
        assert_eq!(resolved.as_deref(), Some("owner-scope")); // safety: test-only assertion
    }

    #[test]
    fn resolve_routine_notification_user_rejects_missing_values() {
        let metadata = serde_json::json!({
            "notify_user": "   ",
        });

        assert_eq!(resolve_routine_notification_user(&metadata), None); // safety: test-only assertion
    }

    #[test]
    fn chat_tool_execution_metadata_prefers_message_routing_target() {
        let message = IncomingMessage::new("desktop", "owner-scope", "hello")
            .with_sender_id("desktop-user")
            .with_thread("thread-7")
            .with_metadata(serde_json::json!({
                "surface": "desktop",
                "window_id": "main",
            }));

        let metadata = chat_tool_execution_metadata(&message);
        assert_eq!(
            metadata.get("notify_channel").and_then(|v| v.as_str()),
            Some("desktop")
        ); // safety: test-only assertion
        assert_eq!(
            metadata.get("notify_user").and_then(|v| v.as_str()),
            Some("desktop-user")
        ); // safety: test-only assertion
        assert_eq!(
            metadata.get("notify_thread_id").and_then(|v| v.as_str()),
            Some("thread-7")
        ); // safety: test-only assertion
    }

    #[test]
    fn chat_tool_execution_metadata_falls_back_to_user_scope_without_route() {
        let message = IncomingMessage::new("desktop", "owner-scope", "hello").with_sender_id("");

        let metadata = chat_tool_execution_metadata(&message);
        assert_eq!(
            metadata.get("notify_channel").and_then(|v| v.as_str()),
            Some("desktop")
        ); // safety: test-only assertion
        assert_eq!(
            metadata.get("notify_user").and_then(|v| v.as_str()),
            Some("owner-scope")
        ); // safety: test-only assertion
        assert_eq!(
            metadata.get("notify_thread_id"),
            Some(&serde_json::Value::Null)
        ); // safety: test-only assertion
    }

    #[test]
    fn targeted_routine_notifications_do_not_fallback_without_owner_route() {
        let error = ChannelError::MissingRoutingTarget {
            name: "desktop".to_string(),
            reason: "No stored owner routing target for channel 'desktop'.".to_string(),
        };

        assert!(!should_fallback_routine_notification(&error)); // safety: test-only assertion
    }

    #[test]
    fn targeted_routine_notifications_may_fallback_for_other_errors() {
        let error = ChannelError::SendFailed {
            name: "desktop".to_string(),
            reason: "timeout talking to channel".to_string(),
        };

        assert!(should_fallback_routine_notification(&error)); // safety: test-only assertion
    }

    #[test]
    fn single_message_console_detection_requires_console_channel_and_metadata_flag() {
        let console = IncomingMessage::new("desktop-console", "owner-scope", "hello")
            .with_metadata(serde_json::json!({ "single_message_mode": true }));
        let desktop = IncomingMessage::new("desktop", "owner-scope", "hello")
            .with_metadata(serde_json::json!({ "single_message_mode": true }));
        let plain_console = IncomingMessage::new("desktop-console", "owner-scope", "hello");

        assert!(is_single_message_console(&console)); // safety: test-only assertion
        assert!(!is_single_message_console(&desktop)); // safety: test-only assertion
        assert!(!is_single_message_console(&plain_console)); // safety: test-only assertion
    }

    #[test]
    fn split_into_stream_chunks_preserves_unicode_content() {
        let text = "调用一个工具试试。Hello! Tools are working correctly. 🎉 这是第二句，用来验证分片不会切坏 UTF-8。";
        let chunks = split_into_stream_chunks(text);

        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn split_into_stream_chunks_splits_medium_text_for_visible_streaming() {
        let text = "This response should be split into multiple visible chunks for the desktop UI.";
        let chunks = split_into_stream_chunks(text);

        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), text);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn mounted_workspace_redirect_rejects_raw_disk_paths() {
        let (db, _db_dir) = test_db().await;
        let agent = make_test_agent_with_db(db);
        let user_id = "mount-redirect-user";

        let mount_dir = tempfile::tempdir().expect("mount tempdir");
        let nested_dir = mount_dir.path().join("src");
        std::fs::create_dir_all(&nested_dir).expect("create nested dir");
        let raw_file = nested_dir.join("lib.rs");
        std::fs::write(&raw_file, "pub fn mounted() {}\n").expect("write mounted file");

        let tenant = agent.tenant_ctx(user_id).await;
        let workspace = tenant.workspace().cloned().expect("workspace");
        let mount = workspace
            .create_mount("project", mount_dir.path().display().to_string(), true)
            .await
            .expect("create mount");

        let redirect = agent
            .mounted_workspace_redirect_for_tool(
                user_id,
                "read_file",
                &serde_json::json!({ "path": raw_file.display().to_string() }),
            )
            .await
            .expect("redirect message");

        let expected_uri = WorkspaceUri::mount_uri(mount.mount.id, Some("src/lib.rs"));
        assert!(
            redirect.contains(&expected_uri),
            "redirect should include workspace uri, got: {redirect}"
        );
        assert!(
            redirect.contains("workspace_read"),
            "redirect should point the agent to workspace_read, got: {redirect}"
        );
        assert!(
            !redirect.contains(&raw_file.display().to_string()),
            "redirect should not echo the raw disk path back to the agent, got: {redirect}"
        );
    }
}
