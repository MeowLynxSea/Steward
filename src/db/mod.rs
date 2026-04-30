//! Database abstraction layer.
//!
//! Provides a backend-agnostic `Database` trait that unifies all persistence
//! operations. The only supported backend is libSQL (Turso's SQLite fork)
//! for embedded/edge deployment.
//!
//! The existing `Store`, `Repository`, `SecretsStore`, and `WasmToolStore`
//! types become thin wrappers that delegate to `Arc<dyn Database>`.

#[cfg(feature = "libsql")]
pub mod libsql;

#[cfg(feature = "libsql")]
pub mod libsql_migrations;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::agent::BrokenTool;
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::context::{ActionRecord, JobContext, JobState};
use crate::conversation_recall::{
    ConversationRecallDoc, ConversationRecallHit, ConversationTurnView,
};
use crate::error::DatabaseError;
use crate::error::WorkspaceError;
use crate::history::{
    AgentJobRecord, AgentJobSummary, ConversationMessage, ConversationSummary, JobEventRecord,
    LlmCallRecord, LocalJobRecord, LocalJobSummary, SettingRow,
};
use crate::memory::{
    AssociativeEdge, CreateMemoryAliasInput, MemoryChangeSet, MemoryChangeSetRow,
    MemoryChildEntry, MemoryEpisode, MemoryGlossaryEntry, MemoryIndexEntry, MemoryNodeDetail,
    MemorySearchDoc, MemorySearchHit, MemorySidebarSection, MemorySpace, MemoryTimelineEntry,
    MemoryVersion, NewMemoryNodeInput, NodeActivation, UpdateMemoryNodeInput, WmSnapshotEntry,
};
use crate::task_runtime::{TaskRecord, TaskTimelineEntry};
use crate::task_templates::TaskTemplateRecord;
use crate::workspace::{
    AllowlistActionRequest, ConflictResolutionRequest, CreateAllowlistRequest,
    CreateCheckpointRequest, WorkspaceAllowlistBaselineRequest, WorkspaceAllowlistCheckpoint,
    WorkspaceAllowlistDetail, WorkspaceAllowlistDiff, WorkspaceAllowlistDiffRequest,
    WorkspaceAllowlistFileView, WorkspaceAllowlistHistory, WorkspaceAllowlistHistoryRequest,
    WorkspaceAllowlistRestoreRequest, WorkspaceAllowlistSummary, WorkspaceTreeEntry,
};
use crate::workspace::{MemoryChunk, MemoryDocument, WorkspaceEntry};
use crate::workspace::{SearchConfig, SearchResult};

/// Create a database backend from configuration, run migrations, and return it.
///
/// This is the shared helper for CLI commands and other call sites that need
/// a simple `Arc<dyn Database>` without retaining backend-specific handles
/// (e.g., `libsql_conn` for the secrets store). The main agent
/// startup in `main.rs` uses its own initialization block because it also
/// captures those backend-specific handles.
pub async fn connect_from_config(
    config: &crate::config::DatabaseConfig,
) -> Result<Arc<dyn Database>, DatabaseError> {
    let (db, _handles) = connect_with_handles(config).await?;
    Ok(db)
}

/// Backend-specific handles retained after database connection.
///
/// These are needed by satellite stores (e.g., `SecretsStore`) that require
/// a backend-specific handle rather than the generic `Arc<dyn Database>`.
#[derive(Default)]
pub struct DatabaseHandles {
    #[cfg(feature = "libsql")]
    pub libsql_db: Option<Arc<::libsql::Database>>,
}

/// Connect to the database, run migrations, and return both the generic
/// `Database` trait object and the backend-specific handles.
pub async fn connect_with_handles(
    config: &crate::config::DatabaseConfig,
) -> Result<(Arc<dyn Database>, DatabaseHandles), DatabaseError> {
    let mut handles = DatabaseHandles::default();

    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };
            backend.run_migrations().await?;
            tracing::debug!("libSQL database connected and migrations applied");

            handles.libsql_db = Some(backend.shared_db());

            Ok((Arc::new(backend) as Arc<dyn Database>, handles))
        }
        #[allow(unreachable_patterns)]
        _ => Err(DatabaseError::Pool(format!(
            "Database backend '{}' is not available. Only libsql is supported.",
            config.backend
        ))),
    }
}

