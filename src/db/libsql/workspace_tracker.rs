use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use chrono::Utc;
use libsql::params;
use serde_json::json;
use tokio::process::Command;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_opt_text, get_opt_ts, get_text};
use crate::bootstrap::steward_base_dir;
use crate::error::WorkspaceError;
use crate::tools::builtin::path_utils::canonicalize_stripped;
use crate::workspace::{
    AllowlistedFileStatus, WorkspaceAllowlistChangeKind, WorkspaceAllowlistDiff,
    WorkspaceAllowlistDiffRequest, WorkspaceAllowlistRevisionKind,
    WorkspaceAllowlistRevisionSource, normalize_allowlist_path,
};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Create a new `git` command with platform-specific flags.
/// On Windows this sets `CREATE_NO_WINDOW` to prevent flashing console windows.
fn new_git_command() -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("git");
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new("git")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllowlistTrackerKind {
    ExternalGit,
    InternalGit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllowlistTrackerStatus {
    Ready,
    SyncPending,
    NeedsRepair,
}

#[derive(Debug, Clone)]
pub(crate) struct AllowlistTrackerRecord {
    pub allowlist_id: Uuid,
    pub tracker_kind: AllowlistTrackerKind,
    pub status: AllowlistTrackerStatus,
    pub repo_root: String,
    pub git_dir: String,
    pub work_tree: String,
    pub allowlist_scope: String,
    pub baseline_anchor: Option<String>,
    pub head_anchor: Option<String>,
    pub baseline_revision_id: Option<Uuid>,
    pub head_revision_id: Option<Uuid>,
    pub last_verified_at: Option<chrono::DateTime<Utc>>,
    pub metadata_json: serde_json::Value,
}

#[derive(Debug, Clone)]
pub(crate) struct TrackerChange {
    pub path: String,
    pub old_path: Option<String>,
    pub change_kind: WorkspaceAllowlistChangeKind,
    pub status: AllowlistedFileStatus,
    pub is_binary: bool,
    pub before_content: Option<Vec<u8>>,
    pub after_content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedNameStatusEntry {
    status: String,
    old_repo_path: Option<String>,
    repo_path: String,
}

#[derive(Debug, Clone)]
struct ExternalGitDiscovery {
    repo_root: String,
    git_dir: String,
    allowlist_scope: String,
}

fn tracker_kind_to_str(value: AllowlistTrackerKind) -> &'static str {
    match value {
        AllowlistTrackerKind::ExternalGit => "external_git",
        AllowlistTrackerKind::InternalGit => "internal_git",
    }
}

fn tracker_kind_from_str(value: &str) -> AllowlistTrackerKind {
    match value {
        "external_git" => AllowlistTrackerKind::ExternalGit,
        _ => AllowlistTrackerKind::InternalGit,
    }
}

fn tracker_status_to_str(value: AllowlistTrackerStatus) -> &'static str {
    match value {
        AllowlistTrackerStatus::Ready => "ready",
        AllowlistTrackerStatus::SyncPending => "sync_pending",
        AllowlistTrackerStatus::NeedsRepair => "needs_repair",
    }
}

fn tracker_status_from_str(value: &str) -> AllowlistTrackerStatus {
    match value {
        "sync_pending" => AllowlistTrackerStatus::SyncPending,
        "needs_repair" => AllowlistTrackerStatus::NeedsRepair,
        _ => AllowlistTrackerStatus::Ready,
    }
}

fn revision_kind_to_str(value: WorkspaceAllowlistRevisionKind) -> &'static str {
    match value {
        WorkspaceAllowlistRevisionKind::Initial => "initial",
        WorkspaceAllowlistRevisionKind::ToolWrite => "tool_write",
        WorkspaceAllowlistRevisionKind::ToolPatch => "tool_patch",
        WorkspaceAllowlistRevisionKind::ToolMove => "tool_move",
        WorkspaceAllowlistRevisionKind::ToolDelete => "tool_delete",
        WorkspaceAllowlistRevisionKind::Shell => "shell",
        WorkspaceAllowlistRevisionKind::FsWatch => "fs_watch",
        WorkspaceAllowlistRevisionKind::ManualRefresh => "manual_refresh",
        WorkspaceAllowlistRevisionKind::Restore => "restore",
        WorkspaceAllowlistRevisionKind::Accept => "accept",
    }
}

fn revision_source_to_str(value: WorkspaceAllowlistRevisionSource) -> &'static str {
    match value {
        WorkspaceAllowlistRevisionSource::WorkspaceTool => "workspace_tool",
        WorkspaceAllowlistRevisionSource::Shell => "shell",
        WorkspaceAllowlistRevisionSource::External => "external",
        WorkspaceAllowlistRevisionSource::System => "system",
    }
}

fn change_kind_to_str(value: WorkspaceAllowlistChangeKind) -> &'static str {
    match value {
        WorkspaceAllowlistChangeKind::Added => "added",
        WorkspaceAllowlistChangeKind::Modified => "modified",
        WorkspaceAllowlistChangeKind::Deleted => "deleted",
        WorkspaceAllowlistChangeKind::Moved => "moved",
    }
}

fn status_to_str(status: AllowlistedFileStatus) -> &'static str {
    match status {
        AllowlistedFileStatus::Clean => "clean",
        AllowlistedFileStatus::Modified => "modified",
        AllowlistedFileStatus::Added => "added",
        AllowlistedFileStatus::Deleted => "deleted",
        AllowlistedFileStatus::Conflicted => "conflicted",
        AllowlistedFileStatus::BinaryModified => "binary_modified",
    }
}

fn summarize_change_count(changes: usize) -> Option<String> {
    if changes == 0 {
        None
    } else if changes == 1 {
        Some("1 file changed".to_string())
    } else {
        Some(format!("{changes} files changed"))
    }
}

fn parse_porcelain_status_z_paths(output: &[u8]) -> Vec<String> {
    let mut paths = BTreeSet::new();
    let mut entries = output
        .split(|byte| *byte == b'\0')
        .filter(|entry| !entry.is_empty());

    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }

        let status = &entry[..2];
        let path = String::from_utf8_lossy(&entry[3..]).into_owned();
        if !path.is_empty() {
            paths.insert(path);
        }

        let is_rename_or_copy = status.contains(&b'R') || status.contains(&b'C');
        if is_rename_or_copy {
            if let Some(original_path) = entries.next() {
                if !original_path.is_empty() {
                    paths.insert(String::from_utf8_lossy(original_path).into_owned());
                }
            }
        }
    }

    paths.into_iter().collect()
}

