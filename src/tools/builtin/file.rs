//! File operation tools for reading, writing, and navigating the filesystem.
//!
//! These tools provide controlled access to the filesystem with:
//! - Path validation and sandboxing
//! - Size limits on read/write operations
//! - Support for common development tasks

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::fs;

use crate::context::JobContext;
use crate::tools::builtin::path_utils::validate_path;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};
use crate::workspace::paths as ws_paths;
use crate::workspace::{ResolvedWorkspaceMountPath, Workspace};

fn is_legacy_memory_workspace_path(path: &str) -> bool {
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(path);

    matches!(filename, ws_paths::HEARTBEAT)
}

/// Maximum file size for reading (1MB).
const MAX_READ_SIZE: u64 = 1024 * 1024;

/// Maximum file size for writing (5MB).
const MAX_WRITE_SIZE: usize = 5 * 1024 * 1024;

/// Maximum directory listing entries.
const MAX_DIR_ENTRIES: usize = 500;

async fn resolve_workspace_path_metadata(
    resolver: Option<&Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>>,
    user_id: &str,
    path: &Path,
) -> Result<Option<(Arc<Workspace>, ResolvedWorkspaceMountPath)>, ToolError> {
    let Some(resolver) = resolver else {
        return Ok(None);
    };
    let workspace = resolver.resolve(user_id).await;
    let resolved = workspace
        .resolve_mount_path(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Mounted path resolve failed: {e}")))?;
    Ok(resolved.map(|resolved| (workspace, resolved)))
}

async fn refresh_workspace_path_metadata(
    resolver: Option<&Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>>,
    user_id: &str,
    path: &Path,
) -> Result<Option<ResolvedWorkspaceMountPath>, ToolError> {
    let Some(resolver) = resolver else {
        return Ok(None);
    };
    let workspace = resolver.resolve(user_id).await;
    workspace
        .refresh_mount_for_disk_path(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Mounted refresh failed: {e}")))
}

/// Read file contents tool.
#[derive(Default)]
pub struct ReadFileTool {
    base_dir: Option<PathBuf>,
    workspace_resolver: Option<Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>>,
}

impl std::fmt::Debug for ReadFileTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadFileTool")
            .field("base_dir", &self.base_dir)
            .field("workspace_resolver", &self.workspace_resolver.is_some())
            .finish()
    }
}

impl ReadFileTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_workspace_resolver(
        mut self,
        resolver: Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>,
    ) -> Self {
        self.workspace_resolver = Some(resolver);
        self
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self::new().with_workspace_resolver(Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(workspace),
        ))
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file from the LOCAL FILESYSTEM. NOT for workspace document paths \
         (use workspace_read for those). If the path is inside a mounted workspace directory, \
         this still reads the real mounted file directly. Use `workspace://<mount-id>/...` \
         via workspace_read when you want workspace-native addressing and diff/history context. \
         Reading unmounted disk paths may require ask-mode approval. \
         Returns file content as text. For large files, you can specify offset and limit to read a portion."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read. Mounted files may be addressed either by real disk path here or by `workspace://<mount-id>/...` via workspace_read."
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-indexed, optional)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (optional)"
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
        let path_str = require_str(&params, "path")?;

        let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = params.get("limit").and_then(|v| v.as_u64());

        let start = std::time::Instant::now();

        let path = validate_path(path_str, self.base_dir.as_deref())?;
        let mounted = resolve_workspace_path_metadata(
            self.workspace_resolver.as_ref(),
            &ctx.user_id,
            &path,
        )
        .await?;

        // Check file size
        let metadata = fs::metadata(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Cannot access file: {}", e)))?;

        if metadata.len() > MAX_READ_SIZE {
            return Err(ToolError::ExecutionFailed(format!(
                "File too large ({} bytes). Maximum is {} bytes. Use offset/limit for partial reads.",
                metadata.len(),
                MAX_READ_SIZE
            )));
        }

        // Read file
        let content = fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read file: {}", e)))?;

        // Apply offset and limit
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start_line = if offset > 0 {
            offset.saturating_sub(1)
        } else {
            0
        };
        let end_line = if let Some(lim) = limit {
            (start_line + lim as usize).min(total_lines)
        } else {
            total_lines
        };

        let selected_lines: Vec<String> = lines[start_line..end_line]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>6}│ {}", start_line + i + 1, line))
            .collect();

        let result = serde_json::json!({
            "content": selected_lines.join("\n"),
            "total_lines": total_lines,
            "lines_shown": end_line - start_line,
            "path": path.display().to_string(),
            "workspace_uri": mounted.as_ref().map(|(_, resolved)| resolved.workspace_uri.clone()),
            "workspace_mount_id": mounted
                .as_ref()
                .map(|(_, resolved)| resolved.mount_id.to_string())
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // File content could contain anything
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Host
    }
}