/// Create a secrets store from database and secrets configuration.
///
/// This is the shared factory for CLI commands and other call sites that need
/// a `SecretsStore` without going through the full `AppBuilder`. Mirrors the
/// pattern of [`connect_from_config`] but returns a secrets-specific store.
pub async fn create_secrets_store(
    config: &crate::config::DatabaseConfig,
    crypto: Arc<crate::secrets::SecretsCrypto>,
) -> Result<Arc<dyn crate::secrets::SecretsStore + Send + Sync>, DatabaseError> {
    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };
            backend.run_migrations().await?;

            Ok(Arc::new(crate::secrets::LibSqlSecretsStore::new(
                backend.shared_db(),
                crypto,
            )))
        }
        #[allow(unreachable_patterns)]
        _ => Err(DatabaseError::Pool(format!(
            "Database backend '{}' is not available for secrets. Only libsql is supported.",
            config.backend
        ))),
    }
}

// ==================== Wizard / testing helpers ====================

/// Connect to the database WITHOUT running migrations, validating
/// prerequisites when applicable.
///
/// Returns both the `Database` trait object and backend-specific handles.
/// Used by the wizard to test connectivity before committing — call
/// [`Database::run_migrations`] on the returned trait object when ready.
pub async fn connect_without_migrations(
    config: &crate::config::DatabaseConfig,
) -> Result<(Arc<dyn Database>, DatabaseHandles), DatabaseError> {
    let mut handles = DatabaseHandles::default();

    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };

            handles.libsql_db = Some(backend.shared_db());

            Ok((Arc::new(backend) as Arc<dyn Database>, handles))
        }
        #[allow(unreachable_patterns)]
        _ => Err(DatabaseError::Pool(format!(
            "Database backend '{}' is not available. Only libsql is supported.",
            config.backend
        ))),
    }
}

// ==================== User management record types ====================

/// A registered user.
#[derive(Debug, Clone)]
pub struct UserRecord {
    /// User identifier (string, matches existing `user_id` throughout the codebase).
    pub id: String,
    pub email: Option<String>,
    pub display_name: String,
    /// `active`, `suspended`, or `deactivated`.
    pub status: String,
    /// `admin` or `member`.
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    /// Who created/invited this user (nullable for bootstrap users).
    pub created_by: Option<String>,
    pub metadata: serde_json::Value,
}