fn parse_name_status_z_entries(output: &[u8]) -> Vec<ParsedNameStatusEntry> {
    let mut parsed = Vec::new();
    let mut entries = output
        .split(|byte| *byte == b'\0')
        .filter(|entry| !entry.is_empty());

    while let Some(status_entry) = entries.next() {
        let status = String::from_utf8_lossy(status_entry).into_owned();
        if status.is_empty() {
            continue;
        }

        let is_rename_or_copy = status.starts_with('R') || status.starts_with('C');
        let old_repo_path = if is_rename_or_copy {
            entries
                .next()
                .map(|entry| String::from_utf8_lossy(entry).into_owned())
        } else {
            None
        };
        let Some(repo_path_entry) = entries.next() else {
            break;
        };
        let repo_path = String::from_utf8_lossy(repo_path_entry).into_owned();
        if repo_path.is_empty() {
            continue;
        }

        parsed.push(ParsedNameStatusEntry {
            status,
            old_repo_path,
            repo_path,
        });
    }

    parsed
}

fn quote_revspec_path(path: &str) -> String {
    if path.contains(':') {
        path.replace(':', "\\:")
    } else {
        path.to_string()
    }
}

impl AllowlistTrackerRecord {
    fn repo_root_path(&self) -> PathBuf {
        PathBuf::from(&self.repo_root)
    }

    fn git_dir_path(&self) -> PathBuf {
        PathBuf::from(&self.git_dir)
    }

    fn work_tree_path(&self) -> PathBuf {
        PathBuf::from(&self.work_tree)
    }

    pub(crate) fn repo_path_for_allowlist_path(&self, allowlist_path: &str) -> String {
        if self.allowlist_scope.is_empty() {
            allowlist_path.to_string()
        } else if allowlist_path.is_empty() {
            self.allowlist_scope.clone()
        } else {
            format!("{}/{}", self.allowlist_scope, allowlist_path)
        }
    }

    pub(crate) fn allowlist_path_from_repo_path(&self, repo_path: &str) -> Option<String> {
        let normalized = repo_path.replace('\\', "/");
        if self.allowlist_scope.is_empty() {
            normalize_allowlist_path(&normalized).ok()
        } else if normalized == self.allowlist_scope {
            Some(String::new())
        } else {
            normalized
                .strip_prefix(&(self.allowlist_scope.clone() + "/"))
                .and_then(|value| normalize_allowlist_path(value).ok())
        }
    }

    fn root_pathspec(&self) -> String {
        if self.allowlist_scope.is_empty() {
            ".".to_string()
        } else {
            self.allowlist_scope.clone()
        }
    }

    fn main_ref(&self) -> String {
        match self.tracker_kind {
            AllowlistTrackerKind::InternalGit => "refs/heads/steward/main".to_string(),
            AllowlistTrackerKind::ExternalGit => {
                format!("refs/steward/allowlists/{}/main", self.allowlist_id)
            }
        }
    }

    fn head_ref(&self) -> String {
        match self.tracker_kind {
            AllowlistTrackerKind::InternalGit => "refs/steward/head".to_string(),
            AllowlistTrackerKind::ExternalGit => {
                format!("refs/steward/allowlists/{}/head", self.allowlist_id)
            }
        }
    }

    fn baseline_ref(&self) -> String {
        match self.tracker_kind {
            AllowlistTrackerKind::InternalGit => "refs/steward/baseline".to_string(),
            AllowlistTrackerKind::ExternalGit => {
                format!("refs/steward/allowlists/{}/baseline", self.allowlist_id)
            }
        }
    }
}

impl LibSqlBackend {
    pub(crate) async fn load_allowlist_tracker(
        &self,
        allowlist_id: Uuid,
    ) -> Result<Option<AllowlistTrackerRecord>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT allowlist_id, tracker_kind, status, repo_root, git_dir, work_tree,
                        allowlist_scope, baseline_anchor, head_anchor, baseline_revision_id,
                        head_revision_id, last_verified_at, metadata_json
                 FROM workspace_allowlist_trackers
                 WHERE allowlist_id = ?1",
                params![allowlist_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("tracker query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("tracker query failed: {e}"),
            })?
        else {
            return Ok(None);
        };

        let metadata_json = get_opt_text(&row, 12)
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_else(|| json!({}));

