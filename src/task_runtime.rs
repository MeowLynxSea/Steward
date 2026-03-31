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
    Idle,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPendingOperation {
    pub request_id: Uuid,
    pub tool_name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub allow_always: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub mode: TaskMode,
    pub status: TaskStatus,
    pub title: String,
    pub updated_at: DateTime<Utc>,
    pub pending_operation: Option<TaskPendingOperation>,
    pub route: TaskRoute,
    pub last_error: Option<String>,
}

impl TaskRecord {
    fn new(message: &IncomingMessage, thread_id: Uuid) -> Self {
        Self {
            id: thread_id,
            mode: TaskMode::Ask,
            status: TaskStatus::Idle,
            title: crate::agent::truncate_for_preview(&message.content, 80),
            updated_at: Utc::now(),
            pending_operation: None,
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
        task.pending_operation = None;
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
        task.pending_operation = Some(TaskPendingOperation {
            request_id: pending.request_id,
            tool_name: pending.tool_name.clone(),
            description: pending.description.clone(),
            parameters: pending.display_parameters.clone(),
            allow_always: pending.allow_always,
        });
        task.updated_at = Utc::now();
    }

    pub async fn mark_completed(&self, task_id: Uuid) {
        if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
            task.status = TaskStatus::Completed;
            task.pending_operation = None;
            task.last_error = None;
            task.updated_at = Utc::now();
        }
    }

    pub async fn mark_failed(&self, task_id: Uuid, error: impl Into<String>) {
        if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
            task.status = TaskStatus::Failed;
            task.pending_operation = None;
            task.last_error = Some(error.into());
            task.updated_at = Utc::now();
        }
    }

    pub async fn mark_rejected(&self, task_id: Uuid, reason: impl Into<String>) {
        if let Some(task) = self.tasks.write().await.get_mut(&task_id) {
            task.status = TaskStatus::Rejected;
            task.pending_operation = None;
            task.last_error = Some(reason.into());
            task.updated_at = Utc::now();
        }
    }

    pub async fn toggle_mode(&self, task_id: Uuid, mode: TaskMode) -> Option<TaskRecord> {
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(&task_id)?;
        task.mode = mode;
        task.updated_at = Utc::now();
        Some(task.clone())
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
        assert_eq!(task.status, TaskStatus::Idle);
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
