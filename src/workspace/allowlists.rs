use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as DeError};
use std::path::{Component, Path};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::WorkspaceError;

pub fn encode_allowlist_id(id: Uuid) -> String {
    URL_SAFE_NO_PAD.encode(id.as_bytes())
}

pub fn parse_allowlist_id(input: &str) -> Result<Uuid, WorkspaceError> {
    if let Ok(uuid) = Uuid::parse_str(input) {
        return Ok(uuid);
    }

    let decoded =
        URL_SAFE_NO_PAD
            .decode(input.as_bytes())
            .map_err(|_| WorkspaceError::InvalidDocType {
                doc_type: input.to_string(),
            })?;
    let bytes: [u8; 16] = decoded
        .try_into()
        .map_err(|_| WorkspaceError::InvalidDocType {
            doc_type: input.to_string(),
        })?;
    Ok(Uuid::from_bytes(bytes))
}

mod allowlist_id_serde {
    use super::*;

    pub fn serialize<S>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_allowlist_id(*id))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Uuid, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_allowlist_id(&raw).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTreeEntryKind {
    MemoryRoot,
    AllowlistsRoot,
    MemoryDirectory,
    MemoryFile,
    Allowlist,
    AllowlistedDirectory,
    AllowlistedFile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AllowlistedFileStatus {
    Clean,
    Modified,
    Added,
    Deleted,
    Conflicted,
    BinaryModified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAllowlistRevisionKind {
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
pub enum WorkspaceAllowlistRevisionSource {
    WorkspaceTool,
    Shell,
    External,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAllowlistChangeKind {
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
    pub status: Option<AllowlistedFileStatus>,
    pub updated_at: Option<DateTime<Utc>>,
    pub content_preview: Option<String>,
    pub bypass_write: Option<bool>,
    pub dirty_count: usize,
    pub conflict_count: usize,
    pub pending_delete_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAllowlist {
    #[serde(with = "allowlist_id_serde")]
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
pub struct WorkspaceAllowlistSummary {
    pub allowlist: WorkspaceAllowlist,
    pub dirty_count: usize,
    pub conflict_count: usize,
    pub pending_delete_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAllowlistCheckpoint {
    pub id: Uuid,
    #[serde(with = "allowlist_id_serde")]
    pub allowlist_id: Uuid,
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
pub struct WorkspaceAllowlistDetail {
    pub summary: WorkspaceAllowlistSummary,
    pub baseline_revision_id: Option<Uuid>,
    pub head_revision_id: Option<Uuid>,
    pub checkpoints: Vec<WorkspaceAllowlistCheckpoint>,
    pub open_change_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAllowlistRevision {
    pub id: Uuid,
    #[serde(with = "allowlist_id_serde")]
    pub allowlist_id: Uuid,
    pub parent_revision_id: Option<Uuid>,
    pub kind: WorkspaceAllowlistRevisionKind,
    pub source: WorkspaceAllowlistRevisionSource,
    pub trigger: Option<String>,
    pub summary: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAllowlistHistory {
    #[serde(with = "allowlist_id_serde")]
    pub allowlist_id: Uuid,
    pub baseline_revision_id: Option<Uuid>,
    pub head_revision_id: Option<Uuid>,
    pub revisions: Vec<WorkspaceAllowlistRevision>,
    pub checkpoints: Vec<WorkspaceAllowlistCheckpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowlistedFileDiff {
    pub path: String,
    pub uri: String,
    pub status: AllowlistedFileStatus,
    pub change_kind: WorkspaceAllowlistChangeKind,
    pub is_binary: bool,
    pub base_content: Option<String>,
    pub working_content: Option<String>,
    pub remote_content: Option<String>,
    pub diff_text: Option<String>,
    pub conflict_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAllowlistDiff {
    #[serde(with = "allowlist_id_serde")]
    pub allowlist_id: Uuid,
    pub from_revision_id: Option<Uuid>,
    pub to_revision_id: Option<Uuid>,
    pub entries: Vec<AllowlistedFileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAllowlistFileView {
    #[serde(with = "allowlist_id_serde")]
    pub allowlist_id: Uuid,
    pub path: String,
    pub uri: String,
    pub disk_path: String,
    pub status: AllowlistedFileStatus,
    pub is_binary: bool,
    pub content: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedWorkspaceAllowlistPath {
    #[serde(with = "allowlist_id_serde")]
    pub allowlist_id: Uuid,
    pub relative_path: Option<String>,
    pub workspace_uri: String,
    pub disk_path: String,
    pub source_root: String,
}

#[derive(Debug, Clone)]
pub struct CreateAllowlistRequest {
    pub user_id: String,
    pub display_name: String,
    pub source_root: String,
    pub bypass_write: bool,
}

#[derive(Debug, Clone)]
pub struct CreateCheckpointRequest {
    pub user_id: String,
    pub allowlist_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub label: Option<String>,
    pub summary: Option<String>,
    pub created_by: String,
    pub is_auto: bool,
}

#[derive(Debug, Clone)]
pub struct AllowlistActionRequest {
    pub user_id: String,
    pub allowlist_id: Uuid,
    pub scope_path: Option<String>,
    pub checkpoint_id: Option<Uuid>,
    pub set_as_baseline: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceAllowlistHistoryRequest {
    pub user_id: String,
    pub allowlist_id: Uuid,
    pub scope_path: Option<String>,
    pub limit: usize,
    pub since: Option<DateTime<Utc>>,
    pub include_checkpoints: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceAllowlistDiffRequest {
    pub user_id: String,
    pub allowlist_id: Uuid,
    pub scope_path: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub include_content: bool,
    pub max_files: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceAllowlistRestoreRequest {
    pub user_id: String,
    pub allowlist_id: Uuid,
    pub scope_path: Option<String>,
    pub target: String,
    pub set_as_baseline: bool,
    pub dry_run: bool,
    pub create_checkpoint_before_restore: bool,
    pub created_by: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceAllowlistBaselineRequest {
    pub user_id: String,
    pub allowlist_id: Uuid,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct ConflictResolutionRequest {
    pub user_id: String,
    pub allowlist_id: Uuid,
    pub path: String,
    pub resolution: String,
    pub renamed_copy_path: Option<String>,
    pub merged_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceUri {
    Root,
    AllowlistRoot(Uuid),
    AllowlistPath(Uuid, String),
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

        let rest = if rest == "allowlists" {
            ""
        } else {
            rest.strip_prefix("allowlists/").unwrap_or(rest)
        };

        if rest.is_empty() {
            return Ok(Some(Self::Root));
        }

        let (allowlist_id, path) = match rest.split_once('/') {
            Some((id, tail)) => (id, Some(tail)),
            None => (rest, None),
        };
        let allowlist_id = parse_allowlist_id(allowlist_id)?;
        let normalized = match path {
            Some(path) if !path.is_empty() => normalize_allowlist_path(path)?,
            _ => String::new(),
        };

        Ok(Some(match normalized.is_empty() {
            true => Self::AllowlistRoot(allowlist_id),
            false => Self::AllowlistPath(allowlist_id, normalized),
        }))
    }

    pub fn root_uri() -> &'static str {
        "workspace://"
    }

    pub fn allowlist_uri(allowlist_id: Uuid, path: Option<&str>) -> String {
        let public_id = encode_allowlist_id(allowlist_id);
        match path {
            Some(path) if !path.is_empty() => format!("workspace://{public_id}/{path}"),
            _ => format!("workspace://{public_id}"),
        }
    }
}

pub fn normalize_allowlist_path(path: &str) -> Result<String, WorkspaceError> {
    if path.contains('\0') {
        return Err(WorkspaceError::IoError {
            reason: "allowlist path contains null byte".to_string(),
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
                        reason: format!("allowlist path escapes root: {path}"),
                    });
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(WorkspaceError::IoError {
                    reason: format!("allowlist path must be relative: {path}"),
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
    fn parse_root_and_allowlists_root() {
        assert_eq!(
            WorkspaceUri::parse("workspace://").unwrap(),
            Some(WorkspaceUri::Root)
        );
        assert_eq!(
            WorkspaceUri::parse("workspace://allowlists").unwrap(),
            Some(WorkspaceUri::Root)
        );
    }

    #[test]
    fn parse_direct_and_allowlists_paths() {
        let id = Uuid::new_v4();
        let short_id = encode_allowlist_id(id);
        assert_eq!(
            WorkspaceUri::parse(&format!("workspace://{short_id}/src/lib.rs")).unwrap(),
            Some(WorkspaceUri::AllowlistPath(id, "src/lib.rs".to_string()))
        );
        assert_eq!(
            WorkspaceUri::parse(&format!("workspace://allowlists/{short_id}/src/lib.rs")).unwrap(),
            Some(WorkspaceUri::AllowlistPath(id, "src/lib.rs".to_string()))
        );
        assert_eq!(
            WorkspaceUri::parse(&format!("workspace://{id}/src/lib.rs")).unwrap(),
            Some(WorkspaceUri::AllowlistPath(id, "src/lib.rs".to_string()))
        );
    }

    #[test]
    fn rejects_allowlist_path_escape() {
        let id = Uuid::new_v4();
        let short_id = encode_allowlist_id(id);
        let err =
            WorkspaceUri::parse(&format!("workspace://{short_id}/../secret.txt")).unwrap_err();
        assert!(err.to_string().contains("escapes root"));
    }

    #[test]
    fn roundtrips_short_allowlist_ids() {
        let id = Uuid::new_v4();
        let short_id = encode_allowlist_id(id);
        assert_eq!(parse_allowlist_id(&short_id).unwrap(), id);
    }

    #[test]
    fn normalizes_internal_parent_segments() {
        assert_eq!(
            normalize_allowlist_path("src/bin/../lib.rs").unwrap(),
            "src/lib.rs"
        );
    }
}
