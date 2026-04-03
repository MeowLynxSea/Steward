//! Job management tools.
//!
//! These tools manage locally executed jobs:
//! - Create new jobs/tasks
//! - List existing jobs
//! - Check job status
//! - Cancel running jobs
//! - Read job event logs
//! - Send follow-up prompts to running jobs

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::bootstrap::ironclaw_base_dir;
use crate::channels::IncomingMessage;
use crate::context::{ContextManager, JobContext, JobState};
use crate::db::Database;
use crate::secrets::SecretsStore;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

/// Lazy scheduler reference, filled after Agent::new creates the Scheduler.
///
/// Solves the chicken-and-egg: tools are registered before the Scheduler exists
/// (Scheduler needs the ToolRegistry). Created empty, filled after Agent::new.
pub type SchedulerSlot = Arc<RwLock<Option<Arc<crate::agent::Scheduler>>>>;

/// Resolve a job ID from a full UUID or a short prefix (like git short SHAs).
///
/// Tries full UUID parse first. If that fails, treats the input as a hex prefix
/// and searches the context manager for a unique match.
async fn resolve_job_id(input: &str, context_manager: &ContextManager) -> Result<Uuid, ToolError> {
    if let Ok(id) = Uuid::parse_str(input) {
        return Ok(id);
    }

    if input.len() < 4 {
        return Err(ToolError::InvalidParameters(
            "job ID prefix must be at least 4 hex characters".to_string(),
        ));
    }

    let input_lower = input.to_lowercase();
    let all_ids = context_manager.all_jobs().await;
    let matches: Vec<Uuid> = all_ids
        .into_iter()
        .filter(|id| {
            let hex = id.to_string().replace('-', "");
            hex.starts_with(&input_lower)
        })
        .collect();

    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(ToolError::InvalidParameters(format!(
            "no job found matching prefix '{input}'"
        ))),
        n => Err(ToolError::InvalidParameters(format!(
            "ambiguous prefix '{input}' matches {n} jobs, provide more characters"
        ))),
    }
}

fn projects_base() -> PathBuf {
    ironclaw_base_dir().join("projects")
}

fn canonicalize_or_create(path: &Path) -> Result<PathBuf, ToolError> {
    std::fs::create_dir_all(path).map_err(|e| {
        ToolError::ExecutionFailed(format!(
            "failed to create project directory {}: {}",
            path.display(),
            e
        ))
    })?;

    path.canonicalize().map_err(|e| {
        ToolError::ExecutionFailed(format!(
            "failed to canonicalize project directory {}: {}",
            path.display(),
            e
        ))
    })
}

/// Resolve the project directory, creating it if needed.
///
/// If no explicit directory is provided, create a managed project directory
/// under `~/.ironcowork/projects/{job_id}`.
///
/// If a directory is provided, accept any accessible local path and create it
/// on demand. Relative paths are resolved against the current process cwd.
fn resolve_project_dir(explicit: Option<PathBuf>, project_id: Uuid) -> Result<PathBuf, ToolError> {
    match explicit {
        Some(path) => {
            let resolved = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!(
                            "failed to determine current directory: {e}"
                        ))
                    })?
                    .join(path)
            };
            canonicalize_or_create(&resolved)
        }
        None => {
            let base = projects_base();
            std::fs::create_dir_all(&base).map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "failed to create projects base {}: {}",
                    base.display(),
                    e
                ))
            })?;
            canonicalize_or_create(&base.join(project_id.to_string()))
        }
    }
}

fn execution_strategy_from_params(params: &serde_json::Value) -> Result<String, ToolError> {
    match params.get("mode").and_then(|value| value.as_str()) {
        None | Some("native") => Ok("native".to_string()),
        Some("claude_code") => Ok("claude_code".to_string()),
        Some(other) => Err(ToolError::InvalidParameters(format!(
            "unsupported mode '{other}', expected 'native' or 'claude_code'"
        ))),
    }
}

