//! Workspace-related WorkspaceStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use uuid::Uuid;

use super::workspace_tracker::AllowlistTrackerStatus;
use super::{
    LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_opt_ts, get_text, get_ts,
    row_to_memory_document,
};
use crate::db::WorkspaceStore;
use crate::error::{DatabaseError, WorkspaceError};
use crate::tools::builtin::path_utils::{canonicalize_stripped, normalize_lexical};
#[cfg(test)]
use crate::workspace::WorkspaceAllowlistChangeKind;
use crate::workspace::{
    AllowlistActionRequest, AllowlistedFileStatus, ConflictResolutionRequest,
    CreateAllowlistRequest, CreateCheckpointRequest, MemoryChunk, MemoryDocument, RankedResult,
    SearchConfig, SearchResult, WorkspaceAllowlist, WorkspaceAllowlistBaselineRequest,
    WorkspaceAllowlistCheckpoint, WorkspaceAllowlistDetail, WorkspaceAllowlistDiff,
    WorkspaceAllowlistDiffRequest, WorkspaceAllowlistFileView, WorkspaceAllowlistHistory,
    WorkspaceAllowlistHistoryRequest, WorkspaceAllowlistRestoreRequest, WorkspaceAllowlistRevision,
    WorkspaceAllowlistRevisionKind, WorkspaceAllowlistRevisionSource, WorkspaceAllowlistSummary,
    WorkspaceEntry, WorkspaceMountKind, WorkspaceTreeEntry, WorkspaceTreeEntryKind, WorkspaceUri,
    fuse_results, normalize_allowlist_path,
};

use chrono::Utc;

/// Verify that `disk_path` (after canonicalization) still lives under `source_root`.
/// This prevents symlink escapes from the allowlist boundary.
fn ensure_allowlist_containment(
    disk_path: &std::path::Path,
    source_root: &str,
) -> Result<(), WorkspaceError> {
    let root = std::path::Path::new(source_root);
    let canonical_root = canonicalize_stripped(root).map_err(|e| WorkspaceError::IoError {
        reason: format!("failed to canonicalize allowlist root: {e}"),
    })?;

    let canonical_path =
        canonicalize_stripped(disk_path).unwrap_or_else(|_| normalize_lexical(disk_path));

    if !canonical_path.starts_with(&canonical_root) {
        return Err(WorkspaceError::IoError {
            reason: format!("path escapes allowlist: {}", disk_path.display()),
        });
    }
    Ok(())
}

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
struct AllowlistFileRecord {
    path: String,
    status: AllowlistedFileStatus,
    is_binary: bool,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub(crate) struct AllowlistStateRecord {
    pub baseline_revision_id: Option<Uuid>,
    pub head_revision_id: Option<Uuid>,
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

fn revision_source_from_str(value: &str) -> WorkspaceAllowlistRevisionSource {
    match value {
        "shell" => WorkspaceAllowlistRevisionSource::Shell,
        "external" => WorkspaceAllowlistRevisionSource::External,
        "system" => WorkspaceAllowlistRevisionSource::System,
        _ => WorkspaceAllowlistRevisionSource::WorkspaceTool,
    }
}

fn mount_kind_from_str(value: &str) -> WorkspaceMountKind {
    match value {
        "default" => WorkspaceMountKind::Default,
        "skills" => WorkspaceMountKind::Skills,
        _ => WorkspaceMountKind::User,
    }
}

fn mount_kind_to_str(value: WorkspaceMountKind) -> &'static str {
    match value {
        WorkspaceMountKind::User => "user",
        WorkspaceMountKind::Default => "default",
        WorkspaceMountKind::Skills => "skills",
    }
}

fn mount_kind_label(value: WorkspaceMountKind) -> &'static str {
    match value {
        WorkspaceMountKind::User => "User",
        WorkspaceMountKind::Default => "Default",
        WorkspaceMountKind::Skills => "Skills",
    }
}

fn fixed_allowlist_id_for_mount_kind(mount_kind: WorkspaceMountKind) -> Option<Uuid> {
    match mount_kind {
        WorkspaceMountKind::User => None,
        WorkspaceMountKind::Default => Some(crate::workspace::default_allowlist_uuid()),
        WorkspaceMountKind::Skills => Some(crate::workspace::skills_allowlist_uuid()),
    }
}

fn mount_kind_supports_tracking(mount_kind: WorkspaceMountKind) -> bool {
    mount_kind == WorkspaceMountKind::User
}

fn allowlist_supports_tracking(allowlist: &WorkspaceAllowlist) -> bool {
    mount_kind_supports_tracking(allowlist.mount_kind)
}

fn scope_matches(path: &str, scope: Option<&str>) -> bool {
    match scope {
        None => true,
        Some(scope) if scope.is_empty() => true,
        Some(scope) => path == scope || path.starts_with(&format!("{scope}/")),
    }
}