/// An API token for authenticating requests (hash stored, never plaintext).
#[derive(Debug, Clone)]
pub struct ApiTokenRecord {
    pub id: Uuid,
    pub user_id: String,
    /// Human label (e.g. "my-laptop", "ci-bot").
    pub name: String,
    /// First 8 hex chars of the plaintext token for display/identification.
    pub token_prefix: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Soft-revoke timestamp. Non-null means revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

// ==================== Sub-traits ====================
//
// Each sub-trait groups related persistence methods. The `Database` supertrait
// combines them all, so existing `Arc<dyn Database>` consumers keep working.
// Leaf consumers can depend on a specific sub-trait instead.

#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError>;
    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError>;
    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError>;
    async fn update_conversation_message_content(
        &self,
        message_id: Uuid,
        content: &str,
    ) -> Result<(), DatabaseError>;
    async fn update_conversation_message_metadata(
        &self,
        message_id: Uuid,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<bool, DatabaseError>;
    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    async fn list_conversation_ids_for_channel(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Vec<Uuid>, DatabaseError>;
    async fn list_conversations_all_channels(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError>;
    /// Read-only lookup for an existing routine conversation. Returns `None`
    /// if the routine has never executed (no conversation created yet).
    async fn find_routine_conversation(
        &self,
        routine_id: Uuid,
        user_id: &str,
    ) -> Result<Option<Uuid>, DatabaseError>;
    async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError>;
    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError>;
    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError>;
    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;
    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError>;
    async fn list_conversation_turns(
        &self,
        conversation_id: Uuid,
        include_tool_calls: bool,
    ) -> Result<Vec<ConversationTurnView>, DatabaseError>;
    async fn upsert_conversation_recall_doc(
        &self,
        doc: &ConversationRecallDoc,
    ) -> Result<(), DatabaseError>;
    async fn search_conversation_recall(
        &self,
        user_id: &str,
        query: &str,
        query_embedding: Option<&[f32]>,
        config: &crate::retrieval::SearchConfig,
        exclude_conversation_id: Option<Uuid>,
    ) -> Result<Vec<ConversationRecallHit>, DatabaseError>;
    async fn backfill_conversation_recall_for_user(
        &self,
        user_id: &str,
    ) -> Result<usize, DatabaseError>;
    async fn list_conversation_recall_docs_without_embeddings(
        &self,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<ConversationRecallDoc>, DatabaseError>;
    async fn list_recent_conversation_recall(
        &self,
        user_id: &str,
        limit: usize,
        exclude_conversation_id: Option<Uuid>,
    ) -> Result<Vec<ConversationRecallHit>, DatabaseError>;
    async fn update_conversation_recall_doc_embedding(
        &self,
        doc_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), DatabaseError>;
    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn delete_conversation(&self, conversation_id: Uuid) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError>;
    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError>;
    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError>;
    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError>;
    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError>;
    async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, DatabaseError>;
    async fn list_agent_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<AgentJobRecord>, DatabaseError>;
    async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError>;
    async fn agent_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<AgentJobSummary, DatabaseError>;
    /// Get the failure reason for a single agent job (O(1) lookup).
    async fn get_agent_job_failure_reason(&self, id: Uuid)
    -> Result<Option<String>, DatabaseError>;
    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError>;
    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError>;
    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError>;
    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError>;
    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait LocalJobStore: Send + Sync {
    async fn save_local_job(&self, job: &LocalJobRecord) -> Result<(), DatabaseError>;
    async fn get_local_job(&self, id: Uuid) -> Result<Option<LocalJobRecord>, DatabaseError>;
    async fn list_local_jobs(&self) -> Result<Vec<LocalJobRecord>, DatabaseError>;
    async fn update_local_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError>;
    async fn cleanup_stale_local_jobs(&self) -> Result<u64, DatabaseError>;
    async fn local_job_summary(&self) -> Result<LocalJobSummary, DatabaseError>;
    async fn list_local_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<LocalJobRecord>, DatabaseError>;
    async fn local_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<LocalJobSummary, DatabaseError>;
    async fn local_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn update_local_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError>;
    async fn get_local_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError>;
    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError>;
}

#[async_trait]
pub trait RoutineStore: Send + Sync {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;
    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError>;
    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError>;
    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError>;
    async fn list_all_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;
    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError>;
    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError>;
    async fn transition_routine_run_to_running(
        &self,
        id: Uuid,
        started_at: DateTime<Utc>,
    ) -> Result<bool, DatabaseError>;
    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError>;
    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError>;
    async fn list_queued_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError>;
    async fn list_stale_lightweight_routine_runs(
        &self,
        before: DateTime<Utc>,
    ) -> Result<Vec<RoutineRun>, DatabaseError>;
    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError>;
    async fn count_running_routine_runs_batch(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, i64>, DatabaseError>;

    /// Fetch the last run status for multiple routines in a single query.
    /// Returns a map from routine_id to its most recent RunStatus.
    /// Routines with no runs are omitted from the result.
    async fn batch_get_last_run_status(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, RunStatus>, DatabaseError>;

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError>;
    async fn get_webhook_routine_by_path(
        &self,
        path: &str,
        user_id: Option<&str>,
    ) -> Result<Option<Routine>, DatabaseError>;

    /// List routine runs that were dispatched as full_job but have not yet
    /// been finalized (status='running' with a linked job_id).
    async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError>;
}

#[async_trait]
pub trait ToolFailureStore: Send + Sync {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError>;
    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError>;
    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError>;
    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait SettingsStore: Send + Sync {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;
    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError>;
    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError>;
    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError>;
    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError>;
    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError>;
    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait TemplateStore: Send + Sync {
    async fn list_task_templates(
        &self,
        user_id: &str,
    ) -> Result<Vec<TaskTemplateRecord>, DatabaseError>;
    async fn get_task_template(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<Option<TaskTemplateRecord>, DatabaseError>;
    async fn create_task_template(
        &self,
        user_id: &str,
        template: &TaskTemplateRecord,
    ) -> Result<(), DatabaseError>;
    async fn update_task_template(
        &self,
        user_id: &str,
        template: &TaskTemplateRecord,
    ) -> Result<bool, DatabaseError>;
    async fn delete_task_template(&self, user_id: &str, id: &str) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn upsert_task_record(
        &self,
        user_id: &str,
        task: &TaskRecord,
    ) -> Result<(), DatabaseError>;
    async fn get_task_record(
        &self,
        user_id: &str,
        task_id: Uuid,
    ) -> Result<Option<TaskRecord>, DatabaseError>;
    async fn list_task_records(&self, user_id: &str) -> Result<Vec<TaskRecord>, DatabaseError>;
    async fn append_task_timeline(
        &self,
        user_id: &str,
        task_id: Uuid,
        event: &str,
        task: &TaskRecord,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn list_task_timeline(
        &self,
        user_id: &str,
        task_id: Uuid,
    ) -> Result<Vec<TaskTimelineEntry>, DatabaseError>;
}

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError>;
    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError>;
    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError>;
    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError>;
    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError>;
    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError>;
    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError>;
    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;
    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError>;
    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError>;

    // ==================== Multi-scope read methods ====================
    //
    // Default implementations loop over user_ids calling single-scope methods,
    // then merge results. Backends can override with efficient SQL.

    /// Hybrid search across multiple user scopes, merging results by score.
    async fn hybrid_search_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        if user_ids.len() > 1 {
            tracing::debug!(
                scope_count = user_ids.len(),
                "hybrid_search_multi: using default per-scope RRF merge; \
                 cross-scope score comparison may be unreliable"
            );
        }
        let mut all_results = Vec::new();
        for uid in user_ids {
            let results = self
                .hybrid_search(uid, agent_id, query, embedding, config)
                .await?;
            all_results.extend(results);
        }
        // Re-sort by score descending and truncate to limit
        all_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_results.truncate(config.limit);
        Ok(all_results)
    }

    /// List all file paths across multiple user scopes.
    async fn list_all_paths_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let mut all_paths = Vec::new();
        for uid in user_ids {
            let paths = self.list_all_paths(uid, agent_id).await?;
            all_paths.extend(paths);
        }
        all_paths.sort();
        all_paths.dedup();
        Ok(all_paths)
    }

    /// Get a document by path, searching across multiple user scopes.
    ///
    /// Returns the first match found (tries each user_id in order).
    async fn get_document_by_path_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        for uid in user_ids {
            match self.get_document_by_path(uid, agent_id, path).await {
                Ok(doc) => return Ok(doc),
                Err(WorkspaceError::DocumentNotFound { .. }) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(WorkspaceError::DocumentNotFound {
            doc_type: path.to_string(),
            user_id: format!("[{}]", user_ids.join(", ")),
        })
    }

    /// List directory contents across multiple user scopes.
    async fn list_directory_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let mut all_entries = Vec::new();
        for uid in user_ids {
            all_entries.extend(self.list_directory(uid, agent_id, directory).await?);
        }
        Ok(crate::workspace::merge_workspace_entries(all_entries))
    }

    async fn list_workspace_tree(
        &self,
        _user_id: &str,
        _agent_id: Option<Uuid>,
        _uri: &str,
    ) -> Result<Vec<WorkspaceTreeEntry>, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "list_workspace_tree".to_string(),
        })
    }

    async fn create_workspace_allowlist(
        &self,
        _request: &CreateAllowlistRequest,
    ) -> Result<WorkspaceAllowlistSummary, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "create_workspace_allowlist".to_string(),
        })
    }

    async fn list_workspace_allowlists(
        &self,
        _user_id: &str,
    ) -> Result<Vec<WorkspaceAllowlistSummary>, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "list_workspace_allowlists".to_string(),
        })
    }

    async fn get_workspace_allowlist(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "get_workspace_allowlist".to_string(),
        })
    }

    async fn delete_workspace_allowlist(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "delete_workspace_allowlist".to_string(),
        })
    }

    async fn read_workspace_allowlist_file(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _path: &str,
    ) -> Result<WorkspaceAllowlistFileView, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "read_workspace_allowlist_file".to_string(),
        })
    }

    async fn write_workspace_allowlist_file(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _path: &str,
        _content: &[u8],
    ) -> Result<WorkspaceAllowlistFileView, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "write_workspace_allowlist_file".to_string(),
        })
    }

    async fn delete_workspace_allowlist_file(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _path: &str,
    ) -> Result<WorkspaceAllowlistFileView, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "delete_workspace_allowlist_file".to_string(),
        })
    }

    async fn diff_workspace_allowlist(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _scope_path: Option<&str>,
    ) -> Result<WorkspaceAllowlistDiff, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "diff_workspace_allowlist".to_string(),
        })
    }

    async fn diff_workspace_allowlist_between(
        &self,
        _request: &WorkspaceAllowlistDiffRequest,
    ) -> Result<WorkspaceAllowlistDiff, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "diff_workspace_allowlist_between".to_string(),
        })
    }

    async fn create_workspace_checkpoint(
        &self,
        _request: &CreateCheckpointRequest,
    ) -> Result<WorkspaceAllowlistCheckpoint, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "create_workspace_checkpoint".to_string(),
        })
    }

    async fn list_workspace_checkpoints(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _limit: Option<usize>,
    ) -> Result<Vec<WorkspaceAllowlistCheckpoint>, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "list_workspace_checkpoints".to_string(),
        })
    }

    async fn delete_workspace_checkpoint(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _checkpoint_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "delete_workspace_checkpoint".to_string(),
        })
    }

    async fn list_workspace_allowlist_history(
        &self,
        _request: &WorkspaceAllowlistHistoryRequest,
    ) -> Result<WorkspaceAllowlistHistory, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "list_workspace_allowlist_history".to_string(),
        })
    }

    async fn keep_workspace_allowlist(
        &self,
        _request: &AllowlistActionRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "keep_workspace_allowlist".to_string(),
        })
    }

    async fn revert_workspace_allowlist(
        &self,
        _request: &AllowlistActionRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "revert_workspace_allowlist".to_string(),
        })
    }

    async fn resolve_workspace_allowlist_conflict(
        &self,
        _request: &ConflictResolutionRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "resolve_workspace_allowlist_conflict".to_string(),
        })
    }

    async fn move_workspace_allowlist_file(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _source_path: &str,
        _destination_path: &str,
        _overwrite: bool,
    ) -> Result<WorkspaceAllowlistFileView, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "move_workspace_allowlist_file".to_string(),
        })
    }

    async fn delete_workspace_allowlist_tree(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _path: &str,
        _missing_ok: bool,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "delete_workspace_allowlist_tree".to_string(),
        })
    }

    async fn restore_workspace_allowlist(
        &self,
        _request: &WorkspaceAllowlistRestoreRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "restore_workspace_allowlist".to_string(),
        })
    }

    async fn set_workspace_allowlist_baseline(
        &self,
        _request: &WorkspaceAllowlistBaselineRequest,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "set_workspace_allowlist_baseline".to_string(),
        })
    }

    async fn refresh_workspace_allowlist(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
        _scope_path: Option<&str>,
    ) -> Result<WorkspaceAllowlistDetail, WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "refresh_workspace_allowlist".to_string(),
        })
    }

    async fn sync_workspace_allowlist_watch(
        &self,
        _user_id: &str,
        _allowlist_id: Uuid,
    ) -> Result<(), WorkspaceError> {
        Err(WorkspaceError::Unsupported {
            operation: "sync_workspace_allowlist_watch".to_string(),
        })
    }
}

