//! History store types for the current libSQL-backed runtime.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

/// Record for an LLM call to be persisted.
#[derive(Debug, Clone)]
pub struct LlmCallRecord<'a> {
    pub job_id: Option<Uuid>,
    pub conversation_id: Option<Uuid>,
    pub provider: &'a str,
    pub model: &'a str,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: Decimal,
    pub purpose: Option<&'a str>,
}

// ==================== Local Jobs ====================

/// Record for a locally executed autonomous job, persisted in the `agent_jobs`
/// table with `source = 'local'`.
#[derive(Debug, Clone)]
pub struct LocalJobRecord {
    pub id: Uuid,
    pub task: String,
    pub status: String,
    pub user_id: String,
    pub project_dir: String,
    pub success: Option<bool>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Serialized JSON of `Vec<CredentialGrant>` for restart support.
    /// Stored in the `description` column of `agent_jobs` (unused for local jobs).
    pub credential_grants_json: String,
}

/// Summary of local job counts grouped by status.
#[derive(Debug, Clone, Default)]
pub struct LocalJobSummary {
    pub total: usize,
    pub creating: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub interrupted: usize,
}

/// Lightweight record for agent (non-local) jobs, used by the web Jobs tab.
#[derive(Debug, Clone)]
pub struct AgentJobRecord {
    pub id: Uuid,
    pub title: String,
    pub status: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
}

/// Summary counts for agent (non-local) jobs.
#[derive(Debug, Clone, Default)]
pub struct AgentJobSummary {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub stuck: usize,
}

impl AgentJobSummary {
    /// Accumulate a status/count pair into the summary buckets.
    pub fn add_count(&mut self, status: &str, count: usize) {
        self.total += count;
        match status {
            "pending" => self.pending += count,
            "in_progress" => self.in_progress += count,
            "completed" | "submitted" | "accepted" => self.completed += count,
            "failed" | "cancelled" => self.failed += count,
            "stuck" => self.stuck += count,
            _ => {}
        }
    }
}

// ==================== Job Events ====================

/// A persisted job streaming event (from worker or Claude Code bridge).
#[derive(Debug, Clone)]
pub struct JobEventRecord {
    pub id: i64,
    pub job_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

// ==================== Conversation Persistence ====================

/// Summary of a conversation for the thread list.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub id: Uuid,
    /// First user message, truncated to 100 chars.
    pub title: Option<String>,
    pub message_count: i64,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    /// Thread type extracted from metadata (e.g. "assistant", "thread").
    pub thread_type: Option<String>,
    /// Channel-like source tag that owns this conversation (e.g. "desktop", "routine", "heartbeat").
    pub channel: String,
}

/// A single message in a conversation.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

// ==================== Settings ====================

/// A single setting row from the database.
#[derive(Debug, Clone)]
pub struct SettingRow {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_summary_has_channel_field() {
        // Regression: ConversationSummary must include a `channel` field
        // so desktop/runtime flows can distinguish thread origins.
        let summary = ConversationSummary {
            id: Uuid::nil(),
            title: Some("Hello".to_string()),
            message_count: 1,
            started_at: Utc::now(),
            last_activity: Utc::now(),
            thread_type: Some("thread".to_string()),
            channel: "desktop".to_string(),
        };
        assert_eq!(summary.channel, "desktop");
    }

    #[test]
    fn test_conversation_summary_channel_various_values() {
        for ch in ["desktop", "routine", "heartbeat"] {
            let summary = ConversationSummary {
                id: Uuid::nil(),
                title: None,
                message_count: 0,
                started_at: Utc::now(),
                last_activity: Utc::now(),
                thread_type: None,
                channel: ch.to_string(),
            };
            assert_eq!(summary.channel, ch);
        }
    }
}
