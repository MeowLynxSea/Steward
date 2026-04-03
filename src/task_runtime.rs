//! Runtime task state used by the desktop-first API.

use std::collections::{HashMap, hash_map::Entry};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::agent::session::PendingApproval;
use crate::channels::IncomingMessage;
use crate::db::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskMode {
    #[default]
    Ask,
    Yolo,
}

impl TaskMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Yolo => "yolo",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "yolo" => Self::Yolo,
            _ => Self::Ask,
        }
    }
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
    Cancelled,
    Rejected,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "waiting_approval" => Self::WaitingApproval,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "rejected" => Self::Rejected,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOperation {
    pub kind: String,
    pub tool_name: String,
    pub parameters: Value,
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
    pub metadata: Value,
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
    pub correlation_id: String,
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
    pub result_metadata: Option<Value>,
}

impl TaskRecord {
    fn new(message: &IncomingMessage, thread_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: thread_id,
            correlation_id: thread_id.to_string(),
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
            result_metadata: None,
        }
    }

    fn workflow(
        task_id: Uuid,
        template_id: impl Into<String>,
        title: impl Into<String>,
        mode: TaskMode,
        result_metadata: Option<Value>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: task_id,
            correlation_id: task_id.to_string(),
            template_id: template_id.into(),
            mode,
            status: TaskStatus::Queued,
            title: title.into(),
            created_at: now,
            updated_at: now,
            current_step: Some(TaskCurrentStep {
                id: format!("task-{task_id}"),
                kind: "log".to_string(),
                title: "Queued".to_string(),
            }),
            pending_approval: None,
            route: TaskRoute::default(),
            last_error: None,
            result_metadata,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTimelineEntry {
    pub sequence: u64,
    pub correlation_id: String,
    pub event: String,
    pub status: TaskStatus,
    pub mode: TaskMode,
    pub current_step: Option<TaskCurrentStep>,
    pub pending_approval: Option<TaskPendingApproval>,
    pub last_error: Option<String>,
    pub result_metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDetail {
    pub task: TaskRecord,
    pub timeline: Vec<TaskTimelineEntry>,
}

#[derive(Default)]
pub struct TaskRuntime {
    tasks: RwLock<HashMap<Uuid, TaskRecord>>,
    store: Option<Arc<dyn Database>>,
    owner_id: Option<String>,
}

impl TaskRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_store(owner_id: String, store: Arc<dyn Database>) -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            store: Some(store),
            owner_id: Some(owner_id),
        }
    }

    pub async fn ensure_task(&self, message: &IncomingMessage, thread_id: Uuid) -> TaskRecord {
        let mut created = false;
        let task = {
            let mut tasks = self.tasks.write().await;
            match tasks.entry(thread_id) {
                Entry::Occupied(mut entry) => {
                    let task = entry.get_mut();
                    task.title = crate::agent::truncate_for_preview(&message.content, 80);
                    task.route = TaskRoute::from_message(message, thread_id);
                    task.correlation_id = thread_id.to_string();
                    task.updated_at = Utc::now();
                    task.clone()
                }
                Entry::Vacant(entry) => {
                    let task = TaskRecord::new(message, thread_id);
                    created = true;
                    entry.insert(task.clone());
                    task
                }
            }
        };

        self.persist_task(&task).await;
        if created {
            tracing::info!(
                task_id = %task.id,
                correlation_id = %task.correlation_id,
                owner_id = %task.route.owner_id,
                "task created"
            );
            self.append_timeline(
                &task,
                "task.created",
                task.result_metadata.clone().unwrap_or_else(|| json!({})),
            )
            .await;
        }

        task
    }

    pub async fn create_workflow_task(
        &self,
        template_id: impl Into<String>,
        title: impl Into<String>,
        mode: TaskMode,
        result_metadata: Option<Value>,
    ) -> TaskRecord {
        let task_id = Uuid::new_v4();
        let task = TaskRecord::workflow(task_id, template_id, title, mode, result_metadata);
        self.tasks.write().await.insert(task_id, task.clone());
        self.persist_task(&task).await;
        self.append_timeline(
            &task,
            "task.created",
            task.result_metadata.clone().unwrap_or_else(|| json!({})),
        )
        .await;
        task
    }

    pub async fn get_task(&self, task_id: Uuid) -> Option<TaskRecord> {
        if let Some(task) = self.tasks.read().await.get(&task_id).cloned() {
            return Some(task);
        }

        let task = self.load_task_from_store(task_id).await?;
        self.tasks.write().await.insert(task_id, task.clone());
        Some(task)
    }

    pub async fn get_task_detail(&self, task_id: Uuid) -> Option<TaskDetail> {
        let task = self.get_task(task_id).await?;
        let timeline = self.load_timeline(task_id).await;
        Some(TaskDetail { task, timeline })
    }

    pub async fn list_tasks(&self) -> Vec<TaskRecord> {
        let mut merged = HashMap::new();

        if let Some((store, owner_id)) = self.persistence() {
            match store.list_task_records(owner_id).await {
                Ok(tasks) => {
                    for task in tasks {
                        merged.insert(task.id, task);
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to list persisted task records");
                }
            }
        }

        for task in self.tasks.read().await.values().cloned() {
            merged.insert(task.id, task);
        }

        let mut tasks: Vec<_> = merged.into_values().collect();
        tasks.sort_by_key(|task| task.updated_at);
        tasks.reverse();
        tasks
    }

    pub async fn mode_for_task(&self, task_id: Uuid) -> TaskMode {
        self.get_task(task_id)
            .await
            .map(|task| task.mode)
            .unwrap_or_default()
    }

    pub async fn mark_running(&self, message: &IncomingMessage, task_id: Uuid) {
        self.apply_update(
            task_id,
            Some(message),
            "task.step.started",
            json!({ "title": "Running" }),
            |task| {
                task.status = TaskStatus::Running;
                task.current_step = Some(TaskCurrentStep {
                    id: format!("run-{task_id}"),
                    kind: "log".to_string(),
                    title: "Running".to_string(),
                });
                task.pending_approval = None;
                task.last_error = None;
                task.result_metadata = None;
            },
        )
        .await;
    }

    pub async fn mark_waiting_approval(
        &self,
        message: &IncomingMessage,
        task_id: Uuid,
        pending: &PendingApproval,
    ) {
        self.apply_update(
            task_id,
            Some(message),
            "task.waiting_approval",
            json!({
                "approval_id": pending.request_id,
                "risk": infer_risk(&pending.tool_name),
            }),
            |task| {
                let display_parameters = sanitize_task_parameters(&pending.display_parameters);
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
                        path: extract_path(&display_parameters),
                        destination_path: extract_destination_path(&display_parameters),
                        parameters: display_parameters,
                    }],
                    allow_always: pending.allow_always,
                });
            },
        )
        .await;
    }

    pub async fn set_waiting_approval(
        &self,
        task_id: Uuid,
        pending: TaskPendingApproval,
    ) -> Option<TaskRecord> {
        let summary = pending.summary.clone();
        self.apply_update(
            task_id,
            None,
            "task.waiting_approval",
            json!({ "approval_id": pending.id, "risk": pending.risk }),
            |task| {
                task.status = TaskStatus::WaitingApproval;
                task.current_step = Some(TaskCurrentStep {
                    id: pending.id.to_string(),
                    kind: "approval".to_string(),
                    title: summary,
                });
                task.pending_approval = Some(pending);
            },
        )
        .await
    }

    pub async fn mark_completed(&self, task_id: Uuid) {
        self.mark_completed_with_result(task_id, Some(json!({ "outcome": "completed" })))
            .await;
    }

    pub async fn mark_completed_with_result(&self, task_id: Uuid, result_metadata: Option<Value>) {
        self.apply_update(
            task_id,
            None,
            "task.completed",
            json!({ "result": "completed" }),
            |task| {
                task.status = TaskStatus::Completed;
                task.current_step = Some(TaskCurrentStep {
                    id: format!("completed-{task_id}"),
                    kind: "result".to_string(),
                    title: "Completed".to_string(),
                });
                task.pending_approval = None;
                task.last_error = None;
                task.result_metadata = result_metadata.clone();
            },
        )
        .await;
    }

    pub async fn mark_failed(&self, task_id: Uuid, error: impl Into<String>) {
        self.mark_failed_with_result(task_id, error, None).await;
    }

    pub async fn mark_failed_with_result(
        &self,
        task_id: Uuid,
        error: impl Into<String>,
        result_metadata: Option<Value>,
    ) {
        let error = error.into();
        self.apply_update(
            task_id,
            None,
            "task.failed",
            json!({ "error": error }),
            |task| {
                task.status = TaskStatus::Failed;
                task.current_step = Some(TaskCurrentStep {
                    id: format!("failed-{task_id}"),
                    kind: "result".to_string(),
                    title: "Failed".to_string(),
                });
                task.pending_approval = None;
                task.last_error = Some(error.clone());
                task.result_metadata = result_metadata
                    .clone()
                    .or_else(|| Some(json!({ "failure_reason": error })));
            },
        )
        .await;
    }

    pub async fn mark_cancelled(&self, task_id: Uuid, reason: impl Into<String>) {
        let reason = reason.into();
        self.apply_update(
            task_id,
            None,
            "task.cancelled",
            json!({ "reason": reason }),
            |task| {
                task.status = TaskStatus::Cancelled;
                task.current_step = Some(TaskCurrentStep {
                    id: format!("cancelled-{task_id}"),
                    kind: "result".to_string(),
                    title: "Cancelled".to_string(),
                });
                task.pending_approval = None;
                task.last_error = Some(reason.clone());
                task.result_metadata = Some(json!({ "cancel_reason": reason }));
            },
        )
        .await;
    }

    pub async fn mark_rejected(&self, task_id: Uuid, reason: impl Into<String>) {
        let reason = reason.into();
        self.apply_update(
            task_id,
            None,
            "task.rejected",
            json!({ "reason": reason }),
            |task| {
                task.status = TaskStatus::Rejected;
                task.current_step = Some(TaskCurrentStep {
                    id: format!("rejected-{task_id}"),
                    kind: "result".to_string(),
                    title: "Rejected".to_string(),
                });
                task.pending_approval = None;
                task.last_error = Some(reason.clone());
                task.result_metadata = Some(json!({ "rejection_reason": reason }));
            },
        )
        .await;
    }

    pub async fn toggle_mode(&self, task_id: Uuid, mode: TaskMode) -> Option<TaskRecord> {
        self.apply_update(
            task_id,
            None,
            "task.mode_changed",
            json!({ "mode": mode.as_str() }),
            |task| {
                task.mode = mode;
                task.current_step = Some(TaskCurrentStep {
                    id: format!("mode-{task_id}"),
                    kind: "log".to_string(),
                    title: format!("Mode changed to {}", mode.as_str()),
                });
            },
        )
        .await
    }

    pub async fn update_result_metadata(
        &self,
        task_id: Uuid,
        result_metadata: Value,
    ) -> Option<TaskRecord> {
        self.apply_update(
            task_id,
            None,
            "task.updated",
            json!({ "result_metadata": result_metadata }),
            |task| {
                task.result_metadata = Some(result_metadata.clone());
            },
        )
        .await
    }

    async fn apply_update<F>(
        &self,
        task_id: Uuid,
        message: Option<&IncomingMessage>,
        event: &str,
        metadata: Value,
        mutate: F,
    ) -> Option<TaskRecord>
    where
        F: FnOnce(&mut TaskRecord),
    {
        let mut task = self.get_task(task_id).await?;
        if let Some(message) = message {
            task.route = TaskRoute::from_message(message, task_id);
        }
        task.correlation_id = task_id.to_string();
        mutate(&mut task);
        task.updated_at = Utc::now();

        tracing::info!(
            task_id = %task.id,
            correlation_id = %task.correlation_id,
            event,
            status = task.status.as_str(),
            mode = task.mode.as_str(),
            "task state transition"
        );

        self.tasks.write().await.insert(task_id, task.clone());
        self.persist_task(&task).await;
        self.append_timeline(&task, event, metadata).await;
        Some(task)
    }

    async fn load_task_from_store(&self, task_id: Uuid) -> Option<TaskRecord> {
        let (store, owner_id) = self.persistence()?;
        match store.get_task_record(owner_id, task_id).await {
            Ok(task) => task,
            Err(error) => {
                tracing::warn!(%task_id, %error, "failed to load task record");
                None
            }
        }
    }

    async fn load_timeline(&self, task_id: Uuid) -> Vec<TaskTimelineEntry> {
        let Some((store, owner_id)) = self.persistence() else {
            return Vec::new();
        };
        match store.list_task_timeline(owner_id, task_id).await {
            Ok(timeline) => timeline,
            Err(error) => {
                tracing::warn!(%task_id, %error, "failed to load task timeline");
                Vec::new()
            }
        }
    }

    async fn persist_task(&self, task: &TaskRecord) {
        let Some((store, owner_id)) = self.persistence() else {
            return;
        };
        if let Err(error) = store.upsert_task_record(owner_id, task).await {
            tracing::warn!(task_id = %task.id, %error, "failed to persist task record");
        }
    }

    async fn append_timeline(&self, task: &TaskRecord, event: &str, metadata: Value) {
        let Some((store, owner_id)) = self.persistence() else {
            return;
        };
        if let Err(error) = store
            .append_task_timeline(owner_id, task.id, event, task, &metadata)
            .await
        {
            tracing::warn!(task_id = %task.id, event, %error, "failed to persist task timeline");
        } else {
            tracing::debug!(
                task_id = %task.id,
                correlation_id = %task.correlation_id,
                event,
                "persisted task timeline event"
            );
        }
    }

    fn persistence(&self) -> Option<(&Arc<dyn Database>, &str)> {
        Some((self.store.as_ref()?, self.owner_id.as_deref()?))
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

fn extract_path(parameters: &Value) -> Option<String> {
    extract_string_field(parameters, &["path", "source_path", "file_path"])
}

fn extract_destination_path(parameters: &Value) -> Option<String> {
    extract_string_field(parameters, &["destination_path", "target_path", "to"])
}

fn extract_string_field(parameters: &Value, keys: &[&str]) -> Option<String> {
    let object = parameters.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key)?.as_str().map(str::to_string))
}