fn read_project_dir_param(params: &serde_json::Value) -> Result<Option<PathBuf>, ToolError> {
    params
        .get("project_dir")
        .map(|value| {
            value.as_str().map(PathBuf::from).ok_or_else(|| {
                ToolError::InvalidParameters("project_dir must be a string".to_string())
            })
        })
        .transpose()
}

/// Tool for creating a new local job.
pub struct CreateJobTool {
    context_manager: Arc<ContextManager>,
    scheduler_slot: Option<SchedulerSlot>,
    inject_tx: Option<tokio::sync::mpsc::Sender<IncomingMessage>>,
}

impl CreateJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self {
            context_manager,
            scheduler_slot: None,
            inject_tx: None,
        }
    }

    pub fn with_monitor_deps(
        mut self,
        inject_tx: tokio::sync::mpsc::Sender<IncomingMessage>,
    ) -> Self {
        self.inject_tx = Some(inject_tx);
        self
    }

    pub fn with_scheduler_slot(mut self, slot: SchedulerSlot) -> Self {
        self.scheduler_slot = Some(slot);
        self
    }

    pub fn with_secrets(self, _secrets: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self
    }

    async fn execute_local(
        &self,
        title: &str,
        description: &str,
        ctx: &JobContext,
        execution_strategy: &str,
        explicit_dir: Option<PathBuf>,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let project_dir = resolve_project_dir(explicit_dir, Uuid::new_v4())?;
        let metadata = serde_json::json!({
            "execution_strategy": execution_strategy,
            "project_dir": project_dir.display().to_string(),
        });

        if let Some(ref slot) = self.scheduler_slot
            && let Some(ref scheduler) = *slot.read().await
        {
            let job_id = scheduler
                .dispatch_job(&ctx.user_id, title, description, Some(metadata))
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            return Ok(ToolOutput::success(
                serde_json::json!({
                    "job_id": job_id.to_string(),
                    "title": title,
                    "status": "in_progress",
                    "execution_strategy": execution_strategy,
                    "project_dir": project_dir.display().to_string(),
                    "message": format!("Created and scheduled job '{title}'"),
                }),
                start.elapsed(),
            ));
        }

        let job_id = self
            .context_manager
            .create_job_for_user(&ctx.user_id, title, description)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let project_dir_display = project_dir.display().to_string();
        self.context_manager
            .update_context(job_id, |job_ctx| {
                job_ctx.metadata = serde_json::json!({
                    "execution_strategy": execution_strategy,
                    "project_dir": project_dir_display,
                });
            })
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "job_id": job_id.to_string(),
                "title": title,
                "status": "pending",
                "execution_strategy": execution_strategy,
                "project_dir": project_dir.display().to_string(),
                "message": format!("Created job '{title}' (scheduler unavailable)"),
            }),
            start.elapsed(),
        ))
    }
}

#[async_trait]
impl Tool for CreateJobTool {
    fn name(&self) -> &str {
        "create_job"
    }

    fn description(&self) -> &str {
        "Create and execute a local job. Jobs run inside the main IronCowork runtime, using either the native worker or the local Claude Code strategy."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "A short title for the job"
                },
                "description": {
                    "type": "string",
                    "description": "Full description of what needs to be done"
                },
                "mode": {
                    "type": "string",
                    "enum": ["native", "claude_code"],
                    "description": "Execution strategy. 'native' uses IronCowork's local worker. 'claude_code' runs the local Claude CLI."
                },
                "project_dir": {
                    "type": "string",
                    "description": "Optional local working directory for the job. If omitted, IronCowork creates one under ~/.ironcowork/projects/."
                }
            },
            "required": ["title", "description"]
        })
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(5, 30))
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let title = require_str(&params, "title")?;
        let description = require_str(&params, "description")?;
        let mode = execution_strategy_from_params(&params)?;
        let project_dir = read_project_dir_param(&params)?;
        self.execute_local(title, description, ctx, &mode, project_dir)
            .await
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for listing jobs.
pub struct ListJobsTool {
    context_manager: Arc<ContextManager>,
}

