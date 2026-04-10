//! Workspace-related WorkspaceStore implementation for LibSqlBackend.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_opt_ts, get_text, get_ts,
    row_to_memory_document,
};
use crate::db::WorkspaceStore;
use crate::error::{DatabaseError, WorkspaceError};
use crate::workspace::{
    ConflictResolutionRequest, CreateCheckpointRequest, CreateMountRequest, MemoryChunk,
    MemoryDocument, MountActionRequest, MountedFileDiff, MountedFileStatus, RankedResult,
    SearchConfig, SearchResult, WorkspaceEntry, WorkspaceMount, WorkspaceMountCheckpoint,
    WorkspaceMountDetail, WorkspaceMountDiff, WorkspaceMountFileView, WorkspaceMountSummary,
    WorkspaceTreeEntry, WorkspaceTreeEntryKind, WorkspaceUri, fuse_results, normalize_mount_path,
};

use chrono::Utc;

/// Resolve the embedding dimension from environment variables.
///
/// Reads `EMBEDDING_ENABLED`, `EMBEDDING_DIMENSION`, and `EMBEDDING_MODEL`
/// from env vars. Returns `None` if embeddings are disabled.
///
/// Note: this only reads env vars, not persisted `Settings`, because it runs
/// during `run_migrations()` before the full config stack is available. Users
/// who configure embeddings via the settings UI must also set
/// `EMBEDDING_ENABLED=true` in their environment for the vector index to be
/// created. The model→dimension mapping is shared with `EmbeddingsConfig` via
/// `default_dimension_for_model()`.
pub(crate) fn resolve_embedding_dimension() -> Option<usize> {
    let enabled = std::env::var("EMBEDDING_ENABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    if !enabled {
        tracing::debug!("Vector index setup skipped (EMBEDDING_ENABLED not set in env)");
        return None;
    }

    if let Ok(dim_str) = std::env::var("EMBEDDING_DIMENSION")
        && let Ok(dim) = dim_str.parse::<usize>()
        && dim > 0
    {
        return Some(dim);
    }

    let model =
        std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());

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
struct MountFileRecord {
    path: String,
    status: MountedFileStatus,
    is_binary: bool,
    base_snapshot_id: Option<Uuid>,
    working_snapshot_id: Option<Uuid>,
    conflict_reason: Option<String>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextChange {
    base_start: usize,
    base_end: usize,
    new_lines: Vec<String>,
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

fn split_text_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n')
        .map(ToString::to_string)
        .collect()
}

fn lcs_matches(base: &[String], other: &[String]) -> Vec<(usize, usize)> {
    let mut dp = vec![vec![0usize; other.len() + 1]; base.len() + 1];
    for i in (0..base.len()).rev() {
        for j in (0..other.len()).rev() {
            dp[i][j] = if base[i] == other[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut i = 0;
    let mut j = 0;
    let mut matches = Vec::new();
    while i < base.len() && j < other.len() {
        if base[i] == other[j] {
            matches.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    matches
}

fn diff_text_changes(base: &str, other: &str) -> Vec<TextChange> {
    let base_lines = split_text_lines(base);
    let other_lines = split_text_lines(other);
    let matches = lcs_matches(&base_lines, &other_lines);
    let mut changes = Vec::new();
    let mut prev_base = 0usize;
    let mut prev_other = 0usize;

    for (base_idx, other_idx) in matches {
        if base_idx > prev_base || other_idx > prev_other {
            changes.push(TextChange {
                base_start: prev_base,
                base_end: base_idx,
                new_lines: other_lines[prev_other..other_idx].to_vec(),
            });
        }
        prev_base = base_idx + 1;
        prev_other = other_idx + 1;
    }

    if prev_base < base_lines.len() || prev_other < other_lines.len() {
        changes.push(TextChange {
            base_start: prev_base,
            base_end: base_lines.len(),
            new_lines: other_lines[prev_other..].to_vec(),
        });
    }

    changes
}

fn apply_text_changes(base: &[String], changes: &[TextChange]) -> String {
    let mut cursor = 0usize;
    let mut merged = String::new();
    for change in changes {
        if change.base_start > cursor {
            merged.push_str(&base[cursor..change.base_start].concat());
        }
        merged.push_str(&change.new_lines.concat());
        cursor = change.base_end;
    }
    if cursor < base.len() {
        merged.push_str(&base[cursor..].concat());
    }
    merged
}

fn three_way_merge_text(base: &str, remote: &str, working: &str) -> Result<String, String> {
    if remote == working {
        return Ok(working.to_string());
    }
    if remote == base {
        return Ok(working.to_string());
    }
    if working == base {
        return Ok(remote.to_string());
    }

    let remote_changes = diff_text_changes(base, remote);
    let working_changes = diff_text_changes(base, working);
    let mut merged_changes = Vec::new();
    let mut remote_idx = 0usize;
    let mut working_idx = 0usize;

    while remote_idx < remote_changes.len() || working_idx < working_changes.len() {
        match (
            remote_changes.get(remote_idx),
            working_changes.get(working_idx),
        ) {
            (Some(remote_change), Some(working_change)) => {
                let same_insert_point = remote_change.base_start == remote_change.base_end
                    && working_change.base_start == working_change.base_end
                    && remote_change.base_start == working_change.base_start;
                let overlaps = same_insert_point
                    || (remote_change.base_start < working_change.base_end
                        && working_change.base_start < remote_change.base_end);

                if overlaps {
                    if remote_change == working_change {
                        merged_changes.push(remote_change.clone());
                        remote_idx += 1;
                        working_idx += 1;
                        continue;
                    }
                    return Err(format!(
                        "Text conflict around base lines {}..{}",
                        remote_change.base_start + 1,
                        remote_change.base_end.max(working_change.base_end)
                    ));
                }

                let remote_first = remote_change.base_start < working_change.base_start
                    || (remote_change.base_start == working_change.base_start
                        && remote_change.base_end <= working_change.base_end);
                if remote_first {
                    merged_changes.push(remote_change.clone());
                    remote_idx += 1;
                } else {
                    merged_changes.push(working_change.clone());
                    working_idx += 1;
                }
            }
            (Some(remote_change), None) => {
                merged_changes.push(remote_change.clone());
                remote_idx += 1;
            }
            (None, Some(working_change)) => {
                merged_changes.push(working_change.clone());
                working_idx += 1;
            }
            (None, None) => break,
        }
    }

    Ok(apply_text_changes(&split_text_lines(base), &merged_changes))
}

fn status_from_str(value: &str) -> MountedFileStatus {
    match value {
        "modified" => MountedFileStatus::Modified,
        "added" => MountedFileStatus::Added,
        "pending_delete" => MountedFileStatus::PendingDelete,
        "conflicted" => MountedFileStatus::Conflicted,
        "binary_modified" => MountedFileStatus::BinaryModified,
        _ => MountedFileStatus::Clean,
    }
}

fn status_to_str(status: MountedFileStatus) -> &'static str {
    match status {
        MountedFileStatus::Clean => "clean",
        MountedFileStatus::Modified => "modified",
        MountedFileStatus::Added => "added",
        MountedFileStatus::PendingDelete => "pending_delete",
        MountedFileStatus::Conflicted => "conflicted",
        MountedFileStatus::BinaryModified => "binary_modified",
    }
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

    async fn replace_mount_record_with_snapshot(
        &self,
        mount_id: Uuid,
        path: &str,
        snapshot: &SnapshotRecord,
    ) -> Result<MountFileRecord, WorkspaceError> {
        self.upsert_mount_file_record(
            mount_id,
            path,
            MountedFileStatus::Clean,
            snapshot.is_binary,
            Some(snapshot.id),
            Some(snapshot.id),
            Some(snapshot.hash.clone()),
            Some(snapshot.hash.clone()),
            Some(snapshot.hash.clone()),
            None,
        )
        .await
    }

    async fn sync_record_to_disk_snapshot(
        &self,
        mount_id: Uuid,
        path: &str,
        disk_path: &Path,
    ) -> Result<MountFileRecord, WorkspaceError> {
        let bytes = Self::read_disk_bytes(disk_path).await?.ok_or_else(|| {
            WorkspaceError::MountPathNotFound {
                mount_id: mount_id.to_string(),
                path: path.to_string(),
            }
        })?;
        let snapshot = self.insert_snapshot(mount_id, path, &bytes).await?;
        self.replace_mount_record_with_snapshot(mount_id, path, &snapshot)
            .await
    }

    async fn fetch_mount(
        &self,
        user_id: &str,
        mount_id: Uuid,
    ) -> Result<WorkspaceMount, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, display_name, source_root, bypass_read, bypass_write, created_at, updated_at
                 FROM workspace_mounts
                 WHERE user_id = ?1 AND id = ?2",
                params![user_id, mount_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("mount query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("mount query failed: {e}"),
            })?
        else {
            return Err(WorkspaceError::MountNotFound {
                mount_id: mount_id.to_string(),
            });
        };

        Ok(WorkspaceMount {
            id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("invalid mount id: {e}"),
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

    async fn list_mount_summaries_internal(
        &self,
        user_id: &str,
    ) -> Result<Vec<WorkspaceMountSummary>, WorkspaceError> {
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
                       COALESCE(SUM(CASE WHEN f.status = 'pending_delete' THEN 1 ELSE 0 END), 0)
                FROM workspace_mounts m
                LEFT JOIN workspace_mount_files f ON f.mount_id = m.id
                WHERE m.user_id = ?1
                GROUP BY m.id, m.user_id, m.display_name, m.source_root, m.bypass_read, m.bypass_write,
                         m.created_at, m.updated_at
                ORDER BY m.updated_at DESC
                "#,
                params![user_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("mount list failed: {e}"),
            })?;

        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("mount list failed: {e}"),
            })?
        {
            let mount = WorkspaceMount {
                id: Uuid::parse_str(&get_text(&row, 0)).map_err(|e| {
                    WorkspaceError::SearchFailed {
                        reason: format!("invalid mount id: {e}"),
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
            result.push(WorkspaceMountSummary {
                mount,
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
                "SELECT id, content, is_binary, content_hash FROM workspace_mount_snapshots WHERE id = ?1",
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
        mount_id: Uuid,
        path: &str,
        content: &[u8],
    ) -> Result<SnapshotRecord, WorkspaceError> {
        let snapshot = SnapshotRecord {
            id: Uuid::new_v4(),
            content: content.to_vec(),
            is_binary: classify_binary(content),
            hash: compute_content_hash(content),
        };
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "INSERT INTO workspace_mount_snapshots (id, mount_id, relative_path, content, is_binary, content_hash, size_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                snapshot.id.to_string(),
                mount_id.to_string(),
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

    async fn load_mount_file_record(
        &self,
        mount_id: Uuid,
        path: &str,
    ) -> Result<Option<MountFileRecord>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT relative_path, status, is_binary, base_snapshot_id, working_snapshot_id, conflict_reason, updated_at
                 FROM workspace_mount_files
                 WHERE mount_id = ?1 AND relative_path = ?2",
                params![mount_id.to_string(), path],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("mount file query failed: {e}"),
            })?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("mount file query failed: {e}"),
            })?
        else {
            return Ok(None);
        };
        Ok(Some(MountFileRecord {
            path: get_text(&row, 0),
            status: status_from_str(&get_text(&row, 1)),
            is_binary: get_i64(&row, 2) != 0,
            base_snapshot_id: get_opt_text(&row, 3).and_then(|v| Uuid::parse_str(&v).ok()),
            working_snapshot_id: get_opt_text(&row, 4).and_then(|v| Uuid::parse_str(&v).ok()),
            conflict_reason: get_opt_text(&row, 5),
            updated_at: get_ts(&row, 6),
        }))
    }

    async fn upsert_mount_file_record(
        &self,
        mount_id: Uuid,
        path: &str,
        status: MountedFileStatus,
        is_binary: bool,
        base_snapshot_id: Option<Uuid>,
        working_snapshot_id: Option<Uuid>,
        base_hash: Option<String>,
        working_hash: Option<String>,
        remote_hash: Option<String>,
        conflict_reason: Option<String>,
    ) -> Result<MountFileRecord, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
            INSERT INTO workspace_mount_files (
                mount_id, relative_path, status, is_binary, base_snapshot_id, working_snapshot_id,
                remote_hash, base_hash, working_hash, conflict_reason, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
            ON CONFLICT(mount_id, relative_path) DO UPDATE SET
                status = excluded.status,
                is_binary = excluded.is_binary,
                base_snapshot_id = excluded.base_snapshot_id,
                working_snapshot_id = excluded.working_snapshot_id,
                remote_hash = excluded.remote_hash,
                base_hash = excluded.base_hash,
                working_hash = excluded.working_hash,
                conflict_reason = excluded.conflict_reason,
                updated_at = excluded.updated_at
            "#,
            params![
                mount_id.to_string(),
                path,
                status_to_str(status),
                if is_binary { 1 } else { 0 },
                base_snapshot_id.map(|v| v.to_string()),
                working_snapshot_id.map(|v| v.to_string()),
                remote_hash,
                base_hash,
                working_hash,
                conflict_reason,
                now
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("mount file upsert failed: {e}"),
        })?;
        self.load_mount_file_record(mount_id, path)
            .await?
            .ok_or_else(|| WorkspaceError::MountPathNotFound {
                mount_id: mount_id.to_string(),
                path: path.to_string(),
            })
    }

    async fn ensure_mount_file_loaded(
        &self,
        user_id: &str,
        mount_id: Uuid,
        path: &str,
    ) -> Result<MountFileRecord, WorkspaceError> {
        let path = normalize_mount_path(path)?;
        if let Some(existing) = self.load_mount_file_record(mount_id, &path).await? {
            return Ok(existing);
        }
        let mount = self.fetch_mount(user_id, mount_id).await?;
        let disk_path = Path::new(&mount.source_root).join(&path);
        let bytes =
            tokio::fs::read(&disk_path)
                .await
                .map_err(|e| WorkspaceError::MountPathNotFound {
                    mount_id: mount_id.to_string(),
                    path: format!("{path} ({e})"),
                })?;
        let snapshot = self.insert_snapshot(mount_id, &path, &bytes).await?;
        self.upsert_mount_file_record(
            mount_id,
            &path,
            MountedFileStatus::Clean,
            snapshot.is_binary,
            Some(snapshot.id),
            Some(snapshot.id),
            Some(snapshot.hash.clone()),
            Some(snapshot.hash.clone()),
            Some(snapshot.hash.clone()),
            None,
        )
        .await
    }

    async fn list_mount_file_records(
        &self,
        mount_id: Uuid,
        prefix: Option<&str>,
    ) -> Result<Vec<MountFileRecord>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let pattern = if let Some(value) = prefix {
            let normalized = normalize_mount_path(value)?;
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
                "SELECT relative_path, status, is_binary, base_snapshot_id, working_snapshot_id, conflict_reason, updated_at
                 FROM workspace_mount_files
                 WHERE mount_id = ?1 AND relative_path LIKE ?2
                 ORDER BY relative_path",
                params![mount_id.to_string(), pattern],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("mount file list failed: {e}"),
            })?;
        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("mount file list failed: {e}"),
            })?
        {
            result.push(MountFileRecord {
                path: get_text(&row, 0),
                status: status_from_str(&get_text(&row, 1)),
                is_binary: get_i64(&row, 2) != 0,
                base_snapshot_id: get_opt_text(&row, 3).and_then(|v| Uuid::parse_str(&v).ok()),
                working_snapshot_id: get_opt_text(&row, 4).and_then(|v| Uuid::parse_str(&v).ok()),
                conflict_reason: get_opt_text(&row, 5),
                updated_at: get_ts(&row, 6),
            });
        }
        Ok(result)
    }

    async fn collect_checkpoint_chain(
        &self,
        mount_id: Uuid,
    ) -> Result<Vec<WorkspaceMountCheckpoint>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT id, mount_id, parent_checkpoint_id, label, summary, created_by, is_auto, base_generation, created_at
                 FROM workspace_mount_checkpoints
                 WHERE mount_id = ?1
                 ORDER BY created_at DESC",
                params![mount_id.to_string()],
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
            let changed_files = self.list_checkpoint_files(checkpoint_id).await?;
            result.push(WorkspaceMountCheckpoint {
                id: checkpoint_id,
                mount_id: Uuid::parse_str(&get_text(&row, 1)).map_err(|e| {
                    WorkspaceError::SearchFailed {
                        reason: format!("invalid checkpoint mount id: {e}"),
                    }
                })?,
                parent_checkpoint_id: get_opt_text(&row, 2).and_then(|v| Uuid::parse_str(&v).ok()),
                label: get_opt_text(&row, 3),
                summary: get_opt_text(&row, 4),
                created_by: get_text(&row, 5),
                is_auto: get_i64(&row, 6) != 0,
                base_generation: get_i64(&row, 7),
                created_at: get_ts(&row, 8),
                changed_files,
            });
        }
        Ok(result)
    }

