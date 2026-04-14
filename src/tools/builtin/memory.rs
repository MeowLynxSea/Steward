//! Workspace document tools.
//!
//! These tools allow the agent to:
//! - Search indexed workspace documents
//! - Read and write files in the workspace document store
//!
//! # Usage
//!
//! The agent should use `workspace_search` before answering questions that
//! depend on indexed workspace content.
//!
//! Use `workspace_write` to persist or update workspace documents.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use crate::workspace::{Workspace, WorkspaceUri, encode_allowlist_id, paths};

// ── WorkspaceResolver ──────────────────────────────────────────────

/// Resolves a workspace for a given user ID.
///
/// In single-user mode, always returns the same workspace.
/// In multi-tenant mode, creates per-user workspaces on demand.
#[async_trait]
pub trait WorkspaceResolver: Send + Sync {
    async fn resolve(&self, user_id: &str) -> Arc<Workspace>;
}

/// Returns a fixed workspace regardless of user ID (single-user mode).
pub struct FixedWorkspaceResolver {
    workspace: Arc<Workspace>,
}

impl FixedWorkspaceResolver {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl WorkspaceResolver for FixedWorkspaceResolver {
    async fn resolve(&self, _user_id: &str) -> Arc<Workspace> {
        Arc::clone(&self.workspace)
    }
}

/// Per-user workspace cache retained after removing the legacy web gateway.
pub struct WorkspacePool {
    db: Arc<dyn crate::db::Database>,
    embeddings: Option<Arc<dyn crate::workspace::EmbeddingProvider>>,
    embedding_cache_config: crate::workspace::EmbeddingCacheConfig,
    search_config: crate::config::WorkspaceSearchConfig,
    workspace_config: crate::config::WorkspaceConfig,
    cache: tokio::sync::RwLock<std::collections::HashMap<String, Arc<Workspace>>>,
}

impl WorkspacePool {
    pub fn new(
        db: Arc<dyn crate::db::Database>,
        embeddings: Option<Arc<dyn crate::workspace::EmbeddingProvider>>,
        embedding_cache_config: crate::workspace::EmbeddingCacheConfig,
        search_config: crate::config::WorkspaceSearchConfig,
        workspace_config: crate::config::WorkspaceConfig,
    ) -> Self {
        Self {
            db,
            embeddings,
            embedding_cache_config,
            search_config,
            workspace_config,
            cache: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    fn build_workspace(&self, user_id: &str) -> Workspace {
        let mut ws = Workspace::new_with_db(user_id, Arc::clone(&self.db))
            .with_search_config(&self.search_config);

        if let Some(ref emb) = self.embeddings {
            ws = ws.with_embeddings_cached(Arc::clone(emb), self.embedding_cache_config.clone());
        }

        if !self.workspace_config.read_scopes.is_empty() {
            ws = ws.with_additional_read_scopes(self.workspace_config.read_scopes.clone());
        }

        ws.with_memory_layers(self.workspace_config.memory_layers.clone())
            .with_allowlist_watch_config(
                self.workspace_config.allowlist_watch_enabled,
                self.workspace_config.allowlist_watch_interval_ms,
            )
    }
}

#[async_trait]
impl WorkspaceResolver for WorkspacePool {
    async fn resolve(&self, user_id: &str) -> Arc<Workspace> {
        {
            let cache = self.cache.read().await;
            if let Some(ws) = cache.get(user_id) {
                return Arc::clone(ws);
            }
        }

        let mut cache = self.cache.write().await;
        if let Some(ws) = cache.get(user_id) {
            return Arc::clone(ws);
        }

        let ws = Arc::new(self.build_workspace(user_id));
        cache.insert(user_id.to_string(), Arc::clone(&ws));
        ws
    }
}

/// Detect paths that are clearly local filesystem references, not workspace-memory docs.
///
/// Examples:
/// - `/Users/.../file.md` (Unix absolute)
/// - `C:\Users\...` or `D:/work/...` (Windows absolute)
/// - `~/notes.md` (home expansion shorthand)
fn looks_like_filesystem_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }

