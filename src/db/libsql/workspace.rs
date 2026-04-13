//! Workspace-related WorkspaceStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_opt_ts, get_text, get_ts,
    row_to_memory_document,
};
use crate::db::WorkspaceStore;
use crate::error::{DatabaseError, WorkspaceError};
use crate::workspace::{
    AllowlistActionRequest, AllowlistedFileDiff, AllowlistedFileStatus, ConflictResolutionRequest,
    CreateAllowlistRequest, CreateCheckpointRequest, MemoryChunk, MemoryDocument, RankedResult,
    SearchConfig, SearchResult, WorkspaceAllowlist, WorkspaceAllowlistBaselineRequest,
    WorkspaceAllowlistChangeKind, WorkspaceAllowlistCheckpoint, WorkspaceAllowlistDetail,
    WorkspaceAllowlistDiff, WorkspaceAllowlistDiffRequest, WorkspaceAllowlistFileView,
    WorkspaceAllowlistHistory, WorkspaceAllowlistHistoryRequest, WorkspaceAllowlistRestoreRequest,
    WorkspaceAllowlistRevision, WorkspaceAllowlistRevisionKind, WorkspaceAllowlistRevisionSource,
    WorkspaceAllowlistSummary, WorkspaceEntry, WorkspaceTreeEntry, WorkspaceTreeEntryKind,
    WorkspaceUri, fuse_results, normalize_allowlist_path,
};

use chrono::Utc;

/// Resolve the embedding dimension from environment variables.
///
/// Reads `EMBEDDING_ENABLED`, `EMBEDDING_DIMENSION`, and `EMBEDDING_MODEL`
/// from env vars or runtime overrides. Returns `None` if embeddings are disabled.
///
/// The model→dimension mapping is shared with `EmbeddingsConfig` via
/// `default_dimension_for_model()`.
pub(crate) fn resolve_embedding_dimension() -> Option<usize> {
    let enabled = crate::config::env_or_override("EMBEDDING_ENABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    if !enabled {
        tracing::debug!("Vector index setup skipped (EMBEDDING_ENABLED not set in env)");
        return None;
    }

    if let Some(dim_str) = crate::config::env_or_override("EMBEDDING_DIMENSION")
        && let Ok(dim) = dim_str.parse::<usize>()
        && dim > 0
    {
        return Some(dim);
    }

    let model = crate::config::env_or_override("EMBEDDING_MODEL")
        .unwrap_or_else(|| "text-embedding-3-small".to_string());

    Some(crate::config::embeddings::default_dimension_for_model(
        &model,
    ))
}

#[derive(Debug, Clone)]
struct SnapshotRecord {
    id: Uuid,
    content: Vec<u8>,
    is_binary: bool,
    hash: String,
}

#[derive(Debug, Clone)]
struct AllowlistFileRecord {
    path: String,
    status: AllowlistedFileStatus,
    is_binary: bool,
    base_snapshot_id: Option<Uuid>,
    working_snapshot_id: Option<Uuid>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ManifestEntry {
    snapshot_id: Uuid,
    is_binary: bool,
    size_bytes: i64,
    modified_at: Option<i64>,
    file_mode: Option<i64>,
}

#[derive(Debug, Clone)]
struct AllowlistStateRecord {
    baseline_revision_id: Option<Uuid>,
    head_revision_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
struct RevisionRef {
    id: Uuid,
    parent_revision_id: Option<Uuid>,
    kind: WorkspaceAllowlistRevisionKind,
    source: WorkspaceAllowlistRevisionSource,
    trigger: Option<String>,
    summary: Option<String>,
    created_by: String,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ManifestChange {
    path: String,
    before: Option<ManifestEntry>,
    after: Option<ManifestEntry>,
    status: AllowlistedFileStatus,
    change_kind: WorkspaceAllowlistChangeKind,
    is_binary: bool,
}

fn compute_content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn classify_binary(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_err()
}

fn bytes_to_text(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(ToString::to_string)
}

fn status_from_str(value: &str) -> AllowlistedFileStatus {
    match value {
        "modified" => AllowlistedFileStatus::Modified,
        "added" => AllowlistedFileStatus::Added,
        "deleted" | "pending_delete" => AllowlistedFileStatus::Deleted,
        "conflicted" => AllowlistedFileStatus::Conflicted,
        "binary_modified" => AllowlistedFileStatus::BinaryModified,
        _ => AllowlistedFileStatus::Clean,
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

fn revision_kind_from_str(value: &str) -> WorkspaceAllowlistRevisionKind {
    match value {
        "tool_write" => WorkspaceAllowlistRevisionKind::ToolWrite,
        "tool_patch" => WorkspaceAllowlistRevisionKind::ToolPatch,
        "tool_move" => WorkspaceAllowlistRevisionKind::ToolMove,
        "tool_delete" => WorkspaceAllowlistRevisionKind::ToolDelete,
        "shell" => WorkspaceAllowlistRevisionKind::Shell,
        "fs_watch" => WorkspaceAllowlistRevisionKind::FsWatch,
        "manual_refresh" => WorkspaceAllowlistRevisionKind::ManualRefresh,
        "restore" => WorkspaceAllowlistRevisionKind::Restore,
        "accept" => WorkspaceAllowlistRevisionKind::Accept,
        _ => WorkspaceAllowlistRevisionKind::Initial,
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

fn revision_source_from_str(value: &str) -> WorkspaceAllowlistRevisionSource {
    match value {
        "shell" => WorkspaceAllowlistRevisionSource::Shell,
        "external" => WorkspaceAllowlistRevisionSource::External,
        "system" => WorkspaceAllowlistRevisionSource::System,
        _ => WorkspaceAllowlistRevisionSource::WorkspaceTool,
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

fn scope_matches(path: &str, scope: Option<&str>) -> bool {
    match scope {
        None => true,
        Some(scope) if scope.is_empty() => true,
        Some(scope) => path == scope || path.starts_with(&format!("{scope}/")),
    }
}

fn manifest_entry_equals(left: Option<&ManifestEntry>, right: Option<&ManifestEntry>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.snapshot_id == right.snapshot_id
                && left.file_mode == right.file_mode
                && left.is_binary == right.is_binary
                && left.size_bytes == right.size_bytes
        }
        _ => false,
    }
}

fn allowlist_status_for_entries(
    before: Option<&ManifestEntry>,
    after: Option<&ManifestEntry>,
) -> Option<(AllowlistedFileStatus, WorkspaceAllowlistChangeKind, bool)> {
    match (before, after) {
        (None, Some(after)) => Some((
            AllowlistedFileStatus::Added,
            WorkspaceAllowlistChangeKind::Added,
            after.is_binary,
        )),
        (Some(before), None) => Some((
            AllowlistedFileStatus::Deleted,
            WorkspaceAllowlistChangeKind::Deleted,
            before.is_binary,
        )),
        (Some(before), Some(after)) if !manifest_entry_equals(Some(before), Some(after)) => Some((
            if before.is_binary || after.is_binary {
                AllowlistedFileStatus::BinaryModified
            } else {
                AllowlistedFileStatus::Modified
            },
            WorkspaceAllowlistChangeKind::Modified,
            before.is_binary || after.is_binary,
        )),
        _ => None,
    }
}

fn summarize_change_counts(changes: usize) -> Option<String> {
    if changes == 0 {
        None
    } else if changes == 1 {
        Some("1 file changed".to_string())
    } else {
        Some(format!("{changes} files changed"))
    }
}

fn render_text_diff(path: &str, before: Option<&str>, after: Option<&str>) -> Option<String> {
    match (before, after) {
        (Some(before), Some(after)) if before != after => Some(format!(
            "--- from/{path}\n+++ to/{path}\n- {}\n+ {}",
            before.replace('\n', "\n- "),
            after.replace('\n', "\n+ ")
        )),
        (None, Some(after)) => Some(format!("+++ to/{path}\n+ {}", after.replace('\n', "\n+ "))),
        (Some(before), None) => Some(format!(
            "--- from/{path}\n- {}",
            before.replace('\n', "\n- ")
        )),
        _ => None,
    }
}

fn metadata_file_mode(metadata: &std::fs::Metadata) -> Option<i64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        Some(i64::from(metadata.permissions().mode()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn metadata_mtime(metadata: &std::fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs() as i64)
}

fn collect_manifest_changes(
    before: &BTreeMap<String, ManifestEntry>,
    after: &BTreeMap<String, ManifestEntry>,
    scope: Option<&str>,
) -> Vec<ManifestChange> {
    let mut paths = BTreeSet::new();
    for path in before.keys() {
        if scope_matches(path, scope) {
            paths.insert(path.clone());
        }
    }
    for path in after.keys() {
        if scope_matches(path, scope) {
            paths.insert(path.clone());
        }
    }

    let mut changes = Vec::new();
    for path in paths {
        let before_entry = before.get(&path).cloned();
        let after_entry = after.get(&path).cloned();
        if let Some((status, change_kind, is_binary)) =
            allowlist_status_for_entries(before_entry.as_ref(), after_entry.as_ref())
        {
            changes.push(ManifestChange {
                path,
                before: before_entry,
                after: after_entry,
                status,
                change_kind,
                is_binary,
            });
        }
    }

    let mut deletes: HashMap<String, usize> = HashMap::new();
    let mut adds: HashMap<String, usize> = HashMap::new();
    for (idx, change) in changes.iter().enumerate() {
        match change.change_kind {
            WorkspaceAllowlistChangeKind::Deleted => {
                if let Some(before) = &change.before {
                    deletes.insert(before.snapshot_id.to_string(), idx);
                }
            }
            WorkspaceAllowlistChangeKind::Added => {
                if let Some(after) = &change.after {
                    adds.insert(after.snapshot_id.to_string(), idx);
                }
            }
            _ => {}
        }
    }
    for (hash_key, delete_idx) in deletes {
        if let Some(add_idx) = adds.get(&hash_key) {
            if let Some(delete_change) = changes.get_mut(delete_idx) {
                delete_change.change_kind = WorkspaceAllowlistChangeKind::Moved;
            }
            if let Some(add_change) = changes.get_mut(*add_idx) {
                add_change.change_kind = WorkspaceAllowlistChangeKind::Moved;
            }
        }
    }

    changes
}

impl LibSqlBackend {
    async fn read_snapshot_required(
        &self,
        snapshot_id: Uuid,
    ) -> Result<SnapshotRecord, WorkspaceError> {
        self.read_snapshot(snapshot_id)
            .await?
            .ok_or_else(|| WorkspaceError::SearchFailed {
                reason: format!("missing snapshot {snapshot_id}"),
            })
    }

    async fn read_disk_bytes(path: &Path) -> Result<Option<Vec<u8>>, WorkspaceError> {
        match tokio::fs::read(path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(WorkspaceError::IoError {
                reason: format!("failed to read {}: {error}", path.display()),
            }),
        }
    }

    async fn load_allowlist_state_record(
        &self,
        allowlist_id: Uuid,
    ) -> Result<Option<AllowlistStateRecord>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT baseline_revision_id, head_revision_id
                 FROM workspace_allowlist_state
                 WHERE allowlist_id = ?1",
                params![allowlist_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist state query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist state query failed: {e}"),
            })?
        else {
            return Ok(None);
        };

        Ok(Some(AllowlistStateRecord {
            baseline_revision_id: get_opt_text(&row, 0).and_then(|v| Uuid::parse_str(&v).ok()),
            head_revision_id: get_opt_text(&row, 1).and_then(|v| Uuid::parse_str(&v).ok()),
        }))
    }

    async fn save_allowlist_state_record(
        &self,
        allowlist_id: Uuid,
        baseline_revision_id: Option<Uuid>,
        head_revision_id: Option<Uuid>,
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
            INSERT INTO workspace_allowlist_state (
                allowlist_id, baseline_revision_id, head_revision_id, last_reconciled_at
            )
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(allowlist_id) DO UPDATE SET
                baseline_revision_id = excluded.baseline_revision_id,
                head_revision_id = excluded.head_revision_id,
                last_reconciled_at = excluded.last_reconciled_at
            "#,
            params![
                allowlist_id.to_string(),
                baseline_revision_id.map(|v| v.to_string()),
                head_revision_id.map(|v| v.to_string()),
                now
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("allowlist state upsert failed: {e}"),
        })?;
        Ok(())
    }

    async fn touch_allowlist_updated_at(&self, allowlist_id: Uuid) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "UPDATE workspace_allowlists SET updated_at = ?2 WHERE id = ?1",
            params![allowlist_id.to_string(), fmt_ts(&Utc::now())],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("allowlist timestamp update failed: {e}"),
        })?;
        Ok(())
    }

    async fn load_revision_record(
        &self,
        allowlist_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<RevisionRef>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT id, parent_revision_id, kind, source, trigger, summary, created_by, created_at
                 FROM workspace_allowlist_revisions
                 WHERE allowlist_id = ?1 AND id = ?2",
                params![allowlist_id.to_string(), revision_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("revision query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("revision query failed: {e}"),
            })?
        else {
            return Ok(None);
        };

        Ok(Some(RevisionRef {
            id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("invalid revision id: {e}"),
            })?,
            parent_revision_id: get_opt_text(&row, 1).and_then(|v| Uuid::parse_str(&v).ok()),
            kind: revision_kind_from_str(&get_text(&row, 2)),
            source: revision_source_from_str(&get_text(&row, 3)),
            trigger: get_opt_text(&row, 4),
            summary: get_opt_text(&row, 5),
            created_by: get_text(&row, 6),
            created_at: get_ts(&row, 7),
        }))
    }