#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn ensure_memory_space(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        slug: &str,
        title: &str,
    ) -> Result<MemorySpace, DatabaseError>;

    async fn create_memory_changeset(
        &self,
        space_id: Uuid,
        origin: &str,
        summary: Option<&str>,
    ) -> Result<MemoryChangeSet, DatabaseError>;

    async fn complete_memory_changeset(
        &self,
        changeset_id: Uuid,
        status: &str,
    ) -> Result<(), DatabaseError>;

    async fn get_memory_changeset_rows(
        &self,
        changeset_id: Uuid,
    ) -> Result<Vec<MemoryChangeSetRow>, DatabaseError>;

    async fn rollback_memory_changeset(&self, changeset_id: Uuid) -> Result<(), DatabaseError>;

    async fn create_memory_node(
        &self,
        input: &NewMemoryNodeInput,
    ) -> Result<MemoryNodeDetail, DatabaseError>;

    async fn update_memory_node(
        &self,
        space_id: Uuid,
        input: &UpdateMemoryNodeInput,
    ) -> Result<MemoryNodeDetail, DatabaseError>;

    async fn create_memory_alias(
        &self,
        input: &CreateMemoryAliasInput,
    ) -> Result<crate::memory::MemoryRoute, DatabaseError>;

    async fn delete_memory_node(
        &self,
        space_id: Uuid,
        route_or_node: &str,
        changeset_id: Option<Uuid>,
    ) -> Result<(), DatabaseError>;

    async fn get_memory_node(
        &self,
        space_id: Uuid,
        route_or_node: &str,
    ) -> Result<Option<MemoryNodeDetail>, DatabaseError>;

    async fn search_memory_graph(
        &self,
        space_id: Uuid,
        query: &str,
        limit: usize,
        domains: &[String],
    ) -> Result<Vec<MemorySearchHit>, DatabaseError>;

    async fn vector_search_memory_graph(
        &self,
        space_id: Uuid,
        embedding: &[f32],
        limit: usize,
        domains: &[String],
    ) -> Result<Vec<MemorySearchHit>, DatabaseError>;

    async fn list_memory_sidebar(
        &self,
        space_id: Uuid,
        limit_per_section: usize,
    ) -> Result<Vec<MemorySidebarSection>, DatabaseError>;

    async fn list_memory_timeline(
        &self,
        space_id: Uuid,
        limit: usize,
    ) -> Result<Vec<MemoryTimelineEntry>, DatabaseError>;

    async fn list_memory_reviews(
        &self,
        space_id: Uuid,
    ) -> Result<Vec<MemoryChangeSet>, DatabaseError>;

    async fn get_memory_versions(&self, node_id: Uuid)
    -> Result<Vec<MemoryVersion>, DatabaseError>;

    async fn list_memory_boot_nodes(
        &self,
        space_id: Uuid,
        max_visibility: Option<crate::memory::MemoryVisibility>,
    ) -> Result<Vec<MemoryNodeDetail>, DatabaseError>;

    async fn upsert_memory_boot_route(
        &self,
        space_id: Uuid,
        route_or_node: &str,
        load_priority: i32,
    ) -> Result<crate::memory::MemoryRoute, DatabaseError>;

    async fn delete_memory_boot_route(
        &self,
        space_id: Uuid,
        route_or_node: &str,
    ) -> Result<(), DatabaseError>;

    async fn list_memory_index(
        &self,
        space_id: Uuid,
        domain: Option<&str>,
    ) -> Result<Vec<MemoryIndexEntry>, DatabaseError>;

    async fn list_memory_recent(
        &self,
        space_id: Uuid,
        limit: usize,
        domain: Option<&str>,
    ) -> Result<Vec<MemoryIndexEntry>, DatabaseError>;

    async fn list_memory_glossary(
        &self,
        space_id: Uuid,
    ) -> Result<Vec<MemoryGlossaryEntry>, DatabaseError>;

    async fn list_memory_children(
        &self,
        space_id: Uuid,
        parent_node_id: Uuid,
        limit: usize,
    ) -> Result<Vec<MemoryChildEntry>, DatabaseError>;

    async fn get_memory_search_doc(
        &self,
        space_id: Uuid,
        route_id: Uuid,
    ) -> Result<Option<MemorySearchDoc>, DatabaseError>;

    async fn list_memory_search_docs_without_embeddings(
        &self,
        space_id: Uuid,
        limit: usize,
    ) -> Result<Vec<MemorySearchDoc>, DatabaseError>;

    async fn update_memory_search_doc_embedding(
        &self,
        route_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), DatabaseError>;

    // ---- Brain: Node Activation ----
    async fn upsert_node_activation(
        &self,
        space_id: Uuid,
        node_id: Uuid,
        delta: f64,
    ) -> Result<NodeActivation, DatabaseError>;

    async fn decay_all_activations(
        &self,
        space_id: Uuid,
        decay_factor: f64,
    ) -> Result<(), DatabaseError>;

    async fn list_top_activated(
        &self,
        space_id: Uuid,
        limit: usize,
    ) -> Result<Vec<NodeActivation>, DatabaseError>;

    async fn get_node_activation(
        &self,
        space_id: Uuid,
        node_id: Uuid,
    ) -> Result<Option<NodeActivation>, DatabaseError>;

    // ---- Brain: Associative Edges ----
    async fn create_associative_edge(
        &self,
        space_id: Uuid,
        source_node_id: Uuid,
        target_node_id: Uuid,
        edge_type: &str,
        weight: f64,
        confidence: f64,
        evidence: &[String],
    ) -> Result<AssociativeEdge, DatabaseError>;

    async fn reinforce_associative_edge(
        &self,
        space_id: Uuid,
        source_node_id: Uuid,
        target_node_id: Uuid,
        edge_type: &str,
        delta: f64,
        boost: f64,
        evidence: &str,
    ) -> Result<AssociativeEdge, DatabaseError>;

    async fn list_associative_neighbors(
        &self,
        space_id: Uuid,
        node_id: Uuid,
        limit: usize,
    ) -> Result<Vec<(AssociativeEdge, Uuid)>, DatabaseError>;

    // ---- Brain: Episodes ----
    async fn create_episode(
        &self,
        space_id: Uuid,
        node_id: Uuid,
        episode_type: &str,
        trigger_uri: Option<&str>,
        trigger_text: Option<&str>,
        working_memory_snapshot: &[WmSnapshotEntry],
        activation_strength: f64,
    ) -> Result<MemoryEpisode, DatabaseError>;

    async fn list_episodes(
        &self,
        node_id: Uuid,
        limit: usize,
    ) -> Result<Vec<MemoryEpisode>, DatabaseError>;

    async fn list_recent_episodes(
        &self,
        space_id: Uuid,
        since: DateTime<Utc>,
    ) -> Result<Vec<MemoryEpisode>, DatabaseError>;

}