/// Write file contents tool.
#[derive(Default)]
pub struct WriteFileTool {
    base_dir: Option<PathBuf>,
    workspace_resolver: Option<Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>>,
}

impl std::fmt::Debug for WriteFileTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteFileTool")
            .field("base_dir", &self.base_dir)
            .field("workspace_resolver", &self.workspace_resolver.is_some())
            .finish()
    }
}

impl WriteFileTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_workspace_resolver(
        mut self,
        resolver: Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>,
    ) -> Self {
        self.workspace_resolver = Some(resolver);
        self
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self::new().with_workspace_resolver(Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(workspace),
        ))
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file on the LOCAL FILESYSTEM. NOT for workspace documents \
         (use workspace_write for that). If the target is inside a mounted workspace directory, \
         this updates the real mounted file directly. Use `workspace://<mount-id>/...` via workspace_write \
         when you want workspace-native addressing, diff/history tooling, and revision-oriented workflows. \
         Writing unmounted disk paths may require ask-mode approval. \
         Creates the file if it doesn't exist, overwrites if it does. Parent directories are created automatically. Use apply_patch for targeted edits."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write. Mounted files may be addressed either by real disk path here or by `workspace://<mount-id>/...` via workspace_write."
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = require_str(&params, "path")?;

        if is_legacy_memory_workspace_path(path_str) {
            return Err(ToolError::InvalidParameters(format!(
                "'{}' is a workspace procedure document and should not be edited through raw filesystem tools. \
                 Use workspace_write or workspace_read for workspace-managed documents, and use graph memory tools for Steward memory.",
                path_str
            )));
        }

        let content = require_str(&params, "content")?;

        let start = std::time::Instant::now();

        // Check content size
        if content.len() > MAX_WRITE_SIZE {
            return Err(ToolError::InvalidParameters(format!(
                "Content too large ({} bytes). Maximum is {} bytes.",
                content.len(),
                MAX_WRITE_SIZE
            )));
        }

        let path = validate_path(path_str, self.base_dir.as_deref())?;

        // Create parent directories
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to create directories: {}", e))
            })?;
        }

        // Write file
        fs::write(&path, content)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write file: {}", e)))?;

        let mounted = refresh_workspace_path_metadata(
            self.workspace_resolver.as_ref(),
            &ctx.user_id,
            &path,
        )
        .await?;

        let result = serde_json::json!({
            "path": path.display().to_string(),
            "bytes_written": content.len(),
            "success": true,
            "workspace_uri": mounted.as_ref().map(|resolved| resolved.workspace_uri.clone()),
            "workspace_mount_id": mounted.as_ref().map(|resolved| resolved.mount_id.to_string())
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false // We're writing, not reading external data
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Host
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

/// Move a file to a new path.
#[derive(Default)]
pub struct MoveFileTool {
    base_dir: Option<PathBuf>,
    workspace_resolver: Option<Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>>,
}

impl std::fmt::Debug for MoveFileTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoveFileTool")
            .field("base_dir", &self.base_dir)
            .field("workspace_resolver", &self.workspace_resolver.is_some())
            .finish()
    }
}