    async fn list_revision_refs(
        &self,
        allowlist_id: Uuid,
        limit: Option<usize>,
        since: Option<chrono::DateTime<Utc>>,
    ) -> Result<Vec<RevisionRef>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = if let Some(since) = since {
            let mut sql = String::from(
                "SELECT id, parent_revision_id, kind, source, trigger, summary, created_by, created_at
                 FROM workspace_allowlist_revisions
                 WHERE allowlist_id = ?1 AND created_at >= ?2
                 ORDER BY created_at DESC",
            );
            if let Some(limit) = limit {
                sql.push_str(&format!(" LIMIT {}", limit as i64));
            }
            conn.query(&sql, params![allowlist_id.to_string(), fmt_ts(&since)])
                .await
        } else {
            let mut sql = String::from(
                "SELECT id, parent_revision_id, kind, source, trigger, summary, created_by, created_at
                 FROM workspace_allowlist_revisions
                 WHERE allowlist_id = ?1
                 ORDER BY created_at DESC",
            );
            if let Some(limit) = limit {
                sql.push_str(&format!(" LIMIT {}", limit as i64));
            }
            conn.query(&sql, params![allowlist_id.to_string()]).await
        }
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("revision history query failed: {e}"),
        })?;
        let mut revisions = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("revision history query failed: {e}"),
            })?
        {
            revisions.push(RevisionRef {
                id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| {
                    WorkspaceError::SearchFailed {
                        reason: format!("invalid revision id: {e}"),
                    }
                })?,
                parent_revision_id: get_opt_text(&row, 1).and_then(|v| Uuid::parse_str(&v).ok()),
                kind: revision_kind_from_str(&get_text(&row, 2)),
                source: revision_source_from_str(&get_text(&row, 3)),
                trigger: get_opt_text(&row, 4),
                summary: get_opt_text(&row, 5),
                created_by: get_text(&row, 6),
                created_at: get_ts(&row, 7),
            });
        }
        Ok(revisions)
    }

    async fn load_manifest_entries(
        &self,
        revision_id: Uuid,
    ) -> Result<BTreeMap<String, ManifestEntry>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT relative_path, snapshot_id, file_mode, size_bytes, modified_at, is_binary
                 FROM workspace_allowlist_manifests
                 WHERE revision_id = ?1
                 ORDER BY relative_path",
                params![revision_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("manifest query failed: {e}"),
            })?;
        let mut entries = BTreeMap::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("manifest query failed: {e}"),
            })?
        {
            let snapshot_id_text =
                get_opt_text(&row, 1).ok_or_else(|| WorkspaceError::SearchFailed {
                    reason: "manifest row missing snapshot".to_string(),
                })?;
            let snapshot_id =
                Uuid::parse_str(&snapshot_id_text).map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("invalid manifest snapshot id: {e}"),
                })?;
            entries.insert(
                get_text(&row, 0),
                ManifestEntry {
                    snapshot_id,
                    file_mode: row.get::<Option<i64>>(2).ok().flatten(),
                    size_bytes: get_i64(&row, 3),
                    modified_at: row.get::<Option<i64>>(4).ok().flatten(),
                    is_binary: get_i64(&row, 5) != 0,
                },
            );
        }
        Ok(entries)
    }

    async fn write_manifest_entries(
        &self,
        revision_id: Uuid,
        manifest: &BTreeMap<String, ManifestEntry>,
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        for (path, entry) in manifest {
            conn.execute(
                "INSERT INTO workspace_allowlist_manifests (
                    revision_id, relative_path, snapshot_id, file_mode, size_bytes, modified_at, is_binary
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    revision_id.to_string(),
                    path.clone(),
                    entry.snapshot_id.to_string(),
                    entry.file_mode,
                    entry.size_bytes,
                    entry.modified_at,
                    if entry.is_binary { 1 } else { 0 }
                ],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("manifest insert failed: {e}"),
            })?;
        }
        Ok(())
    }

    async fn insert_revision_record(
        &self,
        allowlist_id: Uuid,
        parent_revision_id: Option<Uuid>,
        kind: WorkspaceAllowlistRevisionKind,
        source: WorkspaceAllowlistRevisionSource,
        trigger: Option<String>,
        summary: Option<String>,
        created_by: impl Into<String>,
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
                parent_revision_id.map(|v| v.to_string()),
                revision_kind_to_str(kind),
                revision_source_to_str(source),
                trigger,
                summary,
                created_by.into(),
                fmt_ts(&Utc::now())
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("revision insert failed: {e}"),
        })?;
        Ok(revision_id)
    }

    async fn insert_revision_changes(
        &self,
        revision_id: Uuid,
        changes: &[ManifestChange],
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        for change in changes {
            let rename_from = if change.change_kind == WorkspaceAllowlistChangeKind::Moved {
                change.before.as_ref().map(|_| change.path.clone())
            } else {
                None
            };
            let rename_to = if change.change_kind == WorkspaceAllowlistChangeKind::Moved {
                change.after.as_ref().map(|_| change.path.clone())
            } else {
                None
            };
            conn.execute(
                "INSERT INTO workspace_allowlist_revision_files (
                    revision_id, relative_path, change_kind, before_snapshot_id, after_snapshot_id,
                    before_mode, after_mode, is_binary, rename_from, rename_to
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    revision_id.to_string(),
                    change.path.clone(),
                    change_kind_to_str(change.change_kind),
                    change.before.as_ref().map(|v| v.snapshot_id.to_string()),
                    change.after.as_ref().map(|v| v.snapshot_id.to_string()),
                    change.before.as_ref().and_then(|v| v.file_mode),
                    change.after.as_ref().and_then(|v| v.file_mode),
                    if change.is_binary { 1 } else { 0 },
                    rename_from,
                    rename_to
                ],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("revision change insert failed: {e}"),
            })?;
        }
        Ok(())
    }

    async fn list_revision_changed_files(
        &self,
        revision_id: Uuid,
        scope: Option<&str>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT relative_path FROM workspace_allowlist_revision_files
                 WHERE revision_id = ?1
                 ORDER BY relative_path",
                params![revision_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("revision files query failed: {e}"),
            })?;
        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("revision files query failed: {e}"),
            })?
        {
            let path = get_text(&row, 0);
            if scope_matches(&path, scope) {
                result.push(path);
            }
        }
        Ok(result)
    }

    async fn clear_allowlist_live_cache(&self, allowlist_id: Uuid) -> Result<(), WorkspaceError> {
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
        Ok(())
    }

    async fn sync_allowlist_live_cache(
        &self,
        allowlist_id: Uuid,
        baseline_revision_id: Option<Uuid>,
        head_revision_id: Option<Uuid>,
    ) -> Result<(), WorkspaceError> {
        let baseline = match baseline_revision_id {
            Some(revision_id) => self.load_manifest_entries(revision_id).await?,
            None => BTreeMap::new(),
        };
        let head = match head_revision_id {
            Some(revision_id) => self.load_manifest_entries(revision_id).await?,
            None => BTreeMap::new(),
        };
        self.clear_allowlist_live_cache(allowlist_id).await?;
        for change in collect_manifest_changes(&baseline, &head, None) {
            let base_snapshot_id = change.before.as_ref().map(|v| v.snapshot_id);
            let working_snapshot_id = change.after.as_ref().map(|v| v.snapshot_id);
            self.upsert_allowlist_file_record(
                allowlist_id,
                &change.path,
                change.status,
                change.is_binary,
                base_snapshot_id,
                working_snapshot_id,
            )
            .await?;
        }
        Ok(())
    }

    async fn create_revision_from_manifest(
        &self,
        allowlist_id: Uuid,
        parent_revision_id: Option<Uuid>,
        previous_manifest: &BTreeMap<String, ManifestEntry>,
        next_manifest: &BTreeMap<String, ManifestEntry>,
        kind: WorkspaceAllowlistRevisionKind,
        source: WorkspaceAllowlistRevisionSource,
        trigger: Option<String>,
        summary: Option<String>,
        created_by: impl Into<String>,
    ) -> Result<Uuid, WorkspaceError> {
        let changes = collect_manifest_changes(previous_manifest, next_manifest, None);
        let revision_id = self
            .insert_revision_record(
                allowlist_id,
                parent_revision_id,
                kind,
                source,
                trigger,
                summary.or_else(|| summarize_change_counts(changes.len())),
                created_by,
            )
            .await?;
        self.write_manifest_entries(revision_id, next_manifest)
            .await?;
        self.insert_revision_changes(revision_id, &changes).await?;
        Ok(revision_id)
    }

    async fn get_checkpoint_record(
        &self,
        allowlist_id: Uuid,
        checkpoint_id: Uuid,
    ) -> Result<Option<WorkspaceAllowlistCheckpoint>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT id, allowlist_id, revision_id, parent_checkpoint_id, label, summary, created_by, is_auto, base_generation, created_at
                 FROM workspace_allowlist_checkpoints
                 WHERE allowlist_id = ?1 AND id = ?2",
                params![allowlist_id.to_string(), checkpoint_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("checkpoint query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("checkpoint query failed: {e}"),
            })?
        else {
            return Ok(None);
        };
        let revision_id = get_opt_text(&row, 2)
            .and_then(|v| Uuid::parse_str(&v).ok())
            .ok_or_else(|| WorkspaceError::SearchFailed {
                reason: "checkpoint missing revision_id".to_string(),
            })?;
        Ok(Some(WorkspaceAllowlistCheckpoint {
            id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("invalid checkpoint id: {e}"),
            })?,
            allowlist_id: Uuid::parse_str(&get_text(&row, 1)).map_err(|e| {
                WorkspaceError::SearchFailed {
                    reason: format!("invalid checkpoint allowlist id: {e}"),
                }
            })?,
            revision_id,
            parent_checkpoint_id: get_opt_text(&row, 3).and_then(|v| Uuid::parse_str(&v).ok()),
            label: get_opt_text(&row, 4),
            summary: get_opt_text(&row, 5),
            created_by: get_text(&row, 6),
            is_auto: get_i64(&row, 7) != 0,
            base_generation: get_i64(&row, 8),
            created_at: get_ts(&row, 9),
            changed_files: self.list_revision_changed_files(revision_id, None).await?,
        }))
    }

    async fn create_checkpoint_record(
        &self,
        allowlist_id: Uuid,
        revision_id: Uuid,
        label: Option<String>,
        summary: Option<String>,
        created_by: String,
        is_auto: bool,
    ) -> Result<WorkspaceAllowlistCheckpoint, WorkspaceError> {
        let parent = self
            .collect_checkpoint_chain(allowlist_id)
            .await?
            .first()
            .map(|checkpoint| checkpoint.id);
        let checkpoint_id = Uuid::new_v4();
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "INSERT INTO workspace_allowlist_checkpoints (
                id, allowlist_id, revision_id, parent_checkpoint_id, label, summary, created_by, is_auto, base_generation, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                checkpoint_id.to_string(),
                allowlist_id.to_string(),
                revision_id.to_string(),
                parent.map(|v| v.to_string()),
                label,
                summary,
                created_by,
                if is_auto { 1 } else { 0 },
                0_i64,
                fmt_ts(&Utc::now())
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("checkpoint insert failed: {e}"),
        })?;
        self.get_checkpoint_record(allowlist_id, checkpoint_id)
            .await?
            .ok_or_else(|| WorkspaceError::SearchFailed {
                reason: "checkpoint missing after create".to_string(),
            })
    }

    async fn scan_allowlist_manifest(
        &self,
        allowlist_id: Uuid,
        source_root: &Path,
    ) -> Result<BTreeMap<String, ManifestEntry>, WorkspaceError> {
        fn collect_paths(
            root: &Path,
            dir: &Path,
            entries: &mut Vec<(String, PathBuf, std::fs::Metadata)>,
        ) -> Result<(), WorkspaceError> {
            let read_dir = std::fs::read_dir(dir).map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to read directory {}: {e}", dir.display()),
            })?;
            for entry in read_dir {
                let entry = entry.map_err(|e| WorkspaceError::IoError {
                    reason: format!("failed to read directory entry: {e}"),
                })?;
                let path = entry.path();
                let metadata = entry.metadata().map_err(|e| WorkspaceError::IoError {
                    reason: format!("failed to read metadata {}: {e}", path.display()),
                })?;
                if metadata.is_dir() {
                    collect_paths(root, &path, entries)?;
                } else if metadata.is_file() {
                    let rel = path
                        .strip_prefix(root)
                        .map_err(|e| WorkspaceError::IoError {
                            reason: format!("failed to strip allowlist prefix: {e}"),
                        })?
                        .to_string_lossy()
                        .replace('\\', "/");
                    entries.push((rel, path, metadata));
                }
            }
            Ok(())
        }

        let mut files = Vec::new();
        collect_paths(source_root, source_root, &mut files)?;
        files.sort_by(|left, right| left.0.cmp(&right.0));

        let mut manifest = BTreeMap::new();
        for (relative_path, absolute_path, metadata) in files {
            let bytes =
                tokio::fs::read(&absolute_path)
                    .await
                    .map_err(|e| WorkspaceError::IoError {
                        reason: format!("failed to read {}: {e}", absolute_path.display()),
                    })?;
            let snapshot = self
                .insert_snapshot(allowlist_id, &relative_path, &bytes)
                .await?;
            manifest.insert(
                relative_path,
                ManifestEntry {
                    snapshot_id: snapshot.id,
                    is_binary: snapshot.is_binary,
                    size_bytes: metadata.len() as i64,
                    modified_at: metadata_mtime(&metadata),
                    file_mode: metadata_file_mode(&metadata),
                },
            );
        }
        Ok(manifest)
    }

    async fn materialize_legacy_allowlist_overlay_if_needed(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<bool, WorkspaceError> {
        let records = self.list_allowlist_file_records(allowlist_id, None).await?;
        if records.is_empty() {
            return Ok(false);
        }
        if records
            .iter()
            .any(|record| record.status == AllowlistedFileStatus::Conflicted)
        {
            return Err(WorkspaceError::AllowlistConflict {
                path: allowlist_id.to_string(),
                reason: "legacy overlay conflict blocks migration to real filesystem mode"
                    .to_string(),
            });
        }
        let has_dirty = records
            .iter()
            .any(|record| record.status != AllowlistedFileStatus::Clean);
        if !has_dirty {
            return Ok(false);
        }

        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        for record in records {
            let disk_path = Path::new(&allowlist.source_root).join(&record.path);
            match record.status {
                AllowlistedFileStatus::Deleted => {
                    if tokio::fs::try_exists(&disk_path).await.map_err(|e| {
                        WorkspaceError::IoError {
                            reason: format!("failed to check {}: {e}", disk_path.display()),
                        }
                    })? {
                        tokio::fs::remove_file(&disk_path).await.map_err(|e| {
                            WorkspaceError::IoError {
                                reason: format!("failed to delete {}: {e}", disk_path.display()),
                            }
                        })?;
                    }
                }
                AllowlistedFileStatus::Clean => {}
                _ => {
                    let snapshot_id = record
                        .working_snapshot_id
                        .or(record.base_snapshot_id)
                        .ok_or_else(|| WorkspaceError::AllowlistConflict {
                            path: record.path.clone(),
                            reason: "legacy overlay record is missing content snapshot".to_string(),
                        })?;
                    let snapshot = self.read_snapshot_required(snapshot_id).await?;
                    if let Some(parent) = disk_path.parent() {
                        tokio::fs::create_dir_all(parent).await.map_err(|e| {
                            WorkspaceError::IoError {
                                reason: format!("failed to create {}: {e}", parent.display()),
                            }
                        })?;
                    }
                    tokio::fs::write(&disk_path, snapshot.content)
                        .await
                        .map_err(|e| WorkspaceError::IoError {
                            reason: format!("failed to write {}: {e}", disk_path.display()),
                        })?;
                }
            }
        }
        Ok(true)
    }

    async fn ensure_allowlist_initialized(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<AllowlistStateRecord, WorkspaceError> {
        if let Some(state) = self.load_allowlist_state_record(allowlist_id).await? {
            return Ok(state);
        }

        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        let initial_disk_manifest = self
            .scan_allowlist_manifest(allowlist_id, Path::new(&allowlist.source_root))
            .await?;
        let initial_revision_id = self
            .create_revision_from_manifest(
                allowlist_id,
                None,
                &BTreeMap::new(),
                &initial_disk_manifest,
                WorkspaceAllowlistRevisionKind::Initial,
                WorkspaceAllowlistRevisionSource::System,
                Some("initial_scan".to_string()),
                Some("initial disk scan".to_string()),
                "system",
            )
            .await?;
        self.create_checkpoint_record(
            allowlist_id,
            initial_revision_id,
            Some("pre-real-fs-migration".to_string()),
            Some("state before real filesystem migration".to_string()),
            "system".to_string(),
            true,
        )
        .await?;

        let overlay_materialized = self
            .materialize_legacy_allowlist_overlay_if_needed(user_id, allowlist_id)
            .await?;
        let (baseline_revision_id, head_revision_id) = if overlay_materialized {
            let migrated_manifest = self
                .scan_allowlist_manifest(allowlist_id, Path::new(&allowlist.source_root))
                .await?;
            let migration_revision_id = self
                .create_revision_from_manifest(
                    allowlist_id,
                    Some(initial_revision_id),
                    &initial_disk_manifest,
                    &migrated_manifest,
                    WorkspaceAllowlistRevisionKind::ManualRefresh,
                    WorkspaceAllowlistRevisionSource::System,
                    Some("migration_import".to_string()),
                    Some("imported legacy overlay into real filesystem".to_string()),
                    "system",
                )
                .await?;
            (Some(migration_revision_id), Some(migration_revision_id))
        } else {
            (Some(initial_revision_id), Some(initial_revision_id))
        };

        self.save_allowlist_state_record(allowlist_id, baseline_revision_id, head_revision_id)
            .await?;
        self.sync_allowlist_live_cache(allowlist_id, baseline_revision_id, head_revision_id)
            .await?;
        self.touch_allowlist_updated_at(allowlist_id).await?;

        Ok(AllowlistStateRecord {
            baseline_revision_id,
            head_revision_id,
        })
    }

    async fn reconcile_allowlist(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        kind: WorkspaceAllowlistRevisionKind,
        source: WorkspaceAllowlistRevisionSource,
        trigger: Option<String>,
        summary: Option<String>,
        created_by: impl Into<String>,
    ) -> Result<AllowlistStateRecord, WorkspaceError> {
        let mut state = self
            .ensure_allowlist_initialized(user_id, allowlist_id)
            .await?;
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        let head_manifest = match state.head_revision_id {
            Some(revision_id) => self.load_manifest_entries(revision_id).await?,
            None => BTreeMap::new(),
        };
        let disk_manifest = self
            .scan_allowlist_manifest(allowlist_id, Path::new(&allowlist.source_root))
            .await?;
        let changes = collect_manifest_changes(&head_manifest, &disk_manifest, None);
        if !changes.is_empty() {
            let new_revision_id = self
                .create_revision_from_manifest(
                    allowlist_id,
                    state.head_revision_id,
                    &head_manifest,
                    &disk_manifest,
                    kind,
                    source,
                    trigger,
                    summary.or_else(|| summarize_change_counts(changes.len())),
                    created_by,
                )
                .await?;
            state.head_revision_id = Some(new_revision_id);
        }
        self.save_allowlist_state_record(
            allowlist_id,
            state.baseline_revision_id,
            state.head_revision_id,
        )
        .await?;
        self.sync_allowlist_live_cache(
            allowlist_id,
            state.baseline_revision_id,
            state.head_revision_id,
        )
        .await?;
        self.touch_allowlist_updated_at(allowlist_id).await?;
        Ok(state)
    }

    async fn resolve_revision_target(
        &self,
        allowlist_id: Uuid,
        state: &AllowlistStateRecord,
        target: &str,
    ) -> Result<Uuid, WorkspaceError> {
        match target {
            "baseline" => {
                state
                    .baseline_revision_id
                    .ok_or_else(|| WorkspaceError::AllowlistConflict {
                        path: allowlist_id.to_string(),
                        reason: "baseline revision is not set".to_string(),
                    })
            }
            "head" => state
                .head_revision_id
                .ok_or_else(|| WorkspaceError::AllowlistConflict {
                    path: allowlist_id.to_string(),
                    reason: "head revision is not set".to_string(),
                }),
            other => {
                let parsed =
                    Uuid::parse_str(other).map_err(|_| WorkspaceError::AllowlistConflict {
                        path: allowlist_id.to_string(),
                        reason: format!("unknown revision target '{other}'"),
                    })?;
                if let Some(checkpoint) = self.get_checkpoint_record(allowlist_id, parsed).await? {
                    return Ok(checkpoint.revision_id);
                }
                if self
                    .load_revision_record(allowlist_id, parsed)
                    .await?
                    .is_some()
                {
                    return Ok(parsed);
                }
                Err(WorkspaceError::AllowlistConflict {
                    path: allowlist_id.to_string(),
                    reason: format!("unknown revision/checkpoint '{other}'"),
                })
            }
        }
    }

    async fn fetch_allowlist(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<WorkspaceAllowlist, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, display_name, source_root, bypass_read, bypass_write, created_at, updated_at
                 FROM workspace_allowlists
                 WHERE user_id = ?1 AND id = ?2",
                params![user_id, allowlist_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist query failed: {e}"),
            })?
        else {
            return Err(WorkspaceError::AllowlistNotFound {
                allowlist_id: allowlist_id.to_string(),
            });
        };

        Ok(WorkspaceAllowlist {
            id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("invalid allowlist id: {e}"),
            })?,
            user_id: get_text(&row, 1),
            display_name: get_text(&row, 2),
            source_root: get_text(&row, 3),
            bypass_read: get_i64(&row, 4) != 0,
            bypass_write: get_i64(&row, 5) != 0,
            created_at: get_ts(&row, 6),
            updated_at: get_ts(&row, 7),
        })
    }

    async fn list_allowlist_summaries_internal(
        &self,
        user_id: &str,
    ) -> Result<Vec<WorkspaceAllowlistSummary>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                r#"
                SELECT m.id, m.user_id, m.display_name, m.source_root, m.bypass_read, m.bypass_write,
                       m.created_at, m.updated_at,
                       COALESCE(SUM(CASE WHEN f.status != 'clean' THEN 1 ELSE 0 END), 0),
                       COALESCE(SUM(CASE WHEN f.status = 'conflicted' THEN 1 ELSE 0 END), 0),
                       COALESCE(SUM(CASE WHEN f.status = 'deleted' THEN 1 ELSE 0 END), 0)
                FROM workspace_allowlists m
                LEFT JOIN workspace_allowlist_files f ON f.allowlist_id = m.id
                WHERE m.user_id = ?1
                GROUP BY m.id, m.user_id, m.display_name, m.source_root, m.bypass_read, m.bypass_write,
                         m.created_at, m.updated_at
                ORDER BY m.updated_at DESC
                "#,
                params![user_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist list failed: {e}"),
            })?;

        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist list failed: {e}"),
            })?
        {
            let allowlist = WorkspaceAllowlist {
                id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| {
                    WorkspaceError::SearchFailed {
                        reason: format!("invalid allowlist id: {e}"),
                    }
                })?,
                user_id: get_text(&row, 1),
                display_name: get_text(&row, 2),
                source_root: get_text(&row, 3),
                bypass_read: get_i64(&row, 4) != 0,
                bypass_write: get_i64(&row, 5) != 0,
                created_at: get_ts(&row, 6),
                updated_at: get_ts(&row, 7),
            };
            result.push(WorkspaceAllowlistSummary {
                allowlist,
                dirty_count: get_i64(&row, 8) as usize,
                conflict_count: get_i64(&row, 9) as usize,
                pending_delete_count: get_i64(&row, 10) as usize,
            });
        }
        Ok(result)
    }

    async fn read_snapshot(
        &self,
        snapshot_id: Uuid,
    ) -> Result<Option<SnapshotRecord>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT id, content, is_binary, content_hash FROM workspace_allowlist_snapshots WHERE id = ?1",
                params![snapshot_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("snapshot query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("snapshot query failed: {e}"),
            })?
        else {
            return Ok(None);
        };
        Ok(Some(SnapshotRecord {
            id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("invalid snapshot id: {e}"),
            })?,
            content: row
                .get::<Vec<u8>>(1)
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("invalid snapshot content: {e}"),
                })?,
            is_binary: get_i64(&row, 2) != 0,
            hash: get_text(&row, 3),
        }))
    }

    async fn insert_snapshot(
        &self,
        allowlist_id: Uuid,
        path: &str,
        content: &[u8],
    ) -> Result<SnapshotRecord, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let hash = compute_content_hash(content);
        let is_binary = classify_binary(content);
        let mut existing_rows = conn
            .query(
                "SELECT id, content, is_binary, content_hash
                 FROM workspace_allowlist_snapshots
                 WHERE allowlist_id = ?1 AND relative_path = ?2 AND content_hash = ?3
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![allowlist_id.to_string(), path, hash.clone()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("snapshot dedupe query failed: {e}"),
            })?;
        if let Some(row) = existing_rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("snapshot dedupe query failed: {e}"),
            })?
        {
            return Ok(SnapshotRecord {
                id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| {
                    WorkspaceError::SearchFailed {
                        reason: format!("invalid snapshot id: {e}"),
                    }
                })?,
                content: row
                    .get::<Vec<u8>>(1)
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("invalid snapshot content: {e}"),
                    })?,
                is_binary: get_i64(&row, 2) != 0,
                hash: get_text(&row, 3),
            });
        }
        let snapshot = SnapshotRecord {
            id: Uuid::new_v4(),
            content: content.to_vec(),
            is_binary,
            hash,
        };
        conn.execute(
            "INSERT INTO workspace_allowlist_snapshots (id, allowlist_id, relative_path, content, is_binary, content_hash, size_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                snapshot.id.to_string(),
                allowlist_id.to_string(),
                path,
                snapshot.content.clone(),
                if snapshot.is_binary { 1 } else { 0 },
                snapshot.hash.clone(),
                snapshot.content.len() as i64
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("snapshot insert failed: {e}"),
        })?;
        Ok(snapshot)
    }

    async fn load_allowlist_file_record(
        &self,
        allowlist_id: Uuid,
        path: &str,
    ) -> Result<Option<AllowlistFileRecord>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT relative_path, status, is_binary, base_snapshot_id, working_snapshot_id, updated_at
                 FROM workspace_allowlist_files
                 WHERE allowlist_id = ?1 AND relative_path = ?2",
                params![allowlist_id.to_string(), path],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist file query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist file query failed: {e}"),
            })?
        else {
            return Ok(None);
        };
        Ok(Some(AllowlistFileRecord {
            path: get_text(&row, 0),
            status: status_from_str(&get_text(&row, 1)),
            is_binary: get_i64(&row, 2) != 0,
            base_snapshot_id: get_opt_text(&row, 3).and_then(|v| Uuid::parse_str(&v).ok()),
            working_snapshot_id: get_opt_text(&row, 4).and_then(|v| Uuid::parse_str(&v).ok()),
            updated_at: get_ts(&row, 5),
        }))
    }

    async fn upsert_allowlist_file_record(
        &self,
        allowlist_id: Uuid,
        path: &str,
        status: AllowlistedFileStatus,
        is_binary: bool,
        base_snapshot_id: Option<Uuid>,
        working_snapshot_id: Option<Uuid>,
    ) -> Result<AllowlistFileRecord, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
            INSERT INTO workspace_allowlist_files (
                allowlist_id, relative_path, status, is_binary, base_snapshot_id, working_snapshot_id,
                remote_hash, base_hash, working_hash, conflict_reason, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
            ON CONFLICT(allowlist_id, relative_path) DO UPDATE SET
                status = excluded.status,
                is_binary = excluded.is_binary,
                base_snapshot_id = excluded.base_snapshot_id,
                working_snapshot_id = excluded.working_snapshot_id,
                updated_at = excluded.updated_at
            "#,
            params![
                allowlist_id.to_string(),
                path,
                status_to_str(status),
                if is_binary { 1 } else { 0 },
                base_snapshot_id.map(|v| v.to_string()),
                working_snapshot_id.map(|v| v.to_string()),
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                now
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("allowlist file upsert failed: {e}"),
        })?;
        self.load_allowlist_file_record(allowlist_id, path)
            .await?
            .ok_or_else(|| WorkspaceError::AllowlistPathNotFound {
                allowlist_id: allowlist_id.to_string(),
                path: path.to_string(),
            })
    }

    async fn list_allowlist_file_records(
        &self,
        allowlist_id: Uuid,
        prefix: Option<&str>,
    ) -> Result<Vec<AllowlistFileRecord>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let pattern = if let Some(value) = prefix {
            let normalized = normalize_allowlist_path(value)?;
            if normalized.is_empty() {
                "%".to_string()
            } else {
                format!("{normalized}%")
            }
        } else {
            "%".to_string()
        };
        let mut rows = conn
            .query(
                "SELECT relative_path, status, is_binary, base_snapshot_id, working_snapshot_id, updated_at
                 FROM workspace_allowlist_files
                 WHERE allowlist_id = ?1 AND relative_path LIKE ?2
                 ORDER BY relative_path",
                params![allowlist_id.to_string(), pattern],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist file list failed: {e}"),
            })?;
        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist file list failed: {e}"),
            })?
        {
            result.push(AllowlistFileRecord {
                path: get_text(&row, 0),
                status: status_from_str(&get_text(&row, 1)),
                is_binary: get_i64(&row, 2) != 0,
                base_snapshot_id: get_opt_text(&row, 3).and_then(|v| Uuid::parse_str(&v).ok()),
                working_snapshot_id: get_opt_text(&row, 4).and_then(|v| Uuid::parse_str(&v).ok()),
                updated_at: get_ts(&row, 5),
            });
        }
        Ok(result)
    }

    async fn collect_checkpoint_chain(
        &self,
        allowlist_id: Uuid,
    ) -> Result<Vec<WorkspaceAllowlistCheckpoint>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT id, allowlist_id, revision_id, parent_checkpoint_id, label, summary, created_by, is_auto, base_generation, created_at
                 FROM workspace_allowlist_checkpoints
                 WHERE allowlist_id = ?1
                 ORDER BY created_at DESC",
                params![allowlist_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("checkpoint query failed: {e}"),
            })?;
        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("checkpoint query failed: {e}"),
            })?
        {
            let checkpoint_id =
                Uuid::parse_str(&get_text(&row, 0)).map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("invalid checkpoint id: {e}"),
                })?;
            let revision_id = get_opt_text(&row, 2)
                .and_then(|v| Uuid::parse_str(&v).ok())
                .ok_or_else(|| WorkspaceError::SearchFailed {
                    reason: format!("checkpoint {checkpoint_id} missing revision_id"),
                })?;
            let changed_files = self.list_revision_changed_files(revision_id, None).await?;
            result.push(WorkspaceAllowlistCheckpoint {
                id: checkpoint_id,
                allowlist_id: Uuid::parse_str(&get_text(&row, 1)).map_err(|e| {
                    WorkspaceError::SearchFailed {
                        reason: format!("invalid checkpoint allowlist id: {e}"),
                    }
                })?,
                revision_id,
                parent_checkpoint_id: get_opt_text(&row, 3).and_then(|v| Uuid::parse_str(&v).ok()),
                label: get_opt_text(&row, 4),
                summary: get_opt_text(&row, 5),
                created_by: get_text(&row, 6),
                is_auto: get_i64(&row, 7) != 0,
                base_generation: get_i64(&row, 8),
                created_at: get_ts(&row, 9),
                changed_files,
            });
        }
        Ok(result)
    }

    async fn build_allowlist_detail_internal(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        let state = self
            .ensure_allowlist_initialized(user_id, allowlist_id)
            .await?;
        let summary = self
            .list_allowlist_summaries_internal(user_id)
            .await?
            .into_iter()
            .find(|summary| summary.allowlist.id == allowlist_id)
            .ok_or_else(|| WorkspaceError::AllowlistNotFound {
                allowlist_id: allowlist_id.to_string(),
            })?;
        let checkpoints = self.collect_checkpoint_chain(allowlist_id).await?;
        Ok(WorkspaceAllowlistDetail {
            open_change_count: summary.dirty_count,
            summary,
            baseline_revision_id: state.baseline_revision_id,
            head_revision_id: state.head_revision_id,
            checkpoints,
        })
    }
}