    if Path::new(path).is_absolute() || path.starts_with("~/") {
        return true;
    }

    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn is_legacy_memory_target(target: &str) -> bool {
    matches!(target, "heartbeat") || target == paths::HEARTBEAT
}

fn is_workspace_allowlist_uri(path: &str) -> bool {
    path.starts_with("workspace://")
}

fn parse_workspace_allowlist_target(path: &str) -> Result<(uuid::Uuid, Option<String>), ToolError> {
    match WorkspaceUri::parse(path)
        .map_err(|e| ToolError::ExecutionFailed(format!("Path parse failed: {e}")))?
    {
        Some(WorkspaceUri::AllowlistRoot(allowlist_id)) => Ok((allowlist_id, None)),
        Some(WorkspaceUri::AllowlistPath(allowlist_id, allowlist_path)) => {
            Ok((allowlist_id, Some(allowlist_path)))
        }
        Some(WorkspaceUri::Root) => Err(ToolError::InvalidParameters(
            "workspace:// root is not a valid allowlist target for this tool".to_string(),
        )),
        None => Err(ToolError::InvalidParameters(format!(
            "'{}' is not a allowlisted workspace URI",
            path
        ))),
    }
}

/// Map workspace write errors to tool errors, using `NotAuthorized` for
/// injection rejections so the LLM gets a clear signal to stop.
fn map_write_err(e: crate::error::WorkspaceError) -> ToolError {
    match e {
        crate::error::WorkspaceError::InjectionRejected { path, reason } => {
            ToolError::NotAuthorized(format!(
                "content rejected for '{path}': prompt injection detected ({reason})"
            ))
        }
        other => ToolError::ExecutionFailed(format!("Write failed: {other}")),
    }
}

fn persist_bootstrap_completed(workspace: &Workspace) {
    workspace.mark_bootstrap_completed();

    if !cfg!(test) {
        let toml_path = crate::settings::Settings::default_toml_path();
        if let Ok(Some(mut settings)) = crate::settings::Settings::load_toml(&toml_path)
            && !settings.bootstrap_onboarding_completed
        {
            settings.bootstrap_onboarding_completed = true;
            if let Err(e) = settings.save_toml(&toml_path) {
                tracing::warn!("failed to persist bootstrap_onboarding_completed: {e}");
            }
        }
    }
}

/// Mark onboarding bootstrap as completed.
///
/// This is the explicit public replacement for the removed
/// bootstrap-clearing legacy memory alias flow.
pub struct BootstrapCompleteTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl BootstrapCompleteTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for BootstrapCompleteTool {
    fn name(&self) -> &str {
        "bootstrap_complete"
    }

