//! Runtime task state used by the desktop-first API.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::agent::session::PendingApproval;
use crate::channels::IncomingMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskMode {
    #[default]
    Ask,
    Yolo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Queued,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOperation {
    pub kind: String,
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub path: Option<String>,
    pub destination_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPendingApproval {
    pub id: Uuid,
    pub risk: String,
    pub summary: String,
    pub operations: Vec<TaskOperation>,
    pub allow_always: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCurrentStep {
    pub id: String,
    pub kind: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskRoute {
    pub channel: String,
    pub user_id: String,
    pub owner_id: String,
    pub sender_id: String,
    pub thread_id: String,
    pub metadata: serde_json::Value,
    pub timezone: Option<String>,
}

impl TaskRoute {
    pub fn from_message(message: &IncomingMessage, thread_id: Uuid) -> Self {
        Self {
            channel: message.channel.clone(),
            user_id: message.user_id.clone(),
            owner_id: message.owner_id.clone(),
            sender_id: message.sender_id.clone(),
            thread_id: message
                .thread_id
                .clone()
                .unwrap_or_else(|| thread_id.to_string()),
            metadata: message.metadata.clone(),
            timezone: message.timezone.clone(),
        }
    }

    pub fn to_incoming_message(&self, content: impl Into<String>) -> IncomingMessage {
        IncomingMessage::new(&self.channel, &self.user_id, content)
            .with_owner_id(self.owner_id.clone())
            .with_sender_id(self.sender_id.clone())
            .with_thread(self.thread_id.clone())
            .with_metadata(self.metadata.clone())
            .with_timezone(self.timezone.clone().unwrap_or_else(|| "UTC".to_string()))
            .into_internal()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: Uuid,
    pub template_id: String,
    pub mode: TaskMode,
    pub status: TaskStatus,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current_step: Option<TaskCurrentStep>,
    pub pending_approval: Option<TaskPendingApproval>,
    #[serde(skip_serializing, skip_deserializing)]
    pub route: TaskRoute,
    pub last_error: Option<String>,
}

impl TaskRecord {
    fn new(message: &IncomingMessage, thread_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: thread_id,
            template_id: "legacy:session-thread".to_string(),
            mode: TaskMode::Ask,
            status: TaskStatus::Queued,
            title: crate::agent::truncate_for_preview(&message.content, 80),
            created_at: now,
            updated_at: now,
            current_step: Some(TaskCurrentStep {
                id: format!("task-{thread_id}"),
                kind: "log".to_string(),
                title: "Queued".to_string(),
            }),
            pending_approval: None,
            route: TaskRoute::from_message(message, thread_id),
            last_error: None,
        }
    }
}

#[derive(Default)]
pub struct TaskRuntime {
    tasks: RwLock<HashMap<Uuid, TaskRecord>>,
}

impl TaskRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn ensure_task(&self, message: &IncomingMessage, thread_id: Uuid) -> TaskRecord {
        let mut tasks = self.tasks.write().await;
        let entry = tasks
            .entry(thread_id)
            .or_insert_with(|| TaskRecord::new(message, thread_id));
        entry.title = crate::agent::truncate_for_preview(&message.content, 80);
        entry.route = TaskRoute::from_message(message, thread_id);
        entry.updated_at = Utc::now();
        entry.clone()
    }

    pub async fn get_task(&self, task_id: Uuid) -> Option<TaskRecord> {
        self.tasks.read().await.get(&task_id).cloned()
    }

    pub async fn list_tasks(&self) -> Vec<TaskRecord> {
        let mut tasks: Vec<_> = self.tasks.read().await.values().cloned().collect();
        tasks.sort_by_key(|task| task.updated_at);
        tasks.reverse();
        tasks
    }

    pub async fn mode_for_task(&self, task_id: Uuid) -> TaskMode {
        self.tasks
            .read()
            .await
            .get(&task_id)
            .map(|task| task.mode)
            .unwrap_or_default()
    }

    pub async fn mark_running(&self, message: &IncomingMessage, task_id: Uuid) {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .entry(task_id)
            .or_insert_with(|| TaskRecord::new(message, task_id));
        task.route = TaskRoute::from_message(message, task_id);
        task.status = TaskStatus::Running;
        task.current_step = Some(TaskCurrentStep {
            id: format!("run-{task_id}"),
            kind: "log".to_string(),
            title: "Running".to_string(),
        });
        task.pending_approval = None;
        task.last_error = None;
        task.updated_at = Utc::now();
    }

    pub async fn mark_waiting_approval(
        &self,
        message: &IncomingMessage,
        task_id: Uuid,
        pending: &PendingApproval,
    ) {
        let mut tasks = self.tasks.write().await;
        let task = tasks
            .entry(task_id)
            .or_insert_with(|| TaskRecord::new(message, task_id));
        task.route = TaskRoute::from_message(message, task_id);
        task.status = TaskStatus::WaitingApproval;
        task.current_step = Some(TaskCurrentStep {
            id: pending.request_id.to_string(),
            kind: "approval".to_string(),
            title: pending.description.clone(),
        });
        task.pending_approval = Some(TaskPendingApproval {
            id: pending.request_id,
            risk: infer_risk(&pending.tool_name).to_string(),
            summary: pending.description.clone(),
            operations: vec![TaskOperation {
                kind: "tool_call".to_string(),
                tool_name: pending.tool_name.clone(),
                path: extract_path(&pending.display_parameters),
                destination_path: extract_destination_path(&pending.display_parameters),
                parameters: pending.display_parameters.clone(),
            }],
            allow_always: pending.allow_always,
        });
        task.updated_at = Utc::now();
    }

    pub async fn mark_completed(&self, task_id: Uuid) {
        if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
            task.status = TaskStatus::Completed;
            task.current_step = Some(TaskCurrentStep {
                id: format!("completed-{task_id}"),
                kind: "result".to_string(),
                title: "Completed".to_string(),
            });
            task.pending_approval = None;
            task.last_error = None;
            task.updated_at = Utc::now();
        }
    }

    pub async fn mark_failed(&self, task_id: Uuid, error: impl Into<String>) {
        if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
            let error = error.into();
            task.status = TaskStatus::Failed;
            task.current_step = Some(TaskCurrentStep {
                id: format!("failed-{task_id}"),
                kind: "result".to_string(),
                title: "Failed".to_string(),
            });
            task.pending_approval = None;
            task.last_error = Some(error);
            task.updated_at = Utc::now();
        }
    }

    pub async fn mark_rejected(&self, task_id: Uuid, reason: impl Into<String>) {
        if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
            let reason = reason.into();
            task.status = TaskStatus::Rejected;
            task.current_step = Some(TaskCurrentStep {
                id: format!("rejected-{task_id}"),
                kind: "result".to_string(),
                title: "Rejected".to_string(),
            });
            task.pending_approval = None;
            task.last_error = Some(reason);
            task.updated_at = Utc::now();
        }
    }

    pub async fn toggle_mode(&self, task_id: Uuid, mode: TaskMode) -> Option<TaskRecord> {
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(&task_id)?;
        task.mode = mode;
        task.current_step = Some(TaskCurrentStep {
            id: format!("mode-{task_id}"),
            kind: "log".to_string(),
            title: format!("Mode changed to {}", serialize_mode(mode)),
        });
        task.updated_at = Utc::now();
        Some(task.clone())
    }
}