impl LibSqlBackend {
    /// Ensure the `libsql_vector_idx` on `memory_chunks.embedding` matches the
    /// configured embedding dimension.
    ///
    /// The V9 migration dropped the vector index (and changed `F32_BLOB(1536)`
    /// to `BLOB`) to support flexible dimensions. This method restores a
    /// properly-typed `F32_BLOB(N)` column and creates the vector index.
    ///
    /// Tracks the active dimension in `_migrations` version `0` — a reserved
    /// metadata row where `name` stores the dimension as a string. Version 0
    /// is never used by incremental migrations (which start at 9), so there
    /// is no collision. If the stored dimension matches, this is a no-op.
    ///
    /// **Precondition:** `run_migrations()` must have been called first so that
    /// the `_migrations` table exists. This is guaranteed when called from
    /// `Database::run_migrations()`, but callers using this directly must
    /// ensure migrations have run.
    pub async fn ensure_vector_index(&self, dimension: usize) -> Result<(), DatabaseError> {
        if dimension == 0 || dimension > 65536 {
            return Err(DatabaseError::Migration(format!(
                "ensure_vector_index: dimension {dimension} out of valid range (1..=65536)"
            )));
        }

        let conn = self.connect().await?;

        // Check current dimension from _migrations version=0 (reserved metadata row).
        // The block scope ensures `rows` is dropped before `conn.transaction()` —
        // holding a result set open would cause "database table is locked" errors.
        let current_dim = {
            let mut rows = conn
                .query("SELECT name FROM _migrations WHERE version = 0", ())
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!("Failed to check vector index metadata: {e}"))
                })?;

            rows.next().await.ok().flatten().and_then(|row| {
                row.get::<String>(0)
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
            })
        };

        if current_dim == Some(dimension) {
            tracing::debug!(
                dimension,
                "Vector index already matches configured dimension"
            );
            return Ok(());
        }

        tracing::info!(
            old_dimension = ?current_dim,
            new_dimension = dimension,
            "Rebuilding memory_chunks table for vector index"
        );

        let tx = conn.transaction().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "ensure_vector_index: failed to start transaction: {e}"
            ))
        })?;

        // 1. Drop FTS triggers that reference the old table
        tx.execute_batch(
            "DROP TRIGGER IF EXISTS memory_chunks_fts_insert;
             DROP TRIGGER IF EXISTS memory_chunks_fts_delete;
             DROP TRIGGER IF EXISTS memory_chunks_fts_update;",
        )
        .await
        .map_err(|e| DatabaseError::Migration(format!("Failed to drop FTS triggers: {e}")))?;

        // 2. Drop old vector index
        tx.execute_batch("DROP INDEX IF EXISTS idx_memory_chunks_embedding;")
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!("Failed to drop old vector index: {e}"))
            })?;

        // 3. Drop stale temp table (if a previous attempt crashed) and create fresh
        tx.execute_batch("DROP TABLE IF EXISTS memory_chunks_new;")
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!("Failed to drop stale memory_chunks_new: {e}"))
            })?;

        let create_sql = format!(
            "CREATE TABLE memory_chunks_new (
                _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                embedding F32_BLOB({dimension}),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                UNIQUE (document_id, chunk_index)
            )"
        );
        tx.execute_batch(&create_sql).await.map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to create memory_chunks_new with F32_BLOB({dimension}): {e}"
            ))
        })?;

        // 4. Copy data — embeddings with wrong byte length get NULLed
        //    (they will be re-embedded on next background pass).
        //    _rowid is explicitly preserved so the FTS5 content table
        //    (memory_chunks_fts, content_rowid='_rowid') stays in sync.
        let expected_bytes = dimension * 4;
        let copy_sql = format!(
            "INSERT INTO memory_chunks_new
                (_rowid, id, document_id, chunk_index, content, embedding, created_at)
             SELECT _rowid, id, document_id, chunk_index, content,
                    CASE WHEN length(embedding) = {expected_bytes} THEN embedding ELSE NULL END,
                    created_at
             FROM memory_chunks"
        );
        tx.execute_batch(&copy_sql).await.map_err(|e| {
            DatabaseError::Migration(format!("Failed to copy data to memory_chunks_new: {e}"))
        })?;

        // 5. Swap tables
        tx.execute_batch(
            "DROP TABLE memory_chunks;
             ALTER TABLE memory_chunks_new RENAME TO memory_chunks;",
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to swap memory_chunks tables: {e}"))
        })?;

        // 6. Recreate document index + vector index
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);
             CREATE INDEX IF NOT EXISTS idx_memory_chunks_embedding ON memory_chunks(libsql_vector_idx(embedding));",
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to create indexes: {e}"))
        })?;

        // 7. Recreate FTS triggers
        tx.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_insert AFTER INSERT ON memory_chunks BEGIN
                INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_delete AFTER DELETE ON memory_chunks BEGIN
                INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
                    VALUES ('delete', old._rowid, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_update AFTER UPDATE ON memory_chunks BEGIN
                INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
                    VALUES ('delete', old._rowid, old.content);
                INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
            END;",
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to recreate FTS triggers: {e}"))
        })?;

        // 8. Upsert dimension into _migrations(version=0)
        tx.execute(
            "INSERT INTO _migrations (version, name) VALUES (0, ?1)
             ON CONFLICT(version) DO UPDATE SET name = ?1,
                applied_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![dimension.to_string()],
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to record vector index dimension: {e}"))
        })?;

        tx.commit().await.map_err(|e| {
            DatabaseError::Migration(format!("ensure_vector_index: commit failed: {e}"))
        })?;

        tracing::info!(dimension, "Vector index created successfully");
        Ok(())
    }
}

