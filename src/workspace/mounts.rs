use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    PendingDelete,
    Conflicted,
    BinaryModified,
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
    pub checkpoints: Vec<WorkspaceMountCheckpoint>,
    pub open_change_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountedFileDiff {
    pub path: String,
    pub uri: String,
    pub status: MountedFileStatus,
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
    pub entries: Vec<MountedFileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMountFileView {
    pub mount_id: Uuid,
    pub path: String,
    pub uri: String,
    pub status: MountedFileStatus,
    pub is_binary: bool,
    pub content: Option<String>,
    pub updated_at: DateTime<Utc>,
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
    MemoryRoot,
    MemoryPath(String),
    MountsRoot,
    MountRoot(Uuid),
    MountPath(Uuid, String),
}

impl WorkspaceUri {
    pub fn parse(input: &str) -> Option<Self> {
        if !input.starts_with("workspace://") {
            return None;
        }

        let rest = input.trim_start_matches("workspace://").trim_matches('/');
        if rest.is_empty() {
            return Some(Self::MemoryRoot);
        }
        if rest == "memory" {
            return Some(Self::MemoryRoot);
        }
        if let Some(path) = rest.strip_prefix("memory/") {
            return Some(Self::MemoryPath(path.to_string()));
        }
        if rest == "mounts" {
            return Some(Self::MountsRoot);
        }
        if let Some(mount_rest) = rest.strip_prefix("mounts/") {
            let (mount_id, path) = match mount_rest.split_once('/') {
                Some((id, tail)) => (id, Some(tail.to_string())),
                None => (mount_rest, None),
            };
            let mount_id = Uuid::parse_str(mount_id).ok()?;
            return Some(match path {
                Some(path) if !path.is_empty() => Self::MountPath(mount_id, path),
                _ => Self::MountRoot(mount_id),
            });
        }
        None
    }

    pub fn root_uri() -> &'static str {
        "workspace://"
    }

    pub fn memory_uri(path: &str) -> String {
        if path.is_empty() {
            "workspace://memory".to_string()
        } else {
            format!("workspace://memory/{path}")
        }
    }

    pub fn mount_uri(mount_id: Uuid, path: Option<&str>) -> String {
        match path {
            Some(path) if !path.is_empty() => format!("workspace://mounts/{mount_id}/{path}"),
            _ => format!("workspace://mounts/{mount_id}"),
        }
    }
}