impl ListJobsTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for ListJobsTool {
    fn name(&self) -> &str {
        "list_jobs"
    }

    fn description(&self) -> &str {
        "List all jobs or filter by status. Shows job IDs, titles, and current status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "string",
                    "description": "Filter by status: 'active', 'completed', 'failed', 'all' (default: 'all')",
                    "enum": ["active", "completed", "failed", "all"]
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let filter = params
            .get("filter")
            .and_then(|value| value.as_str())
            .unwrap_or("all");

        let job_ids = match filter {
            "active" => self.context_manager.active_jobs_for(&ctx.user_id).await,
            _ => self.context_manager.all_jobs_for(&ctx.user_id).await,
        };

        let mut jobs = Vec::new();
        for job_id in job_ids {
            if let Ok(job_ctx) = self.context_manager.get_context(job_id).await {
                let include = match filter {
                    "completed" => job_ctx.state == JobState::Completed,
                    "failed" => job_ctx.state == JobState::Failed,
                    "active" => job_ctx.state.is_active(),
                    _ => true,
                };

                if include {
                    jobs.push(serde_json::json!({
                        "job_id": job_id.to_string(),
                        "title": job_ctx.title,
                        "status": format!("{}", job_ctx.state),
                        "created_at": job_ctx.created_at.to_rfc3339(),
                    }));
                }
            }
        }

        let summary = self.context_manager.summary_for(&ctx.user_id).await;
        Ok(ToolOutput::success(
            serde_json::json!({
                "jobs": jobs,
                "summary": {
                    "total": summary.total,
                    "pending": summary.pending,
                    "in_progress": summary.in_progress,
                    "completed": summary.completed,
                    "failed": summary.failed,
                }
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for checking job status.
pub struct JobStatusTool {
    context_manager: Arc<ContextManager>,
}

impl JobStatusTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for JobStatusTool {
    fn name(&self) -> &str {
        "job_status"
    }

    fn description(&self) -> &str {
        "Check the status and details of a specific job by its ID."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let requester_id = ctx.user_id.clone();
        let job_id_str = require_str(&params, "job_id")?;
        let job_id = resolve_job_id(job_id_str, &self.context_manager).await?;

        match self.context_manager.get_context(job_id).await {
            Ok(job_ctx) => {
                if job_ctx.user_id != requester_id {
                    return Ok(ToolOutput::success(
                        serde_json::json!({
                            "error": "Job not found"
                        }),
                        start.elapsed(),
                    ));
                }

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "job_id": job_id.to_string(),
                        "title": job_ctx.title,
                        "description": job_ctx.description,
                        "status": format!("{}", job_ctx.state),
                        "created_at": job_ctx.created_at.to_rfc3339(),
                        "started_at": job_ctx.started_at.map(|t| t.to_rfc3339()),
                        "completed_at": job_ctx.completed_at.map(|t| t.to_rfc3339()),
                        "actual_cost": job_ctx.actual_cost.to_string(),
                        "fallback_deliverable": job_ctx.metadata.get("fallback_deliverable"),
                    }),
                    start.elapsed(),
                ))
            }
            Err(e) => Ok(ToolOutput::success(
                serde_json::json!({
                    "error": format!("Job not found: {}", e)
                }),
                start.elapsed(),
            )),
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for canceling a job.
pub struct CancelJobTool {
    context_manager: Arc<ContextManager>,
}

impl CancelJobTool {
    pub fn new(context_manager: Arc<ContextManager>) -> Self {
        Self { context_manager }
    }
}

#[async_trait]
impl Tool for CancelJobTool {
    fn name(&self) -> &str {
        "cancel_job"
    }

    fn description(&self) -> &str {
        "Cancel a running or pending job. The job will be marked as cancelled and stopped."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let requester_id = ctx.user_id.clone();
        let job_id_str = require_str(&params, "job_id")?;
        let job_id = resolve_job_id(job_id_str, &self.context_manager).await?;

        match self
            .context_manager
            .update_context(job_id, |job_ctx| {
                if job_ctx.user_id != requester_id {
                    return Err("Job not found".to_string());
                }
                job_ctx.transition_to(JobState::Cancelled, Some("Cancelled by user".to_string()))
            })
            .await
        {
            Ok(Ok(())) => Ok(ToolOutput::success(
                serde_json::json!({
                    "job_id": job_id.to_string(),
                    "status": "cancelled",
                    "message": "Job cancelled successfully",
                }),
                start.elapsed(),
            )),
            Ok(Err(reason)) => Ok(ToolOutput::success(
                serde_json::json!({
                    "error": format!("Cannot cancel job: {reason}")
                }),
                start.elapsed(),
            )),
            Err(e) => Ok(ToolOutput::success(
                serde_json::json!({
                    "error": format!("Job not found: {e}")
                }),
                start.elapsed(),
            )),
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for reading job event logs.
pub struct JobEventsTool {
    store: Arc<dyn Database>,
    context_manager: Arc<ContextManager>,
}

impl JobEventsTool {
    pub fn new(store: Arc<dyn Database>, context_manager: Arc<ContextManager>) -> Self {
        Self {
            store,
            context_manager,
        }
    }
}

#[async_trait]
impl Tool for JobEventsTool {
    fn name(&self) -> &str {
        "job_events"
    }

    fn description(&self) -> &str {
        "Read the event log for a job. Shows messages, tool calls, results, and status changes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of events to return (default 50, most recent)"
                }
            },
            "required": ["job_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let job_id_str = require_str(&params, "job_id")?;
        let job_id = resolve_job_id(job_id_str, &self.context_manager).await?;

        let job_ctx = self
            .context_manager
            .get_context(job_id)
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!("job {job_id} not found or context unavailable"))
            })?;

        if job_ctx.user_id != ctx.user_id {
            return Err(ToolError::ExecutionFailed(format!(
                "job {job_id} does not belong to current user"
            )));
        }

        const MAX_EVENT_LIMIT: i64 = 1000;
        let limit = params
            .get("limit")
            .and_then(|value| value.as_i64())
            .unwrap_or(50)
            .clamp(1, MAX_EVENT_LIMIT);

        let events = self
            .store
            .list_job_events(job_id, Some(limit))
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to load job events: {e}")))?;

        let recent: Vec<serde_json::Value> = events
            .iter()
            .map(|event| {
                serde_json::json!({
                    "event_type": event.event_type,
                    "data": event.data,
                    "created_at": event.created_at.to_rfc3339(),
                })
            })
            .collect();

        Ok(ToolOutput::success(
            serde_json::json!({
                "job_id": job_id.to_string(),
                "total_events": events.len(),
                "returned": recent.len(),
                "events": recent,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}

/// Tool for sending follow-up prompts to a running job.
pub struct JobPromptTool {
    scheduler_slot: SchedulerSlot,
    context_manager: Arc<ContextManager>,
}

impl JobPromptTool {
    pub fn new(scheduler_slot: SchedulerSlot, context_manager: Arc<ContextManager>) -> Self {
        Self {
            scheduler_slot,
            context_manager,
        }
    }
}

#[async_trait]
impl Tool for JobPromptTool {
    fn name(&self) -> &str {
        "job_prompt"
    }

    fn description(&self) -> &str {
        "Send a follow-up prompt to a running job. Use this to provide extra instructions or answer a job's questions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job ID (full UUID or short prefix, e.g. 'f2854dd8')"
                },
                "content": {
                    "type": "string",
                    "description": "The follow-up prompt text to send"
                }
            },
            "required": ["job_id", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let job_id_str = require_str(&params, "job_id")?;
        let content = require_str(&params, "content")?;
        let job_id = resolve_job_id(job_id_str, &self.context_manager).await?;

        let job_ctx = self
            .context_manager
            .get_context(job_id)
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!("job {job_id} not found or context unavailable"))
            })?;

        if job_ctx.user_id != ctx.user_id {
            return Err(ToolError::ExecutionFailed(format!(
                "job {job_id} does not belong to current user"
            )));
        }

        let scheduler = self
            .scheduler_slot
            .read()
            .await
            .clone()
            .ok_or_else(|| ToolError::ExecutionFailed("scheduler unavailable".to_string()))?;

        scheduler
            .send_message(job_id, content.to_string())
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "job_id": job_id.to_string(),
                "status": "sent",
                "message": "Prompt delivered to running job",
                "content": content,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_job_tool_local() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager.clone());

        let params = serde_json::json!({
            "title": "Test Job",
            "description": "A test job description"
        });

        let ctx = JobContext::default();
        let result = tool.execute(params, &ctx).await.unwrap();

        let job_id = result.result.get("job_id").unwrap().as_str().unwrap();
        assert!(!job_id.is_empty());
        assert_eq!(
            result.result.get("status").unwrap().as_str(),
            Some("pending")
        );
    }

    #[test]
    fn test_schema_exposes_local_modes() {
        let manager = Arc::new(ContextManager::new(5));
        let tool = CreateJobTool::new(manager);
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("title"));
        assert!(props.contains_key("description"));
        assert!(props.contains_key("mode"));
        assert!(props.contains_key("project_dir"));
        assert!(!props.contains_key("wait"));
        assert!(!props.contains_key("credentials"));
    }

    #[tokio::test]
    async fn test_list_jobs_tool() {
        let manager = Arc::new(ContextManager::new(5));
        manager.create_job("Job 1", "Desc 1").await.unwrap();
        manager.create_job("Job 2", "Desc 2").await.unwrap();

        let tool = ListJobsTool::new(manager);
        let result = tool
            .execute(serde_json::json!({}), &JobContext::default())
            .await
            .unwrap();
        let jobs = result.result.get("jobs").unwrap().as_array().unwrap();
        assert_eq!(jobs.len(), 2);
    }

    #[tokio::test]
    async fn test_job_status_tool() {
        let manager = Arc::new(ContextManager::new(5));
        let job_id = manager.create_job("Test Job", "Description").await.unwrap();

        let tool = JobStatusTool::new(manager);
        let result = tool
            .execute(
                serde_json::json!({
                    "job_id": job_id.to_string()
                }),
                &JobContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(
            result.result.get("title").unwrap().as_str(),
            Some("Test Job")
        );
    }

    #[tokio::test]
    async fn test_cancel_job_running() {
        let manager = Arc::new(ContextManager::new(5));
        let job_id = manager
            .create_job_for_user("default", "Running Job", "In progress")
            .await
            .unwrap();
        manager
            .update_context(job_id, |ctx| ctx.transition_to(JobState::InProgress, None))
            .await
            .unwrap()
            .unwrap();

        let tool = CancelJobTool::new(Arc::clone(&manager));
        let result = tool
            .execute(
                serde_json::json!({ "job_id": job_id.to_string() }),
                &JobContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(
            result.result.get("status").and_then(|v| v.as_str()),
            Some("cancelled")
        );
        let updated = manager.get_context(job_id).await.unwrap();
        assert_eq!(updated.state, JobState::Cancelled);
    }

    #[test]
    fn test_resolve_project_dir_auto() {
        let project_id = Uuid::new_v4();
        let dir = resolve_project_dir(None, project_id).unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with(project_id.to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_project_dir_allows_existing_outside_managed_base() {
        let tmp = tempfile::tempdir().unwrap();
        let explicit = tmp.path().join("custom-project");
        let dir = resolve_project_dir(Some(explicit.clone()), Uuid::new_v4()).unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with("custom-project"));
    }

    #[tokio::test]
    async fn test_job_prompt_tool_requires_approval() {
        let slot = Arc::new(RwLock::new(None));
        let tool = JobPromptTool::new(slot, Arc::new(ContextManager::new(5)));
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
    }
}