#[async_trait]
impl WorkspaceStore for LibSqlBackend {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2 AND path = ?3
                "#,
                params![user_id, agent_id_str.as_deref(), path],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })? {
            Some(row) => Ok(row_to_memory_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: path.to_string(),
                user_id: user_id.to_string(),
            }),
        }
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents WHERE id = ?1
                "#,
                params![id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })? {
            Some(row) => Ok(row_to_memory_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: "unknown".to_string(),
                user_id: "unknown".to_string(),
            }),
        }
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        // Try get
        match self.get_document_by_path(user_id, agent_id, path).await {
            Ok(doc) => return Ok(doc),
            Err(WorkspaceError::DocumentNotFound { .. }) => {}
            Err(e) => return Err(e),
        }

        // Create
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let id = Uuid::new_v4();
        let agent_id_str = agent_id.map(|id| id.to_string());
        conn.execute(
            r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, ?3, ?4, '', '{}')
                ON CONFLICT (user_id, agent_id, path) DO NOTHING
                "#,
            params![id.to_string(), user_id, agent_id_str.as_deref(), path],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Insert failed: {}", e),
        })?;

        self.get_document_by_path(user_id, agent_id, path).await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE memory_documents SET content = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.to_string(), content, now],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Update failed: {}", e),
        })?;
        Ok(())
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        let doc = self.get_document_by_path(user_id, agent_id, path).await?;
        self.delete_chunks(doc.id).await?;

        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        conn.execute(
            "DELETE FROM memory_documents WHERE user_id = ?1 AND agent_id IS ?2 AND path = ?3",
            params![user_id, agent_id_str.as_deref(), path],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Delete failed: {}", e),
        })?;
        Ok(())
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let dir = if !directory.is_empty() && !directory.ends_with('/') {
            format!("{}/", directory)
        } else {
            directory.to_string()
        };

        let agent_id_str = agent_id.map(|id| id.to_string());
        let pattern = if dir.is_empty() {
            "%".to_string()
        } else {
            format!("{}%", dir)
        };

        let mut rows = conn
            .query(
                r#"
                SELECT path, updated_at, substr(content, 1, 200) as content_preview
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2
                  AND (?3 = '%' OR path LIKE ?3)
                ORDER BY path
                "#,
                params![user_id, agent_id_str.as_deref(), pattern],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List directory failed: {}", e),
            })?;

        let mut entries_map: HashMap<String, WorkspaceEntry> = HashMap::new();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            let full_path = get_text(&row, 0);
            let updated_at = get_opt_ts(&row, 1);
            let content_preview = get_opt_text(&row, 2);

            let relative = if dir.is_empty() {
                &full_path
            } else if let Some(stripped) = full_path.strip_prefix(&dir) {
                stripped
            } else {
                continue;
            };

            let child_name = if let Some(slash_pos) = relative.find('/') {
                &relative[..slash_pos]
            } else {
                relative
            };

            if child_name.is_empty() {
                continue;
            }

            let is_dir = relative.contains('/');
            let entry_path = if dir.is_empty() {
                child_name.to_string()
            } else {
                format!("{}{}", dir, child_name)
            };

            entries_map
                .entry(child_name.to_string())
                .and_modify(|e| {
                    if is_dir {
                        e.is_directory = true;
                        e.content_preview = None;
                    }
                    if let (Some(existing), Some(new)) = (&e.updated_at, &updated_at)
                        && new > existing
                    {
                        e.updated_at = Some(*new);
                    }
                })
                .or_insert(WorkspaceEntry {
                    path: entry_path,
                    is_directory: is_dir,
                    updated_at,
                    content_preview: if is_dir { None } else { content_preview },
                });
        }

        let mut entries: Vec<WorkspaceEntry> = entries_map.into_values().collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                "SELECT path FROM memory_documents WHERE user_id = ?1 AND agent_id IS ?2 ORDER BY path",
                params![user_id, agent_id_str.as_deref()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List paths failed: {}", e),
            })?;

        let mut paths = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            paths.push(get_text(&row, 0));
        }
        Ok(paths)
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2
                ORDER BY updated_at DESC
                "#,
                params![user_id, agent_id_str.as_deref()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        let mut docs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            docs.push(row_to_memory_document(&row));
        }
        Ok(docs)
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "DELETE FROM memory_chunks WHERE document_id = ?1",
            params![document_id.to_string()],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Delete failed: {}", e),
        })?;
        Ok(())
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: e.to_string(),
            })?;
        let id = Uuid::new_v4();
        // Note: embedding dimension is not validated here — the F32_BLOB(N)
        // column type created by ensure_vector_index() enforces byte length at
        // the libSQL level and will reject mismatched dimensions.
        let embedding_blob = embedding.map(|e| {
            let bytes: Vec<u8> = e.iter().flat_map(|f| f.to_le_bytes()).collect();
            bytes
        });

        conn.execute(
            r#"
                INSERT INTO memory_chunks (id, document_id, chunk_index, content, embedding)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
            params![
                id.to_string(),
                document_id.to_string(),
                chunk_index as i64,
                content,
                embedding_blob.map(libsql::Value::Blob),
            ],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Insert failed: {}", e),
        })?;
        Ok(id)
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::EmbeddingFailed {
                reason: e.to_string(),
            })?;
        let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

        conn.execute(
            "UPDATE memory_chunks SET embedding = ?2 WHERE id = ?1",
            params![chunk_id.to_string(), libsql::Value::Blob(bytes)],
        )
        .await
        .map_err(|e| WorkspaceError::EmbeddingFailed {
            reason: format!("Update failed: {}", e),
        })?;
        Ok(())
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT c.id, c.document_id, c.chunk_index, c.content, c.created_at
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = ?1 AND d.agent_id IS ?2
                  AND c.embedding IS NULL
                LIMIT ?3
                "#,
                params![user_id, agent_id_str.as_deref(), limit as i64],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        let mut chunks = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            chunks.push(MemoryChunk {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                document_id: get_text(&row, 1).parse().unwrap_or_default(),
                chunk_index: get_i64(&row, 2) as i32,
                content: get_text(&row, 3),
                embedding: None,
                created_at: get_ts(&row, 4),
            });
        }
        Ok(chunks)
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let pre_limit = config.pre_fusion_limit as i64;

        let fts_results = if config.use_fts {
            let mut rows = conn
                .query(
                    r#"
                    SELECT c.id, c.document_id, d.path, c.content
                    FROM memory_chunks_fts fts
                    JOIN memory_chunks c ON c._rowid = fts.rowid
                    JOIN memory_documents d ON d.id = c.document_id
                    WHERE d.user_id = ?1 AND d.agent_id IS ?2
                      AND memory_chunks_fts MATCH ?3
                    ORDER BY rank
                    LIMIT ?4
                    "#,
                    params![user_id, agent_id_str.as_deref(), query, pre_limit],
                )
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("FTS query failed: {}", e),
                })?;

            let mut results = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("FTS row fetch failed: {}", e),
                })?
            {
                results.push(RankedResult {
                    chunk_id: get_text(&row, 0).parse().unwrap_or_default(),
                    document_id: get_text(&row, 1).parse().unwrap_or_default(),
                    document_path: get_text(&row, 2),
                    content: get_text(&row, 3),
                    rank: results.len() as u32 + 1,
                });
            }
            results
        } else {
            Vec::new()
        };

        let vector_results = if let (true, Some(emb)) = (config.use_vector, embedding) {
            let vector_json = format!(
                "[{}]",
                emb.iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );

            // vector_top_k requires a libsql_vector_idx index created by
            // ensure_vector_index(). If the index is missing (embeddings not
            // configured or dimension mismatch), fall back to FTS-only.
            match conn
                .query(
                    r#"
                    SELECT c.id, c.document_id, d.path, c.content
                    FROM vector_top_k('idx_memory_chunks_embedding', vector(?1), ?2) AS top_k
                    JOIN memory_chunks c ON c._rowid = top_k.id
                    JOIN memory_documents d ON d.id = c.document_id
                    WHERE d.user_id = ?3 AND d.agent_id IS ?4
                    "#,
                    params![vector_json, pre_limit, user_id, agent_id_str.as_deref()],
                )
                .await
            {
                Ok(mut rows) => {
                    let mut results = Vec::new();
                    while let Some(row) =
                        rows.next()
                            .await
                            .map_err(|e| WorkspaceError::SearchFailed {
                                reason: format!("Vector row fetch failed: {}", e),
                            })?
                    {
                        results.push(RankedResult {
                            chunk_id: get_text(&row, 0).parse().unwrap_or_default(),
                            document_id: get_text(&row, 1).parse().unwrap_or_default(),
                            document_path: get_text(&row, 2),
                            content: get_text(&row, 3),
                            rank: results.len() as u32 + 1,
                        });
                    }
                    results
                }
                Err(e) => {
                    tracing::warn!(
                        "Vector index query failed (ensure_vector_index may not have run \
                         or dimension mismatch), falling back to FTS-only: {e}"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        if embedding.is_some() && !config.use_vector {
            tracing::warn!(
                "Embedding provided but vector search is disabled in config; using FTS-only results"
            );
        }

        Ok(fuse_results(fts_results, vector_results, config))
    }

    async fn list_workspace_tree(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        uri: &str,
    ) -> Result<Vec<WorkspaceTreeEntry>, WorkspaceError> {
        let parsed = WorkspaceUri::parse(uri)?.ok_or_else(|| WorkspaceError::InvalidDocType {
            doc_type: uri.to_string(),
        })?;
        match parsed {
            WorkspaceUri::Root => {
                let mut entries: Vec<WorkspaceTreeEntry> = self
                    .list_directory(user_id, agent_id, "")
                    .await?
                    .into_iter()
                    .map(|entry| WorkspaceTreeEntry {
                        name: entry.name().to_string(),
                        path: entry.path.clone(),
                        uri: entry.path.clone(),
                        is_directory: entry.is_directory,
                        kind: if entry.is_directory {
                            WorkspaceTreeEntryKind::MemoryDirectory
                        } else {
                            WorkspaceTreeEntryKind::MemoryFile
                        },
                        status: None,
                        updated_at: entry.updated_at,
                        content_preview: entry.content_preview,
                        bypass_write: None,
                        dirty_count: 0,
                        conflict_count: 0,
                        pending_delete_count: 0,
                    })
                    .collect();

                entries.extend(
                    self.list_allowlist_summaries_internal(user_id)
                        .await?
                        .into_iter()
                        .map(|summary| WorkspaceTreeEntry {
                            name: summary.allowlist.display_name.clone(),
                            path: summary.allowlist.id.to_string(),
                            uri: WorkspaceUri::allowlist_uri(summary.allowlist.id, None),
                            is_directory: true,
                            kind: WorkspaceTreeEntryKind::Allowlist,
                            status: None,
                            updated_at: Some(summary.allowlist.updated_at),
                            content_preview: None,
                            bypass_write: Some(summary.allowlist.bypass_write),
                            dirty_count: summary.dirty_count,
                            conflict_count: summary.conflict_count,
                            pending_delete_count: summary.pending_delete_count,
                        }),
                );

                entries.sort_by(
                    |left, right| match (left.is_directory, right.is_directory) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => left.name.cmp(&right.name),
                    },
                );

                Ok(entries)
            }
            WorkspaceUri::AllowlistRoot(allowlist_id) => {
                let has_allowlist_state = self
                    .load_allowlist_state_record(allowlist_id)
                    .await?
                    .is_some();
                let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
                let prefix = String::new();
                let dir_path = Path::new(&allowlist.source_root).join(&prefix);
                let mut entries_map: BTreeMap<String, WorkspaceTreeEntry> = BTreeMap::new();

                if dir_path.is_dir() {
                    let read_dir =
                        std::fs::read_dir(&dir_path).map_err(|e| WorkspaceError::IoError {
                            reason: format!(
                                "failed to list allowlist directory {}: {e}",
                                dir_path.display()
                            ),
                        })?;
                    for entry in read_dir {
                        let entry = entry.map_err(|e| WorkspaceError::IoError {
                            reason: format!("failed to read allowlist dir entry: {e}"),
                        })?;
                        let name = entry.file_name().to_string_lossy().to_string();
                        let rel = normalize_allowlist_path(&name)?;
                        let is_directory = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                        entries_map.insert(
                            name.clone(),
                            WorkspaceTreeEntry {
                                name,
                                path: rel.clone(),
                                uri: WorkspaceUri::allowlist_uri(allowlist_id, Some(&rel)),
                                is_directory,
                                kind: if is_directory {
                                    WorkspaceTreeEntryKind::AllowlistedDirectory
                                } else {
                                    WorkspaceTreeEntryKind::AllowlistedFile
                                },
                                status: None,
                                updated_at: None,
                                content_preview: None,
                                bypass_write: Some(allowlist.bypass_write),
                                dirty_count: 0,
                                conflict_count: 0,
                                pending_delete_count: 0,
                            },
                        );
                    }
                }

                if has_allowlist_state {
                    for record in self
                        .list_allowlist_file_records(allowlist_id, Some(&prefix))
                        .await?
                    {
                        let relative = record.path.clone();
                        let child_name = relative.split('/').next().unwrap_or("").to_string();
                        if child_name.is_empty() {
                            continue;
                        }
                        let child_path = child_name.clone();
                        let is_directory = relative.contains('/');
                        let entry =
                            entries_map
                                .entry(child_name.clone())
                                .or_insert(WorkspaceTreeEntry {
                                    name: child_name.clone(),
                                    path: child_path.clone(),
                                    uri: WorkspaceUri::allowlist_uri(
                                        allowlist_id,
                                        Some(&child_path),
                                    ),
                                    is_directory,
                                    kind: if is_directory {
                                        WorkspaceTreeEntryKind::AllowlistedDirectory
                                    } else {
                                        WorkspaceTreeEntryKind::AllowlistedFile
                                    },
                                    status: None,
                                    updated_at: Some(record.updated_at),
                                    content_preview: None,
                                    bypass_write: Some(allowlist.bypass_write),
                                    dirty_count: 0,
                                    conflict_count: 0,
                                    pending_delete_count: 0,
                                });
                        if is_directory {
                            entry.is_directory = true;
                            entry.kind = WorkspaceTreeEntryKind::AllowlistedDirectory;
                            entry.dirty_count +=
                                usize::from(record.status != AllowlistedFileStatus::Clean);
                            entry.conflict_count +=
                                usize::from(record.status == AllowlistedFileStatus::Conflicted);
                            entry.pending_delete_count +=
                                usize::from(record.status == AllowlistedFileStatus::Deleted);
                        } else {
                            entry.status = Some(record.status);
                            entry.updated_at = Some(record.updated_at);
                        }
                    }
                }

                Ok(entries_map.into_values().collect())
            }
            WorkspaceUri::AllowlistPath(allowlist_id, prefix) => {
                let has_allowlist_state = self
                    .load_allowlist_state_record(allowlist_id)
                    .await?
                    .is_some();
                let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
                let dir_path = Path::new(&allowlist.source_root).join(&prefix);
                let mut entries_map: BTreeMap<String, WorkspaceTreeEntry> = BTreeMap::new();

                if dir_path.is_dir() {
                    let read_dir =
                        std::fs::read_dir(&dir_path).map_err(|e| WorkspaceError::IoError {
                            reason: format!(
                                "failed to list allowlist directory {}: {e}",
                                dir_path.display()
                            ),
                        })?;
                    for entry in read_dir {
                        let entry = entry.map_err(|e| WorkspaceError::IoError {
                            reason: format!("failed to read allowlist dir entry: {e}"),
                        })?;
                        let name = entry.file_name().to_string_lossy().to_string();
                        let rel = normalize_allowlist_path(&if prefix.is_empty() {
                            name.clone()
                        } else {
                            format!("{prefix}/{name}")
                        })?;
                        let is_directory = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                        entries_map.insert(
                            name.clone(),
                            WorkspaceTreeEntry {
                                name,
                                path: rel.clone(),
                                uri: WorkspaceUri::allowlist_uri(allowlist_id, Some(&rel)),
                                is_directory,
                                kind: if is_directory {
                                    WorkspaceTreeEntryKind::AllowlistedDirectory
                                } else {
                                    WorkspaceTreeEntryKind::AllowlistedFile
                                },
                                status: None,
                                updated_at: None,
                                content_preview: None,
                                bypass_write: Some(allowlist.bypass_write),
                                dirty_count: 0,
                                conflict_count: 0,
                                pending_delete_count: 0,
                            },
                        );
                    }
                }

                if has_allowlist_state {
                    for record in self
                        .list_allowlist_file_records(allowlist_id, Some(&prefix))
                        .await?
                    {
                        let relative = if prefix.is_empty() {
                            record.path.clone()
                        } else if let Some(rest) = record.path.strip_prefix(&(prefix.clone() + "/"))
                        {
                            rest.to_string()
                        } else {
                            continue;
                        };
                        let child_name = relative.split('/').next().unwrap_or("").to_string();
                        if child_name.is_empty() {
                            continue;
                        }
                        let child_path = if prefix.is_empty() {
                            child_name.clone()
                        } else {
                            format!("{prefix}/{child_name}")
                        };
                        let is_directory = relative.contains('/');
                        let entry =
                            entries_map
                                .entry(child_name.clone())
                                .or_insert(WorkspaceTreeEntry {
                                    name: child_name.clone(),
                                    path: child_path.clone(),
                                    uri: WorkspaceUri::allowlist_uri(
                                        allowlist_id,
                                        Some(&child_path),
                                    ),
                                    is_directory,
                                    kind: if is_directory {
                                        WorkspaceTreeEntryKind::AllowlistedDirectory
                                    } else {
                                        WorkspaceTreeEntryKind::AllowlistedFile
                                    },
                                    status: None,
                                    updated_at: Some(record.updated_at),
                                    content_preview: None,
                                    bypass_write: Some(allowlist.bypass_write),
                                    dirty_count: 0,
                                    conflict_count: 0,
                                    pending_delete_count: 0,
                                });
                        if is_directory {
                            entry.is_directory = true;
                            entry.kind = WorkspaceTreeEntryKind::AllowlistedDirectory;
                            entry.dirty_count +=
                                usize::from(record.status != AllowlistedFileStatus::Clean);
                            entry.conflict_count +=
                                usize::from(record.status == AllowlistedFileStatus::Conflicted);
                            entry.pending_delete_count +=
                                usize::from(record.status == AllowlistedFileStatus::Deleted);
                        } else {
                            entry.status = Some(record.status);
                            entry.updated_at = Some(record.updated_at);
                        }
                    }
                }

                Ok(entries_map.into_values().collect())
            }
        }
    }

    async fn create_workspace_allowlist(
        &self,
        request: &CreateAllowlistRequest,
    ) -> Result<WorkspaceAllowlistSummary, WorkspaceError> {
        let source_root =
            std::fs::canonicalize(&request.source_root).map_err(|e| WorkspaceError::IoError {
                reason: format!("allowlist source is not accessible: {e}"),
            })?;
        if !source_root.is_dir() {
            return Err(WorkspaceError::IoError {
                reason: "allowlist source must be a directory".to_string(),
            });
        }
        let allowlist_id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "INSERT INTO workspace_allowlists (id, user_id, display_name, source_root, bypass_read, bypass_write, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?6)",
            params![
                allowlist_id.to_string(),
                request.user_id.clone(),
                request.display_name.clone(),
                source_root.display().to_string(),
                if request.bypass_write { 1 } else { 0 },
                now
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("allowlist insert failed: {e}"),
        })?;
        self.ensure_allowlist_initialized(&request.user_id, allowlist_id)
            .await?;
        self.list_allowlist_summaries_internal(&request.user_id)
            .await?
            .into_iter()
            .find(|summary| summary.allowlist.id == allowlist_id)
            .ok_or_else(|| WorkspaceError::AllowlistNotFound {
                allowlist_id: allowlist_id.to_string(),
            })
    }

    async fn list_workspace_allowlists(
        &self,
        user_id: &str,
    ) -> Result<Vec<WorkspaceAllowlistSummary>, WorkspaceError> {
        let summaries = self.list_allowlist_summaries_internal(user_id).await?;
        for summary in &summaries {
            self.ensure_allowlist_initialized(user_id, summary.allowlist.id)
                .await?;
        }
        self.list_allowlist_summaries_internal(user_id).await
    }

    async fn get_workspace_allowlist(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        self.reconcile_allowlist(
            user_id,
            allowlist_id,
            WorkspaceAllowlistRevisionKind::ManualRefresh,
            WorkspaceAllowlistRevisionSource::External,
            Some("get_allowlist".to_string()),
            None,
            "system",
        )
        .await?;
        self.build_allowlist_detail_internal(user_id, allowlist_id)
            .await
    }

    async fn read_workspace_allowlist_file(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        path: &str,
    ) -> Result<WorkspaceAllowlistFileView, WorkspaceError> {
        let normalized = normalize_allowlist_path(path)?;
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        let disk_path = Path::new(&allowlist.source_root).join(&normalized);
        let bytes = Self::read_disk_bytes(&disk_path).await?.ok_or_else(|| {
            WorkspaceError::AllowlistPathNotFound {
                allowlist_id: allowlist_id.to_string(),
                path: normalized.clone(),
            }
        })?;
        let record = self
            .load_allowlist_file_record(allowlist_id, &normalized)
            .await?;
        let status = record
            .as_ref()
            .map(|value| value.status)
            .unwrap_or(AllowlistedFileStatus::Clean);
        let is_binary = classify_binary(&bytes);
        Ok(WorkspaceAllowlistFileView {
            allowlist_id,
            path: normalized.clone(),
            uri: WorkspaceUri::allowlist_uri(allowlist_id, Some(&normalized)),
            disk_path: disk_path.display().to_string(),
            status,
            is_binary,
            content: bytes_to_text(&bytes),
            updated_at: Utc::now(),
        })
    }

    async fn write_workspace_allowlist_file(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        path: &str,
        content: &[u8],
    ) -> Result<WorkspaceAllowlistFileView, WorkspaceError> {
        let normalized = normalize_allowlist_path(path)?;
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        let disk_path = Path::new(&allowlist.source_root).join(&normalized);
        if let Some(parent) = disk_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| WorkspaceError::IoError {
                    reason: format!("failed to create {}: {e}", parent.display()),
                })?;
        }
        tokio::fs::write(&disk_path, content)
            .await
            .map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to write {}: {e}", disk_path.display()),
            })?;
        self.reconcile_allowlist(
            user_id,
            allowlist_id,
            WorkspaceAllowlistRevisionKind::ToolWrite,
            WorkspaceAllowlistRevisionSource::WorkspaceTool,
            Some(normalized.clone()),
            Some(format!("updated {}", normalized)),
            "workspace_write",
        )
        .await?;
        self.read_workspace_allowlist_file(user_id, allowlist_id, &normalized)
            .await
    }

    async fn delete_workspace_allowlist_file(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        path: &str,
    ) -> Result<WorkspaceAllowlistFileView, WorkspaceError> {
        let normalized = normalize_allowlist_path(path)?;
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        let disk_path = Path::new(&allowlist.source_root).join(&normalized);
        let metadata = tokio::fs::metadata(&disk_path).await.map_err(|_| {
            WorkspaceError::AllowlistPathNotFound {
                allowlist_id: allowlist_id.to_string(),
                path: normalized.clone(),
            }
        })?;
        if metadata.is_dir() {
            return Err(WorkspaceError::IoError {
                reason: format!(
                    "workspace_delete only removes files; use workspace_delete_tree for {}",
                    normalized
                ),
            });
        }
        tokio::fs::remove_file(&disk_path)
            .await
            .map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to delete {}: {e}", disk_path.display()),
            })?;
        self.reconcile_allowlist(
            user_id,
            allowlist_id,
            WorkspaceAllowlistRevisionKind::ToolDelete,
            WorkspaceAllowlistRevisionSource::WorkspaceTool,
            Some(normalized.clone()),
            Some(format!("deleted {}", normalized)),
            "workspace_delete",
        )
        .await?;
        let record = self
            .load_allowlist_file_record(allowlist_id, &normalized)
            .await?;
        Ok(WorkspaceAllowlistFileView {
            allowlist_id,
            path: normalized.clone(),
            uri: WorkspaceUri::allowlist_uri(allowlist_id, Some(&normalized)),
            disk_path: disk_path.display().to_string(),
            status: record
                .as_ref()
                .map(|value| value.status)
                .unwrap_or(AllowlistedFileStatus::Clean),
            is_binary: record
                .as_ref()
                .map(|value| value.is_binary)
                .unwrap_or(false),
            content: None,
            updated_at: Utc::now(),
        })
    }

    async fn diff_workspace_allowlist(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        scope_path: Option<&str>,
    ) -> Result<WorkspaceAllowlistDiff, WorkspaceError> {
        self.diff_workspace_allowlist_between(&WorkspaceAllowlistDiffRequest {
            user_id: user_id.to_string(),
            allowlist_id,
            scope_path: scope_path.map(ToString::to_string),
            from: Some("baseline".to_string()),
            to: Some("head".to_string()),
            include_content: true,
            max_files: None,
        })
        .await
    }

    async fn create_workspace_checkpoint(
        &self,
        request: &CreateCheckpointRequest,
    ) -> Result<WorkspaceAllowlistCheckpoint, WorkspaceError> {
        let state = self
            .reconcile_allowlist(
                &request.user_id,
                request.allowlist_id,
                WorkspaceAllowlistRevisionKind::ManualRefresh,
                WorkspaceAllowlistRevisionSource::External,
                Some("checkpoint_create".to_string()),
                None,
                &request.created_by,
            )
            .await?;
        let revision_id = match request.revision_id {
            Some(revision_id) => revision_id,
            None => state
                .head_revision_id
                .ok_or_else(|| WorkspaceError::AllowlistConflict {
                    path: request.allowlist_id.to_string(),
                    reason: "cannot checkpoint allowlist without a head revision".to_string(),
                })?,
        };
        self.create_checkpoint_record(
            request.allowlist_id,
            revision_id,
            request.label.clone(),
            request.summary.clone(),
            request.created_by.clone(),
            request.is_auto,
        )
        .await
    }

    async fn list_workspace_checkpoints(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        limit: Option<usize>,
    ) -> Result<Vec<WorkspaceAllowlistCheckpoint>, WorkspaceError> {
        self.ensure_allowlist_initialized(user_id, allowlist_id)
            .await?;
        let mut checkpoints = self.collect_checkpoint_chain(allowlist_id).await?;
        if let Some(limit) = limit {
            checkpoints.truncate(limit);
        }
        Ok(checkpoints)
    }

    async fn list_workspace_allowlist_history(
        &self,
        request: &WorkspaceAllowlistHistoryRequest,
    ) -> Result<WorkspaceAllowlistHistory, WorkspaceError> {
        let scope = request
            .scope_path
            .as_deref()
            .map(normalize_allowlist_path)
            .transpose()?;
        let state = self
            .reconcile_allowlist(
                &request.user_id,
                request.allowlist_id,
                WorkspaceAllowlistRevisionKind::ManualRefresh,
                WorkspaceAllowlistRevisionSource::External,
                Some("history".to_string()),
                None,
                "system",
            )
            .await?;
        let mut revisions = Vec::new();
        for revision in self
            .list_revision_refs(request.allowlist_id, Some(request.limit), request.since)
            .await?
        {
            let changed_files = self
                .list_revision_changed_files(revision.id, scope.as_deref())
                .await?;
            if scope.is_some() && changed_files.is_empty() {
                continue;
            }
            revisions.push(WorkspaceAllowlistRevision {
                id: revision.id,
                allowlist_id: request.allowlist_id,
                parent_revision_id: revision.parent_revision_id,
                kind: revision.kind,
                source: revision.source,
                trigger: revision.trigger,
                summary: revision.summary,
                created_by: revision.created_by,
                created_at: revision.created_at,
                changed_files,
            });
        }

        let checkpoints = if request.include_checkpoints {
            self.collect_checkpoint_chain(request.allowlist_id)
                .await?
                .into_iter()
                .filter(|checkpoint| {
                    scope.is_none()
                        || checkpoint
                            .changed_files
                            .iter()
                            .any(|path| scope_matches(path, scope.as_deref()))
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(WorkspaceAllowlistHistory {
            allowlist_id: request.allowlist_id,
            baseline_revision_id: state.baseline_revision_id,
            head_revision_id: state.head_revision_id,
            revisions,
            checkpoints,
        })
    }

    async fn diff_workspace_allowlist_between(
        &self,
        request: &WorkspaceAllowlistDiffRequest,
    ) -> Result<WorkspaceAllowlistDiff, WorkspaceError> {
        let scope = request
            .scope_path
            .as_deref()
            .map(normalize_allowlist_path)
            .transpose()?;
        let state = self
            .reconcile_allowlist(
                &request.user_id,
                request.allowlist_id,
                WorkspaceAllowlistRevisionKind::ManualRefresh,
                WorkspaceAllowlistRevisionSource::External,
                Some("workspace_diff".to_string()),
                None,
                "system",
            )
            .await?;
        let from_revision_id = self
            .resolve_revision_target(
                request.allowlist_id,
                &state,
                request.from.as_deref().unwrap_or("baseline"),
            )
            .await?;
        let to_revision_id = self
            .resolve_revision_target(
                request.allowlist_id,
                &state,
                request.to.as_deref().unwrap_or("head"),
            )
            .await?;
        let from_manifest = self.load_manifest_entries(from_revision_id).await?;
        let to_manifest = self.load_manifest_entries(to_revision_id).await?;
        let allowlist = self
            .fetch_allowlist(&request.user_id, request.allowlist_id)
            .await?;
        let mut changes = collect_manifest_changes(&from_manifest, &to_manifest, scope.as_deref());
        if let Some(max_files) = request.max_files {
            changes.truncate(max_files);
        }

        let mut entries = Vec::new();
        for change in changes {
            let base_snapshot = match change.before.as_ref() {
                Some(entry) => Some(self.read_snapshot_required(entry.snapshot_id).await?),
                None => None,
            };
            let working_snapshot = match change.after.as_ref() {
                Some(entry) => Some(self.read_snapshot_required(entry.snapshot_id).await?),
                None => None,
            };
            let base_content = if request.include_content {
                base_snapshot
                    .as_ref()
                    .and_then(|snapshot| bytes_to_text(&snapshot.content))
            } else {
                None
            };
            let working_content = if request.include_content {
                working_snapshot
                    .as_ref()
                    .and_then(|snapshot| bytes_to_text(&snapshot.content))
            } else {
                None
            };
            let remote_content =
                if to_revision_id == state.head_revision_id.unwrap_or(to_revision_id) {
                    let disk_path = Path::new(&allowlist.source_root).join(&change.path);
                    Self::read_disk_bytes(&disk_path)
                        .await?
                        .and_then(|bytes| bytes_to_text(&bytes))
                } else {
                    None
                };
            let diff_text = if request.include_content && !change.is_binary {
                render_text_diff(
                    &change.path,
                    base_content.as_deref(),
                    working_content.as_deref(),
                )
            } else {
                None
            };
            entries.push(AllowlistedFileDiff {
                path: change.path.clone(),
                uri: WorkspaceUri::allowlist_uri(request.allowlist_id, Some(&change.path)),
                status: change.status,
                change_kind: change.change_kind,
                is_binary: change.is_binary,
                base_content,
                working_content,
                remote_content,
                diff_text,
                conflict_reason: None,
            });
        }

        Ok(WorkspaceAllowlistDiff {
            allowlist_id: request.allowlist_id,
            from_revision_id: Some(from_revision_id),
            to_revision_id: Some(to_revision_id),
            entries,
        })
    }

    async fn keep_workspace_allowlist(
        &self,
        request: &AllowlistActionRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        let state = self
            .reconcile_allowlist(
                &request.user_id,
                request.allowlist_id,
                WorkspaceAllowlistRevisionKind::ManualRefresh,
                WorkspaceAllowlistRevisionSource::External,
                Some("keep_allowlist".to_string()),
                None,
                "system",
            )
            .await?;
        let head_revision_id =
            state
                .head_revision_id
                .ok_or_else(|| WorkspaceError::AllowlistConflict {
                    path: request.allowlist_id.to_string(),
                    reason: "cannot keep allowlist without a head revision".to_string(),
                })?;
        let head_manifest = self.load_manifest_entries(head_revision_id).await?;
        let accept_revision_id = self
            .create_revision_from_manifest(
                request.allowlist_id,
                Some(head_revision_id),
                &head_manifest,
                &head_manifest,
                WorkspaceAllowlistRevisionKind::Accept,
                WorkspaceAllowlistRevisionSource::System,
                Some("keep_allowlist".to_string()),
                Some("accepted current workspace tree as baseline".to_string()),
                "system",
            )
            .await?;
        self.save_allowlist_state_record(
            request.allowlist_id,
            Some(accept_revision_id),
            Some(accept_revision_id),
        )
        .await?;
        self.sync_allowlist_live_cache(
            request.allowlist_id,
            Some(accept_revision_id),
            Some(accept_revision_id),
        )
        .await?;
        self.touch_allowlist_updated_at(request.allowlist_id)
            .await?;
        self.build_allowlist_detail_internal(&request.user_id, request.allowlist_id)
            .await
    }

    async fn revert_workspace_allowlist(
        &self,
        request: &AllowlistActionRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        let target = request
            .checkpoint_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "baseline".to_string());
        self.restore_workspace_allowlist(&WorkspaceAllowlistRestoreRequest {
            user_id: request.user_id.clone(),
            allowlist_id: request.allowlist_id,
            scope_path: request.scope_path.clone(),
            target,
            set_as_baseline: request.set_as_baseline,
            dry_run: false,
            create_checkpoint_before_restore: true,
            created_by: "system".to_string(),
        })
        .await
    }

    async fn resolve_workspace_allowlist_conflict(
        &self,
        request: &ConflictResolutionRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        let normalized = normalize_allowlist_path(&request.path)?;
        let allowlist = self
            .fetch_allowlist(&request.user_id, request.allowlist_id)
            .await?;
        let disk_path = Path::new(&allowlist.source_root).join(&normalized);
        match request.resolution.as_str() {
            "keep_disk" => {
                self.refresh_workspace_allowlist(&request.user_id, request.allowlist_id, None)
                    .await?;
            }
            "keep_workspace" => {
                self.refresh_workspace_allowlist(&request.user_id, request.allowlist_id, None)
                    .await?;
            }
            "write_copy" => {
                let copy_path = request.renamed_copy_path.clone().ok_or_else(|| {
                    WorkspaceError::AllowlistConflict {
                        path: normalized.clone(),
                        reason: "renamed_copy_path is required".to_string(),
                    }
                })?;
                let copy_path = normalize_allowlist_path(&copy_path)?;
                let bytes = Self::read_disk_bytes(&disk_path).await?.ok_or_else(|| {
                    WorkspaceError::AllowlistPathNotFound {
                        allowlist_id: request.allowlist_id.to_string(),
                        path: normalized.clone(),
                    }
                })?;
                let disk_path = Path::new(&allowlist.source_root).join(&copy_path);
                if let Some(parent) = disk_path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        WorkspaceError::IoError {
                            reason: format!("failed to create {}: {e}", parent.display()),
                        }
                    })?;
                }
                tokio::fs::write(&disk_path, bytes)
                    .await
                    .map_err(|e| WorkspaceError::IoError {
                        reason: format!("failed to write copy {}: {e}", disk_path.display()),
                    })?;
                self.reconcile_allowlist(
                    &request.user_id,
                    request.allowlist_id,
                    WorkspaceAllowlistRevisionKind::ToolWrite,
                    WorkspaceAllowlistRevisionSource::WorkspaceTool,
                    Some(copy_path),
                    Some("wrote copy during conflict resolution".to_string()),
                    "workspace_conflict",
                )
                .await?;
            }
            "manual_merge" => {
                let merged_content = request.merged_content.clone().ok_or_else(|| {
                    WorkspaceError::AllowlistConflict {
                        path: normalized.clone(),
                        reason: "merged_content is required".to_string(),
                    }
                })?;
                if let Some(parent) = disk_path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        WorkspaceError::IoError {
                            reason: format!("failed to create {}: {e}", parent.display()),
                        }
                    })?;
                }
                tokio::fs::write(&disk_path, merged_content.as_bytes())
                    .await
                    .map_err(|e| WorkspaceError::IoError {
                        reason: format!("failed to write merged file {}: {e}", disk_path.display()),
                    })?;
                self.reconcile_allowlist(
                    &request.user_id,
                    request.allowlist_id,
                    WorkspaceAllowlistRevisionKind::ToolPatch,
                    WorkspaceAllowlistRevisionSource::WorkspaceTool,
                    Some(normalized),
                    Some("manual merge resolution".to_string()),
                    "workspace_conflict",
                )
                .await?;
            }
            other => {
                return Err(WorkspaceError::AllowlistConflict {
                    path: request.path.clone(),
                    reason: format!("unknown resolution '{other}'"),
                });
            }
        }
        self.build_allowlist_detail_internal(&request.user_id, request.allowlist_id)
            .await
    }

    async fn move_workspace_allowlist_file(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        source_path: &str,
        destination_path: &str,
        overwrite: bool,
    ) -> Result<WorkspaceAllowlistFileView, WorkspaceError> {
        let source_path = normalize_allowlist_path(source_path)?;
        let destination_path = normalize_allowlist_path(destination_path)?;
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        let source_disk_path = Path::new(&allowlist.source_root).join(&source_path);
        let destination_disk_path = Path::new(&allowlist.source_root).join(&destination_path);

        let source_metadata = tokio::fs::metadata(&source_disk_path).await.map_err(|_| {
            WorkspaceError::AllowlistPathNotFound {
                allowlist_id: allowlist_id.to_string(),
                path: source_path.clone(),
            }
        })?;
        if source_metadata.is_dir() {
            return Err(WorkspaceError::IoError {
                reason: format!("workspace_move only supports files: {}", source_path),
            });
        }
        if !overwrite
            && tokio::fs::try_exists(&destination_disk_path)
                .await
                .map_err(|e| WorkspaceError::IoError {
                    reason: format!(
                        "failed to check destination {}: {e}",
                        destination_disk_path.display()
                    ),
                })?
        {
            return Err(WorkspaceError::IoError {
                reason: format!("destination already exists: {}", destination_path),
            });
        }
        if let Some(parent) = destination_disk_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| WorkspaceError::IoError {
                    reason: format!("failed to create {}: {e}", parent.display()),
                })?;
        }
        tokio::fs::rename(&source_disk_path, &destination_disk_path)
            .await
            .map_err(|e| WorkspaceError::IoError {
                reason: format!(
                    "failed to move {} -> {}: {e}",
                    source_disk_path.display(),
                    destination_disk_path.display()
                ),
            })?;
        self.reconcile_allowlist(
            user_id,
            allowlist_id,
            WorkspaceAllowlistRevisionKind::ToolMove,
            WorkspaceAllowlistRevisionSource::WorkspaceTool,
            Some(format!("{source_path} -> {destination_path}")),
            Some(format!("moved {} to {}", source_path, destination_path)),
            "workspace_move",
        )
        .await?;
        self.read_workspace_allowlist_file(user_id, allowlist_id, &destination_path)
            .await
    }

    async fn delete_workspace_allowlist_tree(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        path: &str,
        missing_ok: bool,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        let normalized = normalize_allowlist_path(path)?;
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        let disk_path = Path::new(&allowlist.source_root).join(&normalized);
        match tokio::fs::metadata(&disk_path).await {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(WorkspaceError::IoError {
                        reason: format!(
                            "workspace_delete_tree requires a directory: {}",
                            normalized
                        ),
                    });
                }
            }
            Err(_) if missing_ok => {
                return self
                    .build_allowlist_detail_internal(user_id, allowlist_id)
                    .await;
            }
            Err(_) => {
                return Err(WorkspaceError::AllowlistPathNotFound {
                    allowlist_id: allowlist_id.to_string(),
                    path: normalized,
                });
            }
        }
        tokio::fs::remove_dir_all(&disk_path)
            .await
            .map_err(|e| WorkspaceError::IoError {
                reason: format!("failed to delete directory {}: {e}", disk_path.display()),
            })?;
        self.reconcile_allowlist(
            user_id,
            allowlist_id,
            WorkspaceAllowlistRevisionKind::ToolDelete,
            WorkspaceAllowlistRevisionSource::WorkspaceTool,
            Some(path.to_string()),
            Some(format!("deleted directory tree {}", path)),
            "workspace_delete_tree",
        )
        .await?;
        self.build_allowlist_detail_internal(user_id, allowlist_id)
            .await
    }

    async fn restore_workspace_allowlist(
        &self,
        request: &WorkspaceAllowlistRestoreRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        let scope = request
            .scope_path
            .as_deref()
            .map(normalize_allowlist_path)
            .transpose()?;
        let mut state = self
            .reconcile_allowlist(
                &request.user_id,
                request.allowlist_id,
                WorkspaceAllowlistRevisionKind::ManualRefresh,
                WorkspaceAllowlistRevisionSource::External,
                Some("restore_prepare".to_string()),
                None,
                &request.created_by,
            )
            .await?;
        let head_revision_id =
            state
                .head_revision_id
                .ok_or_else(|| WorkspaceError::AllowlistConflict {
                    path: request.allowlist_id.to_string(),
                    reason: "cannot restore allowlist without a head revision".to_string(),
                })?;
        let target_revision_id = self
            .resolve_revision_target(request.allowlist_id, &state, &request.target)
            .await?;
        if request.dry_run {
            return self
                .build_allowlist_detail_internal(&request.user_id, request.allowlist_id)
                .await;
        }
        if request.create_checkpoint_before_restore {
            let label = format!("auto-pre-restore-{}", Utc::now().format("%Y%m%d%H%M%S"));
            self.create_checkpoint_record(
                request.allowlist_id,
                head_revision_id,
                Some(label),
                Some(format!(
                    "automatic checkpoint before restoring {}",
                    request.target
                )),
                request.created_by.clone(),
                true,
            )
            .await?;
        }

        let allowlist = self
            .fetch_allowlist(&request.user_id, request.allowlist_id)
            .await?;
        let current_manifest = self.load_manifest_entries(head_revision_id).await?;
        let target_manifest = self.load_manifest_entries(target_revision_id).await?;
        let desired_manifest = if let Some(scope) = scope.as_deref() {
            let mut desired = current_manifest.clone();
            desired.retain(|path, _| !scope_matches(path, Some(scope)));
            for (path, entry) in &target_manifest {
                if scope_matches(path, Some(scope)) {
                    desired.insert(path.clone(), entry.clone());
                }
            }
            desired
        } else {
            target_manifest.clone()
        };
        let changes = collect_manifest_changes(&current_manifest, &desired_manifest, None);
        let mut recovery_bundle = Vec::new();
        for change in &changes {
            let disk_path = Path::new(&allowlist.source_root).join(&change.path);
            recovery_bundle.push((disk_path.clone(), Self::read_disk_bytes(&disk_path).await?));
        }

        let apply_result = async {
            for change in &changes {
                let disk_path = Path::new(&allowlist.source_root).join(&change.path);
                match &change.after {
                    Some(entry) => {
                        let snapshot = self.read_snapshot_required(entry.snapshot_id).await?;
                        if let Some(parent) = disk_path.parent() {
                            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                                WorkspaceError::IoError {
                                    reason: format!("failed to create {}: {e}", parent.display()),
                                }
                            })?;
                        }
                        tokio::fs::write(&disk_path, snapshot.content)
                            .await
                            .map_err(|e| WorkspaceError::IoError {
                                reason: format!("failed to restore {}: {e}", disk_path.display()),
                            })?;
                    }
                    None => {
                        if tokio::fs::try_exists(&disk_path).await.map_err(|e| {
                            WorkspaceError::IoError {
                                reason: format!("failed to check {}: {e}", disk_path.display()),
                            }
                        })? {
                            tokio::fs::remove_file(&disk_path).await.map_err(|e| {
                                WorkspaceError::IoError {
                                    reason: format!(
                                        "failed to delete {}: {e}",
                                        disk_path.display()
                                    ),
                                }
                            })?;
                        }
                    }
                }
            }
            Ok::<(), WorkspaceError>(())
        }
        .await;

        if let Err(error) = apply_result {
            for (path, bytes) in recovery_bundle.into_iter().rev() {
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

        state = self
            .reconcile_allowlist(
                &request.user_id,
                request.allowlist_id,
                WorkspaceAllowlistRevisionKind::Restore,
                WorkspaceAllowlistRevisionSource::System,
                Some(request.target.clone()),
                Some(format!("restored workspace to {}", request.target)),
                &request.created_by,
            )
            .await?;
        if request.set_as_baseline {
            self.save_allowlist_state_record(
                request.allowlist_id,
                state.head_revision_id,
                state.head_revision_id,
            )
            .await?;
            self.sync_allowlist_live_cache(
                request.allowlist_id,
                state.head_revision_id,
                state.head_revision_id,
            )
            .await?;
        }
        self.build_allowlist_detail_internal(&request.user_id, request.allowlist_id)
            .await
    }

    async fn set_workspace_allowlist_baseline(
        &self,
        request: &WorkspaceAllowlistBaselineRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        let state = self
            .reconcile_allowlist(
                &request.user_id,
                request.allowlist_id,
                WorkspaceAllowlistRevisionKind::ManualRefresh,
                WorkspaceAllowlistRevisionSource::External,
                Some("baseline_set".to_string()),
                None,
                "system",
            )
            .await?;
        let target_revision_id = self
            .resolve_revision_target(request.allowlist_id, &state, &request.target)
            .await?;
        self.save_allowlist_state_record(
            request.allowlist_id,
            Some(target_revision_id),
            state.head_revision_id,
        )
        .await?;
        self.sync_allowlist_live_cache(
            request.allowlist_id,
            Some(target_revision_id),
            state.head_revision_id,
        )
        .await?;
        self.touch_allowlist_updated_at(request.allowlist_id)
            .await?;
        self.build_allowlist_detail_internal(&request.user_id, request.allowlist_id)
            .await
    }

    async fn refresh_workspace_allowlist(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        _scope_path: Option<&str>,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        self.reconcile_allowlist(
            user_id,
            allowlist_id,
            WorkspaceAllowlistRevisionKind::ManualRefresh,
            WorkspaceAllowlistRevisionSource::External,
            Some("workspace_refresh".to_string()),
            Some("manual workspace refresh".to_string()),
            "system",
        )
        .await?;
        self.build_allowlist_detail_internal(user_id, allowlist_id)
            .await
    }

    async fn sync_workspace_allowlist_watch(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.reconcile_allowlist(
            user_id,
            allowlist_id,
            WorkspaceAllowlistRevisionKind::FsWatch,
            WorkspaceAllowlistRevisionSource::External,
            Some("workspace_watch".to_string()),
            Some("background allowlisted tree watch".to_string()),
            "system",
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    /// Helper: create a file-backed backend with migrations applied.
    async fn setup_backend() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test_vector.db");
        let backend = LibSqlBackend::new_local(&db_path).await.expect("new_local");
        backend.run_migrations().await.expect("migrations");
        (backend, dir)
    }

    /// Helper: insert a document and chunk with an optional embedding.
    async fn insert_test_chunk(
        backend: &LibSqlBackend,
        user_id: &str,
        path: &str,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> (Uuid, Uuid) {
        let conn = backend.connect().await.expect("connect");
        let doc_id = Uuid::new_v4();
        let now = super::fmt_ts(&Utc::now());
        conn.execute(
            "INSERT INTO memory_documents (id, user_id, path, content, created_at, updated_at, metadata)
             VALUES (?1, ?2, ?3, '', ?4, ?4, '{}')",
            params![doc_id.to_string(), user_id, path, now],
        )
        .await
        .expect("insert doc");
        let chunk_id = backend
            .insert_chunk(doc_id, 0, content, embedding)
            .await
            .expect("insert chunk");
        (doc_id, chunk_id)
    }

    #[tokio::test]
    async fn test_ensure_vector_index_enables_vector_search() {
        let (backend, _dir) = setup_backend().await;

        // Create vector index with dim=4
        backend.ensure_vector_index(4).await.expect("ensure dim=4");
        // Insert a chunk with a 4-dim embedding
        let embedding = [1.0_f32, 0.0, 0.0, 0.0];
        let (_doc_id, _chunk_id) = insert_test_chunk(
            &backend,
            "test",
            "notes.md",
            "hello world",
            Some(&embedding),
        )
        .await;

        // Query using vector_top_k — should find the chunk
        let conn = backend.connect().await.expect("connect");
        let mut rows = conn
            .query(
                r#"SELECT c.id
                   FROM vector_top_k('idx_memory_chunks_embedding', vector('[1,0,0,0]'), 5) AS top_k
                   JOIN memory_chunks c ON c._rowid = top_k.id"#,
                (),
            )
            .await
            .expect("vector_top_k query");
        let row = rows
            .next()
            .await
            .expect("row fetch")
            .expect("expected a result row");
        let id: String = row.get(0).expect("get id");
        assert!(!id.is_empty(), "vector search should return the chunk");
    }

    #[tokio::test]
    async fn test_ensure_vector_index_dimension_change() {
        let (backend, _dir) = setup_backend().await;

        // Create with dim=4 and insert data
        backend.ensure_vector_index(4).await.expect("ensure dim=4");
        let embedding_4d = [1.0_f32, 2.0, 3.0, 4.0];
        insert_test_chunk(&backend, "test", "a.md", "content a", Some(&embedding_4d)).await;

        // Recreate with dim=8 — old 4-dim embeddings should be NULLed
        backend.ensure_vector_index(8).await.expect("ensure dim=8");
        // Verify metadata updated
        let conn = backend.connect().await.expect("connect");
        let mut rows = conn
            .query("SELECT name FROM _migrations WHERE version = 0", ())
            .await
            .expect("query metadata");
        let row = rows.next().await.expect("fetch").expect("metadata row");
        let dim_str: String = row.get(0).expect("get name");
        assert_eq!(dim_str, "8");
        // Verify old embedding was NULLed (wrong byte length for dim=8)
        let mut rows = conn
            .query("SELECT embedding IS NULL FROM memory_chunks LIMIT 1", ())
            .await
            .expect("query embedding");
        let row = rows.next().await.expect("fetch").expect("chunk row");
        let is_null: i64 = row.get(0).expect("get is_null");
        assert_eq!(
            is_null, 1,
            "old 4-dim embedding should be NULLed after dim change to 8"
        );
    }

    #[tokio::test]
    async fn test_ensure_vector_index_noop_when_unchanged() {
        let (backend, _dir) = setup_backend().await;

        // Create with dim=4 and insert data
        backend.ensure_vector_index(4).await.expect("ensure dim=4");
        let embedding = [1.0_f32, 0.0, 0.0, 0.0];
        insert_test_chunk(&backend, "test", "b.md", "content b", Some(&embedding)).await;

        // Run again with same dimension — should be a no-op
        backend
            .ensure_vector_index(4)
            .await
            .expect("ensure dim=4 again");
        // Verify data is untouched (embedding not NULLed)
        let conn = backend.connect().await.expect("connect");
        let mut rows = conn
            .query(
                "SELECT embedding IS NOT NULL FROM memory_chunks LIMIT 1",
                (),
            )
            .await
            .expect("query embedding");
        let row = rows.next().await.expect("fetch").expect("chunk row");
        let has_embedding: i64 = row.get(0).expect("get");
        assert_eq!(
            has_embedding, 1,
            "embedding should be preserved on no-op call"
        );
    }

    #[tokio::test]
    async fn test_hybrid_search_returns_vector_results() {
        let (backend, _dir) = setup_backend().await;

        // Create vector index with dim=4
        backend.ensure_vector_index(4).await.expect("ensure dim=4");
        // Insert chunk with embedding and searchable content
        let embedding = [0.5_f32, 0.5, 0.0, 0.0];
        insert_test_chunk(
            &backend,
            "user1",
            "notes.md",
            "quantum computing research",
            Some(&embedding),
        )
        .await;

        // Search via the WorkspaceStore trait with vector enabled
        let query_emb = [0.5_f32, 0.5, 0.0, 0.0];
        let config = SearchConfig::default().with_limit(5);
        let results = backend
            .hybrid_search("user1", None, "quantum", Some(&query_emb), &config)
            .await
            .expect("hybrid_search");
        assert!(!results.is_empty(), "hybrid search should return results");
        let first = &results[0];
        assert!(
            first.vector_rank.is_some(),
            "result should have a vector_rank"
        );
        assert_eq!(first.content, "quantum computing research");
    }

    #[tokio::test]
    async fn test_workspace_allowlist_real_disk_diff_keep_revert() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("allowlisted-project");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");
        std::fs::write(
            allowlist_root.join("main.rs"),
            "fn main() {\n    println!(\"v1\");\n}\n",
        )
        .expect("seed file");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "project".to_string(),
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create allowlist");

        let initial_detail = backend
            .get_workspace_allowlist("default", allowlist.allowlist.id)
            .await
            .expect("get allowlist");
        assert_eq!(
            initial_detail.baseline_revision_id,
            initial_detail.head_revision_id
        );

        let file = backend
            .read_workspace_allowlist_file("default", allowlist.allowlist.id, "main.rs")
            .await
            .expect("read allowlisted file");
        assert!(file.content.as_deref().is_some_and(|v| v.contains("v1")));

        backend
            .write_workspace_allowlist_file(
                "default",
                allowlist.allowlist.id,
                "main.rs",
                b"fn main() {\n    println!(\"v2\");\n}\n",
            )
            .await
            .expect("write allowlisted file");
        let disk_after_write =
            std::fs::read_to_string(allowlist_root.join("main.rs")).expect("read disk after write");
        assert!(disk_after_write.contains("v2"));

        let diff = backend
            .diff_workspace_allowlist("default", allowlist.allowlist.id, None)
            .await
            .expect("diff allowlist");
        assert_eq!(diff.entries.len(), 1);
        assert_eq!(diff.entries[0].status, AllowlistedFileStatus::Modified);
        assert_eq!(
            diff.entries[0].change_kind,
            WorkspaceAllowlistChangeKind::Modified
        );
        assert!(
            diff.entries[0]
                .working_content
                .as_deref()
                .is_some_and(|v| v.contains("v2"))
        );

        backend
            .revert_workspace_allowlist(&AllowlistActionRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                scope_path: Some("main.rs".to_string()),
                checkpoint_id: None,
                set_as_baseline: false,
            })
            .await
            .expect("revert file");

        let reverted = backend
            .read_workspace_allowlist_file("default", allowlist.allowlist.id, "main.rs")
            .await
            .expect("read reverted file");
        assert!(
            reverted
                .content
                .as_deref()
                .is_some_and(|v| v.contains("v1"))
        );
        let disk_after_revert = std::fs::read_to_string(allowlist_root.join("main.rs"))
            .expect("read disk after revert");
        assert!(disk_after_revert.contains("v1"));

        backend
            .write_workspace_allowlist_file(
                "default",
                allowlist.allowlist.id,
                "main.rs",
                b"fn main() {\n    println!(\"kept\");\n}\n",
            )
            .await
            .expect("write allowlisted file again");
        backend
            .keep_workspace_allowlist(&AllowlistActionRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                scope_path: Some("main.rs".to_string()),
                checkpoint_id: None,
                set_as_baseline: true,
            })
            .await
            .expect("keep file");

        let post_keep_detail = backend
            .get_workspace_allowlist("default", allowlist.allowlist.id)
            .await
            .expect("get post-keep allowlist");
        assert_eq!(
            post_keep_detail.baseline_revision_id,
            post_keep_detail.head_revision_id
        );
        let clean_diff = backend
            .diff_workspace_allowlist("default", allowlist.allowlist.id, None)
            .await
            .expect("diff after keep");
        assert!(clean_diff.entries.is_empty());

        let disk_content =
            std::fs::read_to_string(allowlist_root.join("main.rs")).expect("read disk after keep");
        assert!(disk_content.contains("kept"));
    }

    #[tokio::test]
    async fn test_workspace_root_lists_allowlists_directly() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("root-project");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");
        let agents = backend
            .get_or_create_document_by_path("default", None, "AGENTS.md")
            .await
            .expect("create AGENTS");
        backend
            .update_document(agents.id, "# agent rules")
            .await
            .expect("update AGENTS");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "root-project".to_string(),
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create allowlist");

        let entries = backend
            .list_workspace_tree("default", None, "workspace://")
            .await
            .expect("list workspace root");
        assert!(
            entries.iter().any(|entry| {
                entry.kind == WorkspaceTreeEntryKind::Allowlist
                    && entry.path == allowlist.allowlist.id.to_string()
                    && entry.uri == WorkspaceUri::allowlist_uri(allowlist.allowlist.id, None)
            }),
            "workspace root should include allowlisted folders"
        );
        assert!(
            entries.iter().any(|entry| {
                entry.kind == WorkspaceTreeEntryKind::MemoryFile && entry.path == "AGENTS.md"
            }),
            "workspace root should include workspace-owned files"
        );

        let legacy_entries = backend
            .list_workspace_tree("default", None, "workspace://allowlists")
            .await
            .expect("list legacy workspace root");
        assert!(
            legacy_entries.iter().any(|entry| {
                entry.kind == WorkspaceTreeEntryKind::Allowlist
                    && entry.uri == WorkspaceUri::allowlist_uri(allowlist.allowlist.id, None)
            }),
            "legacy root alias should still expose allowlisted folders"
        );
    }

    #[tokio::test]
    async fn test_workspace_allowlist_rejects_parent_escape() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("escape-project");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");
        std::fs::write(dir.path().join("secret.txt"), "secret").expect("seed sibling file");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "escape-project".to_string(),
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create allowlist");

        let read_err = backend
            .read_workspace_allowlist_file("default", allowlist.allowlist.id, "../secret.txt")
            .await
            .expect_err("reject escaped read");
        assert!(read_err.to_string().contains("escapes root"));

        let write_err = backend
            .write_workspace_allowlist_file(
                "default",
                allowlist.allowlist.id,
                "../written.txt",
                b"owned",
            )
            .await
            .expect_err("reject escaped write");
        assert!(write_err.to_string().contains("escapes root"));
        assert!(
            !dir.path().join("written.txt").exists(),
            "escaped write must not create files outside the allowlist"
        );

        let tree_err = backend
            .list_workspace_tree(
                "default",
                None,
                &format!("workspace://{}/../secret.txt", allowlist.allowlist.id),
            )
            .await
            .expect_err("reject escaped tree path");
        assert!(tree_err.to_string().contains("escapes root"));
    }

    #[tokio::test]
    async fn test_workspace_allowlist_delete_is_immediate_and_restore_recovers() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("delete-project");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");
        std::fs::write(allowlist_root.join("delete.txt"), "hello").expect("seed file");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "delete-project".to_string(),
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create allowlist");

        backend
            .delete_workspace_allowlist_file("default", allowlist.allowlist.id, "delete.txt")
            .await
            .expect("delete file");
        assert!(
            !allowlist_root.join("delete.txt").exists(),
            "disk file should be deleted immediately"
        );

        let diff = backend
            .diff_workspace_allowlist("default", allowlist.allowlist.id, None)
            .await
            .expect("diff after delete");
        assert_eq!(diff.entries.len(), 1);
        assert_eq!(diff.entries[0].status, AllowlistedFileStatus::Deleted);
        assert_eq!(
            diff.entries[0].change_kind,
            WorkspaceAllowlistChangeKind::Deleted
        );

        backend
            .revert_workspace_allowlist(&AllowlistActionRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                scope_path: Some("delete.txt".to_string()),
                checkpoint_id: None,
                set_as_baseline: false,
            })
            .await
            .expect("restore baseline");
        assert!(
            allowlist_root.join("delete.txt").exists(),
            "restore should bring back the deleted file"
        );
    }

    mod resolve_dimension {
        use super::*;
        use crate::config::helpers::lock_env;

        fn clear_embedding_env() {
            // SAFETY: called under ENV_MUTEX
            unsafe {
                std::env::remove_var("EMBEDDING_ENABLED");
                std::env::remove_var("EMBEDDING_DIMENSION");
                std::env::remove_var("EMBEDDING_MODEL");
            }
        }

        #[test]
        fn returns_none_when_disabled() {
            let _guard = lock_env();
            clear_embedding_env();
            assert!(resolve_embedding_dimension().is_none());
        }

        #[test]
        fn returns_explicit_dimension() {
            let _guard = lock_env();
            clear_embedding_env();
            // SAFETY: under ENV_MUTEX
            unsafe {
                std::env::set_var("EMBEDDING_ENABLED", "true");
                std::env::set_var("EMBEDDING_DIMENSION", "768");
            }
            assert_eq!(resolve_embedding_dimension(), Some(768));
            unsafe {
                std::env::remove_var("EMBEDDING_ENABLED");
                std::env::remove_var("EMBEDDING_DIMENSION");
            }
        }

        #[test]
        fn infers_from_model() {
            let _guard = lock_env();
            clear_embedding_env();
            // SAFETY: under ENV_MUTEX
            unsafe {
                std::env::set_var("EMBEDDING_ENABLED", "1");
                std::env::set_var("EMBEDDING_MODEL", "all-minilm");
            }
            assert_eq!(resolve_embedding_dimension(), Some(384));
            unsafe {
                std::env::remove_var("EMBEDDING_ENABLED");
                std::env::remove_var("EMBEDDING_MODEL");
            }
        }

        #[test]
        fn defaults_to_1536_for_unknown_model() {
            let _guard = lock_env();
            clear_embedding_env();
            // SAFETY: under ENV_MUTEX
            unsafe {
                std::env::set_var("EMBEDDING_ENABLED", "true");
                std::env::set_var("EMBEDDING_MODEL", "some-unknown-model");
            }
            assert_eq!(resolve_embedding_dimension(), Some(1536));
            unsafe {
                std::env::remove_var("EMBEDDING_ENABLED");
                std::env::remove_var("EMBEDDING_MODEL");
            }
        }
    }

    #[tokio::test]
    async fn test_workspace_allowlist_checkpoint_history_and_restore() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("checkpoint-project");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");
        std::fs::write(allowlist_root.join("main.txt"), "v1\n").expect("seed file");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "checkpoint-project".to_string(),
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create allowlist");

        backend
            .write_workspace_allowlist_file("default", allowlist.allowlist.id, "main.txt", b"v2\n")
            .await
            .expect("write v2");
        let checkpoint = backend
            .create_workspace_checkpoint(&CreateCheckpointRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                revision_id: None,
                label: Some("v2".to_string()),
                summary: Some("before v3".to_string()),
                created_by: "test".to_string(),
                is_auto: false,
            })
            .await
            .expect("create checkpoint");

        backend
            .write_workspace_allowlist_file("default", allowlist.allowlist.id, "main.txt", b"v3\n")
            .await
            .expect("write v3");

        let history = backend
            .list_workspace_allowlist_history(&WorkspaceAllowlistHistoryRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                scope_path: None,
                limit: 20,
                since: None,
                include_checkpoints: true,
            })
            .await
            .expect("history");
        assert!(!history.revisions.is_empty());
        assert!(
            history
                .checkpoints
                .iter()
                .any(|value| value.id == checkpoint.id && value.label.as_deref() == Some("v2"))
        );

        backend
            .restore_workspace_allowlist(&WorkspaceAllowlistRestoreRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                scope_path: None,
                target: checkpoint.id.to_string(),
                set_as_baseline: false,
                dry_run: false,
                create_checkpoint_before_restore: true,
                created_by: "test".to_string(),
            })
            .await
            .expect("restore checkpoint");

        let disk =
            std::fs::read_to_string(allowlist_root.join("main.txt")).expect("read restored disk");
        assert_eq!(disk, "v2\n");
    }

    #[tokio::test]
    async fn test_workspace_allowlist_write_new_file_then_keep_clears_diff() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("new-file-project");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "new-file-project".to_string(),
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create allowlist");

        backend
            .write_workspace_allowlist_file(
                "default",
                allowlist.allowlist.id,
                "nested/new.txt",
                b"hello world\n",
            )
            .await
            .expect("write new allowlisted file");
        assert!(allowlist_root.join("nested/new.txt").exists());
        let diff = backend
            .diff_workspace_allowlist("default", allowlist.allowlist.id, None)
            .await
            .expect("diff after write");
        assert_eq!(diff.entries.len(), 1);
        assert_eq!(diff.entries[0].status, AllowlistedFileStatus::Added);
        assert_eq!(
            diff.entries[0].change_kind,
            WorkspaceAllowlistChangeKind::Added
        );

        backend
            .keep_workspace_allowlist(&AllowlistActionRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                scope_path: Some("nested/new.txt".to_string()),
                checkpoint_id: None,
                set_as_baseline: true,
            })
            .await
            .expect("keep new file");

        let disk =
            std::fs::read_to_string(allowlist_root.join("nested/new.txt")).expect("read disk");
        assert_eq!(disk, "hello world\n");
        let clean_diff = backend
            .diff_workspace_allowlist("default", allowlist.allowlist.id, None)
            .await
            .expect("diff after keep");
        assert!(clean_diff.entries.is_empty());
    }

    #[tokio::test]
    async fn test_workspace_tree_lists_nested_allowlist_directory_without_refreshing_whole_allowlist()
     {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("tree-project");
        std::fs::create_dir_all(allowlist_root.join("nested"))
            .expect("create nested allowlist root");
        std::fs::write(allowlist_root.join("nested/child.txt"), "hello").expect("seed nested file");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "tree-project".to_string(),
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create allowlist");

        let root_entries = backend
            .list_workspace_tree(
                "default",
                None,
                &WorkspaceUri::allowlist_uri(allowlist.allowlist.id, None),
            )
            .await
            .expect("list allowlist root");
        assert!(
            root_entries.iter().any(|entry| {
                entry.kind == WorkspaceTreeEntryKind::AllowlistedDirectory
                    && entry.path == "nested"
                    && entry.uri
                        == WorkspaceUri::allowlist_uri(allowlist.allowlist.id, Some("nested"))
            }),
            "allowlist root should expose nested directory"
        );

        let nested_entries = backend
            .list_workspace_tree(
                "default",
                None,
                &WorkspaceUri::allowlist_uri(allowlist.allowlist.id, Some("nested")),
            )
            .await
            .expect("list nested allowlist directory");
        assert!(
            nested_entries.iter().any(|entry| {
                entry.kind == WorkspaceTreeEntryKind::AllowlistedFile
                    && entry.path == "nested/child.txt"
                    && entry.uri
                        == WorkspaceUri::allowlist_uri(
                            allowlist.allowlist.id,
                            Some("nested/child.txt"),
                        )
            }),
            "nested directory should expose child file"
        );
    }
}