    fn description(&self) -> &str {
        "Mark first-run onboarding as complete by clearing BOOTSTRAP.md and persisting the completion flag."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let workspace = self.resolver.resolve(&ctx.user_id).await;

        workspace
            .write(paths::BOOTSTRAP, "")
            .await
            .map_err(map_write_err)?;
        persist_bootstrap_completed(&workspace);

        Ok(ToolOutput::success(
            serde_json::json!({
                "target": paths::BOOTSTRAP,
                "completed": true
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for searching allowlisted workspace content.
///
/// Performs hybrid search (FTS + semantic) across indexed allowlisted workspace content.
pub struct WorkspaceSearchTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceSearchTool {
    /// Create a new workspace search tool with a workspace resolver.
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    /// Create from a fixed workspace (backward compatibility).
    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceSearchTool {
    fn name(&self) -> &str {
        "workspace_search"
    }

    fn description(&self) -> &str {
        "Search indexed allowlisted workspace content. Use this when you need \
         project context from allowlisted files rather than graph-native long-term memory. \
         Returns relevant snippets with relevance scores."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query. Use natural language to describe what you're looking for."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5, max: 20)",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let query = require_str(&params, "query")?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(20) as usize;

        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let results = workspace
            .search(query, limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Search failed: {}", e)))?;

        let result_count = results.len();
        let output = serde_json::json!({
            "query": query,
            "results": results.into_iter().map(|r| serde_json::json!({
                "content": r.content,
                "score": r.score,
                "path": r.document_path,
                "document_id": r.document_id.to_string(),
                "is_hybrid_match": r.is_hybrid(),
            })).collect::<Vec<_>>(),
            "result_count": result_count,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal workspace content, trusted content
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

/// Tool for writing allowlisted workspace files.
///
/// Use this to create or update allowlisted workspace files via `workspace://` URIs.
pub struct WorkspaceWriteTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceWriteTool {
    /// Create a new workspace write tool with a workspace resolver.
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    /// Create from a fixed workspace (backward compatibility).
    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceWriteTool {
    fn name(&self) -> &str {
        "workspace_write"
    }

    fn description(&self) -> &str {
        "Write to allowlisted workspace files via `workspace://` URIs. \
         Use this only for real allowlisted project files such as `workspace://<allowlist-id>/src/main.rs`. \
         Do NOT use this for Steward memory or heartbeat procedures; use create_memory/update_memory for \
         agent memory, and use the workspace document APIs for workspace-owned files. Use write_file for raw local filesystem paths."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The content to write to a workspace document."
                },
                "target": {
                    "type": "string",
                    "description": "Allowlisted workspace file target, always as a `workspace://<allowlist-id>/...` URI. Do not pass legacy aliases such as 'heartbeat' or direct workspace document names like 'HEARTBEAT.md'."
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to existing content. If false, replace entirely.",
                    "default": true
                }
            },
            "required": ["content", "target"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let content = require_str(&params, "content")?;

        let target = require_str(&params, "target")?;

        if looks_like_filesystem_path(target) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' looks like a local filesystem path. workspace_write only works with workspace document paths. \
                 Use write_file for filesystem writes. For opening files in an editor, use shell with: open \"<absolute_path>\".",
                target
            )));
        }

        if is_legacy_memory_target(target) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is a legacy workspace memory target and is no longer Steward's runtime memory source. \
                 Use create_memory or update_memory for durable/episodic/procedural graph memory.",
                target
            )));
        }

        if !is_workspace_allowlist_uri(target) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is not a allowlisted workspace URI. workspace_write only operates on allowlisted files via `workspace://<allowlist-id>/...`. \
                 Use write_file for raw local filesystem writes, or create_memory/update_memory for Steward memory.",
                target
            )));
        }

        let workspace = self.resolver.resolve(&ctx.user_id).await;

        if content.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "content cannot be empty".to_string(),
            ));
        }

        let append = params
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let resolved_path = target.to_string();
        // Allowlisted workspace files now map directly to the real filesystem.
        // Append uses an explicit read-modify-write so workspace:// paths behave
        // the same way as raw filesystem append semantics.
        if append {
            let existing = match workspace.read(&resolved_path).await {
                Ok(doc) => Some(doc.content),
                Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => None,
                Err(err) => return Err(ToolError::ExecutionFailed(format!("Read failed: {err}"))),
            };
            let merged = match existing {
                Some(existing) if !existing.is_empty() => format!("{existing}\n{content}"),
                _ => content.to_string(),
            };
            workspace
                .write(&resolved_path, &merged)
                .await
                .map_err(map_write_err)?;
        } else {
            workspace
                .write(&resolved_path, content)
                .await
                .map_err(map_write_err)?;
        }