    async fn list_checkpoint_files(
        &self,
        checkpoint_id: Uuid,
    ) -> Result<Vec<String>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                "SELECT relative_path FROM workspace_mount_checkpoint_files WHERE checkpoint_id = ?1 ORDER BY relative_path",
                params![checkpoint_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("checkpoint files query failed: {e}"),
            })?;
        let mut files = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("checkpoint files query failed: {e}"),
            })?
        {
            files.push(get_text(&row, 0));
        }
        Ok(files)
    }

    async fn build_mount_detail_internal(
        &self,
        user_id: &str,
        mount_id: Uuid,
    ) -> Result<WorkspaceMountDetail, WorkspaceError> {
        let summary = self
            .list_mount_summaries_internal(user_id)
            .await?
            .into_iter()
            .find(|summary| summary.mount.id == mount_id)
            .ok_or_else(|| WorkspaceError::MountNotFound {
                mount_id: mount_id.to_string(),
            })?;
        let checkpoints = self.collect_checkpoint_chain(mount_id).await?;
        Ok(WorkspaceMountDetail {
            open_change_count: summary.dirty_count,
            summary,
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
        _agent_id: Option<Uuid>,
        uri: &str,
    ) -> Result<Vec<WorkspaceTreeEntry>, WorkspaceError> {
        let parsed = WorkspaceUri::parse(uri)?.ok_or_else(|| WorkspaceError::InvalidDocType {
            doc_type: uri.to_string(),
        })?;
        match parsed {
            WorkspaceUri::Root => Ok(self
                .list_mount_summaries_internal(user_id)
                .await?
                .into_iter()
                .map(|summary| WorkspaceTreeEntry {
                    name: summary.mount.display_name.clone(),
                    path: summary.mount.id.to_string(),
                    uri: WorkspaceUri::mount_uri(summary.mount.id, None),
                    is_directory: true,
                    kind: WorkspaceTreeEntryKind::Mount,
                    status: None,
                    updated_at: Some(summary.mount.updated_at),
                    content_preview: None,
                    bypass_write: Some(summary.mount.bypass_write),
                    dirty_count: summary.dirty_count,
                    conflict_count: summary.conflict_count,
                    pending_delete_count: summary.pending_delete_count,
                })
                .collect()),
            WorkspaceUri::MountRoot(mount_id) => {
                let mount = self.fetch_mount(user_id, mount_id).await?;
                let prefix = String::new();
                let dir_path = Path::new(&mount.source_root).join(&prefix);
                let mut entries_map: BTreeMap<String, WorkspaceTreeEntry> = BTreeMap::new();

                if dir_path.is_dir() {
                    let read_dir =
                        std::fs::read_dir(&dir_path).map_err(|e| WorkspaceError::IoError {
                            reason: format!(
                                "failed to list mount directory {}: {e}",
                                dir_path.display()
                            ),
                        })?;
                    for entry in read_dir {
                        let entry = entry.map_err(|e| WorkspaceError::IoError {
                            reason: format!("failed to read mount dir entry: {e}"),
                        })?;
                        let name = entry.file_name().to_string_lossy().to_string();
                        let rel = normalize_mount_path(&name)?;
                        let is_directory = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                        entries_map.insert(
                            name.clone(),
                            WorkspaceTreeEntry {
                                name,
                                path: rel.clone(),
                                uri: WorkspaceUri::mount_uri(mount_id, Some(&rel)),
                                is_directory,
                                kind: if is_directory {
                                    WorkspaceTreeEntryKind::MountedDirectory
                                } else {
                                    WorkspaceTreeEntryKind::MountedFile
                                },
                                status: None,
                                updated_at: None,
                                content_preview: None,
                                bypass_write: Some(mount.bypass_write),
                                dirty_count: 0,
                                conflict_count: 0,
                                pending_delete_count: 0,
                            },
                        );
                    }
                }

                for record in self
                    .list_mount_file_records(mount_id, Some(&prefix))
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
                                uri: WorkspaceUri::mount_uri(mount_id, Some(&child_path)),
                                is_directory,
                                kind: if is_directory {
                                    WorkspaceTreeEntryKind::MountedDirectory
                                } else {
                                    WorkspaceTreeEntryKind::MountedFile
                                },
                                status: None,
                                updated_at: Some(record.updated_at),
                                content_preview: None,
                                bypass_write: Some(mount.bypass_write),
                                dirty_count: 0,
                                conflict_count: 0,
                                pending_delete_count: 0,
                            });
                    if is_directory {
                        entry.is_directory = true;
                        entry.kind = WorkspaceTreeEntryKind::MountedDirectory;
                        entry.dirty_count += usize::from(record.status != MountedFileStatus::Clean);
                        entry.conflict_count +=
                            usize::from(record.status == MountedFileStatus::Conflicted);
                        entry.pending_delete_count +=
                            usize::from(record.status == MountedFileStatus::PendingDelete);
                    } else {
                        entry.status = Some(record.status);
                        entry.updated_at = Some(record.updated_at);
                    }
                }

                Ok(entries_map.into_values().collect())
            }
            WorkspaceUri::MountPath(mount_id, prefix) => {
                let mount = self.fetch_mount(user_id, mount_id).await?;
                let dir_path = Path::new(&mount.source_root).join(&prefix);
                let mut entries_map: BTreeMap<String, WorkspaceTreeEntry> = BTreeMap::new();

                if dir_path.is_dir() {
                    let read_dir =
                        std::fs::read_dir(&dir_path).map_err(|e| WorkspaceError::IoError {
                            reason: format!(
                                "failed to list mount directory {}: {e}",
                                dir_path.display()
                            ),
                        })?;
                    for entry in read_dir {
                        let entry = entry.map_err(|e| WorkspaceError::IoError {
                            reason: format!("failed to read mount dir entry: {e}"),
                        })?;
                        let name = entry.file_name().to_string_lossy().to_string();
                        let rel = normalize_mount_path(&if prefix.is_empty() {
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
                                uri: WorkspaceUri::mount_uri(mount_id, Some(&rel)),
                                is_directory,
                                kind: if is_directory {
                                    WorkspaceTreeEntryKind::MountedDirectory
                                } else {
                                    WorkspaceTreeEntryKind::MountedFile
                                },
                                status: None,
                                updated_at: None,
                                content_preview: None,
                                bypass_write: Some(mount.bypass_write),
                                dirty_count: 0,
                                conflict_count: 0,
                                pending_delete_count: 0,
                            },
                        );
                    }
                }

                for record in self
                    .list_mount_file_records(mount_id, Some(&prefix))
                    .await?
                {
                    let relative = if prefix.is_empty() {
                        record.path.clone()
                    } else if let Some(rest) = record.path.strip_prefix(&(prefix.clone() + "/")) {
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
                                uri: WorkspaceUri::mount_uri(mount_id, Some(&child_path)),
                                is_directory,
                                kind: if is_directory {
                                    WorkspaceTreeEntryKind::MountedDirectory
                                } else {
                                    WorkspaceTreeEntryKind::MountedFile
                                },
                                status: None,
                                updated_at: Some(record.updated_at),
                                content_preview: None,
                                bypass_write: Some(mount.bypass_write),
                                dirty_count: 0,
                                conflict_count: 0,
                                pending_delete_count: 0,
                            });
                    if is_directory {
                        entry.is_directory = true;
                        entry.kind = WorkspaceTreeEntryKind::MountedDirectory;
                        entry.dirty_count += usize::from(record.status != MountedFileStatus::Clean);
                        entry.conflict_count +=
                            usize::from(record.status == MountedFileStatus::Conflicted);
                        entry.pending_delete_count +=
                            usize::from(record.status == MountedFileStatus::PendingDelete);
                    } else {
                        entry.status = Some(record.status);
                        entry.updated_at = Some(record.updated_at);
                    }
                }

                Ok(entries_map.into_values().collect())
            }
        }
    }

    async fn create_workspace_mount(
        &self,
        request: &CreateMountRequest,
    ) -> Result<WorkspaceMountSummary, WorkspaceError> {
        let source_root =
            std::fs::canonicalize(&request.source_root).map_err(|e| WorkspaceError::IoError {
                reason: format!("mount source is not accessible: {e}"),
            })?;
        if !source_root.is_dir() {
            return Err(WorkspaceError::IoError {
                reason: "mount source must be a directory".to_string(),
            });
        }
        let mount_id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "INSERT INTO workspace_mounts (id, user_id, display_name, source_root, bypass_read, bypass_write, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?6)",
            params![
                mount_id.to_string(),
                request.user_id.clone(),
                request.display_name.clone(),
                source_root.display().to_string(),
                if request.bypass_write { 1 } else { 0 },
                now
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("mount insert failed: {e}"),
        })?;
        self.list_mount_summaries_internal(&request.user_id)
            .await?
            .into_iter()
            .find(|summary| summary.mount.id == mount_id)
            .ok_or_else(|| WorkspaceError::MountNotFound {
                mount_id: mount_id.to_string(),
            })
    }

    async fn list_workspace_mounts(
        &self,
        user_id: &str,
    ) -> Result<Vec<WorkspaceMountSummary>, WorkspaceError> {
        self.list_mount_summaries_internal(user_id).await
    }

    async fn get_workspace_mount(
        &self,
        user_id: &str,
        mount_id: Uuid,
    ) -> Result<WorkspaceMountDetail, WorkspaceError> {
        self.build_mount_detail_internal(user_id, mount_id).await
    }

    async fn read_workspace_mount_file(
        &self,
        user_id: &str,
        mount_id: Uuid,
        path: &str,
    ) -> Result<WorkspaceMountFileView, WorkspaceError> {
        let record = self
            .ensure_mount_file_loaded(user_id, mount_id, path)
            .await?;
        let snapshot_id =
            record
                .working_snapshot_id
                .ok_or_else(|| WorkspaceError::MountPathNotFound {
                    mount_id: mount_id.to_string(),
                    path: path.to_string(),
                })?;
        let snapshot = self.read_snapshot(snapshot_id).await?.ok_or_else(|| {
            WorkspaceError::MountPathNotFound {
                mount_id: mount_id.to_string(),
                path: path.to_string(),
            }
        })?;
        Ok(WorkspaceMountFileView {
            mount_id,
            path: record.path.clone(),
            uri: WorkspaceUri::mount_uri(mount_id, Some(&record.path)),
            status: record.status,
            is_binary: snapshot.is_binary,
            content: bytes_to_text(&snapshot.content),
            updated_at: record.updated_at,
        })
    }

    async fn write_workspace_mount_file(
        &self,
        user_id: &str,
        mount_id: Uuid,
        path: &str,
        content: &[u8],
    ) -> Result<WorkspaceMountFileView, WorkspaceError> {
        let normalized = normalize_mount_path(path)?;
        let existing = self.load_mount_file_record(mount_id, &normalized).await?;
        let base = match existing.clone() {
            Some(record) => record,
            None => match self
                .ensure_mount_file_loaded(user_id, mount_id, &normalized)
                .await
            {
                Ok(record) => record,
                Err(WorkspaceError::MountPathNotFound { .. }) => MountFileRecord {
                    path: normalized.clone(),
                    status: MountedFileStatus::Added,
                    is_binary: classify_binary(content),
                    base_snapshot_id: None,
                    working_snapshot_id: None,
                    conflict_reason: None,
                    updated_at: Utc::now(),
                },
                Err(error) => return Err(error),
            },
        };
        let working_snapshot = self.insert_snapshot(mount_id, &normalized, content).await?;
        let status = if working_snapshot.is_binary {
            MountedFileStatus::BinaryModified
        } else if base.base_snapshot_id.is_none() {
            MountedFileStatus::Added
        } else {
            MountedFileStatus::Modified
        };
        let base_hash = match base.base_snapshot_id {
            Some(snapshot_id) => Some(self.read_snapshot_required(snapshot_id).await?.hash),
            None => None,
        };
        let updated = self
            .upsert_mount_file_record(
                mount_id,
                &normalized,
                status,
                working_snapshot.is_binary,
                base.base_snapshot_id,
                Some(working_snapshot.id),
                base_hash.clone(),
                Some(working_snapshot.hash.clone()),
                Some(working_snapshot.hash.clone()),
                None,
            )
            .await?;
        self.read_workspace_mount_file(user_id, mount_id, &updated.path)
            .await
    }

    async fn delete_workspace_mount_file(
        &self,
        user_id: &str,
        mount_id: Uuid,
        path: &str,
    ) -> Result<WorkspaceMountFileView, WorkspaceError> {
        let normalized = normalize_mount_path(path)?;
        let existing = match self.load_mount_file_record(mount_id, &normalized).await? {
            Some(record) => record,
            None => {
                self.ensure_mount_file_loaded(user_id, mount_id, &normalized)
                    .await?
            }
        };
        if existing.base_snapshot_id.is_none() {
            let conn = self
                .connect()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: e.to_string(),
                })?;
            conn.execute(
                "DELETE FROM workspace_mount_files WHERE mount_id = ?1 AND relative_path = ?2",
                params![mount_id.to_string(), normalized.clone()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("failed to drop added mount file state: {e}"),
            })?;
            return Ok(WorkspaceMountFileView {
                mount_id,
                path: normalized.clone(),
                uri: WorkspaceUri::mount_uri(mount_id, Some(&normalized)),
                status: MountedFileStatus::Clean,
                is_binary: existing.is_binary,
                content: None,
                updated_at: Utc::now(),
            });
        }
        let updated = self
            .upsert_mount_file_record(
                mount_id,
                &normalized,
                MountedFileStatus::PendingDelete,
                existing.is_binary,
                existing.base_snapshot_id,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
        Ok(WorkspaceMountFileView {
            mount_id,
            path: updated.path.clone(),
            uri: WorkspaceUri::mount_uri(mount_id, Some(&updated.path)),
            status: updated.status,
            is_binary: updated.is_binary,
            content: None,
            updated_at: updated.updated_at,
        })
    }

    async fn diff_workspace_mount(
        &self,
        user_id: &str,
        mount_id: Uuid,
        scope_path: Option<&str>,
    ) -> Result<WorkspaceMountDiff, WorkspaceError> {
        let mount = self.fetch_mount(user_id, mount_id).await?;
        let prefix = match scope_path {
            Some(path) => Some(normalize_mount_path(path)?),
            None => None,
        };
        let mut entries = Vec::new();
        for record in self
            .list_mount_file_records(mount_id, prefix.as_deref())
            .await?
            .into_iter()
            .filter(|record| record.status != MountedFileStatus::Clean)
        {
            let base_snapshot = match record.base_snapshot_id {
                Some(snapshot_id) => self.read_snapshot(snapshot_id).await?,
                None => None,
            };
            let working_snapshot = match record.working_snapshot_id {
                Some(snapshot_id) => self.read_snapshot(snapshot_id).await?,
                None => None,
            };
            let remote_path = Path::new(&mount.source_root).join(&record.path);
            let remote_content = Self::read_disk_bytes(&remote_path).await?;
            let diff_text = match (
                base_snapshot
                    .as_ref()
                    .and_then(|v| bytes_to_text(&v.content)),
                working_snapshot
                    .as_ref()
                    .and_then(|v| bytes_to_text(&v.content)),
            ) {
                (Some(base), Some(working)) => Some(format!(
                    "--- base/{0}\n+++ working/{0}\n- {1}\n+ {2}",
                    record.path,
                    base.replace('\n', "\n- "),
                    working.replace('\n', "\n+ ")
                )),
                (None, Some(working)) => Some(format!(
                    "+++ working/{}\n+ {}",
                    record.path,
                    working.replace('\n', "\n+ ")
                )),
                _ => None,
            };
            entries.push(MountedFileDiff {
                path: record.path.clone(),
                uri: WorkspaceUri::mount_uri(mount_id, Some(&record.path)),
                status: record.status,
                is_binary: record.is_binary,
                base_content: base_snapshot
                    .as_ref()
                    .and_then(|v| bytes_to_text(&v.content)),
                working_content: working_snapshot
                    .as_ref()
                    .and_then(|v| bytes_to_text(&v.content)),
                remote_content: remote_content.as_ref().and_then(|v| bytes_to_text(v)),
                diff_text,
                conflict_reason: record.conflict_reason.clone(),
            });
        }
        Ok(WorkspaceMountDiff { mount_id, entries })
    }

    async fn create_workspace_checkpoint(
        &self,
        request: &CreateCheckpointRequest,
    ) -> Result<WorkspaceMountCheckpoint, WorkspaceError> {
        let parent = self
            .collect_checkpoint_chain(request.mount_id)
            .await?
            .first()
            .map(|value| value.id);
        let files = self
            .list_mount_file_records(request.mount_id, None)
            .await?
            .into_iter()
            .filter(|record| record.status != MountedFileStatus::Clean)
            .collect::<Vec<_>>();
        let checkpoint_id = Uuid::new_v4();
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "INSERT INTO workspace_mount_checkpoints (id, mount_id, parent_checkpoint_id, label, summary, created_by, is_auto, base_generation, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                checkpoint_id.to_string(),
                request.mount_id.to_string(),
                parent.map(|v| v.to_string()),
                request.label.clone(),
                request.summary.clone(),
                request.created_by.clone(),
                if request.is_auto { 1 } else { 0 },
                0_i64,
                now
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("checkpoint insert failed: {e}"),
        })?;
        for file in &files {
            conn.execute(
                "INSERT INTO workspace_mount_checkpoint_files (checkpoint_id, relative_path, status, snapshot_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    checkpoint_id.to_string(),
                    file.path.clone(),
                    status_to_str(file.status),
                    file.working_snapshot_id.map(|v| v.to_string())
                ],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("checkpoint file insert failed: {e}"),
            })?;
        }
        self.collect_checkpoint_chain(request.mount_id)
            .await?
            .into_iter()
            .find(|checkpoint| checkpoint.id == checkpoint_id)
            .ok_or_else(|| WorkspaceError::SearchFailed {
                reason: "checkpoint missing after create".to_string(),
            })
    }

    async fn keep_workspace_mount(
        &self,
        request: &MountActionRequest,
    ) -> Result<WorkspaceMountDetail, WorkspaceError> {
        let mount = self.fetch_mount(&request.user_id, request.mount_id).await?;
        let target_prefix = match request.scope_path.as_deref() {
            Some(path) => Some(normalize_mount_path(path)?),
            None => None,
        };
        let records = self
            .list_mount_file_records(request.mount_id, target_prefix.as_deref())
            .await?;
        for record in records
            .into_iter()
            .filter(|record| record.status != MountedFileStatus::Clean)
        {
            let disk_path = Path::new(&mount.source_root).join(&record.path);
            match record.status {
                MountedFileStatus::PendingDelete => {
                    let base_snapshot = match record.base_snapshot_id {
                        Some(id) => self.read_snapshot(id).await?,
                        None => None,
                    };
                    if let Some(base_snapshot) = base_snapshot {
                        if let Some(remote_bytes) = Self::read_disk_bytes(&disk_path).await? {
                            let remote_hash = compute_content_hash(&remote_bytes);
                            if remote_hash != base_snapshot.hash {
                                self.upsert_mount_file_record(
                                    request.mount_id,
                                    &record.path,
                                    MountedFileStatus::Conflicted,
                                    record.is_binary,
                                    record.base_snapshot_id,
                                    record.working_snapshot_id,
                                    Some(base_snapshot.hash),
                                    None,
                                    Some(remote_hash),
                                    Some("Disk changed since base snapshot".to_string()),
                                )
                                .await?;
                                continue;
                            }
                        }
                    }
                    if disk_path.exists() {
                        tokio::fs::remove_file(&disk_path).await.map_err(|e| {
                            WorkspaceError::IoError {
                                reason: format!("failed to delete {}: {e}", disk_path.display()),
                            }
                        })?;
                    }
                    let conn = self
                        .connect()
                        .await
                        .map_err(|e| WorkspaceError::SearchFailed {
                            reason: e.to_string(),
                        })?;
                    conn.execute(
                        "DELETE FROM workspace_mount_files WHERE mount_id = ?1 AND relative_path = ?2",
                        params![request.mount_id.to_string(), record.path.clone()],
                    )
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("failed to delete mount file state: {e}"),
                    })?;
                }
                _ => {
                    let working_snapshot = match record.working_snapshot_id {
                        Some(snapshot_id) => self.read_snapshot_required(snapshot_id).await?,
                        None => {
                            return Err(WorkspaceError::MountPathNotFound {
                                mount_id: request.mount_id.to_string(),
                                path: record.path.clone(),
                            });
                        }
                    };
                    let base_snapshot = match record.base_snapshot_id {
                        Some(snapshot_id) => Some(self.read_snapshot_required(snapshot_id).await?),
                        None => None,
                    };
                    let remote_bytes = Self::read_disk_bytes(&disk_path).await?;

                    let final_bytes = if let Some(base_snapshot) = &base_snapshot {
                        if let Some(remote_bytes) = remote_bytes.as_ref() {
                            let remote_hash = compute_content_hash(remote_bytes);
                            if remote_hash == base_snapshot.hash
                                || remote_hash == working_snapshot.hash
                            {
                                working_snapshot.content.clone()
                            } else if working_snapshot.is_binary || base_snapshot.is_binary {
                                self.upsert_mount_file_record(
                                    request.mount_id,
                                    &record.path,
                                    MountedFileStatus::Conflicted,
                                    true,
                                    record.base_snapshot_id,
                                    record.working_snapshot_id,
                                    Some(base_snapshot.hash.clone()),
                                    Some(working_snapshot.hash.clone()),
                                    Some(remote_hash),
                                    Some(
                                        "Binary file changed on disk since base snapshot"
                                            .to_string(),
                                    ),
                                )
                                .await?;
                                continue;
                            } else {
                                let base_text =
                                    bytes_to_text(&base_snapshot.content).ok_or_else(|| {
                                        WorkspaceError::MountConflict {
                                            path: record.path.clone(),
                                            reason: "base snapshot is not valid utf-8".to_string(),
                                        }
                                    })?;
                                let remote_text = bytes_to_text(remote_bytes).ok_or_else(|| {
                                    WorkspaceError::MountConflict {
                                        path: record.path.clone(),
                                        reason: "disk file is not valid utf-8".to_string(),
                                    }
                                })?;
                                let working_text = bytes_to_text(&working_snapshot.content)
                                    .ok_or_else(|| WorkspaceError::MountConflict {
                                        path: record.path.clone(),
                                        reason: "working file is not valid utf-8".to_string(),
                                    })?;
                                match three_way_merge_text(&base_text, &remote_text, &working_text)
                                {
                                    Ok(merged) => merged.into_bytes(),
                                    Err(reason) => {
                                        self.upsert_mount_file_record(
                                            request.mount_id,
                                            &record.path,
                                            MountedFileStatus::Conflicted,
                                            false,
                                            record.base_snapshot_id,
                                            record.working_snapshot_id,
                                            Some(base_snapshot.hash.clone()),
                                            Some(working_snapshot.hash.clone()),
                                            Some(remote_hash),
                                            Some(reason),
                                        )
                                        .await?;
                                        continue;
                                    }
                                }
                            }
                        } else {
                            working_snapshot.content.clone()
                        }
                    } else if let Some(remote_bytes) = remote_bytes.as_ref() {
                        let remote_hash = compute_content_hash(remote_bytes);
                        if remote_hash != working_snapshot.hash {
                            self.upsert_mount_file_record(
                                request.mount_id,
                                &record.path,
                                MountedFileStatus::Conflicted,
                                working_snapshot.is_binary,
                                None,
                                record.working_snapshot_id,
                                None,
                                Some(working_snapshot.hash.clone()),
                                Some(remote_hash),
                                Some("File was created in both workspace and disk with different content".to_string()),
                            )
                            .await?;
                            continue;
                        }
                        working_snapshot.content.clone()
                    } else {
                        working_snapshot.content.clone()
                    };
                    if let Some(parent) = disk_path.parent() {
                        tokio::fs::create_dir_all(parent).await.map_err(|e| {
                            WorkspaceError::IoError {
                                reason: format!("failed to create {}: {e}", parent.display()),
                            }
                        })?;
                    }
                    tokio::fs::write(&disk_path, &final_bytes)
                        .await
                        .map_err(|e| WorkspaceError::IoError {
                            reason: format!("failed to write {}: {e}", disk_path.display()),
                        })?;
                    let final_snapshot = self
                        .insert_snapshot(request.mount_id, &record.path, &final_bytes)
                        .await?;
                    self.replace_mount_record_with_snapshot(
                        request.mount_id,
                        &record.path,
                        &final_snapshot,
                    )
                    .await?;
                }
            }
        }
        self.build_mount_detail_internal(&request.user_id, request.mount_id)
            .await
    }

    async fn revert_workspace_mount(
        &self,
        request: &MountActionRequest,
    ) -> Result<WorkspaceMountDetail, WorkspaceError> {
        let target_prefix = match request.scope_path.as_deref() {
            Some(path) => Some(normalize_mount_path(path)?),
            None => None,
        };
        let records = self
            .list_mount_file_records(request.mount_id, target_prefix.as_deref())
            .await?;
        let checkpoint_map = if let Some(checkpoint_id) = request.checkpoint_id {
            let conn = self
                .connect()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: e.to_string(),
                })?;
            let mut rows = conn
                .query(
                    "SELECT relative_path, status, snapshot_id FROM workspace_mount_checkpoint_files WHERE checkpoint_id = ?1",
                    params![checkpoint_id.to_string()],
                )
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("checkpoint restore query failed: {e}"),
                })?;
            let mut map = HashMap::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("checkpoint restore query failed: {e}"),
                })?
            {
                map.insert(
                    get_text(&row, 0),
                    (
                        status_from_str(&get_text(&row, 1)),
                        get_opt_text(&row, 2).and_then(|value| Uuid::parse_str(&value).ok()),
                    ),
                );
            }
            Some(map)
        } else {
            None
        };

        for record in records {
            if let Some(checkpoint) = &checkpoint_map {
                if let Some((status, snapshot_id)) = checkpoint.get(&record.path) {
                    self.upsert_mount_file_record(
                        request.mount_id,
                        &record.path,
                        *status,
                        record.is_binary,
                        snapshot_id.or(record.base_snapshot_id),
                        *snapshot_id,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                } else {
                    self.upsert_mount_file_record(
                        request.mount_id,
                        &record.path,
                        MountedFileStatus::Clean,
                        record.is_binary,
                        record.base_snapshot_id,
                        record.base_snapshot_id,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                }
            } else {
                self.upsert_mount_file_record(
                    request.mount_id,
                    &record.path,
                    MountedFileStatus::Clean,
                    record.is_binary,
                    record.base_snapshot_id,
                    record.base_snapshot_id,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
            }
        }
        self.build_mount_detail_internal(&request.user_id, request.mount_id)
            .await
    }

    async fn resolve_workspace_mount_conflict(
        &self,
        request: &ConflictResolutionRequest,
    ) -> Result<WorkspaceMountDetail, WorkspaceError> {
        let mount = self.fetch_mount(&request.user_id, request.mount_id).await?;
        let record = self
            .load_mount_file_record(request.mount_id, &normalize_mount_path(&request.path)?)
            .await?
            .ok_or_else(|| WorkspaceError::MountPathNotFound {
                mount_id: request.mount_id.to_string(),
                path: request.path.clone(),
            })?;
        match request.resolution.as_str() {
            "keep_disk" => {
                let disk_path = Path::new(&mount.source_root).join(&record.path);
                self.sync_record_to_disk_snapshot(request.mount_id, &record.path, &disk_path)
                    .await?;
            }
            "keep_workspace" => {
                let snapshot = match record.working_snapshot_id {
                    Some(snapshot_id) => self.read_snapshot_required(snapshot_id).await?,
                    None => {
                        return Err(WorkspaceError::MountConflict {
                            path: record.path.clone(),
                            reason: "missing working snapshot".to_string(),
                        });
                    }
                };
                let disk_path = Path::new(&mount.source_root).join(&record.path);
                if let Some(parent) = disk_path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        WorkspaceError::IoError {
                            reason: format!("failed to create {}: {e}", parent.display()),
                        }
                    })?;
                }
                tokio::fs::write(&disk_path, &snapshot.content)
                    .await
                    .map_err(|e| WorkspaceError::IoError {
                        reason: format!(
                            "failed to write conflict resolution {}: {e}",
                            disk_path.display()
                        ),
                    })?;
                let final_snapshot = self
                    .insert_snapshot(request.mount_id, &record.path, &snapshot.content)
                    .await?;
                self.replace_mount_record_with_snapshot(
                    request.mount_id,
                    &record.path,
                    &final_snapshot,
                )
                .await?;
            }
            "write_copy" => {
                let copy_path = request.renamed_copy_path.clone().ok_or_else(|| {
                    WorkspaceError::MountConflict {
                        path: record.path.clone(),
                        reason: "renamed_copy_path is required".to_string(),
                    }
                })?;
                let copy_path = normalize_mount_path(&copy_path)?;
                let snapshot = match record.working_snapshot_id {
                    Some(snapshot_id) => self.read_snapshot_required(snapshot_id).await?,
                    None => {
                        return Err(WorkspaceError::MountConflict {
                            path: record.path.clone(),
                            reason: "missing working snapshot".to_string(),
                        });
                    }
                };
                let disk_path = Path::new(&mount.source_root).join(&copy_path);
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
                        reason: format!("failed to write copy {}: {e}", disk_path.display()),
                    })?;
                let original_disk_path = Path::new(&mount.source_root).join(&record.path);
                self.sync_record_to_disk_snapshot(
                    request.mount_id,
                    &record.path,
                    &original_disk_path,
                )
                .await?;
            }
            "manual_merge" => {
                let merged_content = request.merged_content.clone().ok_or_else(|| {
                    WorkspaceError::MountConflict {
                        path: record.path.clone(),
                        reason: "merged_content is required".to_string(),
                    }
                })?;
                let disk_path = Path::new(&mount.source_root).join(&record.path);
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
                let snapshot = self
                    .insert_snapshot(request.mount_id, &record.path, merged_content.as_bytes())
                    .await?;
                self.replace_mount_record_with_snapshot(request.mount_id, &record.path, &snapshot)
                    .await?;
            }
            other => {
                return Err(WorkspaceError::MountConflict {
                    path: record.path.clone(),
                    reason: format!("unknown resolution '{other}'"),
                });
            }
        }
        self.build_mount_detail_internal(&request.user_id, request.mount_id)
            .await
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
    async fn test_workspace_mount_round_trip_diff_keep_revert() {
        let (backend, dir) = setup_backend().await;
        let mount_root = dir.path().join("mounted-project");
        std::fs::create_dir_all(&mount_root).expect("create mount root");
        std::fs::write(
            mount_root.join("main.rs"),
            "fn main() {\n    println!(\"v1\");\n}\n",
        )
        .expect("seed file");

        let mount = backend
            .create_workspace_mount(&CreateMountRequest {
                user_id: "default".to_string(),
                display_name: "project".to_string(),
                source_root: mount_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create mount");

        let file = backend
            .read_workspace_mount_file("default", mount.mount.id, "main.rs")
            .await
            .expect("read mounted file");
        assert!(file.content.as_deref().is_some_and(|v| v.contains("v1")));

        backend
            .write_workspace_mount_file(
                "default",
                mount.mount.id,
                "main.rs",
                b"fn main() {\n    println!(\"v2\");\n}\n",
            )
            .await
            .expect("write mounted file");

        let diff = backend
            .diff_workspace_mount("default", mount.mount.id, None)
            .await
            .expect("diff mount");
        assert_eq!(diff.entries.len(), 1);
        assert!(
            diff.entries[0]
                .working_content
                .as_deref()
                .is_some_and(|v| v.contains("v2"))
        );

        backend
            .revert_workspace_mount(&MountActionRequest {
                user_id: "default".to_string(),
                mount_id: mount.mount.id,
                scope_path: Some("main.rs".to_string()),
                checkpoint_id: None,
            })
            .await
            .expect("revert file");

        let reverted = backend
            .read_workspace_mount_file("default", mount.mount.id, "main.rs")
            .await
            .expect("read reverted file");
        assert!(
            reverted
                .content
                .as_deref()
                .is_some_and(|v| v.contains("v1"))
        );

        backend
            .write_workspace_mount_file(
                "default",
                mount.mount.id,
                "main.rs",
                b"fn main() {\n    println!(\"kept\");\n}\n",
            )
            .await
            .expect("write mounted file again");
        backend
            .keep_workspace_mount(&MountActionRequest {
                user_id: "default".to_string(),
                mount_id: mount.mount.id,
                scope_path: Some("main.rs".to_string()),
                checkpoint_id: None,
            })
            .await
            .expect("keep file");

        let disk_content = std::fs::read_to_string(mount_root.join("main.rs")).expect("read disk");
        assert!(disk_content.contains("kept"));
    }

    #[tokio::test]
    async fn test_workspace_root_lists_mounts_directly() {
        let (backend, dir) = setup_backend().await;
        let mount_root = dir.path().join("root-project");
        std::fs::create_dir_all(&mount_root).expect("create mount root");

        let mount = backend
            .create_workspace_mount(&CreateMountRequest {
                user_id: "default".to_string(),
                display_name: "root-project".to_string(),
                source_root: mount_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create mount");

        let entries = backend
            .list_workspace_tree("default", None, "workspace://")
            .await
            .expect("list workspace root");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, WorkspaceTreeEntryKind::Mount);
        assert_eq!(entries[0].path, mount.mount.id.to_string());
        assert_eq!(
            entries[0].uri,
            WorkspaceUri::mount_uri(mount.mount.id, None)
        );

        let legacy_entries = backend
            .list_workspace_tree("default", None, "workspace://mounts")
            .await
            .expect("list legacy workspace root");
        assert_eq!(legacy_entries.len(), 1);
        assert_eq!(
            legacy_entries[0].uri,
            WorkspaceUri::mount_uri(mount.mount.id, None)
        );
    }

    #[tokio::test]
    async fn test_workspace_mount_rejects_parent_escape() {
        let (backend, dir) = setup_backend().await;
        let mount_root = dir.path().join("escape-project");
        std::fs::create_dir_all(&mount_root).expect("create mount root");
        std::fs::write(dir.path().join("secret.txt"), "secret").expect("seed sibling file");

        let mount = backend
            .create_workspace_mount(&CreateMountRequest {
                user_id: "default".to_string(),
                display_name: "escape-project".to_string(),
                source_root: mount_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create mount");

        let read_err = backend
            .read_workspace_mount_file("default", mount.mount.id, "../secret.txt")
            .await
            .expect_err("reject escaped read");
        assert!(read_err.to_string().contains("escapes root"));

        let write_err = backend
            .write_workspace_mount_file("default", mount.mount.id, "../written.txt", b"owned")
            .await
            .expect_err("reject escaped write");
        assert!(write_err.to_string().contains("escapes root"));
        assert!(
            !dir.path().join("written.txt").exists(),
            "escaped write must not create files outside the mount"
        );

        let tree_err = backend
            .list_workspace_tree(
                "default",
                None,
                &format!("workspace://{}/../secret.txt", mount.mount.id),
            )
            .await
            .expect_err("reject escaped tree path");
        assert!(tree_err.to_string().contains("escapes root"));
    }

    #[tokio::test]
    async fn test_workspace_mount_delete_is_pending_until_keep() {
        let (backend, dir) = setup_backend().await;
        let mount_root = dir.path().join("delete-project");
        std::fs::create_dir_all(&mount_root).expect("create mount root");
        std::fs::write(mount_root.join("delete.txt"), "hello").expect("seed file");

        let mount = backend
            .create_workspace_mount(&CreateMountRequest {
                user_id: "default".to_string(),
                display_name: "delete-project".to_string(),
                source_root: mount_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create mount");

        backend
            .delete_workspace_mount_file("default", mount.mount.id, "delete.txt")
            .await
            .expect("mark pending delete");
        assert!(
            mount_root.join("delete.txt").exists(),
            "disk file should remain until keep"
        );

        backend
            .keep_workspace_mount(&MountActionRequest {
                user_id: "default".to_string(),
                mount_id: mount.mount.id,
                scope_path: Some("delete.txt".to_string()),
                checkpoint_id: None,
            })
            .await
            .expect("apply delete");
        assert!(
            !mount_root.join("delete.txt").exists(),
            "disk file should be deleted after keep"
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

    #[test]
    fn test_three_way_merge_text_merges_non_overlapping_changes() {
        let base = "a\nb\nc\n";
        let remote = "a\nb-remote\nc\n";
        let working = "a\nb\nc-working\n";
        let merged = three_way_merge_text(base, remote, working).expect("merge should succeed");
        assert_eq!(merged, "a\nb-remote\nc-working\n");
    }

    #[test]
    fn test_three_way_merge_text_conflicts_on_same_region() {
        let base = "a\nb\nc\n";
        let remote = "a\nremote\nc\n";
        let working = "a\nworking\nc\n";
        let err = three_way_merge_text(base, remote, working).expect_err("merge should conflict");
        assert!(err.contains("Text conflict"));
    }

    #[tokio::test]
    async fn test_workspace_mount_keep_auto_merges_remote_text_change() {
        let (backend, dir) = setup_backend().await;
        let mount_root = dir.path().join("merge-project");
        std::fs::create_dir_all(&mount_root).expect("create mount root");
        std::fs::write(mount_root.join("main.txt"), "a\nb\nc\n").expect("seed file");

        let mount = backend
            .create_workspace_mount(&CreateMountRequest {
                user_id: "default".to_string(),
                display_name: "merge-project".to_string(),
                source_root: mount_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create mount");

        backend
            .read_workspace_mount_file("default", mount.mount.id, "main.txt")
            .await
            .expect("prime base");
        backend
            .write_workspace_mount_file("default", mount.mount.id, "main.txt", b"a\nb\nc-local\n")
            .await
            .expect("write local");

        std::fs::write(mount_root.join("main.txt"), "a\nb-remote\nc\n").expect("remote edit");

        backend
            .keep_workspace_mount(&MountActionRequest {
                user_id: "default".to_string(),
                mount_id: mount.mount.id,
                scope_path: Some("main.txt".to_string()),
                checkpoint_id: None,
            })
            .await
            .expect("keep merged");

        let disk = std::fs::read_to_string(mount_root.join("main.txt")).expect("read merged disk");
        assert_eq!(disk, "a\nb-remote\nc-local\n");
    }

    #[tokio::test]
    async fn test_workspace_mount_write_new_file_then_keep() {
        let (backend, dir) = setup_backend().await;
        let mount_root = dir.path().join("new-file-project");
        std::fs::create_dir_all(&mount_root).expect("create mount root");

        let mount = backend
            .create_workspace_mount(&CreateMountRequest {
                user_id: "default".to_string(),
                display_name: "new-file-project".to_string(),
                source_root: mount_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create mount");

        backend
            .write_workspace_mount_file(
                "default",
                mount.mount.id,
                "nested/new.txt",
                b"hello world\n",
            )
            .await
            .expect("write new mounted file");
        backend
            .keep_workspace_mount(&MountActionRequest {
                user_id: "default".to_string(),
                mount_id: mount.mount.id,
                scope_path: Some("nested/new.txt".to_string()),
                checkpoint_id: None,
            })
            .await
            .expect("keep new file");

        let disk = std::fs::read_to_string(mount_root.join("nested/new.txt")).expect("read disk");
        assert_eq!(disk, "hello world\n");
    }
}