#[async_trait]
pub trait UserStore: Send + Sync {
    // ---- Users ----

    /// Create a new user record.
    async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError>;
    /// Get a user by their string id.
    async fn get_user(&self, id: &str) -> Result<Option<UserRecord>, DatabaseError>;
    /// Get a user by email address.
    async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DatabaseError>;
    /// List users, optionally filtered by status.
    async fn list_users(&self, status: Option<&str>) -> Result<Vec<UserRecord>, DatabaseError>;
    /// Update a user's status (active/suspended/deactivated).
    async fn update_user_status(&self, id: &str, status: &str) -> Result<(), DatabaseError>;
    /// Update a user's role (admin/member).
    async fn update_user_role(&self, id: &str, role: &str) -> Result<(), DatabaseError>;
    /// Update a user's display name and metadata.
    async fn update_user_profile(
        &self,
        id: &str,
        display_name: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    /// Record a login timestamp.
    async fn record_login(&self, id: &str) -> Result<(), DatabaseError>;

    // ---- API Tokens ----

    /// Create a new API token. The `token_hash` is SHA-256 of the plaintext.
    async fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError>;
    /// List tokens for a user (never includes the hash).
    async fn list_api_tokens(&self, user_id: &str) -> Result<Vec<ApiTokenRecord>, DatabaseError>;
    /// Soft-revoke a token. Returns false if the token doesn't exist or doesn't belong to the user.
    async fn revoke_api_token(&self, token_id: Uuid, user_id: &str) -> Result<bool, DatabaseError>;
    /// Look up a token by hash, returning the token record and its owning user.
    /// Only returns active (non-revoked, non-expired) tokens for active users.
    async fn authenticate_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<(ApiTokenRecord, UserRecord)>, DatabaseError>;
    /// Update `last_used_at` for a token.
    async fn record_token_usage(&self, token_id: Uuid) -> Result<(), DatabaseError>;

    /// Check whether any user records exist (for first-run bootstrap detection).
    async fn has_any_users(&self) -> Result<bool, DatabaseError>;

    /// Delete a user and all their data across all user-scoped tables.
    /// Returns false if the user doesn't exist.
    async fn delete_user(&self, id: &str) -> Result<bool, DatabaseError>;

    /// Get per-user LLM usage stats for a time period.
    /// Aggregates from llm_calls via agent_jobs.user_id.
    async fn user_usage_stats(
        &self,
        user_id: Option<&str>,
        since: DateTime<Utc>,
    ) -> Result<Vec<UserUsageStats>, DatabaseError>;

    /// Lightweight per-user summary stats (job count, total cost, last active).
    /// Used by the admin users list to show inline stats.
    async fn user_summary_stats(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<UserSummaryStats>, DatabaseError>;

    /// Create a user and their initial API token atomically.
    /// If either operation fails, both are rolled back.
    async fn create_user_with_token(
        &self,
        user: &UserRecord,
        token_name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError>;
}

/// Per-user LLM usage statistics.
#[derive(Debug, Clone)]
pub struct UserUsageStats {
    pub user_id: String,
    pub model: String,
    pub call_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_cost: Decimal,
}

/// Lightweight per-user summary for the admin users list.
#[derive(Debug, Clone)]
pub struct UserSummaryStats {
    pub user_id: String,
    /// Total agent jobs created by this user.
    pub job_count: i64,
    /// Total LLM spend across all jobs (all-time).
    pub total_cost: Decimal,
    /// Most recent activity (latest job or LLM call timestamp).
    pub last_active_at: Option<DateTime<Utc>>,
}

/// Backend-agnostic database supertrait.
///
/// Combines all sub-traits into one. Existing `Arc<dyn Database>` consumers
/// continue to work; leaf consumers can depend on a specific sub-trait instead.
#[async_trait]
pub trait Database:
    ConversationStore
    + JobStore
    + LocalJobStore
    + RoutineStore
    + ToolFailureStore
    + SettingsStore
    + TemplateStore
    + TaskStore
    + WorkspaceStore
    + MemoryStore
    + UserStore
    + Send
    + Sync
{
    /// Run schema migrations for this backend.
    async fn run_migrations(&self) -> Result<(), DatabaseError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test: `create_secrets_store` selects the correct backend at
    /// runtime based on `DatabaseConfig`.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_create_secrets_store_libsql_backend() {
        use secrecy::SecretString;

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");

        let config = crate::config::DatabaseConfig {
            backend: crate::config::DatabaseBackend::LibSql,
            libsql_path: Some(db_path),
            libsql_url: None,
            libsql_auth_token: None,
        };

        let master_key = SecretString::from("a]".repeat(16));
        let crypto = Arc::new(crate::secrets::SecretsCrypto::new(master_key).unwrap());

        let store = create_secrets_store(&config, crypto).await;
        assert!(
            store.is_ok(),
            "create_secrets_store should succeed for libsql backend"
        );

        // Verify basic operation works
        let store = store.unwrap();
        let exists = store.exists("test_user", "nonexistent_secret").await;
        assert!(exists.is_ok());
        assert!(!exists.unwrap());
    }
}