        Ok(Some(AllowlistTrackerRecord {
            allowlist_id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| {
                WorkspaceError::SearchFailed {
                    reason: format!("invalid tracker allowlist id: {e}"),
                }
            })?,
            tracker_kind: tracker_kind_from_str(&get_text(&row, 1)),
            status: tracker_status_from_str(&get_text(&row, 2)),
            repo_root: get_text(&row, 3),
            git_dir: get_text(&row, 4),
            work_tree: get_text(&row, 5),
            allowlist_scope: get_text(&row, 6),
            baseline_anchor: get_opt_text(&row, 7),
            head_anchor: get_opt_text(&row, 8),
            baseline_revision_id: get_opt_text(&row, 9)
                .and_then(|value| Uuid::parse_str(&value).ok()),
            head_revision_id: get_opt_text(&row, 10).and_then(|value| Uuid::parse_str(&value).ok()),
            last_verified_at: get_opt_ts(&row, 11),
            metadata_json,
        }))
    }

    pub(crate) async fn save_allowlist_tracker(
        &self,
        tracker: &AllowlistTrackerRecord,
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
            INSERT INTO workspace_allowlist_trackers (
                allowlist_id, tracker_kind, status, repo_root, git_dir, work_tree,
                allowlist_scope, baseline_anchor, head_anchor, baseline_revision_id,
                head_revision_id, last_verified_at, metadata_json, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)
            ON CONFLICT(allowlist_id) DO UPDATE SET
                tracker_kind = excluded.tracker_kind,
                status = excluded.status,
                repo_root = excluded.repo_root,
                git_dir = excluded.git_dir,
                work_tree = excluded.work_tree,
                allowlist_scope = excluded.allowlist_scope,
                baseline_anchor = excluded.baseline_anchor,
                head_anchor = excluded.head_anchor,
                baseline_revision_id = excluded.baseline_revision_id,
                head_revision_id = excluded.head_revision_id,
                last_verified_at = excluded.last_verified_at,
                metadata_json = excluded.metadata_json,
                updated_at = excluded.updated_at
            "#,
            params![
                tracker.allowlist_id.to_string(),
                tracker_kind_to_str(tracker.tracker_kind),
                tracker_status_to_str(tracker.status),
                tracker.repo_root.clone(),
                tracker.git_dir.clone(),
                tracker.work_tree.clone(),
                tracker.allowlist_scope.clone(),
                tracker.baseline_anchor.clone(),
                tracker.head_anchor.clone(),
                tracker.baseline_revision_id.map(|value| value.to_string()),
                tracker.head_revision_id.map(|value| value.to_string()),
                tracker.last_verified_at.as_ref().map(fmt_ts),
                tracker.metadata_json.to_string(),
                now
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("tracker upsert failed: {e}"),
        })?;
        Ok(())
    }

    pub(crate) async fn load_tracker_anchor_for_revision(
        &self,
        allowlist_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<String>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT tracker_anchor
                 FROM workspace_allowlist_revision_anchors
                 WHERE allowlist_id = ?1 AND product_revision_id = ?2",
                params![allowlist_id.to_string(), revision_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("revision anchor query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("revision anchor query failed: {e}"),
            })?
        else {
            return Ok(None);
        };
        Ok(get_opt_text(&row, 0))
    }

    async fn save_tracker_anchor_for_revision(
        &self,
        allowlist_id: Uuid,
        revision_id: Uuid,
        anchor: &str,
        anchor_kind: &str,
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "INSERT OR REPLACE INTO workspace_allowlist_revision_anchors (
                product_revision_id, allowlist_id, tracker_anchor, anchor_kind, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                revision_id.to_string(),
                allowlist_id.to_string(),
                anchor,
                anchor_kind,
                fmt_ts(&Utc::now())
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("revision anchor insert failed: {e}"),
        })?;
        Ok(())
    }

    async fn tracker_git_command(
        &self,
        tracker: &AllowlistTrackerRecord,
        index_path: Option<&Path>,
    ) -> Command {
        let mut cmd = new_git_command();
        cmd.current_dir(tracker.repo_root_path())
            .arg("-c")
            .arg("core.bare=false")
            .arg("--git-dir")
            .arg(tracker.git_dir_path())
            .arg("--work-tree")
            .arg(tracker.work_tree_path())
            .stdin(Stdio::null());
        if let Some(index_path) = index_path {
            cmd.env("GIT_INDEX_FILE", index_path);
        }
        cmd
    }

    async fn run_tracker_git_text(
        &self,
        tracker: &AllowlistTrackerRecord,
        index_path: Option<&Path>,
        args: &[String],
    ) -> Result<String, WorkspaceError> {
        let output = self
            .run_tracker_git_bytes(tracker, index_path, args)
            .await?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
    }

    async fn run_tracker_git_bytes(
        &self,
        tracker: &AllowlistTrackerRecord,
        index_path: Option<&Path>,
        args: &[String],
    ) -> Result<Vec<u8>, WorkspaceError> {
        let mut cmd = self.tracker_git_command(tracker, index_path).await;
        cmd.args(args);
        let output = cmd.output().await.map_err(|e| WorkspaceError::IoError {
            reason: format!("failed to execute git: {e}"),
        })?;
        if !output.status.success() {
            return Err(WorkspaceError::SearchFailed {
                reason: format!(
                    "git {} failed: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }
        Ok(output.stdout)
    }

    async fn detect_external_git_tracker(
        source_root: &Path,
    ) -> Result<Option<ExternalGitDiscovery>, WorkspaceError> {
        let top_output = new_git_command()
            .arg("-C")
            .arg(source_root)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .await
            .map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to detect git repository: {e}"),
            })?;
        if !top_output.status.success() {
            return Ok(None);
        }

        let repo_root = String::from_utf8_lossy(&top_output.stdout)
            .trim()
            .to_string();
        let git_dir_output = new_git_command()
            .arg("-C")
            .arg(source_root)
            .args(["rev-parse", "--absolute-git-dir"])
            .output()
            .await
            .map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to resolve git dir: {e}"),
            })?;
        if !git_dir_output.status.success() {
            return Ok(None);
        }
        let git_dir = String::from_utf8_lossy(&git_dir_output.stdout)
            .trim()
            .to_string();

        let repo_root_path =
            canonicalize_stripped(Path::new(&repo_root)).map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to canonicalize repo root: {e}"),
            })?;
        let source_root_path =
            canonicalize_stripped(Path::new(source_root)).map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to canonicalize allowlist root: {e}"),
            })?;
        let allowlist_scope = source_root_path
            .strip_prefix(&repo_root_path)
            .ok()
            .map(|value| value.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        Ok(Some(ExternalGitDiscovery {
            repo_root: repo_root_path.display().to_string(),
            git_dir,
            allowlist_scope,
        }))
    }

    async fn init_internal_git_tracker(
        allowlist_id: Uuid,
        work_tree: &Path,
    ) -> Result<PathBuf, WorkspaceError> {
        let git_dir = steward_base_dir()
            .join("workspace-trackers")
            .join(allowlist_id.to_string())
            .join("git");
        if git_dir.exists() {
            return Ok(git_dir);
        }
        if let Some(parent) = git_dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to create tracker directory: {e}"),
            })?;
        }
        let output = new_git_command()
            .args(["init", "--bare"])
            .arg(&git_dir)
            .output()
            .await
            .map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to initialize tracker repository: {e}"),
            })?;
        if !output.status.success() {
            return Err(WorkspaceError::SearchFailed {
                reason: format!(
                    "failed to initialize tracker repository: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }

        let config_output = new_git_command()
            .arg("--git-dir")
            .arg(&git_dir)
            .args(["config", "advice.defaultBranchName", "false"])
            .output()
            .await
            .map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to configure tracker repository: {e}"),
            })?;
        if !config_output.status.success() {
            return Err(WorkspaceError::SearchFailed {
                reason: format!(
                    "failed to configure tracker repository: {}",
                    String::from_utf8_lossy(&config_output.stderr).trim()
                ),
            });
        }

        if !work_tree.is_dir() {
            return Err(WorkspaceError::IoError {
                reason: "allowlist source must be a directory".to_string(),
            });
        }

        Ok(git_dir)
    }

    pub(crate) async fn update_tracker_refs(
        &self,
        tracker: &AllowlistTrackerRecord,
    ) -> Result<(), WorkspaceError> {
        if let Some(head_anchor) = tracker.head_anchor.as_deref() {
            self.run_tracker_git_text(
                tracker,
                None,
                &[
                    "update-ref".to_string(),
                    tracker.main_ref(),
                    head_anchor.to_string(),
                ],
            )
            .await?;
            self.run_tracker_git_text(
                tracker,
                None,
                &[
                    "update-ref".to_string(),
                    tracker.head_ref(),
                    head_anchor.to_string(),
                ],
            )
            .await?;
        }
        if let Some(baseline_anchor) = tracker.baseline_anchor.as_deref() {
            self.run_tracker_git_text(
                tracker,
                None,
                &[
                    "update-ref".to_string(),
                    tracker.baseline_ref(),
                    baseline_anchor.to_string(),
                ],
            )
            .await?;
        }
        Ok(())
    }

    async fn temp_index_path(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{prefix}-{}.index", Uuid::new_v4()));
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        path
    }

    async fn seed_tracker_index(
        &self,
        tracker: &AllowlistTrackerRecord,
        index_path: &Path,
        anchor: Option<&str>,
    ) -> Result<(), WorkspaceError> {
        if let Some(anchor) = anchor {
            self.run_tracker_git_text(
                tracker,
                Some(index_path),
                &["read-tree".to_string(), anchor.to_string()],
            )
            .await?;
        }
        Ok(())
    }

    async fn anchor_tree_oid(
        &self,
        tracker: &AllowlistTrackerRecord,
        anchor: &str,
    ) -> Result<String, WorkspaceError> {
        self.run_tracker_git_text(
            tracker,
            None,
            &["rev-parse".to_string(), format!("{anchor}^{{tree}}")],
        )
        .await
    }

    async fn capture_worktree_anchor(
        &self,
        tracker: &AllowlistTrackerRecord,
        parent_anchor: Option<&str>,
        repo_paths: &[String],
        message: &str,
    ) -> Result<String, WorkspaceError> {
        let index_path = Self::temp_index_path("steward-workspace-tracker").await;
        self.seed_tracker_index(tracker, &index_path, parent_anchor)
            .await?;

        let pathspecs = if repo_paths.is_empty() {
            vec![tracker.root_pathspec()]
        } else {
            repo_paths.to_vec()
        };

        let mut add_args = vec!["add".to_string(), "-A".to_string(), "--".to_string()];
        add_args.extend(pathspecs);
        if let Err(e) = self
            .run_tracker_git_text(tracker, Some(&index_path), &add_args)
            .await
        {
            // Specific pathspecs can fail when files were deleted but never
            // tracked (e.g. macOS .DS_Store, or files created after the last
            // anchor).  Fall back to staging the entire allowlist scope.
            tracing::debug!("git add with specific paths failed, falling back to root scope: {e}");
            let fallback_args = vec![
                "add".to_string(),
                "-A".to_string(),
                "--".to_string(),
                tracker.root_pathspec(),
            ];
            self.run_tracker_git_text(tracker, Some(&index_path), &fallback_args)
                .await?;
        }

        let tree = self
            .run_tracker_git_text(tracker, Some(&index_path), &["write-tree".to_string()])
            .await?;

        if let Some(parent_anchor) = parent_anchor {
            let parent_tree = self.anchor_tree_oid(tracker, parent_anchor).await?;
            if parent_tree == tree {
                let _ = std::fs::remove_file(&index_path);
                return Ok(parent_anchor.to_string());
            }
        }

        let mut commit_args = vec![
            "commit-tree".to_string(),
            tree,
            "-m".to_string(),
            message.to_string(),
        ];
        if let Some(parent_anchor) = parent_anchor {
            commit_args.push("-p".to_string());
            commit_args.push(parent_anchor.to_string());
        }

        let mut cmd = self.tracker_git_command(tracker, Some(&index_path)).await;
        cmd.env("GIT_AUTHOR_NAME", "Steward")
            .env("GIT_AUTHOR_EMAIL", "steward@localhost")
            .env("GIT_COMMITTER_NAME", "Steward")
            .env("GIT_COMMITTER_EMAIL", "steward@localhost")
            .args(&commit_args);
        let output = cmd.output().await.map_err(|e| WorkspaceError::IoError {
            reason: format!("failed to write tracker commit: {e}"),
        })?;
        let _ = std::fs::remove_file(&index_path);
        if !output.status.success() {
            return Err(WorkspaceError::SearchFailed {
                reason: format!(
                    "failed to write tracker commit: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    async fn collect_dirty_repo_paths(
        &self,
        tracker: &AllowlistTrackerRecord,
        scope_path: Option<&str>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let index_path = Self::temp_index_path("steward-workspace-status").await;
        self.seed_tracker_index(tracker, &index_path, tracker.head_anchor.as_deref())
            .await?;

        let mut args = vec![
            "status".to_string(),
            "--porcelain=v1".to_string(),
            "-z".to_string(),
            "--untracked-files=all".to_string(),
            "--ignored=no".to_string(),
            "--".to_string(),
        ];
        args.push(
            scope_path
                .map(|value| value.to_string())
                .unwrap_or_else(|| tracker.root_pathspec()),
        );
        let output = self
            .run_tracker_git_bytes(tracker, Some(&index_path), &args)
            .await?;
        let _ = std::fs::remove_file(&index_path);

        Ok(parse_porcelain_status_z_paths(&output))
    }

    async fn read_anchor_file_bytes(
        &self,
        tracker: &AllowlistTrackerRecord,
        anchor: &str,
        repo_path: &str,
    ) -> Result<Option<Vec<u8>>, WorkspaceError> {
        let spec = format!("{anchor}:{}", quote_revspec_path(repo_path));
        let mut cmd = self.tracker_git_command(tracker, None).await;
        cmd.arg("show").arg(spec);
        let output = cmd.output().await.map_err(|e| WorkspaceError::IoError {
            reason: format!("failed to read tracker blob: {e}"),
        })?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(output.stdout))
    }

    pub(crate) async fn diff_allowlist_anchors(
        &self,
        tracker: &AllowlistTrackerRecord,
        from_anchor: Option<&str>,
        to_anchor: &str,
        include_content: bool,
        scope_path: Option<&str>,
        max_files: Option<usize>,
    ) -> Result<Vec<TrackerChange>, WorkspaceError> {
        let diff_scope = scope_path
            .map(normalize_allowlist_path)
            .transpose()?
            .map(|value| tracker.repo_path_for_allowlist_path(&value));

        let mut args = vec![
            "diff".to_string(),
            "--name-status".to_string(),
            "-z".to_string(),
            "-M".to_string(),
            "--no-ext-diff".to_string(),
            from_anchor.unwrap_or(EMPTY_TREE).to_string(),
            to_anchor.to_string(),
            "--".to_string(),
            diff_scope.unwrap_or_else(|| tracker.root_pathspec()),
        ];
        let diff_output = self.run_tracker_git_bytes(tracker, None, &args).await?;
        args.clear();

        let mut changes = Vec::new();
        for entry in parse_name_status_z_entries(&diff_output) {
            let status = entry.status.as_str();
            if status.is_empty() {
                continue;
            }

            let (old_repo_path, repo_path, change_kind) = if status.starts_with('R') {
                (
                    entry.old_repo_path.clone(),
                    entry.repo_path.clone(),
                    WorkspaceAllowlistChangeKind::Moved,
                )
            } else {
                (
                    None,
                    entry.repo_path.clone(),
                    match status.chars().next().unwrap_or('M') {
                        'A' => WorkspaceAllowlistChangeKind::Added,
                        'D' => WorkspaceAllowlistChangeKind::Deleted,
                        _ => WorkspaceAllowlistChangeKind::Modified,
                    },
                )
            };

            let Some(path) = tracker.allowlist_path_from_repo_path(&repo_path) else {
                continue;
            };
            let old_path = old_repo_path
                .as_deref()
                .and_then(|value| tracker.allowlist_path_from_repo_path(value));

            if let Some(scope_path) = scope_path {
                let scope_path = normalize_allowlist_path(scope_path)?;
                let matches_scope = path == scope_path
                    || path.starts_with(&(scope_path.clone() + "/"))
                    || old_path.as_ref().is_some_and(|value| {
                        value == &scope_path || value.starts_with(&(scope_path.clone() + "/"))
                    });
                if !matches_scope {
                    continue;
                }
            }

            let before_content = if include_content {
                match change_kind {
                    WorkspaceAllowlistChangeKind::Added => None,
                    WorkspaceAllowlistChangeKind::Moved => {
                        if let Some(from_anchor) = from_anchor {
                            match old_repo_path.as_deref() {
                                Some(old_repo_path) => {
                                    self.read_anchor_file_bytes(tracker, from_anchor, old_repo_path)
                                        .await?
                                }
                                None => None,
                            }
                        } else {
                            None
                        }
                    }
                    _ => match from_anchor {
                        Some(from_anchor) => {
                            self.read_anchor_file_bytes(tracker, from_anchor, &repo_path)
                                .await?
                        }
                        None => None,
                    },
                }
            } else {
                None
            };

            let after_content =
                if include_content && change_kind != WorkspaceAllowlistChangeKind::Deleted {
                    self.read_anchor_file_bytes(tracker, to_anchor, &repo_path)
                        .await?
                } else {
                    None
                };

            let is_binary = before_content
                .as_ref()
                .or(after_content.as_ref())
                .is_some_and(|bytes| std::str::from_utf8(bytes).is_err());

            let status = match change_kind {
                WorkspaceAllowlistChangeKind::Added => AllowlistedFileStatus::Added,
                WorkspaceAllowlistChangeKind::Deleted => AllowlistedFileStatus::Deleted,
                WorkspaceAllowlistChangeKind::Moved | WorkspaceAllowlistChangeKind::Modified => {
                    if is_binary {
                        AllowlistedFileStatus::BinaryModified
                    } else {
                        AllowlistedFileStatus::Modified
                    }
                }
            };

            changes.push(TrackerChange {
                path,
                old_path,
                change_kind,
                status,
                is_binary,
                before_content,
                after_content,
            });
        }

        if let Some(max_files) = max_files {
            changes.truncate(max_files);
        }

        Ok(changes)
    }

    async fn insert_tracker_revision(
        &self,
        allowlist_id: Uuid,
        parent_revision_id: Option<Uuid>,
        anchor: &str,
        changes: &[TrackerChange],
        kind: WorkspaceAllowlistRevisionKind,
        source: WorkspaceAllowlistRevisionSource,
        trigger: Option<String>,
        summary: Option<String>,
        created_by: &str,
    ) -> Result<Uuid, WorkspaceError> {
        let revision_id = Uuid::new_v4();
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "INSERT INTO workspace_allowlist_revisions (
                id, allowlist_id, parent_revision_id, kind, source, trigger, summary, created_by, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                revision_id.to_string(),
                allowlist_id.to_string(),
                parent_revision_id.map(|value| value.to_string()),
                revision_kind_to_str(kind),
                revision_source_to_str(source),
                trigger,
                summary.or_else(|| summarize_change_count(changes.len())),
                created_by.to_string(),
                fmt_ts(&Utc::now())
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("revision insert failed: {e}"),
        })?;

        self.save_tracker_anchor_for_revision(
            allowlist_id,
            revision_id,
            anchor,
            "product_revision",
        )
        .await?;

        for change in changes {
            let is_binary = if change.is_binary { 1 } else { 0 };
            match change.change_kind {
                WorkspaceAllowlistChangeKind::Moved => {
                    if let Some(old_path) = change.old_path.as_deref() {
                        conn.execute(
                            "INSERT OR REPLACE INTO workspace_allowlist_revision_files (
                                revision_id, relative_path, change_kind, is_binary, rename_from, rename_to
                             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![
                                revision_id.to_string(),
                                old_path.to_string(),
                                change_kind_to_str(change.change_kind),
                                is_binary,
                                old_path.to_string(),
                                change.path.clone()
                            ],
                        )
                        .await
                        .map_err(|e| WorkspaceError::SearchFailed {
                            reason: format!("revision file insert failed: {e}"),
                        })?;
                    }
                    conn.execute(
                        "INSERT OR REPLACE INTO workspace_allowlist_revision_files (
                            revision_id, relative_path, change_kind, is_binary, rename_from, rename_to
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            revision_id.to_string(),
                            change.path.clone(),
                            change_kind_to_str(change.change_kind),
                            is_binary,
                            change.old_path.clone(),
                            change.path.clone()
                        ],
                    )
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("revision file insert failed: {e}"),
                    })?;
                }
                _ => {
                    conn.execute(
                        "INSERT OR REPLACE INTO workspace_allowlist_revision_files (
                            revision_id, relative_path, change_kind, is_binary, rename_from, rename_to
                         ) VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
                        params![
                            revision_id.to_string(),
                            change.path.clone(),
                            change_kind_to_str(change.change_kind),
                            is_binary
                        ],
                    )
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("revision file insert failed: {e}"),
                    })?;
                }
            }
        }

        Ok(revision_id)
    }

    pub(crate) async fn ensure_allowlist_tracker(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<AllowlistTrackerRecord, WorkspaceError> {
        if let Some(existing) = self.load_allowlist_tracker(allowlist_id).await? {
            return Ok(existing);
        }

        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        let source_root =
            canonicalize_stripped(Path::new(&allowlist.source_root)).map_err(|e| {
                WorkspaceError::IoError {
                    reason: format!("allowlist source is not accessible: {e}"),
                }
            })?;

        let mut tracker =
            if let Some(external) = Self::detect_external_git_tracker(&source_root).await? {
                AllowlistTrackerRecord {
                    allowlist_id,
                    tracker_kind: AllowlistTrackerKind::ExternalGit,
                    status: AllowlistTrackerStatus::Ready,
                    repo_root: external.repo_root.clone(),
                    git_dir: external.git_dir,
                    work_tree: external.repo_root,
                    allowlist_scope: external.allowlist_scope,
                    baseline_anchor: None,
                    head_anchor: None,
                    baseline_revision_id: None,
                    head_revision_id: None,
                    last_verified_at: None,
                    metadata_json: json!({}),
                }
            } else {
                let git_dir = Self::init_internal_git_tracker(allowlist_id, &source_root).await?;
                AllowlistTrackerRecord {
                    allowlist_id,
                    tracker_kind: AllowlistTrackerKind::InternalGit,
                    status: AllowlistTrackerStatus::Ready,
                    repo_root: source_root.display().to_string(),
                    git_dir: git_dir.display().to_string(),
                    work_tree: source_root.display().to_string(),
                    allowlist_scope: String::new(),
                    baseline_anchor: None,
                    head_anchor: None,
                    baseline_revision_id: None,
                    head_revision_id: None,
                    last_verified_at: None,
                    metadata_json: json!({}),
                }
            };

        if tracker.tracker_kind == AllowlistTrackerKind::ExternalGit {
            tracker.work_tree = tracker.repo_root.clone();
        }

        let initial_anchor = self
            .capture_worktree_anchor(
                &tracker,
                None,
                &[tracker.root_pathspec()],
                "steward initial allowlist baseline",
            )
            .await?;
        let changes = self
            .diff_allowlist_anchors(&tracker, None, &initial_anchor, false, None, None)
            .await?;
        let initial_revision_id = self
            .insert_tracker_revision(
                allowlist_id,
                None,
                &initial_anchor,
                &changes,
                WorkspaceAllowlistRevisionKind::Initial,
                WorkspaceAllowlistRevisionSource::System,
                Some("tracker_init".to_string()),
                Some("initialized git-backed allowlist tracker".to_string()),
                "system",
            )
            .await?;

        tracker.baseline_anchor = Some(initial_anchor.clone());
        tracker.head_anchor = Some(initial_anchor);
        tracker.baseline_revision_id = Some(initial_revision_id);
        tracker.head_revision_id = Some(initial_revision_id);
        tracker.last_verified_at = Some(Utc::now());

        self.save_allowlist_tracker(&tracker).await?;
        self.update_tracker_refs(&tracker).await?;
        Ok(tracker)
    }

    pub(crate) async fn sync_allowlist_from_tracker(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        scope_path: Option<&str>,
        repo_path_hints: Option<Vec<String>>,
        kind: WorkspaceAllowlistRevisionKind,
        source: WorkspaceAllowlistRevisionSource,
        trigger: Option<String>,
        summary: Option<String>,
        created_by: &str,
    ) -> Result<crate::db::libsql::workspace::AllowlistStateRecord, WorkspaceError> {
        let mut tracker = self.ensure_allowlist_tracker(user_id, allowlist_id).await?;
        if tracker.status == AllowlistTrackerStatus::NeedsRepair {
            return Err(WorkspaceError::AllowlistConflict {
                path: allowlist_id.to_string(),
                reason: "allowlist tracker needs repair".to_string(),
            });
        }

        let scope_repo_path = scope_path
            .map(normalize_allowlist_path)
            .transpose()?
            .map(|value| tracker.repo_path_for_allowlist_path(&value));
        let repo_paths = match repo_path_hints {
            Some(paths) => paths,
            None => {
                self.collect_dirty_repo_paths(&tracker, scope_repo_path.as_deref())
                    .await?
            }
        };

        if !repo_paths.is_empty() {
            let message = summary
                .clone()
                .unwrap_or_else(|| "synchronized allowlist tracker".to_string());
            let parent_anchor = tracker.head_anchor.clone();
            let parent_revision_id = tracker.head_revision_id;
            let new_anchor = self
                .capture_worktree_anchor(&tracker, parent_anchor.as_deref(), &repo_paths, &message)
                .await?;
            if parent_anchor.as_deref() != Some(new_anchor.as_str()) {
                let changes = self
                    .diff_allowlist_anchors(
                        &tracker,
                        parent_anchor.as_deref(),
                        &new_anchor,
                        false,
                        None,
                        None,
                    )
                    .await?;
                let revision_id = self
                    .insert_tracker_revision(
                        allowlist_id,
                        parent_revision_id,
                        &new_anchor,
                        &changes,
                        kind,
                        source,
                        trigger,
                        summary,
                        created_by,
                    )
                    .await?;
                tracker.head_anchor = Some(new_anchor);
                tracker.head_revision_id = Some(revision_id);
            }
        }

        tracker.status = AllowlistTrackerStatus::Ready;
        tracker.last_verified_at = Some(Utc::now());
        self.save_allowlist_tracker(&tracker).await?;
        self.update_tracker_refs(&tracker).await?;

        Ok(crate::db::libsql::workspace::AllowlistStateRecord {
            baseline_revision_id: tracker.baseline_revision_id,
            head_revision_id: tracker.head_revision_id,
        })
    }

    pub(crate) async fn rebuild_allowlist_live_cache_from_tracker(
        &self,
        allowlist_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        let Some(tracker) = self.load_allowlist_tracker(allowlist_id).await? else {
            return Ok(());
        };
        let baseline_anchor = tracker.baseline_anchor.clone();
        let head_anchor = tracker.head_anchor.clone();
        let changes = match head_anchor {
            Some(head_anchor) => {
                self.diff_allowlist_anchors(
                    &tracker,
                    baseline_anchor.as_deref(),
                    &head_anchor,
                    false,
                    None,
                    None,
                )
                .await?
            }
            None => Vec::new(),
        };

        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "DELETE FROM workspace_allowlist_files WHERE allowlist_id = ?1",
            params![allowlist_id.to_string()],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("allowlist cache clear failed: {e}"),
        })?;

        let now = fmt_ts(&Utc::now());
        for change in changes {
            let rows = match change.change_kind {
                WorkspaceAllowlistChangeKind::Moved => {
                    let mut rows = Vec::new();
                    if let Some(old_path) = change.old_path.as_deref() {
                        rows.push((old_path.to_string(), AllowlistedFileStatus::Deleted));
                    }
                    rows.push((change.path.clone(), AllowlistedFileStatus::Added));
                    rows
                }
                _ => vec![(change.path.clone(), change.status)],
            };

            for (path, status) in rows {
                conn.execute(
                    "INSERT INTO workspace_allowlist_files (
                        allowlist_id, relative_path, status, is_binary, remote_hash, base_hash, working_hash, conflict_reason, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL, NULL, ?5, ?5)",
                    params![
                        allowlist_id.to_string(),
                        path,
                        status_to_str(status),
                        if change.is_binary { 1 } else { 0 },
                        now.clone()
                    ],
                )
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("allowlist cache insert failed: {e}"),
                })?;
            }
        }

        Ok(())
    }

    pub(crate) async fn build_diff_from_tracker(
        &self,
        request: &WorkspaceAllowlistDiffRequest,
    ) -> Result<WorkspaceAllowlistDiff, WorkspaceError> {
        let tracker = self
            .ensure_allowlist_tracker(&request.user_id, request.allowlist_id)
            .await?;
        let from_anchor = match request.from.as_deref().unwrap_or("baseline") {
            "baseline" => tracker.baseline_anchor.clone(),
            "head" => tracker.head_anchor.clone(),
            other => {
                let revision_id =
                    Uuid::parse_str(other).map_err(|_| WorkspaceError::AllowlistConflict {
                        path: request.allowlist_id.to_string(),
                        reason: format!("unknown revision target '{other}'"),
                    })?;
                self.load_tracker_anchor_for_revision(request.allowlist_id, revision_id)
                    .await?
            }
        };
        let to_target = request.to.as_deref().unwrap_or("head");
        let (to_anchor, to_revision_id) = match to_target {
            "head" => (tracker.head_anchor.clone(), tracker.head_revision_id),
            "baseline" => (
                tracker.baseline_anchor.clone(),
                tracker.baseline_revision_id,
            ),
            other => {
                let parsed =
                    Uuid::parse_str(other).map_err(|_| WorkspaceError::AllowlistConflict {
                        path: request.allowlist_id.to_string(),
                        reason: format!("unknown revision target '{other}'"),
                    })?;
                let revision_id = if let Some(anchor) = self
                    .load_tracker_anchor_for_revision(request.allowlist_id, parsed)
                    .await?
                {
                    (Some(anchor), Some(parsed))
                } else {
                    let checkpoint = self
                        .get_checkpoint_record(request.allowlist_id, parsed)
                        .await?
                        .ok_or_else(|| WorkspaceError::AllowlistConflict {
                            path: request.allowlist_id.to_string(),
                            reason: format!("unknown revision/checkpoint '{other}'"),
                        })?;
                    (
                        self.load_tracker_anchor_for_revision(
                            request.allowlist_id,
                            checkpoint.revision_id,
                        )
                        .await?,
                        Some(checkpoint.revision_id),
                    )
                };
                revision_id
            }
        };

        let to_anchor = to_anchor.ok_or_else(|| WorkspaceError::AllowlistConflict {
            path: request.allowlist_id.to_string(),
            reason: "diff target anchor is not available".to_string(),
        })?;
        let changes = self
            .diff_allowlist_anchors(
                &tracker,
                from_anchor.as_deref(),
                &to_anchor,
                request.include_content,
                request.scope_path.as_deref(),
                request.max_files,
            )
            .await?;

        let allowlist = self
            .fetch_allowlist(&request.user_id, request.allowlist_id)
            .await?;
        let entries = changes
            .into_iter()
            .map(|change| {
                let base_content = change
                    .before_content
                    .as_deref()
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .map(ToString::to_string);
                let working_content = change
                    .after_content
                    .as_deref()
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .map(ToString::to_string);
                let remote_content = if to_revision_id == tracker.head_revision_id {
                    let disk_path = Path::new(&allowlist.source_root).join(&change.path);
                    std::fs::read(&disk_path)
                        .ok()
                        .and_then(|bytes| std::str::from_utf8(&bytes).ok().map(ToString::to_string))
                } else {
                    None
                };
                let diff_text = match (base_content.as_deref(), working_content.as_deref()) {
                    (Some(before), Some(after)) if before != after && !change.is_binary => {
                        Some(format!(
                            "--- from/{}\n+++ to/{}\n- {}\n+ {}",
                            change.old_path.as_deref().unwrap_or(&change.path),
                            change.path,
                            before.replace('\n', "\n- "),
                            after.replace('\n', "\n+ ")
                        ))
                    }
                    (None, Some(after)) if !change.is_binary => Some(format!(
                        "+++ to/{}\n+ {}",
                        change.path,
                        after.replace('\n', "\n+ ")
                    )),
                    (Some(before), None) if !change.is_binary => Some(format!(
                        "--- from/{}\n- {}",
                        change.old_path.as_deref().unwrap_or(&change.path),
                        before.replace('\n', "\n- ")
                    )),
                    _ => None,
                };
                crate::workspace::AllowlistedFileDiff {
                    path: change.path.clone(),
                    uri: crate::workspace::WorkspaceUri::allowlist_uri(
                        request.allowlist_id,
                        Some(&change.path),
                    ),
                    status: change.status,
                    change_kind: change.change_kind,
                    is_binary: change.is_binary,
                    base_content,
                    working_content,
                    remote_content,
                    diff_text,
                    conflict_reason: None,
                }
            })
            .collect();

        Ok(WorkspaceAllowlistDiff {
            allowlist_id: request.allowlist_id,
            from_revision_id: request
                .from
                .as_deref()
                .and_then(|value| Uuid::parse_str(value).ok())
                .or(tracker.baseline_revision_id),
            to_revision_id,
            entries,
        })
    }

    pub(crate) async fn restore_allowlist_from_anchor(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        target_anchor: &str,
        scope_path: Option<&str>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let tracker = self.ensure_allowlist_tracker(user_id, allowlist_id).await?;
        let current_anchor = tracker.head_anchor.clone();
        let Some(current_anchor) = current_anchor else {
            return Ok(Vec::new());
        };

        let changes = self
            .diff_allowlist_anchors(
                &tracker,
                Some(&current_anchor),
                target_anchor,
                false,
                scope_path,
                None,
            )
            .await?;
        if changes.is_empty() {
            return Ok(Vec::new());
        }

        let mut backup = Vec::new();
        for change in &changes {
            if let Some(old_path) = change.old_path.as_deref() {
                let disk_path = tracker.work_tree_path().join(old_path);
                backup.push((disk_path.clone(), std::fs::read(&disk_path).ok()));
            }
            let disk_path = tracker.work_tree_path().join(&change.path);
            backup.push((disk_path.clone(), std::fs::read(&disk_path).ok()));
        }

        let apply = async {
            for change in &changes {
                if let Some(old_path) = change.old_path.as_deref() {
                    let old_disk_path = tracker.work_tree_path().join(old_path);
                    if old_disk_path.exists() {
                        match tokio::fs::remove_file(&old_disk_path).await {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::IsADirectory => {
                                tokio::fs::remove_dir_all(&old_disk_path)
                                    .await
                                    .map_err(|e| WorkspaceError::IoError {
                                        reason: format!(
                                            "failed to remove {}: {e}",
                                            old_disk_path.display()
                                        ),
                                    })?;
                            }
                            Err(error) => {
                                return Err(WorkspaceError::IoError {
                                    reason: format!(
                                        "failed to remove {}: {error}",
                                        old_disk_path.display()
                                    ),
                                });
                            }
                        }
                    }
                }
                let disk_path = tracker.work_tree_path().join(&change.path);
                match change.change_kind {
                    WorkspaceAllowlistChangeKind::Deleted => {
                        if disk_path.exists() {
                            match tokio::fs::remove_file(&disk_path).await {
                                Ok(()) => {}
                                Err(error) if error.kind() == std::io::ErrorKind::IsADirectory => {
                                    tokio::fs::remove_dir_all(&disk_path).await.map_err(|e| {
                                        WorkspaceError::IoError {
                                            reason: format!(
                                                "failed to remove {}: {e}",
                                                disk_path.display()
                                            ),
                                        }
                                    })?;
                                }
                                Err(error) => {
                                    return Err(WorkspaceError::IoError {
                                        reason: format!(
                                            "failed to remove {}: {error}",
                                            disk_path.display()
                                        ),
                                    });
                                }
                            }
                        }
                    }
                    _ => {
                        let repo_path = tracker.repo_path_for_allowlist_path(&change.path);
                        let bytes = self
                            .read_anchor_file_bytes(&tracker, target_anchor, &repo_path)
                            .await?
                            .ok_or_else(|| WorkspaceError::AllowlistPathNotFound {
                                allowlist_id: allowlist_id.to_string(),
                                path: change.path.clone(),
                            })?;
                        if let Some(parent) = disk_path.parent() {
                            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                                WorkspaceError::IoError {
                                    reason: format!("failed to create {}: {e}", parent.display()),
                                }
                            })?;
                        }
                        tokio::fs::write(&disk_path, bytes).await.map_err(|e| {
                            WorkspaceError::IoError {
                                reason: format!("failed to restore {}: {e}", disk_path.display()),
                            }
                        })?;
                    }
                }
            }
            Ok::<(), WorkspaceError>(())
        }
        .await;

        if let Err(error) = apply {
            for (path, bytes) in backup.into_iter().rev() {
                match bytes {
                    Some(bytes) => {
                        if let Some(parent) = path.parent() {
                            let _ = tokio::fs::create_dir_all(parent).await;
                        }
                        let _ = tokio::fs::write(&path, bytes).await;
                    }
                    None => {
                        let _ = tokio::fs::remove_file(&path).await;
                    }
                }
            }
            return Err(error);
        }

        let mut dirty_paths = BTreeSet::new();
        for change in changes {
            if let Some(old_path) = change.old_path {
                dirty_paths.insert(tracker.repo_path_for_allowlist_path(&old_path));
            }
            dirty_paths.insert(tracker.repo_path_for_allowlist_path(&change.path));
        }
        Ok(dirty_paths.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ParsedNameStatusEntry, parse_name_status_z_entries, parse_porcelain_status_z_paths,
    };

    #[test]
    fn parses_unicode_paths_from_porcelain_z_status() {
        let output = "?? 已组-第二次翻转课堂-全.pptx\0".as_bytes();

        let paths = parse_porcelain_status_z_paths(output);

        assert_eq!(paths, vec!["已组-第二次翻转课堂-全.pptx".to_string()]);
    }

    #[test]
    fn parses_rename_entries_and_preserves_leading_spaces() {
        let output = b"R  renamed file.txt\0old name.txt\0??  leading space.txt\0";

        let paths = parse_porcelain_status_z_paths(output);

        assert_eq!(
            paths,
            vec![
                " leading space.txt".to_string(),
                "old name.txt".to_string(),
                "renamed file.txt".to_string(),
            ]
        );
    }

    #[test]
    fn parses_unicode_paths_from_name_status_z_output() {
        let output = "M\0翻转课堂讲稿.docx\0".as_bytes();

        let entries = parse_name_status_z_entries(output);

        assert_eq!(
            entries,
            vec![ParsedNameStatusEntry {
                status: "M".to_string(),
                old_repo_path: None,
                repo_path: "翻转课堂讲稿.docx".to_string(),
            }]
        );
    }

    #[test]
    fn parses_rename_entries_from_name_status_z_output() {
        let output = "R100\0旧名字.pptx\0新名字.pptx\0".as_bytes();

        let entries = parse_name_status_z_entries(output);

        assert_eq!(
            entries,
            vec![ParsedNameStatusEntry {
                status: "R100".to_string(),
                old_repo_path: Some("旧名字.pptx".to_string()),
                repo_path: "新名字.pptx".to_string(),
            }]
        );
    }
}