impl MoveFileTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_workspace_resolver(
        mut self,
        resolver: Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>,
    ) -> Self {
        self.workspace_resolver = Some(resolver);
        self
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self::new().with_workspace_resolver(Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(workspace),
        ))
    }
}

#[async_trait]
impl Tool for MoveFileTool {
    fn name(&self) -> &str {
        "move_file"
    }

    fn description(&self) -> &str {
        "Move or rename a file on the LOCAL FILESYSTEM. If the source or destination is inside a mounted workspace directory, this moves the real mounted file directly. Use workspace_move when you want workspace-native URIs and revision/diff tracking ergonomics. Raw disk moves may require ask-mode approval. Creates parent directories for the destination when needed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "source_path": {
                    "type": "string",
                    "description": "Existing source file path. Mounted files may be addressed either by real disk path here or by `workspace://<mount-id>/...` via workspace_move."
                },
                "destination_path": {
                    "type": "string",
                    "description": "Destination file path. Mounted files may be addressed either by real disk path here or by `workspace://<mount-id>/...` via workspace_move."
                },
                "create_parent": {
                    "type": "boolean",
                    "description": "Create destination parent directories automatically (default true)"
                }
            },
            "required": ["source_path", "destination_path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let source_path = require_str(&params, "source_path")?;
        let destination_path = require_str(&params, "destination_path")?;
        let create_parent = params
            .get("create_parent")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);

        let start = std::time::Instant::now();
        let source = validate_path(source_path, self.base_dir.as_deref())?;
        let destination = validate_path(destination_path, self.base_dir.as_deref())?;

        let metadata = fs::metadata(&source)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Cannot access source file: {}", e)))?;
        if metadata.is_dir() {
            return Err(ToolError::InvalidParameters(
                "move_file only supports regular files".to_string(),
            ));
        }

        if create_parent && let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to create destination directories: {}",
                    e
                ))
            })?;
        }

        fs::rename(&source, &destination)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to move file: {}", e)))?;

        let source_mounted = refresh_workspace_path_metadata(
            self.workspace_resolver.as_ref(),
            &ctx.user_id,
            &source,
        )
        .await?;
        let destination_mounted = refresh_workspace_path_metadata(
            self.workspace_resolver.as_ref(),
            &ctx.user_id,
            &destination,
        )
        .await?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "source_path": source.display().to_string(),
                "destination_path": destination.display().to_string(),
                "success": true,
                "source_workspace_uri": source_mounted
                    .as_ref()
                    .map(|resolved| resolved.workspace_uri.clone()),
                "destination_workspace_uri": destination_mounted
                    .as_ref()
                    .map(|resolved| resolved.workspace_uri.clone())
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Host
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

/// List directory contents tool.
#[derive(Default)]
pub struct ListDirTool {
    base_dir: Option<PathBuf>,
    workspace_resolver: Option<Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>>,
}

impl std::fmt::Debug for ListDirTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListDirTool")
            .field("base_dir", &self.base_dir)
            .field("workspace_resolver", &self.workspace_resolver.is_some())
            .finish()
    }
}