impl LibSqlBackend {
    async fn rekey_allowlist_id(
        tx: &libsql::Transaction,
        from_id: Uuid,
        to_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        if from_id == to_id {
            return Ok(());
        }

        tx.execute_batch("PRAGMA defer_foreign_keys = ON;")
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("failed to defer allowlist foreign keys: {e}"),
            })?;

        for table in [
            "workspace_allowlist_files",
            "workspace_allowlist_checkpoints",
            "workspace_allowlist_revisions",
            "workspace_allowlist_trackers",
            "workspace_allowlist_revision_anchors",
        ] {
            let sql = format!("UPDATE {table} SET allowlist_id = ?2 WHERE allowlist_id = ?1");
            tx.execute(&sql, params![from_id.to_string(), to_id.to_string()])
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("failed to rekey {table}: {e}"),
                })?;
        }

        tx.execute(
            "UPDATE workspace_allowlists SET id = ?2 WHERE id = ?1",
            params![from_id.to_string(), to_id.to_string()],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("failed to rekey workspace_allowlists: {e}"),
        })?;

        Ok(())
    }

    async fn fetch_allowlist_for_tracking(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        operation: &str,
    ) -> Result<WorkspaceAllowlist, WorkspaceError> {
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        if !allowlist_supports_tracking(&allowlist) {
            return Err(WorkspaceError::Unsupported {
                operation: format!(
                    "{operation} is not available for {} mounts",
                    mount_kind_label(allowlist.mount_kind)
                ),
            });
        }
        Ok(allowlist)
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
        Ok(self
            .load_allowlist_tracker(allowlist_id)
            .await?
            .map(|tracker| AllowlistStateRecord {
                baseline_revision_id: tracker.baseline_revision_id,
                head_revision_id: tracker.head_revision_id,
            }))
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
                   AND id IN (
                       SELECT product_revision_id
                       FROM workspace_allowlist_revision_anchors
                       WHERE allowlist_id = ?1
                   )
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
                   AND id IN (
                       SELECT product_revision_id
                       FROM workspace_allowlist_revision_anchors
                       WHERE allowlist_id = ?1
                   )
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

    async fn sync_allowlist_live_cache(
        &self,
        allowlist_id: Uuid,
        _baseline_revision_id: Option<Uuid>,
        _head_revision_id: Option<Uuid>,
    ) -> Result<(), WorkspaceError> {
        self.rebuild_allowlist_live_cache_from_tracker(allowlist_id)
            .await
    }

    pub(crate) async fn get_checkpoint_record(
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
        if self
            .load_tracker_anchor_for_revision(allowlist_id, revision_id)
            .await?
            .is_none()
        {
            return Ok(None);
        }
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

    async fn ensure_allowlist_initialized(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<AllowlistStateRecord, WorkspaceError> {
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        if !allowlist_supports_tracking(&allowlist) {
            return Ok(AllowlistStateRecord {
                baseline_revision_id: None,
                head_revision_id: None,
            });
        }
        let tracker = self.ensure_allowlist_tracker(user_id, allowlist_id).await?;
        self.sync_allowlist_live_cache(
            allowlist_id,
            tracker.baseline_revision_id,
            tracker.head_revision_id,
        )
        .await?;
        self.touch_allowlist_updated_at(allowlist_id).await?;

        Ok(AllowlistStateRecord {
            baseline_revision_id: tracker.baseline_revision_id,
            head_revision_id: tracker.head_revision_id,
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
        let created_by = created_by.into();
        let state = self
            .sync_allowlist_from_tracker(
                user_id,
                allowlist_id,
                None,
                None,
                kind,
                source,
                trigger,
                summary,
                &created_by,
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

    pub(crate) async fn fetch_allowlist(
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
        let query = "SELECT id, user_id, display_name, mount_kind, source_root, bypass_read, bypass_write, created_at, updated_at
             FROM workspace_allowlists
             WHERE user_id = ?1 AND id = ?2";
        let mut rows = conn
            .query(query, params![user_id, allowlist_id.to_string()])
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
            mount_kind: mount_kind_from_str(&get_text(&row, 3)),
            source_root: get_text(&row, 4),
            bypass_read: get_i64(&row, 5) != 0,
            bypass_write: get_i64(&row, 6) != 0,
            created_at: get_ts(&row, 7),
            updated_at: get_ts(&row, 8),
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
                SELECT m.id, m.user_id, m.display_name, m.mount_kind, m.source_root, m.bypass_read, m.bypass_write,
                       m.created_at, m.updated_at,
                       COALESCE(SUM(CASE WHEN f.status != 'clean' THEN 1 ELSE 0 END), 0),
                       COALESCE(SUM(CASE WHEN f.status = 'conflicted' THEN 1 ELSE 0 END), 0),
                       COALESCE(SUM(CASE WHEN f.status = 'deleted' THEN 1 ELSE 0 END), 0)
                FROM workspace_allowlists m
                LEFT JOIN workspace_allowlist_files f ON f.allowlist_id = m.id
                WHERE m.user_id = ?1
                GROUP BY m.id, m.user_id, m.display_name, m.mount_kind, m.source_root, m.bypass_read, m.bypass_write,
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
                mount_kind: mount_kind_from_str(&get_text(&row, 3)),
                source_root: get_text(&row, 4),
                bypass_read: get_i64(&row, 5) != 0,
                bypass_write: get_i64(&row, 6) != 0,
                created_at: get_ts(&row, 7),
                updated_at: get_ts(&row, 8),
            };
            let tracking_enabled = allowlist_supports_tracking(&allowlist);
            result.push(WorkspaceAllowlistSummary {
                allowlist,
                dirty_count: if tracking_enabled {
                    get_i64(&row, 9) as usize
                } else {
                    0
                },
                conflict_count: if tracking_enabled {
                    get_i64(&row, 10) as usize
                } else {
                    0
                },
                pending_delete_count: if tracking_enabled {
                    get_i64(&row, 11) as usize
                } else {
                    0
                },
            });
        }
        Ok(result)
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
                "SELECT relative_path, status, is_binary, updated_at
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
            updated_at: get_ts(&row, 3),
        }))
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
                "SELECT relative_path, status, is_binary, updated_at
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
                updated_at: get_ts(&row, 3),
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
            if self
                .load_tracker_anchor_for_revision(allowlist_id, revision_id)
                .await?
                .is_none()
            {
                continue;
            }
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
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
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
        let checkpoints = if allowlist_supports_tracking(&allowlist) {
            self.collect_checkpoint_chain(allowlist_id).await?
        } else {
            Vec::new()
        };
        Ok(WorkspaceAllowlistDetail {
            open_change_count: if allowlist_supports_tracking(&allowlist) {
                summary.dirty_count
            } else {
                0
            },
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
             CREATE INDEX IF NOT EXISTS idx_memory_chunks_embedding ON memory_chunks(libsql_vector_idx(embedding, 'metric=cosine'));",
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
                        mount_kind: None,
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
                            path: crate::workspace::public_allowlist_id(
                                summary.allowlist.id,
                                summary.allowlist.mount_kind,
                            ),
                            uri: WorkspaceUri::allowlist_uri_with_mount_kind(
                                summary.allowlist.id,
                                summary.allowlist.mount_kind,
                                None,
                            ),
                            is_directory: true,
                            kind: WorkspaceTreeEntryKind::Allowlist,
                            mount_kind: Some(summary.allowlist.mount_kind),
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
                let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
                let has_allowlist_state = allowlist_supports_tracking(&allowlist)
                    && self
                        .load_allowlist_state_record(allowlist_id)
                        .await?
                        .is_some();
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
                                uri: WorkspaceUri::allowlist_uri_with_mount_kind(
                                    allowlist_id,
                                    allowlist.mount_kind,
                                    Some(&rel),
                                ),
                                is_directory,
                                kind: if is_directory {
                                    WorkspaceTreeEntryKind::AllowlistedDirectory
                                } else {
                                    WorkspaceTreeEntryKind::AllowlistedFile
                                },
                                mount_kind: Some(allowlist.mount_kind),
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
                                    uri: WorkspaceUri::allowlist_uri_with_mount_kind(
                                        allowlist_id,
                                        allowlist.mount_kind,
                                        Some(&child_path),
                                    ),
                                    is_directory,
                                    kind: if is_directory {
                                        WorkspaceTreeEntryKind::AllowlistedDirectory
                                    } else {
                                        WorkspaceTreeEntryKind::AllowlistedFile
                                    },
                                    mount_kind: Some(allowlist.mount_kind),
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
                let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
                let has_allowlist_state = allowlist_supports_tracking(&allowlist)
                    && self
                        .load_allowlist_state_record(allowlist_id)
                        .await?
                        .is_some();
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
                                uri: WorkspaceUri::allowlist_uri_with_mount_kind(
                                    allowlist_id,
                                    allowlist.mount_kind,
                                    Some(&rel),
                                ),
                                is_directory,
                                kind: if is_directory {
                                    WorkspaceTreeEntryKind::AllowlistedDirectory
                                } else {
                                    WorkspaceTreeEntryKind::AllowlistedFile
                                },
                                mount_kind: Some(allowlist.mount_kind),
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
                                    uri: WorkspaceUri::allowlist_uri_with_mount_kind(
                                        allowlist_id,
                                        allowlist.mount_kind,
                                        Some(&child_path),
                                    ),
                                    is_directory,
                                    kind: if is_directory {
                                        WorkspaceTreeEntryKind::AllowlistedDirectory
                                    } else {
                                        WorkspaceTreeEntryKind::AllowlistedFile
                                    },
                                    mount_kind: Some(allowlist.mount_kind),
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
        let source_root = canonicalize_stripped(Path::new(&request.source_root)).map_err(|e| {
            WorkspaceError::IoError {
                reason: format!("allowlist source is not accessible: {e}"),
            }
        })?;
        if !source_root.is_dir() {
            return Err(WorkspaceError::IoError {
                reason: "allowlist source must be a directory".to_string(),
            });
        }
        let requested_allowlist_id =
            fixed_allowlist_id_for_mount_kind(request.mount_kind).unwrap_or_else(Uuid::new_v4);
        let now = fmt_ts(&Utc::now());
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let source_root_text = source_root.display().to_string();

        let mut existing_rows = conn
            .query(
                "SELECT id FROM workspace_allowlists WHERE user_id = ?1 AND source_root = ?2 LIMIT 1",
                params![request.user_id.clone(), source_root_text.clone()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist lookup failed: {e}"),
            })?;
        if let Some(row) = existing_rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("allowlist lookup failed: {e}"),
            })?
        {
            let existing_id =
                Uuid::parse_str(&get_text(&row, 0)).map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("invalid allowlist id: {e}"),
                })?;
            // Explicitly drop the row (and its underlying statement clone) before
            // executing the UPDATE so the read statement is fully released. This
            // avoids subtle SQLite transaction-interaction issues on Windows where
            // an outstanding read cursor can interfere with the implicit write
            // transaction started by the UPDATE.
            drop(row);
            drop(existing_rows);
            let allowlist_id = if fixed_allowlist_id_for_mount_kind(request.mount_kind).is_some()
                && existing_id != requested_allowlist_id
            {
                let tx = conn
                    .transaction()
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("allowlist rekey transaction failed: {e}"),
                    })?;
                Self::rekey_allowlist_id(&tx, existing_id, requested_allowlist_id).await?;
                tx.execute(
                    "UPDATE workspace_allowlists
                     SET display_name = ?, mount_kind = ?, bypass_write = ?, updated_at = ?
                     WHERE id = ?",
                    params![
                        request.display_name.clone(),
                        mount_kind_to_str(request.mount_kind),
                        if request.bypass_write { 1 } else { 0 },
                        now.clone(),
                        requested_allowlist_id.to_string(),
                    ],
                )
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("allowlist update failed after rekey: {e}"),
                })?;
                tx.commit()
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("allowlist rekey commit failed: {e}"),
                    })?;
                requested_allowlist_id
            } else {
                let rows_affected = conn
                    .execute(
                        "UPDATE workspace_allowlists
                         SET display_name = ?, mount_kind = ?, bypass_write = ?, updated_at = ?
                         WHERE user_id = ? AND source_root = ?",
                        params![
                            request.display_name.clone(),
                            mount_kind_to_str(request.mount_kind),
                            if request.bypass_write { 1 } else { 0 },
                            now.clone(),
                            request.user_id.clone(),
                            source_root_text,
                        ],
                    )
                    .await
                    .map_err(|e| WorkspaceError::SearchFailed {
                        reason: format!("allowlist update failed: {e}"),
                    })?;
                if rows_affected == 0 {
                    return Err(WorkspaceError::SearchFailed {
                        reason: format!(
                            "allowlist update affected 0 rows: source_root={}",
                            request.source_root
                        ),
                    });
                }
                existing_id
            };
            if mount_kind_supports_tracking(request.mount_kind) {
                self.ensure_allowlist_initialized(&request.user_id, allowlist_id)
                    .await?;
            }
            return self
                .get_workspace_allowlist(&request.user_id, allowlist_id)
                .await
                .map(|detail| detail.summary);
        }

        conn.execute(
            "INSERT INTO workspace_allowlists (id, user_id, display_name, mount_kind, source_root, bypass_read, bypass_write, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?7)",
            params![
                requested_allowlist_id.to_string(),
                request.user_id.clone(),
                request.display_name.clone(),
                mount_kind_to_str(request.mount_kind),
                source_root_text,
                if request.bypass_write { 1 } else { 0 },
                now
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("allowlist insert failed: {e}"),
        })?;
        if mount_kind_supports_tracking(request.mount_kind) {
            self.ensure_allowlist_initialized(&request.user_id, requested_allowlist_id)
                .await?;
        }
        self.list_allowlist_summaries_internal(&request.user_id)
            .await?
            .into_iter()
            .find(|summary| summary.allowlist.id == requested_allowlist_id)
            .ok_or_else(|| WorkspaceError::AllowlistNotFound {
                allowlist_id: requested_allowlist_id.to_string(),
            })
    }

    async fn list_workspace_allowlists(
        &self,
        user_id: &str,
    ) -> Result<Vec<WorkspaceAllowlistSummary>, WorkspaceError> {
        let summaries = self.list_allowlist_summaries_internal(user_id).await?;
        for summary in &summaries {
            if !allowlist_supports_tracking(&summary.allowlist) {
                continue;
            }
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
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        if allowlist_supports_tracking(&allowlist) {
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
        }
        self.build_allowlist_detail_internal(user_id, allowlist_id)
            .await
    }

    async fn delete_workspace_allowlist(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        if allowlist.mount_kind != WorkspaceMountKind::User {
            return Err(WorkspaceError::Unsupported {
                operation: format!(
                    "delete_workspace_allowlist is not available for {} mounts",
                    mount_kind_label(allowlist.mount_kind)
                ),
            });
        }

        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let rows_affected = conn
            .execute(
                "DELETE FROM workspace_allowlists WHERE user_id = ?1 AND id = ?2",
                params![user_id, allowlist_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("failed to delete allowlist: {e}"),
            })?;
        if rows_affected == 0 {
            return Err(WorkspaceError::AllowlistNotFound {
                allowlist_id: allowlist_id.to_string(),
            });
        }

        let tracker_root = crate::bootstrap::steward_base_dir()
            .join("workspace-trackers")
            .join(allowlist_id.to_string());
        if let Err(error) = tokio::fs::remove_dir_all(&tracker_root).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::debug!(
                    "failed to remove tracker data for allowlist {}: {}",
                    allowlist_id,
                    error
                );
            }
        }

        Ok(())
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
        ensure_allowlist_containment(&disk_path, &allowlist.source_root)?;
        let bytes = Self::read_disk_bytes(&disk_path).await?.ok_or_else(|| {
            WorkspaceError::AllowlistPathNotFound {
                allowlist_id: allowlist_id.to_string(),
                path: normalized.clone(),
            }
        })?;
        let record = if allowlist_supports_tracking(&allowlist) {
            self.load_allowlist_file_record(allowlist_id, &normalized)
                .await?
        } else {
            None
        };
        let status = record
            .as_ref()
            .map(|value| value.status)
            .unwrap_or(AllowlistedFileStatus::Clean);
        let is_binary = classify_binary(&bytes);
        Ok(WorkspaceAllowlistFileView {
            allowlist_id,
            path: normalized.clone(),
            uri: WorkspaceUri::allowlist_uri_with_mount_kind(
                allowlist_id,
                allowlist.mount_kind,
                Some(&normalized),
            ),
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
        ensure_allowlist_containment(&disk_path, &allowlist.source_root)?;
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
        if allowlist_supports_tracking(&allowlist) {
            let tracker = self.ensure_allowlist_tracker(user_id, allowlist_id).await?;
            self.sync_allowlist_from_tracker(
                user_id,
                allowlist_id,
                Some(&normalized),
                Some(vec![tracker.repo_path_for_allowlist_path(&normalized)]),
                WorkspaceAllowlistRevisionKind::ToolWrite,
                WorkspaceAllowlistRevisionSource::WorkspaceTool,
                Some(normalized.clone()),
                Some(format!("updated {}", normalized)),
                "workspace_write",
            )
            .await?;
        }
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
        ensure_allowlist_containment(&disk_path, &allowlist.source_root)?;
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
        if allowlist_supports_tracking(&allowlist) {
            let tracker = self.ensure_allowlist_tracker(user_id, allowlist_id).await?;
            if let Err(e) = self
                .sync_allowlist_from_tracker(
                    user_id,
                    allowlist_id,
                    Some(&normalized),
                    Some(vec![tracker.repo_path_for_allowlist_path(&normalized)]),
                    WorkspaceAllowlistRevisionKind::ToolDelete,
                    WorkspaceAllowlistRevisionSource::WorkspaceTool,
                    Some(normalized.clone()),
                    Some(format!("deleted {}", normalized)),
                    "workspace_delete",
                )
                .await
            {
                // Tracker sync can fail for gitignored or untracked files;
                // the disk file is already removed, so log and continue.
                tracing::debug!(
                    "tracker sync after delete of {} failed (non-fatal): {e}",
                    normalized
                );
            }
        }
        let record = if allowlist_supports_tracking(&allowlist) {
            self.load_allowlist_file_record(allowlist_id, &normalized)
                .await?
        } else {
            None
        };
        Ok(WorkspaceAllowlistFileView {
            allowlist_id,
            path: normalized.clone(),
            uri: WorkspaceUri::allowlist_uri_with_mount_kind(
                allowlist_id,
                allowlist.mount_kind,
                Some(&normalized),
            ),
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
        self.fetch_allowlist_for_tracking(user_id, allowlist_id, "workspace diff")
            .await?;
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
        self.fetch_allowlist_for_tracking(
            &request.user_id,
            request.allowlist_id,
            "workspace checkpoints",
        )
        .await?;
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
        self.fetch_allowlist_for_tracking(user_id, allowlist_id, "workspace checkpoints")
            .await?;
        self.ensure_allowlist_initialized(user_id, allowlist_id)
            .await?;
        let mut checkpoints = self.collect_checkpoint_chain(allowlist_id).await?;
        if let Some(limit) = limit {
            checkpoints.truncate(limit);
        }
        Ok(checkpoints)
    }

    async fn delete_workspace_checkpoint(
        &self,
        user_id: &str,
        allowlist_id: Uuid,
        checkpoint_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        self.fetch_allowlist_for_tracking(user_id, allowlist_id, "workspace checkpoints")
            .await?;
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let rows_affected = conn
            .execute(
                "DELETE FROM workspace_allowlist_checkpoints WHERE id = ?1 AND allowlist_id = ?2",
                libsql::params![checkpoint_id.to_string(), allowlist_id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("failed to delete checkpoint: {e}"),
            })?;
        if rows_affected == 0 {
            return Err(WorkspaceError::AllowlistNotFound {
                allowlist_id: checkpoint_id.to_string(),
            });
        }
        Ok(())
    }

    async fn list_workspace_allowlist_history(
        &self,
        request: &WorkspaceAllowlistHistoryRequest,
    ) -> Result<WorkspaceAllowlistHistory, WorkspaceError> {
        self.fetch_allowlist_for_tracking(
            &request.user_id,
            request.allowlist_id,
            "workspace history",
        )
        .await?;
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
        self.fetch_allowlist_for_tracking(&request.user_id, request.allowlist_id, "workspace diff")
            .await?;
        self.reconcile_allowlist(
            &request.user_id,
            request.allowlist_id,
            WorkspaceAllowlistRevisionKind::ManualRefresh,
            WorkspaceAllowlistRevisionSource::External,
            Some("workspace_diff".to_string()),
            None,
            "system",
        )
        .await?;
        self.build_diff_from_tracker(request).await
    }

    async fn keep_workspace_allowlist(
        &self,
        request: &AllowlistActionRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        self.fetch_allowlist_for_tracking(&request.user_id, request.allowlist_id, "workspace diff")
            .await?;
        self.reconcile_allowlist(
            &request.user_id,
            request.allowlist_id,
            WorkspaceAllowlistRevisionKind::ManualRefresh,
            WorkspaceAllowlistRevisionSource::External,
            Some("keep_allowlist".to_string()),
            None,
            "system",
        )
        .await?;
        let mut tracker = self
            .load_allowlist_tracker(request.allowlist_id)
            .await?
            .ok_or_else(|| WorkspaceError::AllowlistNotFound {
                allowlist_id: request.allowlist_id.to_string(),
            })?;
        tracker.baseline_anchor = tracker.head_anchor.clone();
        tracker.baseline_revision_id = tracker.head_revision_id;
        tracker.status = AllowlistTrackerStatus::Ready;
        tracker.last_verified_at = Some(Utc::now());
        self.save_allowlist_tracker(&tracker).await?;
        self.update_tracker_refs(&tracker).await?;
        self.sync_allowlist_live_cache(
            request.allowlist_id,
            tracker.baseline_revision_id,
            tracker.head_revision_id,
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
        self.fetch_allowlist_for_tracking(&request.user_id, request.allowlist_id, "workspace diff")
            .await?;
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
        self.fetch_allowlist_for_tracking(&request.user_id, request.allowlist_id, "workspace diff")
            .await?;
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
                let tracker = self
                    .ensure_allowlist_tracker(&request.user_id, request.allowlist_id)
                    .await?;
                let copy_trigger = copy_path.clone();
                self.sync_allowlist_from_tracker(
                    &request.user_id,
                    request.allowlist_id,
                    Some(&copy_path),
                    Some(vec![tracker.repo_path_for_allowlist_path(&copy_path)]),
                    WorkspaceAllowlistRevisionKind::ToolWrite,
                    WorkspaceAllowlistRevisionSource::WorkspaceTool,
                    Some(copy_trigger),
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
                let tracker = self
                    .ensure_allowlist_tracker(&request.user_id, request.allowlist_id)
                    .await?;
                let normalized_trigger = normalized.clone();
                self.sync_allowlist_from_tracker(
                    &request.user_id,
                    request.allowlist_id,
                    Some(&normalized),
                    Some(vec![tracker.repo_path_for_allowlist_path(&normalized)]),
                    WorkspaceAllowlistRevisionKind::ToolPatch,
                    WorkspaceAllowlistRevisionSource::WorkspaceTool,
                    Some(normalized_trigger),
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
        ensure_allowlist_containment(&source_disk_path, &allowlist.source_root)?;
        ensure_allowlist_containment(&destination_disk_path, &allowlist.source_root)?;

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
        if allowlist_supports_tracking(&allowlist) {
            let tracker = self.ensure_allowlist_tracker(user_id, allowlist_id).await?;
            let repo_paths = vec![
                tracker.repo_path_for_allowlist_path(&source_path),
                tracker.repo_path_for_allowlist_path(&destination_path),
            ];
            self.sync_allowlist_from_tracker(
                user_id,
                allowlist_id,
                None,
                Some(repo_paths),
                WorkspaceAllowlistRevisionKind::ToolMove,
                WorkspaceAllowlistRevisionSource::WorkspaceTool,
                Some(format!("{source_path} -> {destination_path}")),
                Some(format!("moved {} to {}", source_path, destination_path)),
                "workspace_move",
            )
            .await?;
        }
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
        ensure_allowlist_containment(&disk_path, &allowlist.source_root)?;
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
        if allowlist_supports_tracking(&allowlist) {
            self.sync_allowlist_from_tracker(
                user_id,
                allowlist_id,
                Some(&normalized),
                None,
                WorkspaceAllowlistRevisionKind::ToolDelete,
                WorkspaceAllowlistRevisionSource::WorkspaceTool,
                Some(path.to_string()),
                Some(format!("deleted directory tree {}", path)),
                "workspace_delete_tree",
            )
            .await?;
        }
        self.build_allowlist_detail_internal(user_id, allowlist_id)
            .await
    }

    async fn restore_workspace_allowlist(
        &self,
        request: &WorkspaceAllowlistRestoreRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        self.fetch_allowlist_for_tracking(
            &request.user_id,
            request.allowlist_id,
            "workspace history",
        )
        .await?;
        let state = self
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
        let target_anchor = self
            .load_tracker_anchor_for_revision(request.allowlist_id, target_revision_id)
            .await?
            .ok_or_else(|| WorkspaceError::AllowlistConflict {
                path: request.allowlist_id.to_string(),
                reason: format!(
                    "target '{}' is not backed by a tracker anchor",
                    request.target
                ),
            })?;
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
        let changed_repo_paths = self
            .restore_allowlist_from_anchor(
                &request.user_id,
                request.allowlist_id,
                &target_anchor,
                request.scope_path.as_deref(),
            )
            .await?;
        let state = self
            .sync_allowlist_from_tracker(
                &request.user_id,
                request.allowlist_id,
                request.scope_path.as_deref(),
                Some(changed_repo_paths),
                WorkspaceAllowlistRevisionKind::Restore,
                WorkspaceAllowlistRevisionSource::System,
                Some(request.target.clone()),
                Some(format!("restored workspace to {}", request.target)),
                &request.created_by,
            )
            .await?;
        if request.set_as_baseline {
            let mut tracker = self
                .load_allowlist_tracker(request.allowlist_id)
                .await?
                .ok_or_else(|| WorkspaceError::AllowlistNotFound {
                    allowlist_id: request.allowlist_id.to_string(),
                })?;
            tracker.baseline_anchor = tracker.head_anchor.clone();
            tracker.baseline_revision_id = state.head_revision_id;
            tracker.status = AllowlistTrackerStatus::Ready;
            tracker.last_verified_at = Some(Utc::now());
            self.save_allowlist_tracker(&tracker).await?;
            self.update_tracker_refs(&tracker).await?;
            self.sync_allowlist_live_cache(
                request.allowlist_id,
                tracker.baseline_revision_id,
                tracker.head_revision_id,
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
        self.fetch_allowlist_for_tracking(
            &request.user_id,
            request.allowlist_id,
            "workspace history",
        )
        .await?;
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
        let target_anchor = self
            .load_tracker_anchor_for_revision(request.allowlist_id, target_revision_id)
            .await?
            .ok_or_else(|| WorkspaceError::AllowlistConflict {
                path: request.allowlist_id.to_string(),
                reason: format!(
                    "target '{}' is not backed by a tracker anchor",
                    request.target
                ),
            })?;
        let mut tracker = self
            .load_allowlist_tracker(request.allowlist_id)
            .await?
            .ok_or_else(|| WorkspaceError::AllowlistNotFound {
                allowlist_id: request.allowlist_id.to_string(),
            })?;
        tracker.baseline_anchor = Some(target_anchor);
        tracker.baseline_revision_id = Some(target_revision_id);
        tracker.last_verified_at = Some(Utc::now());
        tracker.status = AllowlistTrackerStatus::Ready;
        self.save_allowlist_tracker(&tracker).await?;
        self.update_tracker_refs(&tracker).await?;
        self.sync_allowlist_live_cache(
            request.allowlist_id,
            tracker.baseline_revision_id,
            tracker.head_revision_id,
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
        scope_path: Option<&str>,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        if !allowlist_supports_tracking(&allowlist) {
            return self
                .build_allowlist_detail_internal(user_id, allowlist_id)
                .await;
        }
        self.sync_allowlist_from_tracker(
            user_id,
            allowlist_id,
            scope_path,
            None,
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
        let allowlist = self.fetch_allowlist(user_id, allowlist_id).await?;
        if !allowlist_supports_tracking(&allowlist) {
            return Ok(());
        }
        self.sync_allowlist_from_tracker(
            user_id,
            allowlist_id,
            None,
            None,
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
                mount_kind: WorkspaceMountKind::User,
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
                mount_kind: WorkspaceMountKind::User,
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
                    && entry.path
                        == crate::workspace::public_allowlist_id(
                            allowlist.allowlist.id,
                            allowlist.allowlist.mount_kind,
                        )
                    && entry.uri
                        == WorkspaceUri::allowlist_uri_with_mount_kind(
                            allowlist.allowlist.id,
                            allowlist.allowlist.mount_kind,
                            None,
                        )
                    && entry.mount_kind == Some(WorkspaceMountKind::User)
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
    async fn test_workspace_root_uses_fixed_public_system_mount_ids() {
        let (backend, dir) = setup_backend().await;
        let default_root = dir.path().join("default-fixed-id-root");
        std::fs::create_dir_all(&default_root).expect("create default root");
        let skills_root = dir.path().join("skills-fixed-id-root");
        std::fs::create_dir_all(&skills_root).expect("create skills root");

        let default_allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Default".to_string(),
                mount_kind: WorkspaceMountKind::Default,
                source_root: default_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create default allowlist");

        let skills_allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Skills".to_string(),
                mount_kind: WorkspaceMountKind::Skills,
                source_root: skills_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create skills allowlist");

        let entries = backend
            .list_workspace_tree("default", None, "workspace://")
            .await
            .expect("list workspace root");
        assert!(entries.iter().any(|entry| {
            entry.kind == WorkspaceTreeEntryKind::Allowlist
                && entry.path == "default"
                && entry.uri == "workspace://default"
                && entry.mount_kind == Some(WorkspaceMountKind::Default)
                && entry.name == default_allowlist.allowlist.display_name
        }));
        assert!(entries.iter().any(|entry| {
            entry.kind == WorkspaceTreeEntryKind::Allowlist
                && entry.path == "skills"
                && entry.uri == "workspace://skills"
                && entry.mount_kind == Some(WorkspaceMountKind::Skills)
                && entry.name == skills_allowlist.allowlist.display_name
        }));
    }

    #[tokio::test]
    async fn test_delete_workspace_allowlist_unmounts_without_deleting_disk_tree() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("unmount-project");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");
        std::fs::write(allowlist_root.join("README.md"), "hello").expect("seed allowlist file");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "unmount-project".to_string(),
                mount_kind: WorkspaceMountKind::User,
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create allowlist");

        backend
            .get_workspace_allowlist("default", allowlist.allowlist.id)
            .await
            .expect("initialize allowlist state");
        backend
            .delete_workspace_allowlist("default", allowlist.allowlist.id)
            .await
            .expect("delete allowlist");

        assert!(
            allowlist_root.exists(),
            "deleting an allowlist should not remove its source directory"
        );
        assert!(
            allowlist_root.join("README.md").exists(),
            "deleting an allowlist should not remove disk files"
        );

        let summaries = backend
            .list_workspace_allowlists("default")
            .await
            .expect("list allowlists after delete");
        assert!(
            summaries
                .iter()
                .all(|summary| summary.allowlist.id != allowlist.allowlist.id),
            "deleted allowlist should disappear from summaries"
        );

        let entries = backend
            .list_workspace_tree("default", None, "workspace://")
            .await
            .expect("list workspace root");
        assert!(
            entries.iter().all(|entry| {
                entry.uri
                    != WorkspaceUri::allowlist_uri_with_mount_kind(
                        allowlist.allowlist.id,
                        allowlist.allowlist.mount_kind,
                        None,
                    )
            }),
            "deleted allowlist should disappear from the workspace tree"
        );

        let err = backend
            .get_workspace_allowlist("default", allowlist.allowlist.id)
            .await
            .expect_err("deleted allowlist should no longer be addressable");
        assert!(matches!(err, WorkspaceError::AllowlistNotFound { .. }));
    }

    #[tokio::test]
    async fn test_workspace_allowlist_dedupes_source_root_and_updates_mount_kind() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("skills-root");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");

        let first = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Skills".to_string(),
                mount_kind: WorkspaceMountKind::Skills,
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create skills allowlist");

        let second = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Renamed Skills".to_string(),
                mount_kind: WorkspaceMountKind::Skills,
                source_root: allowlist_root.display().to_string(),
                bypass_write: true,
            })
            .await
            .expect("recreate skills allowlist");

        assert_eq!(
            first.allowlist.id, second.allowlist.id,
            "same source_root should reuse the existing allowlist"
        );
        assert_eq!(second.allowlist.mount_kind, WorkspaceMountKind::Skills);
        assert!(second.allowlist.bypass_write);
        assert_eq!(second.allowlist.display_name, "Renamed Skills");

        let summaries = backend
            .list_workspace_allowlists("default")
            .await
            .expect("list allowlists");
        let skills_entries: Vec<_> = summaries
            .into_iter()
            .filter(|summary| {
                summary.allowlist.id == second.allowlist.id
                    && summary.allowlist.mount_kind == WorkspaceMountKind::Skills
            })
            .collect();
        assert_eq!(skills_entries.len(), 1, "skills root should be idempotent");
    }

    #[tokio::test]
    async fn test_user_allowlist_dedupes_source_root_and_updates_mount_kind() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("user-dedup-root");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");

        let first = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Project".to_string(),
                mount_kind: WorkspaceMountKind::User,
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create user allowlist");

        let second = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Renamed Project".to_string(),
                mount_kind: WorkspaceMountKind::User,
                source_root: allowlist_root.display().to_string(),
                bypass_write: true,
            })
            .await
            .expect("recreate user allowlist");

        assert_eq!(
            first.allowlist.id, second.allowlist.id,
            "same source_root should reuse the existing allowlist"
        );
        assert_eq!(second.allowlist.mount_kind, WorkspaceMountKind::User);
        assert!(second.allowlist.bypass_write);
        assert_eq!(second.allowlist.display_name, "Renamed Project");

        // diff should work because mount_kind is user
        let diff = backend
            .diff_workspace_allowlist("default", second.allowlist.id, None)
            .await
            .expect("diff should succeed for user allowlist");
        assert_eq!(diff.allowlist_id, second.allowlist.id);
    }

    #[tokio::test]
    async fn test_skills_allowlist_rekeys_existing_random_uuid_to_fixed_public_id() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("skills-rekey-root");
        std::fs::create_dir_all(&allowlist_root).expect("create allowlist root");
        std::fs::write(allowlist_root.join("SKILL.md"), "name: rekey-test\n")
            .expect("seed skill manifest");

        let legacy_allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Legacy".to_string(),
                mount_kind: WorkspaceMountKind::User,
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create legacy allowlist");

        backend
            .get_workspace_allowlist("default", legacy_allowlist.allowlist.id)
            .await
            .expect("initialize legacy tracking state");

        let rekeyed = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Skills".to_string(),
                mount_kind: WorkspaceMountKind::Skills,
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("rekey to fixed skills allowlist id");

        assert_eq!(
            rekeyed.allowlist.id,
            crate::workspace::skills_allowlist_uuid()
        );
        assert_eq!(rekeyed.allowlist.mount_kind, WorkspaceMountKind::Skills);

        let conn = backend.connect().await.expect("connect");
        for table in [
            "workspace_allowlist_trackers",
            "workspace_allowlist_revisions",
            "workspace_allowlist_revision_anchors",
        ] {
            let mut rows = conn
                .query(
                    &format!("SELECT COUNT(*) FROM {table} WHERE allowlist_id = ?1"),
                    params![crate::workspace::skills_allowlist_uuid().to_string()],
                )
                .await
                .expect("query rekeyed rows");
            let row = rows
                .next()
                .await
                .expect("fetch count row")
                .expect("count row exists");
            let count = get_i64(&row, 0);
            assert!(
                count > 0,
                "expected {table} rows to follow the fixed skills allowlist id"
            );
        }

        let mut old_id_rows = conn
            .query(
                "SELECT COUNT(*) FROM workspace_allowlists WHERE id = ?1",
                params![legacy_allowlist.allowlist.id.to_string()],
            )
            .await
            .expect("query old allowlist id");
        let row = old_id_rows
            .next()
            .await
            .expect("fetch old id count row")
            .expect("old id count row exists");
        assert_eq!(get_i64(&row, 0), 0);
    }

    #[tokio::test]
    async fn test_skills_allowlist_detail_omits_tracking_state_after_write() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("skills-browse-root");
        std::fs::create_dir_all(&allowlist_root).expect("create skills root");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Skills".to_string(),
                mount_kind: WorkspaceMountKind::Skills,
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create skills allowlist");

        backend
            .write_workspace_allowlist_file(
                "default",
                allowlist.allowlist.id,
                "sample-skill/SKILL.md",
                b"---\nname: sample-skill\n---\n\nPrompt.\n",
            )
            .await
            .expect("write skill file");

        let detail = backend
            .get_workspace_allowlist("default", allowlist.allowlist.id)
            .await
            .expect("get skills allowlist detail");
        assert_eq!(
            detail.summary.allowlist.mount_kind,
            WorkspaceMountKind::Skills
        );
        assert_eq!(detail.open_change_count, 0);
        assert_eq!(detail.summary.dirty_count, 0);
        assert_eq!(detail.summary.conflict_count, 0);
        assert_eq!(detail.summary.pending_delete_count, 0);
        assert!(detail.baseline_revision_id.is_none());
        assert!(detail.head_revision_id.is_none());
        assert!(detail.checkpoints.is_empty());
    }

    #[tokio::test]
    async fn test_skills_allowlist_rejects_diff_and_history() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("skills-no-history-root");
        std::fs::create_dir_all(&allowlist_root).expect("create skills root");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Skills".to_string(),
                mount_kind: WorkspaceMountKind::Skills,
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create skills allowlist");

        let diff_err = backend
            .diff_workspace_allowlist("default", allowlist.allowlist.id, None)
            .await
            .expect_err("skills allowlist should not expose diff");
        assert!(diff_err.to_string().contains("Skills mounts"));

        let history_err = backend
            .list_workspace_allowlist_history(&WorkspaceAllowlistHistoryRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                scope_path: None,
                limit: 20,
                since: None,
                include_checkpoints: true,
            })
            .await
            .expect_err("skills allowlist should not expose history");
        assert!(history_err.to_string().contains("Skills mounts"));
    }

    #[tokio::test]
    async fn test_default_allowlist_rejects_diff_and_history() {
        let (backend, dir) = setup_backend().await;
        let allowlist_root = dir.path().join("default-no-history-root");
        std::fs::create_dir_all(&allowlist_root).expect("create default root");

        let allowlist = backend
            .create_workspace_allowlist(&CreateAllowlistRequest {
                user_id: "default".to_string(),
                display_name: "Default".to_string(),
                mount_kind: WorkspaceMountKind::Default,
                source_root: allowlist_root.display().to_string(),
                bypass_write: false,
            })
            .await
            .expect("create default allowlist");

        backend
            .write_workspace_allowlist_file(
                "default",
                allowlist.allowlist.id,
                "notes/welcome.md",
                b"# hello\n",
            )
            .await
            .expect("write default file");

        let detail = backend
            .get_workspace_allowlist("default", allowlist.allowlist.id)
            .await
            .expect("get default allowlist detail");
        assert_eq!(
            detail.summary.allowlist.mount_kind,
            WorkspaceMountKind::Default
        );
        assert_eq!(detail.open_change_count, 0);
        assert_eq!(detail.summary.dirty_count, 0);
        assert_eq!(detail.summary.conflict_count, 0);
        assert_eq!(detail.summary.pending_delete_count, 0);
        assert!(detail.baseline_revision_id.is_none());
        assert!(detail.head_revision_id.is_none());
        assert!(detail.checkpoints.is_empty());

        let diff_err = backend
            .diff_workspace_allowlist("default", allowlist.allowlist.id, None)
            .await
            .expect_err("default allowlist should not expose diff");
        assert!(diff_err.to_string().contains("Default mounts"));

        let history_err = backend
            .list_workspace_allowlist_history(&WorkspaceAllowlistHistoryRequest {
                user_id: "default".to_string(),
                allowlist_id: allowlist.allowlist.id,
                scope_path: None,
                limit: 20,
                since: None,
                include_checkpoints: true,
            })
            .await
            .expect_err("default allowlist should not expose history");
        assert!(history_err.to_string().contains("Default mounts"));
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
                mount_kind: WorkspaceMountKind::User,
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
                mount_kind: WorkspaceMountKind::User,
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
                mount_kind: WorkspaceMountKind::User,
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
                mount_kind: WorkspaceMountKind::User,
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
                mount_kind: WorkspaceMountKind::User,
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