fn sanitize_task_parameters(parameters: &Value) -> Value {
    match parameters {
        Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in map {
                if is_display_path_key(key) {
                    sanitized.insert(
                        key.clone(),
                        value
                            .as_str()
                            .map(|path| Value::String(format_display_path(path)))
                            .unwrap_or_else(|| sanitize_task_parameters(value)),
                    );
                } else {
                    sanitized.insert(key.clone(), sanitize_task_parameters(value));
                }
            }
            Value::Object(sanitized)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_task_parameters).collect()),
        other => other.clone(),
    }
}

fn is_display_path_key(key: &str) -> bool {
    matches!(
        key,
        "path" | "source_path" | "destination_path" | "target_path" | "to" | "renamed_copy_path"
    )
}

fn format_display_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    let parts: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        [] => "workspace item".to_string(),
        [single] => (*single).to_string(),
        [.., parent, leaf] => format!(".../{parent}/{leaf}"),
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
        assert_eq!(task.correlation_id, task_id.to_string());
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

    #[tokio::test]
    async fn mark_waiting_approval_infers_network_risk_and_correlation_id() {
        let runtime = TaskRuntime::new();
        let task_id = Uuid::new_v4();
        let message = IncomingMessage::new("test", "user-1", "fetch latest docs")
            .with_thread(task_id.to_string());
        runtime.ensure_task(&message, task_id).await;

        let pending = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "http_fetch".to_string(),
            parameters: json!({"url": "https://example.com"}),
            display_parameters: json!({"url": "https://example.com"}),
            description: "fetch a remote url".to_string(),
            tool_call_id: "call_1".to_string(),
            context_messages: Vec::new(),
            deferred_tool_calls: Vec::new(),
            user_timezone: Some("UTC".to_string()),
            allow_always: false,
        };

        runtime
            .mark_waiting_approval(&message, task_id, &pending)
            .await;

        let detail = runtime.get_task_detail(task_id).await.expect("task detail");
        assert_eq!(detail.task.correlation_id, task_id.to_string());
        assert_eq!(detail.task.status, TaskStatus::WaitingApproval);
        assert_eq!(
            detail
                .task
                .pending_approval
                .as_ref()
                .expect("pending approval")
                .risk,
            "network_request"
        );
    }
}
