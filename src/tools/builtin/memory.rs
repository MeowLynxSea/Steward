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
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};
use crate::workspace::{Workspace, paths};

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
    matches!(target, "memory" | "daily_log" | "heartbeat")
        || target == paths::MEMORY
        || target == paths::HEARTBEAT
        || target.starts_with("daily/")
}

fn is_workspace_mount_uri(path: &str) -> bool {
    path.starts_with("workspace://")
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

/// Tool for searching mounted workspace content.
///
/// Performs hybrid search (FTS + semantic) across indexed mounted workspace content.
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
        "Search indexed mounted workspace content. Use this when you need \
         project context from mounted files rather than graph-native long-term memory. \
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
}

/// Tool for writing mounted workspace files.
///
/// Use this to create or update mounted workspace files via `workspace://` URIs.
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
        "Write to mounted workspace files via `workspace://` URIs. \
         Use this only for real mounted project files such as `workspace://<mount-id>/src/main.rs`. \
         Do NOT use this for Steward memory, episodic recall, heartbeat procedures, or legacy \
         memory-file paths like MEMORY.md, HEARTBEAT.md, or daily/*.md; use memory_save for \
         agent memory. Use write_file for raw local filesystem paths."
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
                    "description": "Mounted workspace file target, always as a `workspace://<mount-id>/...` URI. Do not pass legacy memory-file paths such as 'memory', 'daily_log', 'heartbeat', 'MEMORY.md', 'HEARTBEAT.md', or 'daily/...'."
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to existing content. If false, replace entirely.",
                    "default": true
                },
                "layer": {
                    "type": "string",
                    "description": "Memory layer to write to (e.g. 'private', 'household', 'finance'). When omitted, writes to the workspace's default scope."
                },
                "force": {
                    "type": "boolean",
                    "description": "Skip privacy classification and write directly to the specified layer without redirect. Use when you're certain the content belongs in the target layer.",
                    "default": false
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
                 Use memory_save for durable/episodic/procedural graph memory.",
                target
            )));
        }

        if !is_workspace_mount_uri(target) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is not a mounted workspace URI. workspace_write only operates on mounted files via `workspace://<mount-id>/...`. \
                 Use write_file for raw local filesystem writes, or memory_save for Steward memory.",
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

        let layer = params.get("layer").and_then(|v| v.as_str());
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let resolved_path = target.to_string();

        // When a layer is specified, route through layer-aware methods for ALL targets.
        // Otherwise, use default workspace methods (which include injection scanning).
        let layer_result = if let Some(layer_name) = layer {
            let result = if append {
                workspace
                    .append_to_layer(layer_name, &resolved_path, content, force)
                    .await
                    .map_err(map_write_err)?
            } else {
                workspace
                    .write_to_layer(layer_name, &resolved_path, content, force)
                    .await
                    .map_err(map_write_err)?
            };
            Some((result.actual_layer, result.redirected))
        } else {
            // No layer specified — use default workspace methods.
            // Prompt injection scanning for system-prompt files is handled by
            // Workspace::write() / Workspace::append().
            if append {
                workspace
                    .append(&resolved_path, content)
                    .await
                    .map_err(map_write_err)?;
            } else {
                workspace
                    .write(&resolved_path, content)
                    .await
                    .map_err(map_write_err)?;
            }
            None
        };

        let mut output = serde_json::json!({
            "status": "written",
            "path": resolved_path,
            "append": append,
            "content_length": content.len(),
        });
        if let Some((actual_layer, redirected)) = layer_result {
            output["layer"] = serde_json::Value::String(actual_layer);
            output["redirected"] = serde_json::Value::Bool(redirected);
        }

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for reading mounted workspace files.
///
/// Use this to read the full content of a mounted workspace file.
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
        "Read a mounted workspace file via a `workspace://` URI. \
         Use this to read files shown by workspace_tree. NOT for local filesystem files \
         (use read_file for those) and NOT for Steward memory (use memory_open or memory_recall). \
         Do not pass absolute paths like '/Users/...' or 'C:\\...'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Mounted workspace URI to read, e.g. `workspace://<mount-id>/src/main.rs`"
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
                "'{}' looks like a local filesystem path. workspace_read only works with mounted workspace URIs. \
                 Use read_file for filesystem reads. For opening files in an editor, use shell with: open \"<absolute_path>\".",
                path
            )));
        }

        if !is_workspace_mount_uri(path) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is not a mounted workspace URI. workspace_read only operates on `workspace://<mount-id>/...` paths. \
                 Use read_file for raw local filesystem access, or memory_open for graph-native Steward memory.",
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
}

#[async_trait]
impl Tool for WorkspaceTreeTool {
    fn name(&self) -> &str {
        "workspace_tree"
    }

    fn description(&self) -> &str {
        "View mounted workspace trees under `workspace://`. \
         Use workspace_read to read files shown here, NOT read_file. \
         The workspace tree is separate from the local filesystem and represents \
         mounted working directories."
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

        if !is_workspace_mount_uri(path) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is not a mounted workspace URI. workspace_tree only operates on `workspace://` roots and mount paths.",
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

        // Compact output: just the tree array
        Ok(ToolOutput::success(
            serde_json::Value::Array(tree),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
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
        assert!(!looks_like_filesystem_path("MEMORY.md"));
        assert!(!looks_like_filesystem_path("daily/2026-03-11.md"));
        assert!(!looks_like_filesystem_path("projects/alpha/notes.md"));
    }

    #[test]
    fn detects_legacy_memory_targets() {
        assert!(is_legacy_memory_target("memory"));
        assert!(is_legacy_memory_target("daily_log"));
        assert!(is_legacy_memory_target("heartbeat"));
        assert!(is_legacy_memory_target("MEMORY.md"));
        assert!(is_legacy_memory_target("HEARTBEAT.md"));
        assert!(is_legacy_memory_target("daily/2026-03-11.md"));
        assert!(!is_legacy_memory_target("projects/alpha/notes.md"));
        assert!(!is_legacy_memory_target("context/vision.md"));
        assert!(!is_legacy_memory_target("workspace://mount/src/main.rs"));
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
