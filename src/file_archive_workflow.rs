use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::context::JobContext;
use crate::prelude::Tool;
use crate::task_runtime::{TaskMode, TaskOperation, TaskPendingApproval, TaskRecord, TaskRuntime};
use crate::tools::builtin::MoveFileTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveWorkflowParams {
    pub source_path: String,
    pub target_root: String,
    #[serde(default = "default_naming_strategy")]
    pub naming_strategy: String,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowValidationError {
    pub message: String,
    pub field_errors: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowExecutionError {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveExecutionResult {
    pub moved: Vec<Value>,
    pub skipped: Vec<Value>,
    pub failed: Vec<Value>,
}

pub fn validate_archive_params(
    value: &Value,
) -> Result<ArchiveWorkflowParams, WorkflowValidationError> {
    let mut field_errors = HashMap::new();
    let params: ArchiveWorkflowParams =
        serde_json::from_value(value.clone()).map_err(|_| WorkflowValidationError {
            message: "invalid task parameters".to_string(),
            field_errors: HashMap::from([(
                "parameters".to_string(),
                "parameters must be an object".to_string(),
            )]),
        })?;

    if params.source_path.trim().is_empty() {
        field_errors.insert(
            "parameters.source_path".to_string(),
            "source_path is required".to_string(),
        );
    } else if !Path::new(&params.source_path).is_dir() {
        field_errors.insert(
            "parameters.source_path".to_string(),
            "source_path must be an existing directory".to_string(),
        );
    }

    if params.target_root.trim().is_empty() {
        field_errors.insert(
            "parameters.target_root".to_string(),
            "target_root is required".to_string(),
        );
    }

    if !matches!(params.naming_strategy.as_str(), "preserve" | "normalize") {
        field_errors.insert(
            "parameters.naming_strategy".to_string(),
            "naming_strategy must be \"preserve\" or \"normalize\"".to_string(),
        );
    }

    if !field_errors.is_empty() {
        return Err(WorkflowValidationError {
            message: "invalid task parameters".to_string(),
            field_errors,
        });
    }

    Ok(params)
}

pub async fn create_archive_task(
    runtime: Arc<TaskRuntime>,
    params: ArchiveWorkflowParams,
    mode: TaskMode,
) -> Result<TaskRecord, WorkflowExecutionError> {
    let plan = build_archive_plan(&params).await?;
    let title = format!(
        "Archive {}",
        Path::new(&params.source_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("files")
    );
    let result_metadata = json!({
        "workflow": "file_archive",
        "parameters": params,
        "archive_plan": plan.operations,
        "plan_summary": {
            "planned": plan.operations.len(),
            "skipped": plan.skipped.len()
        },
        "execution": {
            "moved": [],
            "skipped": plan.skipped,
            "failed": []
        }
    });

    let task = runtime
        .create_workflow_task("builtin:file-archive", title, mode, Some(result_metadata))
        .await;

    if plan.operations.is_empty() {
        runtime
            .mark_completed_with_result(
                task.id,
                Some(json!({
                    "workflow": "file_archive",
                    "parameters": params,
                    "archive_plan": [],
                    "execution": {
                        "moved": [],
                        "skipped": plan.skipped,
                        "failed": []
                    }
                })),
            )
            .await;
        return runtime
            .get_task(task.id)
            .await
            .ok_or_else(|| WorkflowExecutionError {
                message: "created task could not be reloaded".to_string(),
            });
    }

    if matches!(mode, TaskMode::Ask) {
        runtime
            .set_waiting_approval(
                task.id,
                TaskPendingApproval {
                    id: uuid::Uuid::new_v4(),
                    risk: "file_write".to_string(),
                    summary: format!(
                        "Move {} files into categorized folders",
                        plan.operations.len()
                    ),
                    operations: plan.operations.clone(),
                    allow_always: true,
                },
            )
            .await;
    }

    runtime
        .get_task(task.id)
        .await
        .ok_or_else(|| WorkflowExecutionError {
            message: "created task could not be reloaded".to_string(),
        })
}

pub async fn execute_archive_task(
    runtime: Arc<TaskRuntime>,
    task_id: uuid::Uuid,
) -> Result<TaskRecord, WorkflowExecutionError> {
    let task = runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| WorkflowExecutionError {
            message: format!("task {task_id} not found"),
        })?;

    let params = task
        .result_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("parameters"))
        .cloned()
        .ok_or_else(|| WorkflowExecutionError {
            message: "archive parameters missing from task metadata".to_string(),
        })?;
    let params = validate_archive_params(&params).map_err(|err| WorkflowExecutionError {
        message: err.message,
    })?;

    let operations: Vec<TaskOperation> = task
        .result_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("archive_plan"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| WorkflowExecutionError {
            message: "archive plan missing from task metadata".to_string(),
        })?;

    runtime
        .mark_running(&task.route.to_incoming_message(""), task_id)
        .await;

    let tool = MoveFileTool::new();
    let ctx = JobContext::with_user("default", "Archive Files", "Execute file archive workflow");
    let mut moved = Vec::new();
    let mut failed = Vec::new();
    let skipped = task
        .result_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("execution"))
        .and_then(|execution| execution.get("skipped"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    for operation in &operations {
        let result = tool.execute(operation.parameters.clone(), &ctx).await;
        match result {
            Ok(output) => moved.push(json!({
                "source_path": operation.path,
                "destination_path": operation.destination_path,
                "result": output.result,
            })),
            Err(error) => failed.push(json!({
                "source_path": operation.path,
                "destination_path": operation.destination_path,
                "error": error.to_string(),
            })),
        }
    }

    let result_metadata = json!({
        "workflow": "file_archive",
        "parameters": params,
        "archive_plan": operations,
        "execution": {
            "moved": moved,
            "skipped": skipped,
            "failed": failed
        }
    });

    runtime
        .mark_completed_with_result(task_id, Some(result_metadata))
        .await;

    runtime
        .get_task(task_id)
        .await
        .ok_or_else(|| WorkflowExecutionError {
            message: format!("task {task_id} not found after execution"),
        })
}

pub fn is_file_archive_task(task: &TaskRecord) -> bool {
    task.template_id == "builtin:file-archive"
}

#[derive(Debug)]
struct ArchivePlan {
    operations: Vec<TaskOperation>,
    skipped: Vec<Value>,
}

async fn build_archive_plan(
    params: &ArchiveWorkflowParams,
) -> Result<ArchivePlan, WorkflowExecutionError> {
    let mut entries = tokio::fs::read_dir(&params.source_path)
        .await
        .map_err(|e| WorkflowExecutionError {
            message: format!("failed to read source directory: {e}"),
        })?;

    let mut operations = Vec::new();
    let mut skipped = Vec::new();

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| WorkflowExecutionError {
            message: format!("failed to read source directory entry: {e}"),
        })?
    {
        let path = entry.path();
        let metadata = entry.metadata().await.map_err(|e| WorkflowExecutionError {
            message: format!("failed to read file metadata: {e}"),
        })?;

        if metadata.is_dir() {
            skipped.push(json!({
                "path": path.display().to_string(),
                "reason": "directories are skipped"
            }));
            continue;
        }

        let source_path = path.display().to_string();
        if is_excluded(&source_path, &params.exclude_patterns) {
            skipped.push(json!({
                "path": source_path,
                "reason": "matched exclude pattern"
            }));
            continue;
        }

        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        let destination_filename = if params.naming_strategy == "normalize" {
            normalize_filename(filename)
        } else {
            filename.to_string()
        };
        let category = classify_path(&path);
        let destination_path = Path::new(&params.target_root)
            .join(category)
            .join(destination_filename)
            .display()
            .to_string();

        if destination_path == source_path {
            skipped.push(json!({
                "path": source_path,
                "reason": "source and destination are the same"
            }));
            continue;
        }

        operations.push(TaskOperation {
            kind: "move".to_string(),
            tool_name: "move_file".to_string(),
            parameters: json!({
                "source_path": source_path,
                "destination_path": destination_path,
                "create_parent": true
            }),
            path: Some(path.display().to_string()),
            destination_path: Some(destination_path),
        });
    }

    Ok(ArchivePlan {
        operations,
        skipped,
    })
}

fn default_naming_strategy() -> String {
    "preserve".to_string()
}

fn is_excluded(path: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| !pattern.trim().is_empty() && path.contains(pattern))
}

fn normalize_filename(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push('-');
        }
    }
    while normalized.contains("--") {
        normalized = normalized.replace("--", "-");
    }
    normalized.trim_matches('-').to_string()
}

fn classify_path(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" => "Images",
        "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" | "csv" | "xls" | "xlsx" | "ppt" | "pptx" => {
            "Documents"
        }
        "zip" | "tar" | "gz" | "bz2" | "7z" | "rar" => "Archives",
        "mp3" | "wav" | "m4a" | "flac" | "aac" | "ogg" => "Audio",
        "mp4" | "mov" | "mkv" | "avi" | "webm" => "Video",
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "c" | "cpp" | "h" | "json"
        | "yaml" | "yml" | "toml" => "Code",
        _ => "Other",
    }
}