impl ListDirTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_workspace_resolver(
        mut self,
        resolver: Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>,
    ) -> Self {
        self.workspace_resolver = Some(resolver);
        self
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self::new().with_workspace_resolver(Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(workspace),
        ))
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List contents of a directory on the LOCAL FILESYSTEM. NOT for workspace document trees \
         (use workspace_tree for that). If the directory is mounted into the workspace, this lists the real mounted directory directly. Use `workspace://<mount-id>` via workspace_tree when you want mounted-path status, diff state, and workspace-native browsing. Unmounted raw disk listings may still require ask-mode approval. Shows files and subdirectories with their sizes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list (defaults to current directory). Mounted directories may be addressed either by real disk path here or by `workspace://<mount-id>` via workspace_tree."
                },
                "recursive": {
                    "type": "boolean",
                    "description": "If true, list contents recursively (default false)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum depth for recursive listing (default 3)"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let recursive = params
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_depth = params
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;

        let start = std::time::Instant::now();

        let path = validate_path(path_str, self.base_dir.as_deref())?;
        let mounted = resolve_workspace_path_metadata(
            self.workspace_resolver.as_ref(),
            &ctx.user_id,
            &path,
        )
        .await?;

        let mut entries = Vec::new();
        list_dir_inner(&path, &path, recursive, max_depth, 0, &mut entries).await?;

        // Sort entries
        entries.sort_by(|a, b| {
            let a_is_dir = a.ends_with('/');
            let b_is_dir = b.ends_with('/');
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.cmp(b),
            }
        });

        let truncated = entries.len() > MAX_DIR_ENTRIES;
        if truncated {
            entries.truncate(MAX_DIR_ENTRIES);
        }

        let result = serde_json::json!({
            "path": path.display().to_string(),
            "entries": entries,
            "count": entries.len(),
            "truncated": truncated,
            "workspace_uri": mounted.as_ref().map(|(_, resolved)| resolved.workspace_uri.clone()),
            "workspace_mount_id": mounted
                .as_ref()
                .map(|(_, resolved)| resolved.mount_id.to_string())
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Directory listings are safe
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Host
    }
}

/// Recursively list directory contents.
async fn list_dir_inner(
    base: &Path,
    path: &Path,
    recursive: bool,
    max_depth: usize,
    current_depth: usize,
    entries: &mut Vec<String>,
) -> Result<(), ToolError> {
    if entries.len() >= MAX_DIR_ENTRIES {
        return Ok(());
    }

    let mut dir = fs::read_dir(path)
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read directory: {}", e)))?;

    while let Some(entry) = dir
        .next_entry()
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read entry: {}", e)))?
    {
        if entries.len() >= MAX_DIR_ENTRIES {
            break;
        }

        let entry_path = entry.path();
        let relative = entry_path
            .strip_prefix(base)
            .unwrap_or(&entry_path)
            .to_string_lossy();

        let metadata = entry.metadata().await.ok();
        let is_dir = metadata.as_ref().is_some_and(|m| m.is_dir());

        let display = if is_dir {
            format!("{}/", relative)
        } else {
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            format!("{} ({})", relative, format_size(size))
        };

        entries.push(display);

        if recursive && is_dir && current_depth < max_depth {
            // Skip common non-essential directories
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !matches!(
                name_str.as_ref(),
                "node_modules" | "target" | ".git" | "__pycache__" | "venv" | ".venv"
            ) {
                Box::pin(list_dir_inner(
                    base,
                    &entry_path,
                    recursive,
                    max_depth,
                    current_depth + 1,
                    entries,
                ))
                .await?;
            }
        }
    }

    Ok(())
}

/// Format file size in human-readable form.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Apply patch tool for targeted file edits.
#[derive(Default)]
pub struct ApplyPatchTool {
    base_dir: Option<PathBuf>,
    workspace_resolver: Option<Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>>,
}

impl std::fmt::Debug for ApplyPatchTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplyPatchTool")
            .field("base_dir", &self.base_dir)
            .field("workspace_resolver", &self.workspace_resolver.is_some())
            .finish()
    }
}