fn infer_risk(tool_name: &str) -> &'static str {
    let normalized = tool_name.to_ascii_lowercase();
    if normalized.contains("delete") || normalized.contains("remove") {
        "file_delete"
    } else if normalized.contains("write")
        || normalized.contains("move")
        || normalized.contains("rename")
        || normalized.contains("copy")
    {
        "file_write"
    } else if normalized.contains("http")
        || normalized.contains("fetch")
        || normalized.contains("request")
        || normalized.contains("web")
    {
        "network_request"
    } else {
        "external_side_effect"
    }
}

fn extract_path(parameters: &serde_json::Value) -> Option<String> {
    extract_string_field(parameters, &["path", "source_path", "file_path"])
}

fn extract_destination_path(parameters: &serde_json::Value) -> Option<String> {
    extract_string_field(parameters, &["destination_path", "target_path", "to"])
}

fn extract_string_field(parameters: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let object = parameters.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key)?.as_str().map(str::to_string))
}

fn serialize_mode(mode: TaskMode) -> &'static str {
    match mode {
        TaskMode::Ask => "ask",
        TaskMode::Yolo => "yolo",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ensure_task_reuses_thread_id() {
        let runtime = TaskRuntime::new();
        let task_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "user-1", "organize files");

        let task = runtime.ensure_task(&message, task_id).await;
        assert_eq!(task.id, task_id);
        assert_eq!(task.mode, TaskMode::Ask);
        assert_eq!(task.status, TaskStatus::Queued);
    }

    #[tokio::test]
    async fn toggle_mode_updates_existing_task() {
        let runtime = TaskRuntime::new();
        let task_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "user-1", "organize files");
        runtime.ensure_task(&message, task_id).await;

        let task = runtime
            .toggle_mode(task_id, TaskMode::Yolo)
            .await
            .expect("task exists");

        assert_eq!(task.mode, TaskMode::Yolo);
        assert_eq!(runtime.mode_for_task(task_id).await, TaskMode::Yolo);
    }
}
