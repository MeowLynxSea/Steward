use std::path::{Component, Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::WorkspaceError;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTreeEntryKind {
    MemoryRoot,
    MountsRoot,
    MemoryDirectory,
    MemoryFile,
    Mount,
    MountedDirectory,
    MountedFile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MountedFileStatus {
    Clean,
    Modified,
    Added,
    Deleted,
    Conflicted,
    BinaryModified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMountRevisionKind {
    Initial,
    ToolWrite,
    ToolPatch,
    ToolMove,
    ToolDelete,
    Shell,
    FsWatch,
    ManualRefresh,
    Restore,
    Accept,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMountRevisionSource {
    WorkspaceTool,
    Shell,
    External,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMountChangeKind {
    Added,
    Modified,
    Deleted,
    Moved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTreeEntry {
    pub name: String,
    pub path: String,
    pub uri: String,
    pub is_directory: bool,
    pub kind: WorkspaceTreeEntryKind,
    pub status: Option<MountedFileStatus>,
    pub updated_at: Option<DateTime<Utc>>,
    pub content_preview: Option<String>,
    pub bypass_write: Option<bool>,
    pub dirty_count: usize,
    pub conflict_count: usize,
    pub pending_delete_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMount {
    pub id: Uuid,
    pub user_id: String,
    pub display_name: String,
    #[serde(skip_serializing)]
    pub source_root: String,
    pub bypass_read: bool,
    pub bypass_write: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMountSummary {
    pub mount: WorkspaceMount,
    pub dirty_count: usize,
    pub conflict_count: usize,
    pub pending_delete_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMountCheckpoint {
    pub id: Uuid,
    pub mount_id: Uuid,
    pub revision_id: Uuid,
    pub parent_checkpoint_id: Option<Uuid>,
    pub label: Option<String>,
    pub summary: Option<String>,
    pub created_by: String,
    pub is_auto: bool,
    pub base_generation: i64,
    pub created_at: DateTime<Utc>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMountDetail {
    pub summary: WorkspaceMountSummary,
    pub baseline_revision_id: Option<Uuid>,
    pub head_revision_id: Option<Uuid>,
    pub checkpoints: Vec<WorkspaceMountCheckpoint>,
    pub open_change_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMountRevision {
    pub id: Uuid,
    pub mount_id: Uuid,
    pub parent_revision_id: Option<Uuid>,
    pub kind: WorkspaceMountRevisionKind,
    pub source: WorkspaceMountRevisionSource,
    pub trigger: Option<String>,
    pub summary: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMountHistory {
    pub mount_id: Uuid,
    pub baseline_revision_id: Option<Uuid>,
    pub head_revision_id: Option<Uuid>,
    pub revisions: Vec<WorkspaceMountRevision>,
    pub checkpoints: Vec<WorkspaceMountCheckpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountedFileDiff {
    pub path: String,
    pub uri: String,
    pub status: MountedFileStatus,
    pub change_kind: WorkspaceMountChangeKind,
    pub is_binary: bool,
    pub base_content: Option<String>,
    pub working_content: Option<String>,
    pub remote_content: Option<String>,
    pub diff_text: Option<String>,
    pub conflict_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMountDiff {
    pub mount_id: Uuid,
    pub from_revision_id: Option<Uuid>,
    pub to_revision_id: Option<Uuid>,
    pub entries: Vec<MountedFileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMountFileView {
    pub mount_id: Uuid,
    pub path: String,
    pub uri: String,
    pub disk_path: String,
    pub status: MountedFileStatus,
    pub is_binary: bool,
    pub content: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedWorkspaceMountPath {
    pub mount_id: Uuid,
    pub relative_path: Option<String>,
    pub workspace_uri: String,
    pub disk_path: String,
    pub source_root: String,
}

#[derive(Debug, Clone)]
pub struct CreateMountRequest {
    pub user_id: String,
    pub display_name: String,
    pub source_root: String,
    pub bypass_write: bool,
}

#[derive(Debug, Clone)]
pub struct CreateCheckpointRequest {
    pub user_id: String,
    pub mount_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub label: Option<String>,
    pub summary: Option<String>,
    pub created_by: String,
    pub is_auto: bool,
}

#[derive(Debug, Clone)]
pub struct MountActionRequest {
    pub user_id: String,
    pub mount_id: Uuid,
    pub scope_path: Option<String>,
    pub checkpoint_id: Option<Uuid>,
    pub set_as_baseline: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceMountHistoryRequest {
    pub user_id: String,
    pub mount_id: Uuid,
    pub scope_path: Option<String>,
    pub limit: usize,
    pub since: Option<DateTime<Utc>>,
    pub include_checkpoints: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceMountDiffRequest {
    pub user_id: String,
    pub mount_id: Uuid,
    pub scope_path: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub include_content: bool,
    pub max_files: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceMountRestoreRequest {
    pub user_id: String,
    pub mount_id: Uuid,
    pub scope_path: Option<String>,
    pub target: String,
    pub set_as_baseline: bool,
    pub dry_run: bool,
    pub create_checkpoint_before_restore: bool,
    pub created_by: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceMountBaselineRequest {
    pub user_id: String,
    pub mount_id: Uuid,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct ConflictResolutionRequest {
    pub user_id: String,
    pub mount_id: Uuid,
    pub path: String,
    pub resolution: String,
    pub renamed_copy_path: Option<String>,
    pub merged_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceUri {
    Root,
    MountRoot(Uuid),
    MountPath(Uuid, String),
}

impl WorkspaceUri {
    pub fn parse(input: &str) -> Result<Option<Self>, WorkspaceError> {
        if !input.starts_with("workspace://") {
            return Ok(None);
        }

        let rest = input.trim_start_matches("workspace://").trim_matches('/');
        if rest.is_empty() {
            return Ok(Some(Self::Root));
        }

        let rest = if rest == "mounts" {
            ""
        } else {
            rest.strip_prefix("mounts/").unwrap_or(rest)
        };

        if rest.is_empty() {
            return Ok(Some(Self::Root));
        }

        let (mount_id, path) = match rest.split_once('/') {
            Some((id, tail)) => (id, Some(tail)),
            None => (rest, None),
        };
        let mount_id = Uuid::parse_str(mount_id).map_err(|_| WorkspaceError::InvalidDocType {
            doc_type: input.to_string(),
        })?;
        let normalized = match path {
            Some(path) if !path.is_empty() => normalize_mount_path(path)?,
            _ => String::new(),
        };

        Ok(Some(match normalized.is_empty() {
            true => Self::MountRoot(mount_id),
            false => Self::MountPath(mount_id, normalized),
        }))
    }

    pub fn root_uri() -> &'static str {
        "workspace://"
    }

    pub fn mount_uri(mount_id: Uuid, path: Option<&str>) -> String {
        match path {
            Some(path) if !path.is_empty() => format!("workspace://{mount_id}/{path}"),
            _ => format!("workspace://{mount_id}"),
        }
    }
}

pub fn normalize_mount_path(path: &str) -> Result<String, WorkspaceError> {
    if path.contains('\0') {
        return Err(WorkspaceError::IoError {
            reason: "mount path contains null byte".to_string(),
        });
    }

    let mut normalized = Vec::new();
    for component in Path::new(path.trim()).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment.to_string_lossy().into_owned()),
            Component::ParentDir => {
                if normalized.pop().is_none() {
                    return Err(WorkspaceError::IoError {
                        reason: format!("mount path escapes root: {path}"),
                    });
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(WorkspaceError::IoError {
                    reason: format!("mount path must be relative: {path}"),
                });
            }
        }
    }

    Ok(normalized.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_root_and_legacy_mount_root() {
        assert_eq!(
            WorkspaceUri::parse("workspace://").unwrap(),
            Some(WorkspaceUri::Root)
        );
        assert_eq!(
            WorkspaceUri::parse("workspace://mounts").unwrap(),
            Some(WorkspaceUri::Root)
        );
    }

    #[test]
    fn parse_direct_and_legacy_mount_paths() {
        let id = Uuid::new_v4();
        assert_eq!(
            WorkspaceUri::parse(&format!("workspace://{id}/src/lib.rs")).unwrap(),
            Some(WorkspaceUri::MountPath(id, "src/lib.rs".to_string()))
        );
        assert_eq!(
            WorkspaceUri::parse(&format!("workspace://mounts/{id}/src/lib.rs")).unwrap(),
            Some(WorkspaceUri::MountPath(id, "src/lib.rs".to_string()))
        );
    }

    #[test]
    fn rejects_mount_path_escape() {
        let id = Uuid::new_v4();
        let err = WorkspaceUri::parse(&format!("workspace://{id}/../secret.txt")).unwrap_err();
        assert!(err.to_string().contains("escapes root"));
    }

    #[test]
    fn normalizes_internal_parent_segments() {
        assert_eq!(
            normalize_mount_path("src/bin/../lib.rs").unwrap(),
            "src/lib.rs"
        );
    }
}