impl ApplyPatchTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = Some(dir);
        self
    }

    pub fn with_workspace_resolver(
        mut self,
        resolver: Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>,
    ) -> Self {
        self.workspace_resolver = Some(resolver);
        self
    }

    pub fn from_workspace(workspace: Arc<Workspace>) -> Self {
        Self::new().with_workspace_resolver(Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(workspace),
        ))
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply targeted edits to a file using search/replace. Finds the exact 'old_string' \
         and replaces it with 'new_string'. Use for surgical code changes without rewriting entire files. \
         The old_string must match exactly (including whitespace and indentation). \
         If the file lives inside a mounted workspace directory, prefer mounted workspace paths first; editing unmounted disk files may require ask-mode approval."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace it with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace all occurrences (default false, replaces first only)"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let path_str = require_str(&params, "path")?;

        let old_string = require_str(&params, "old_string")?;

        let new_string = require_str(&params, "new_string")?;

        let replace_all = params
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let start = std::time::Instant::now();

        let path = validate_path(path_str, self.base_dir.as_deref())?;

        // Read current content
        let content = fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to read file: {}", e)))?;

        // Check if old_string exists
        if !content.contains(old_string) {
            return Err(ToolError::ExecutionFailed(format!(
                "Could not find the specified text in {}. Make sure old_string matches exactly.",
                path.display()
            )));
        }

        // Apply replacement
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        // Count replacements
        let replacements = if replace_all {
            content.matches(old_string).count()
        } else {
            1
        };

        // Write back
        fs::write(&path, &new_content)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to write file: {}", e)))?;

        let mounted = refresh_workspace_path_metadata(
            self.workspace_resolver.as_ref(),
            &ctx.user_id,
            &path,
        )
        .await?;

        let result = serde_json::json!({
            "path": path.display().to_string(),
            "replacements": replacements,
            "success": true,
            "workspace_uri": mounted.as_ref().map(|resolved| resolved.workspace_uri.clone()),
            "workspace_mount_id": mounted.as_ref().map(|resolved| resolved.mount_id.to_string())
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false // We're writing, not reading external data
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Host
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(20, 200))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::builtin::path_utils::normalize_lexical;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[cfg(feature = "libsql")]
    async fn mounted_workspace(
    ) -> (
        Arc<Workspace>,
        tempfile::TempDir,
        tempfile::TempDir,
        uuid::Uuid,
        PathBuf,
    ) {
        let (db, db_dir) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("file-tool-user", db));
        let mount_dir = tempfile::tempdir().expect("mount tempdir");
        let mount = workspace
            .create_mount("project", mount_dir.path().display().to_string(), true)
            .await
            .expect("create mount");
        let mount_root = mount_dir.path().to_path_buf();
        (
            workspace,
            db_dir,
            mount_dir,
            mount.mount.id,
            mount_root,
        )
    }

    #[tokio::test]
    async fn test_read_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line 1\nline 2\nline 3\n").unwrap();

        let tool = ReadFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": file_path.to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let content = result.result.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("line 1"));
        assert!(content.contains("line 2"));
    }

    #[tokio::test]
    async fn test_write_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("new_file.txt");

        let tool = WriteFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "content": "hello world"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "hello world");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_write_file_refreshes_mounted_workspace() {
        let (workspace, _db_dir, _mount_dir, mount_id, mount_root) = mounted_workspace().await;
        let tool = WriteFileTool::from_workspace(Arc::clone(&workspace));
        let ctx = JobContext::with_user("file-tool-user", "test", "test");
        let file_path = mount_root.join("src").join("lib.rs");

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.display().to_string(),
                    "content": "pub fn mounted() {}\n"
                }),
                &ctx,
            )
            .await
            .expect("write file");

        let workspace_uri = result
            .result
            .get("workspace_uri")
            .and_then(|value| value.as_str())
            .expect("workspace_uri");
        assert!(workspace_uri.starts_with(&format!("workspace://{mount_id}/")));

        let diff = workspace.diff_mount(mount_id, None).await.expect("mount diff");
        assert_eq!(diff.entries.len(), 1);
        assert_eq!(diff.entries[0].path, "src/lib.rs");
    }

    #[tokio::test]
    async fn test_apply_patch() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("code.rs");
        std::fs::write(&file_path, "fn main() {\n    println!(\"old\");\n}\n").unwrap();

        let tool = ApplyPatchTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": file_path.to_str().unwrap(),
                    "old_string": "println!(\"old\")",
                    "new_string": "println!(\"new\")"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        let content = std::fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("println!(\"new\")"));
    }

    #[tokio::test]
    async fn test_move_file() {
        let dir = TempDir::new().unwrap();
        let source_path = dir.path().join("source.txt");
        let destination_path = dir.path().join("nested/destination.txt");
        std::fs::write(&source_path, "hello").unwrap();

        let tool = MoveFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "source_path": source_path.to_str().unwrap(),
                    "destination_path": destination_path.to_str().unwrap(),
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.result.get("success").unwrap().as_bool().unwrap());
        assert!(!source_path.exists());
        assert_eq!(std::fs::read_to_string(destination_path).unwrap(), "hello");
    }

    #[tokio::test]
    async fn test_write_file_rejects_workspace_paths() {
        let dir = TempDir::new().unwrap();
        let tool = WriteFileTool::new().with_base_dir(dir.path().to_path_buf());
        let ctx = JobContext::default();

        let workspace_files = &["HEARTBEAT.md"];

        for filename in workspace_files {
            let path = dir.path().join(filename);
            let err = tool
                .execute(
                    serde_json::json!({
                        "path": path.to_str().unwrap(),
                        "content": "test"
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();

            let msg = err.to_string();
            assert!(
                msg.contains("graph memory tools") || msg.contains("Steward memory"),
                "Rejection for {} should mention graph memory guidance, got: {}",
                filename,
                msg
            );
        }

        // Non-legacy relative files should still work
        let relative_result = tool
            .execute(
                serde_json::json!({
                    "path": "context/vision.md",
                    "content": "fine"
                }),
                &ctx,
            )
            .await;
        assert!(relative_result.is_ok());

        // Regular files should still work
        let regular_path = dir.path().join("normal.txt");
        let result = tool
            .execute(
                serde_json::json!({
                    "path": regular_path.to_str().unwrap(),
                    "content": "fine"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file1.txt"), "content").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let tool = ListDirTool::new();
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"path": dir.path().to_str().unwrap()}),
                &ctx,
            )
            .await
            .unwrap();

        let entries = result.result.get("entries").unwrap().as_array().unwrap();
        assert!(entries.len() >= 2);
    }

    #[test]
    fn test_normalize_lexical() {
        // Basic .. resolution
        assert_eq!(
            normalize_lexical(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        // Multiple .. components
        assert_eq!(
            normalize_lexical(Path::new("/a/b/c/../../d")),
            PathBuf::from("/a/d")
        );
        // . components stripped
        assert_eq!(
            normalize_lexical(Path::new("/a/./b/./c")),
            PathBuf::from("/a/b/c")
        );
        // Cannot escape root
        assert_eq!(
            normalize_lexical(Path::new("/a/../../..")),
            PathBuf::from("/")
        );
    }

    #[test]
    fn test_validate_path_rejects_traversal_nonexistent_parent() {
        // The critical test: writing to ../../outside/newdir/file with base_dir
        // set should be rejected even when the parent directory does not exist
        // (i.e. canonicalize() cannot resolve it).
        let dir = TempDir::new().unwrap();
        let evil_path = format!(
            "{}/../../outside/newdir/file.txt",
            dir.path().to_str().unwrap()
        );
        let result = validate_path(&evil_path, Some(dir.path()));
        assert!(
            result.is_err(),
            "Should reject traversal via non-existent parent, got: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_rejects_relative_traversal() {
        let dir = TempDir::new().unwrap();
        let result = validate_path("../../etc/passwd", Some(dir.path()));
        assert!(
            result.is_err(),
            "Should reject relative traversal, got: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_allows_valid_nested_write() {
        let dir = TempDir::new().unwrap();
        let result = validate_path("subdir/newfile.txt", Some(dir.path()));
        assert!(
            result.is_ok(),
            "Should allow nested writes within sandbox: {:?}",
            result
        );
    }

    #[test]
    fn test_validate_path_allows_dot_dot_within_sandbox() {
        // a/b/../c resolves to a/c which is still inside the sandbox
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        let result = validate_path("a/b/../c.txt", Some(dir.path()));
        assert!(
            result.is_ok(),
            "Should allow .. that stays within sandbox: {:?}",
            result
        );
    }
}