        let output = serde_json::json!({
            "status": "written",
            "path": resolved_path,
            "append": append,
            "content_length": content.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for reading allowlisted workspace files.
///
/// Use this to read the full content of a allowlisted workspace file.
pub struct WorkspaceReadTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceReadTool {
    /// Create a new workspace read tool with a workspace resolver.
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    /// Create from a fixed workspace (backward compatibility).
    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceReadTool {
    fn name(&self) -> &str {
        "workspace_read"
    }

    fn description(&self) -> &str {
        "Read a allowlisted workspace file via a `workspace://` URI. \
         Use this to read files shown by workspace_tree. NOT for local filesystem files \
         (use read_file for those) and NOT for Steward memory (use read_memory or search_memory). \
         Do not pass absolute paths like '/Users/...' or 'C:\\...'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Allowlisted workspace URI to read, e.g. `workspace://<allowlist-id>/src/main.rs`"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let path = require_str(&params, "path")?;

        if looks_like_filesystem_path(path) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' looks like a local filesystem path. workspace_read only works with allowlisted workspace URIs. \
                 Use read_file for filesystem reads. For opening files in an editor, use shell with: open \"<absolute_path>\".",
                path
            )));
        }

        if !is_workspace_allowlist_uri(path) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is not a allowlisted workspace URI. workspace_read only operates on `workspace://<allowlist-id>/...` paths. \
                 Use read_file for raw local filesystem access, or read_memory for graph-native Steward memory.",
                path
            )));
        }

        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let doc = workspace
            .read(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Read failed: {}", e)))?;

        let output = serde_json::json!({
            "path": doc.path,
            "content": doc.content,
            "word_count": doc.word_count(),
            "updated_at": doc.updated_at.to_rfc3339(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal workspace content
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

/// Tool for viewing workspace structure as a tree.
///
/// Returns a hierarchical view of files and directories with configurable depth.
pub struct WorkspaceTreeTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceTreeTool {
    /// Create a new workspace tree tool with a workspace resolver.
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    /// Create from a fixed workspace (backward compatibility).
    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }

    /// Recursively build tree structure.
    ///
    /// Returns a compact format where directories end with `/` and may have children.
    async fn build_tree(
        workspace: &Arc<Workspace>,
        path: &str,
        current_depth: usize,
        max_depth: usize,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        if current_depth > max_depth {
            return Ok(Vec::new());
        }

        let entries = workspace
            .list(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Tree failed: {}", e)))?;

        let mut result = Vec::new();
        for entry in entries {
            // Directories end with `/`, files don't
            let display_path = if entry.is_directory {
                format!("{}/", entry.name())
            } else {
                entry.name().to_string()
            };

            if entry.is_directory && current_depth < max_depth {
                let children = Box::pin(Self::build_tree(
                    workspace,
                    &entry.path,
                    current_depth + 1,
                    max_depth,
                ))
                .await?;
                if children.is_empty() {
                    result.push(serde_json::Value::String(display_path));
                } else {
                    result.push(serde_json::json!({ display_path: children }));
                }
            } else {
                result.push(serde_json::Value::String(display_path));
            }
        }

        Ok(result)
    }

    async fn build_allowlist_aliases(
        workspace: &Arc<Workspace>,
    ) -> Result<Vec<serde_json::Value>, ToolError> {
        let allowlists = workspace.list_allowlists().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Workspace alias lookup failed: {e}"))
        })?;

        Ok(allowlists
            .into_iter()
            .map(|summary| {
                let id = encode_allowlist_id(summary.allowlist.id);
                serde_json::json!({
                    "id": id,
                    "alias": summary.allowlist.display_name,
                    "uri": WorkspaceUri::allowlist_uri(summary.allowlist.id, None)
                })
            })
            .collect())
    }
}

#[async_trait]
impl Tool for WorkspaceTreeTool {
    fn name(&self) -> &str {
        "workspace_tree"
    }

    fn description(&self) -> &str {
        "View allowlisted workspace trees under `workspace://`. \
         Use workspace_read to read files shown here, NOT read_file. \
         The workspace tree is separate from the local filesystem and represents \
         allowlisted working directories. Paths always use short allowlist ids; \
         alias labels are informational only and cannot be used as selectors."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Root path to start from ('workspace://' for the full workspace root)",
                    "default": "workspace://"
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum depth to traverse (1 = immediate children only)",
                    "default": 1,
                    "minimum": 1,
                    "maximum": 10
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("workspace://");

        if !is_workspace_allowlist_uri(path) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is not a allowlisted workspace URI. workspace_tree only operates on `workspace://` roots and allowlist paths.",
                path
            )));
        }

        let depth = params
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, 10) as usize;

        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let tree = Self::build_tree(&workspace, path, 1, depth).await?;

        let aliases = Self::build_allowlist_aliases(&workspace).await?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "path": path,
                "tree": tree,
                "allowlist_aliases": aliases,
                "note": "Use workspace://<id>/... with the short id. Aliases are human-readable labels only and are not valid path selectors."
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceApplyPatchTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceApplyPatchTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceApplyPatchTool {
    fn name(&self) -> &str {
        "workspace_apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a targeted string replacement to a allowlisted workspace file on the real filesystem."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "replace_all": { "type": "boolean", "default": false }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let path = require_str(&params, "path")?;
        let old_string = require_str(&params, "old_string")?;
        let new_string = require_str(&params, "new_string")?;
        let replace_all = params
            .get("replace_all")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let current = workspace
            .read(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Read failed: {e}")))?;
        if !current.content.contains(old_string) {
            return Err(ToolError::ExecutionFailed(format!(
                "Target text not found in {}",
                path
            )));
        }
        let updated = if replace_all {
            current.content.replace(old_string, new_string)
        } else {
            current.content.replacen(old_string, new_string, 1)
        };
        workspace
            .write(path, &updated)
            .await
            .map_err(map_write_err)?;
        Ok(ToolOutput::success(
            serde_json::json!({
                "path": path,
                "replaced": true,
                "replace_all": replace_all
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceMoveTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceMoveTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceMoveTool {
    fn name(&self) -> &str {
        "workspace_move"
    }

    fn description(&self) -> &str {
        "Move or rename a file inside a allowlisted workspace on the real filesystem."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "source_path": { "type": "string" },
                "destination_path": { "type": "string" },
                "overwrite": { "type": "boolean", "default": false }
            },
            "required": ["source_path", "destination_path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let source_path = require_str(&params, "source_path")?;
        let destination_path = require_str(&params, "destination_path")?;
        let overwrite = params
            .get("overwrite")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let (source_allowlist, source_rel) = parse_workspace_allowlist_target(source_path)?;
        let (destination_allowlist, destination_rel) =
            parse_workspace_allowlist_target(destination_path)?;
        if source_allowlist != destination_allowlist {
            return Err(ToolError::InvalidParameters(
                "workspace_move only supports moves within the same allowlist".to_string(),
            ));
        }
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let view = workspace
            .move_allowlist_file(
                source_allowlist,
                source_rel.as_deref().unwrap_or(""),
                destination_rel.as_deref().unwrap_or(""),
                overwrite,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Move failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(view)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceDeleteTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceDeleteTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceDeleteTool {
    fn name(&self) -> &str {
        "workspace_delete"
    }

    fn description(&self) -> &str {
        "Delete a file from a allowlisted workspace on the real filesystem."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let path = require_str(&params, "path")?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        workspace
            .delete(path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Delete failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::json!({ "path": path, "deleted": true }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceDeleteTreeTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceDeleteTreeTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceDeleteTreeTool {
    fn name(&self) -> &str {
        "workspace_delete_tree"
    }

    fn description(&self) -> &str {
        "Delete a directory tree from a allowlisted workspace on the real filesystem."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "missing_ok": { "type": "boolean", "default": false }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let path = require_str(&params, "path")?;
        let missing_ok = params
            .get("missing_ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let (allowlist_id, allowlist_path) = parse_workspace_allowlist_target(path)?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let detail = workspace
            .delete_allowlist_tree(
                allowlist_id,
                allowlist_path.as_deref().unwrap_or(""),
                missing_ok,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Delete tree failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(detail)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceDiffTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceDiffTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceDiffTool {
    fn name(&self) -> &str {
        "workspace_diff"
    }

    fn description(&self) -> &str {
        "Compare the current real workspace tree with baseline or another revision/checkpoint."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": { "type": "string" },
                "from": { "type": "string" },
                "to": { "type": "string" },
                "include_content": { "type": "boolean", "default": true },
                "max_files": { "type": "integer" }
            },
            "required": ["scope"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let scope = require_str(&params, "scope")?;
        let include_content = params
            .get("include_content")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let max_files = params
            .get("max_files")
            .and_then(|value| value.as_u64())
            .map(|v| v as usize);
        let from = params
            .get("from")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        let to = params
            .get("to")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        let (allowlist_id, scope_path) = parse_workspace_allowlist_target(scope)?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let diff = workspace
            .diff_allowlist_between(
                allowlist_id,
                scope_path,
                from,
                to,
                include_content,
                max_files,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Diff failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(diff)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceHistoryTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceHistoryTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceHistoryTool {
    fn name(&self) -> &str {
        "workspace_history"
    }

    fn description(&self) -> &str {
        "List revisions and checkpoints for a allowlisted workspace."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": { "type": "string" },
                "limit": { "type": "integer", "default": 20 },
                "include_checkpoints": { "type": "boolean", "default": true }
            },
            "required": ["scope"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let scope = require_str(&params, "scope")?;
        let limit = params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(20) as usize;
        let include_checkpoints = params
            .get("include_checkpoints")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let (allowlist_id, scope_path) = parse_workspace_allowlist_target(scope)?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let history = workspace
            .allowlist_history(allowlist_id, scope_path, limit, None, include_checkpoints)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("History failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(history)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceCheckpointCreateTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceCheckpointCreateTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceCheckpointCreateTool {
    fn name(&self) -> &str {
        "workspace_checkpoint_create"
    }

    fn description(&self) -> &str {
        "Create a named checkpoint for the current or specified workspace revision."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": { "type": "string" },
                "label": { "type": "string" },
                "summary": { "type": "string" },
                "revision": { "type": "string" }
            },
            "required": ["scope"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let scope = require_str(&params, "scope")?;
        let label = params
            .get("label")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        let summary = params
            .get("summary")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        let revision = params
            .get("revision")
            .and_then(|value| value.as_str())
            .and_then(|value| uuid::Uuid::parse_str(value).ok());
        let (allowlist_id, _) = parse_workspace_allowlist_target(scope)?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let checkpoint = workspace
            .create_checkpoint(allowlist_id, label, summary, "agent", false, revision)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Checkpoint failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(checkpoint)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceCheckpointListTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceCheckpointListTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceCheckpointListTool {
    fn name(&self) -> &str {
        "workspace_checkpoint_list"
    }

    fn description(&self) -> &str {
        "List checkpoints for a allowlisted workspace."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": { "type": "string" },
                "limit": { "type": "integer" }
            },
            "required": ["scope"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let scope = require_str(&params, "scope")?;
        let limit = params
            .get("limit")
            .and_then(|value| value.as_u64())
            .map(|v| v as usize);
        let (allowlist_id, _) = parse_workspace_allowlist_target(scope)?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let checkpoints = workspace
            .list_allowlist_checkpoints(allowlist_id, limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Checkpoint list failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(checkpoints)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceRestoreTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceRestoreTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceRestoreTool {
    fn name(&self) -> &str {
        "workspace_restore"
    }

    fn description(&self) -> &str {
        "Restore a allowlisted workspace or subtree to a baseline, revision, or checkpoint."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": { "type": "string" },
                "target": { "type": "string" },
                "set_as_baseline": { "type": "boolean", "default": false },
                "dry_run": { "type": "boolean", "default": false },
                "create_checkpoint_before_restore": { "type": "boolean", "default": true }
            },
            "required": ["scope", "target"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let scope = require_str(&params, "scope")?;
        let target = require_str(&params, "target")?;
        let set_as_baseline = params
            .get("set_as_baseline")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let dry_run = params
            .get("dry_run")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let create_checkpoint_before_restore = params
            .get("create_checkpoint_before_restore")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let (allowlist_id, scope_path) = parse_workspace_allowlist_target(scope)?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let detail = workspace
            .restore_allowlist(
                allowlist_id,
                target,
                scope_path,
                set_as_baseline,
                dry_run,
                create_checkpoint_before_restore,
                "agent",
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Restore failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(detail)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceBaselineSetTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceBaselineSetTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceBaselineSetTool {
    fn name(&self) -> &str {
        "workspace_baseline_set"
    }

    fn description(&self) -> &str {
        "Set the baseline revision for a allowlisted workspace without changing disk contents."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": { "type": "string" },
                "target": { "type": "string" }
            },
            "required": ["scope", "target"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let scope = require_str(&params, "scope")?;
        let target = require_str(&params, "target")?;
        let (allowlist_id, _) = parse_workspace_allowlist_target(scope)?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let detail = workspace
            .set_allowlist_baseline(allowlist_id, target)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Set baseline failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(detail)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

pub struct WorkspaceRefreshTool {
    resolver: Arc<dyn WorkspaceResolver>,
}

impl WorkspaceRefreshTool {
    pub fn new(resolver: Arc<dyn WorkspaceResolver>) -> Self {
        Self { resolver }
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self {
            resolver: Arc::new(FixedWorkspaceResolver::new(workspace)),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceRefreshTool {
    fn name(&self) -> &str {
        "workspace_refresh"
    }

    fn description(&self) -> &str {
        "Force a refresh of a allowlisted workspace from the real filesystem."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": { "type": "string" }
            },
            "required": ["scope"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let scope = require_str(&params, "scope")?;
        let (allowlist_id, scope_path) = parse_workspace_allowlist_target(scope)?;
        let workspace = self.resolver.resolve(&ctx.user_id).await;
        let detail = workspace
            .refresh_allowlist(allowlist_id, scope_path.as_deref())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Refresh failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::to_value(detail)
                .map_err(|e| ToolError::ExecutionFailed(format!("Serialize failed: {e}")))?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

// Sanitization tests moved to workspace module (reject_if_injected, is_system_prompt_file).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_filesystem_paths() {
        assert!(looks_like_filesystem_path("/Users/nige/file.md"));
        assert!(looks_like_filesystem_path("C:\\Users\\nige\\file.md"));
        assert!(looks_like_filesystem_path("D:/work/file.md"));
        assert!(looks_like_filesystem_path("~/notes.md"));
    }

    #[test]
    fn allows_workspace_document_paths() {
        assert!(!looks_like_filesystem_path("HEARTBEAT.md"));
        assert!(!looks_like_filesystem_path("projects/alpha/notes.md"));
    }

    #[test]
    fn detects_legacy_memory_targets() {
        assert!(is_legacy_memory_target("memory"));
        assert!(is_legacy_memory_target("daily_log"));
        assert!(is_legacy_memory_target("heartbeat"));
        assert!(is_legacy_memory_target("HEARTBEAT.md"));
        assert!(!is_legacy_memory_target("projects/alpha/notes.md"));
        assert!(!is_legacy_memory_target("context/vision.md"));
        assert!(!is_legacy_memory_target(
            "workspace://allowlist/src/main.rs"
        ));
    }

    #[cfg(feature = "libsql")]
    mod per_user_resolver_tests {
        use super::*;

        async fn make_test_db() -> Arc<dyn crate::db::Database> {
            use crate::db::libsql::LibSqlBackend;
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let db_path = temp_dir.path().join("resolver_test.db");
            let backend = LibSqlBackend::new_local(&db_path)
                .await
                .expect("LibSqlBackend");
            <LibSqlBackend as crate::db::Database>::run_migrations(&backend)
                .await
                .expect("migrations");
            // Leak the tempdir so it outlives the test (cleaned up on process exit).
            std::mem::forget(temp_dir);
            Arc::new(backend)
        }

        #[tokio::test]
        async fn test_workspace_pool_resolver_returns_different_workspaces() {
            let db = make_test_db().await;

            let pool = WorkspacePool::new(
                db,
                None,
                crate::workspace::EmbeddingCacheConfig::default(),
                crate::config::WorkspaceSearchConfig::default(),
                crate::config::WorkspaceConfig::default(),
            );

            let ws_alice = pool.resolve("alice").await;
            let ws_bob = pool.resolve("bob").await;

            // Different user IDs should get different workspaces
            assert_eq!(ws_alice.user_id(), "alice");
            assert_eq!(ws_bob.user_id(), "bob");
            assert!(!Arc::ptr_eq(&ws_alice, &ws_bob));
        }

        #[tokio::test]
        async fn test_workspace_pool_resolver_caches_workspace() {
            let db = make_test_db().await;

            let pool = WorkspacePool::new(
                db,
                None,
                crate::workspace::EmbeddingCacheConfig::default(),
                crate::config::WorkspaceSearchConfig::default(),
                crate::config::WorkspaceConfig::default(),
            );

            let ws1 = pool.resolve("alice").await;
            let ws2 = pool.resolve("alice").await;

            // Same user_id should return the same cached Arc (pointer equality)
            assert!(Arc::ptr_eq(&ws1, &ws2));
        }
    }
}
