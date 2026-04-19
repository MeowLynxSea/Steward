//! Conversation-related ConversationStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_i64, get_json, get_opt_text, get_text, get_ts, opt_text};
use crate::conversation_recall::{
    ConversationRecallDoc, ConversationRecallHit, ConversationToolCallSummary, ConversationTurnView,
};
use crate::db::ConversationStore;
use crate::error::DatabaseError;
use crate::history::{ConversationMessage, ConversationSummary};
use crate::memory::search_terms::sqlite_match_query;
use crate::retrieval::{RankedItem, SearchConfig, fuse_results};

#[derive(Debug, Clone)]
struct ConversationRowMeta {
    channel: String,
    thread_id: String,
}

fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for value in embedding {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn parse_tool_call_json(value: &serde_json::Value) -> ConversationToolCallSummary {
    ConversationToolCallSummary {
        name: value
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        call_id: value
            .get("call_id")
            .or_else(|| value.get("tool_call_id"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        status: if value.get("error").is_some() {
            "error".to_string()
        } else if value.get("completed_at").is_some() {
            "completed".to_string()
        } else {
            "started".to_string()
        },
        parameters: value
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
        result_preview: value
            .get("result_preview")
            .or_else(|| value.get("result"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        error: value
            .get("error")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        rationale: value
            .get("rationale")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        started_at: value
            .get("started_at")
            .and_then(|v| v.as_str())
            .and_then(|raw| super::parse_timestamp(raw).ok()),
        completed_at: value
            .get("completed_at")
            .and_then(|v| v.as_str())
            .and_then(|raw| super::parse_timestamp(raw).ok()),
    }
}

fn parse_tool_call_summary(
    message: &ConversationMessage,
) -> Option<Vec<ConversationToolCallSummary>> {
    let value = serde_json::from_str::<serde_json::Value>(&message.content).ok()?;
    match value {
        serde_json::Value::Array(items) => Some(items.iter().map(parse_tool_call_json).collect()),
        serde_json::Value::Object(ref obj)
            if obj.get("calls").and_then(|v| v.as_array()).is_some() =>
        {
            let calls = obj
                .get("calls")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            Some(calls.iter().map(parse_tool_call_json).collect())
        }
        serde_json::Value::Object(_) => Some(vec![parse_tool_call_json(&value)]),
        _ => None,
    }
}

fn load_turns_from_messages(
    conversation_id: Uuid,
    meta: &ConversationRowMeta,
    messages: &[ConversationMessage],
    include_tool_calls: bool,
) -> Vec<ConversationTurnView> {
    let mut turns = Vec::new();

    for message in messages {
        match message.role.as_str() {
            "user" => turns.push(ConversationTurnView {
                conversation_id,
                channel: meta.channel.clone(),
                thread_id: meta.thread_id.clone(),
                turn_index: turns.len(),
                user_message_id: message.id,
                assistant_message_id: None,
                timestamp: message.created_at,
                user_text: message.content.clone(),
                assistant_text: None,
                tool_calls: Vec::new(),
            }),
            "assistant" => {
                let Some(turn) = turns.last_mut() else {
                    continue;
                };
                match turn.assistant_text.as_mut() {
                    Some(existing) => {
                        if !existing.is_empty() {
                            existing.push_str("\n\n");
                        }
                        existing.push_str(&message.content);
                    }
                    None => turn.assistant_text = Some(message.content.clone()),
                }
                turn.assistant_message_id = Some(message.id);
            }
            "tool_call" | "tool_calls" if include_tool_calls => {
                let Some(turn) = turns.last_mut() else {
                    continue;
                };
                if let Some(mut summaries) = parse_tool_call_summary(message) {
                    turn.tool_calls.append(&mut summaries);
                }
            }
            "thinking" => {}
            _ => {}
        }
    }

    turns
}

fn preview_text(user_text: &str, assistant_text: &str) -> String {
    fn truncate(text: &str, max_chars: usize) -> String {
        let trimmed = text.trim();
        if trimmed.chars().count() <= max_chars {
            return trimmed.to_string();
        }
        let cutoff = trimmed
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(trimmed.len());
        format!("{}...", trimmed[..cutoff].trim())
    }

    format!(
        "User: {}\nAssistant: {}",
        truncate(user_text, 220),
        truncate(assistant_text, 260)
    )
}

impl LibSqlBackend {
    async fn fetch_conversation_row_meta(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<ConversationRowMeta>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT channel, coalesce(thread_id, id) FROM conversations WHERE id = ?1",
                params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| ConversationRowMeta {
                channel: get_text(&row, 0),
                thread_id: get_text(&row, 1),
            }))
    }

    async fn list_messages_for_conversation(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, role, content, metadata, created_at
                FROM conversation_messages
                WHERE conversation_id = ?1
                ORDER BY created_at ASC, rowid ASC
                "#,
                params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut messages = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            messages.push(ConversationMessage {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                role: get_text(&row, 1),
                content: get_text(&row, 2),
                metadata: get_json(&row, 3),
                created_at: get_ts(&row, 4),
            });
        }
        Ok(messages)
    }

    async fn build_turns_for_conversation(
        &self,
        conversation_id: Uuid,
        include_tool_calls: bool,
    ) -> Result<Vec<ConversationTurnView>, DatabaseError> {
        let Some(meta) = self.fetch_conversation_row_meta(conversation_id).await? else {
            return Ok(Vec::new());
        };
        let messages = self.list_messages_for_conversation(conversation_id).await?;
        Ok(load_turns_from_messages(
            conversation_id,
            &meta,
            &messages,
            include_tool_calls,
        ))
    }

    fn turn_to_recall_doc(
        user_id: &str,
        turn: &ConversationTurnView,
    ) -> Option<ConversationRecallDoc> {
        let assistant_message_id = turn.assistant_message_id?;
        let assistant_text = turn.assistant_text.clone()?;
        let search_text = format!("User: {}\nAssistant: {}", turn.user_text, assistant_text);
        Some(ConversationRecallDoc {
            doc_id: assistant_message_id,
            user_id: user_id.to_string(),
            conversation_id: turn.conversation_id,
            channel: turn.channel.clone(),
            thread_id: turn.thread_id.clone(),
            turn_index: turn.turn_index,
            user_message_id: turn.user_message_id,
            assistant_message_id,
            turn_timestamp: turn.timestamp,
            user_text: turn.user_text.clone(),
            assistant_text: assistant_text.clone(),
            search_text: search_text.clone(),
            preview_text: preview_text(&turn.user_text, &assistant_text),
            embedding: None,
            updated_at: Utc::now(),
        })
    }

    fn row_to_recall_doc(row: &libsql::Row) -> ConversationRecallDoc {
        ConversationRecallDoc {
            doc_id: get_text(row, 0).parse().unwrap_or_default(),
            user_id: get_text(row, 1),
            conversation_id: get_text(row, 2).parse().unwrap_or_default(),
            channel: get_text(row, 3),
            thread_id: get_text(row, 4),
            turn_index: get_i64(row, 5).max(0) as usize,
            user_message_id: get_text(row, 6).parse().unwrap_or_default(),
            assistant_message_id: get_text(row, 7).parse().unwrap_or_default(),
            turn_timestamp: get_ts(row, 8),
            user_text: get_text(row, 9),
            assistant_text: get_text(row, 10),
            search_text: get_text(row, 11),
            preview_text: get_text(row, 12),
            embedding: None,
            updated_at: get_ts(row, 13),
        }
    }

    fn row_to_recall_hit(
        row: &libsql::Row,
        score: f32,
        rank: u32,
        from_vector: bool,
    ) -> ConversationRecallHit {
        ConversationRecallHit {
            doc_id: get_text(row, 0).parse().unwrap_or_default(),
            conversation_id: get_text(row, 2).parse().unwrap_or_default(),
            channel: get_text(row, 3),
            thread_id: get_text(row, 4),
            turn_index: get_i64(row, 5).max(0) as usize,
            user_message_id: get_text(row, 6).parse().unwrap_or_default(),
            assistant_message_id: get_text(row, 7).parse().unwrap_or_default(),
            turn_timestamp: get_ts(row, 8),
            user_text: get_text(row, 9),
            assistant_text: get_text(row, 10),
            preview_text: get_text(row, 12),
            score,
            fts_rank: (!from_vector).then_some(rank),
            vector_rank: from_vector.then_some(rank),
            confidence_score: 0.0,
        }
    }

    fn doc_select_columns() -> &'static str {
        "d.doc_id, d.user_id, d.conversation_id, d.channel, d.thread_id, d.turn_index, d.user_message_id, d.assistant_message_id, d.turn_timestamp, d.user_text, d.assistant_text, d.search_text, d.preview_text, d.updated_at"
    }

    pub async fn ensure_conversation_recall_vector_index(
        &self,
        dimension: usize,
    ) -> Result<(), DatabaseError> {
        if dimension == 0 || dimension > 65536 {
            return Err(DatabaseError::Migration(format!(
                "ensure_conversation_recall_vector_index: dimension {dimension} out of valid range (1..=65536)"
            )));
        }

        let conn = self.connect().await?;
        let current_dim = {
            let mut rows = conn
                .query("SELECT name FROM _migrations WHERE version = -3", ())
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!(
                        "Failed to inspect conversation recall vector metadata: {e}"
                    ))
                })?;

            rows.next().await.ok().flatten().and_then(|row| {
                row.get::<String>(0)
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
            })
        };

        if current_dim == Some(dimension) {
            return Ok(());
        }

        let expected_bytes = dimension * 4;
        let tx = conn.transaction().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "ensure_conversation_recall_vector_index: failed to start transaction: {e}"
            ))
        })?;

        tx.execute_batch(
            "DROP TRIGGER IF EXISTS conversation_recall_docs_fts_insert;
             DROP TRIGGER IF EXISTS conversation_recall_docs_fts_delete;
             DROP TRIGGER IF EXISTS conversation_recall_docs_fts_update;
             DROP TABLE IF EXISTS conversation_recall_docs_fts;
             DROP INDEX IF EXISTS idx_conversation_recall_docs_embedding;",
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to drop conversation recall search artifacts: {e}"
            ))
        })?;

        tx.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS conversation_recall_docs_new (
                doc_id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                channel TEXT NOT NULL,
                thread_id TEXT NOT NULL,
                turn_index INTEGER NOT NULL,
                user_message_id TEXT NOT NULL,
                assistant_message_id TEXT NOT NULL,
                turn_timestamp TEXT NOT NULL,
                user_text TEXT NOT NULL,
                assistant_text TEXT NOT NULL,
                search_text TEXT NOT NULL,
                preview_text TEXT NOT NULL,
                embedding F32_BLOB({dimension}),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );

            INSERT OR REPLACE INTO conversation_recall_docs_new
                (doc_id, user_id, conversation_id, channel, thread_id, turn_index, user_message_id, assistant_message_id, turn_timestamp, user_text, assistant_text, search_text, preview_text, embedding, updated_at)
            SELECT
                doc_id, user_id, conversation_id, channel, thread_id, turn_index, user_message_id, assistant_message_id, turn_timestamp,
                user_text, assistant_text, search_text, preview_text,
                CASE WHEN length(embedding) = {expected_bytes} THEN embedding ELSE NULL END,
                updated_at
            FROM conversation_recall_docs;

            DROP TABLE conversation_recall_docs;
            ALTER TABLE conversation_recall_docs_new RENAME TO conversation_recall_docs;

            CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_user_updated
                ON conversation_recall_docs(user_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_conversation_turn
                ON conversation_recall_docs(conversation_id, turn_index);
            CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_thread
                ON conversation_recall_docs(user_id, thread_id);
            CREATE INDEX IF NOT EXISTS idx_conversation_recall_docs_embedding
                ON conversation_recall_docs(libsql_vector_idx(embedding));

            CREATE VIRTUAL TABLE IF NOT EXISTS conversation_recall_docs_fts USING fts5(
                user_text,
                assistant_text,
                search_text,
                preview_text,
                content='conversation_recall_docs',
                content_rowid='rowid'
            );

            INSERT INTO conversation_recall_docs_fts(rowid, user_text, assistant_text, search_text, preview_text)
            SELECT rowid, user_text, assistant_text, search_text, preview_text
            FROM conversation_recall_docs;

            CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_insert AFTER INSERT ON conversation_recall_docs BEGIN
                INSERT INTO conversation_recall_docs_fts(rowid, user_text, assistant_text, search_text, preview_text)
                VALUES (new.rowid, new.user_text, new.assistant_text, new.search_text, new.preview_text);
            END;

            CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_delete AFTER DELETE ON conversation_recall_docs BEGIN
                INSERT INTO conversation_recall_docs_fts(conversation_recall_docs_fts, rowid, user_text, assistant_text, search_text, preview_text)
                VALUES ('delete', old.rowid, old.user_text, old.assistant_text, old.search_text, old.preview_text);
            END;

            CREATE TRIGGER IF NOT EXISTS conversation_recall_docs_fts_update AFTER UPDATE ON conversation_recall_docs BEGIN
                INSERT INTO conversation_recall_docs_fts(conversation_recall_docs_fts, rowid, user_text, assistant_text, search_text, preview_text)
                VALUES ('delete', old.rowid, old.user_text, old.assistant_text, old.search_text, old.preview_text);
                INSERT INTO conversation_recall_docs_fts(rowid, user_text, assistant_text, search_text, preview_text)
                VALUES (new.rowid, new.user_text, new.assistant_text, new.search_text, new.preview_text);
            END;"
        ))
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to rebuild conversation recall docs table: {e}"
            ))
        })?;

        tx.execute(
            "INSERT INTO _migrations (version, name) VALUES (-3, ?1)
             ON CONFLICT(version) DO UPDATE SET name = excluded.name, applied_at = (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![dimension.to_string()],
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to record conversation recall vector dimension: {e}"
            ))
        })?;

        tx.commit().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "ensure_conversation_recall_vector_index: commit failed: {e}"
            ))
        })?;

        Ok(())
    }
}

#[async_trait]
impl ConversationStore for LibSqlBackend {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, thread_id, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id.to_string(), channel, user_id, opt_text(thread_id), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE conversations SET last_activity = ?2 WHERE id = ?1",
            params![id.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        conn.execute(
                "INSERT INTO conversation_messages (id, conversation_id, role, content, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id.to_string(), conversation_id.to_string(), role, content, "{}", now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        self.touch_conversation(conversation_id).await?;
        Ok(id)
    }

    async fn update_conversation_message_content(
        &self,
        message_id: Uuid,
        content: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE conversation_messages SET content = ?2 WHERE id = ?1",
            params![message_id.to_string(), content],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn update_conversation_message_metadata(
        &self,
        message_id: Uuid,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE conversation_messages SET metadata = json_patch(COALESCE(metadata, '{}'), ?2) WHERE id = ?1",
            params![message_id.to_string(), metadata.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let affected = conn
            .execute(
            r#"
                INSERT INTO conversations (id, channel, user_id, thread_id, started_at, last_activity)
                VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                ON CONFLICT (id) DO UPDATE SET last_activity = excluded.last_activity
                WHERE conversations.user_id = excluded.user_id
                  AND conversations.channel = excluded.channel
                "#,
            params![id.to_string(), channel, user_id, opt_text(thread_id), now],
        )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(affected > 0)
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    c.started_at,
                    c.last_activity,
                    c.metadata,
                    c.channel,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id AND m.role = 'user') AS message_count,
                    (SELECT substr(m2.content, 1, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC, m2.rowid ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = ?1 AND c.channel = ?2
                ORDER BY datetime(c.last_activity) DESC
                LIMIT ?3
                "#,
                params![user_id, channel, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let metadata = get_json(&row, 3);
            let thread_type = metadata
                .get("thread_type")
                .and_then(|v| v.as_str())
                .map(String::from);
            let sql_title = get_opt_text(&row, 6);
            let title = sql_title.or_else(|| {
                metadata
                    .get("routine_name")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
            results.push(ConversationSummary {
                id: row
                    .get::<String>(0)
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or_default(),
                started_at: get_ts(&row, 1),
                last_activity: get_ts(&row, 2),
                message_count: get_i64(&row, 5),
                title,
                thread_type,
                channel: get_text(&row, 4),
            });
        }
        Ok(results)
    }

    async fn list_conversation_ids_for_channel(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Vec<Uuid>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id FROM conversations WHERE user_id = ?1 AND channel = ?2",
                params![user_id, channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut ids = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            ids.push(get_text(&row, 0).parse().unwrap_or_default());
        }
        Ok(ids)
    }

    async fn list_conversations_all_channels(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    c.id,
                    c.started_at,
                    c.last_activity,
                    c.metadata,
                    c.channel,
                    (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id AND m.role = 'user') AS message_count,
                    (SELECT substr(m2.content, 1, 100)
                     FROM conversation_messages m2
                     WHERE m2.conversation_id = c.id AND m2.role = 'user'
                     ORDER BY m2.created_at ASC, m2.rowid ASC
                     LIMIT 1
                    ) AS title
                FROM conversations c
                WHERE c.user_id = ?1
                ORDER BY datetime(c.last_activity) DESC
                LIMIT ?2
                "#,
                params![user_id, limit],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let metadata = get_json(&row, 3);
            let thread_type = metadata
                .get("thread_type")
                .and_then(|v| v.as_str())
                .map(String::from);
            let sql_title = get_opt_text(&row, 6);
            let title = sql_title.or_else(|| {
                metadata
                    .get("routine_name")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
            results.push(ConversationSummary {
                id: row
                    .get::<String>(0)
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or_default(),
                started_at: get_ts(&row, 1),
                last_activity: get_ts(&row, 2),
                message_count: get_i64(&row, 5),
                title,
                thread_type,
                channel: get_text(&row, 4),
            });
        }
        Ok(results)
    }

    /// Uses BEGIN IMMEDIATE to serialize concurrent writers and prevent
    /// duplicate routine conversations (TOCTOU race).
    async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let rid = routine_id.to_string();

        conn.execute("BEGIN IMMEDIATE", params![])
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result: Result<Uuid, DatabaseError> = async {
            let mut rows = conn
                .query(
                    r#"
                    SELECT id FROM conversations
                    WHERE user_id = ?1 AND json_extract(metadata, '$.routine_id') = ?2
                    LIMIT 1
                    "#,
                    params![user_id, rid],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

            if let Some(row) = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
            {
                let id_str: String = row.get(0).unwrap_or_default();
                return id_str
                    .parse()
                    .map_err(|_| DatabaseError::Serialization("Invalid UUID".to_string()));
            }

            let id = Uuid::new_v4();
            let now = fmt_ts(&Utc::now());
            let metadata = serde_json::json!({
                "thread_type": "routine",
                "routine_id": routine_id.to_string(),
                "routine_name": routine_name,
            });
            conn.execute(
                "INSERT INTO conversations (id, channel, user_id, metadata, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![id.to_string(), "routine", user_id, metadata.to_string(), now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            Ok(id)
        }
        .await;

        match &result {
            Ok(_) => {
                conn.execute("COMMIT", params![])
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
            Err(_) => {
                let _ = conn.execute("ROLLBACK", params![]).await;
            }
        }
        result
    }

    async fn find_routine_conversation(
        &self,
        routine_id: Uuid,
        user_id: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        let conn = self.connect().await?;
        let rid = routine_id.to_string();
        let mut rows = conn
            .query(
                r#"
                SELECT id FROM conversations
                WHERE user_id = ?1 AND json_extract(metadata, '$.routine_id') = ?2
                LIMIT 1
                "#,
                params![user_id, rid],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str: String = row.get(0).map_err(|e| {
                DatabaseError::Query(format!("Failed to read conversation id: {e}"))
            })?;
            let id = id_str
                .parse()
                .map_err(|_| DatabaseError::Serialization("Invalid UUID".to_string()))?;
            return Ok(Some(id));
        }
        Ok(None)
    }

    /// Uses BEGIN IMMEDIATE to serialize concurrent writers and prevent
    /// duplicate heartbeat conversations (TOCTOU race).
    async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;

        conn.execute("BEGIN IMMEDIATE", params![])
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result: Result<Uuid, DatabaseError> = async {
            let mut rows = conn
                .query(
                    r#"
                    SELECT id FROM conversations
                    WHERE user_id = ?1 AND json_extract(metadata, '$.thread_type') = 'heartbeat'
                    LIMIT 1
                    "#,
                    params![user_id],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

            if let Some(row) = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
            {
                let id_str: String = row.get(0).unwrap_or_default();
                return id_str
                    .parse()
                    .map_err(|_| DatabaseError::Serialization("Invalid UUID".to_string()));
            }

            let id = Uuid::new_v4();
            let now = fmt_ts(&Utc::now());
            let metadata = serde_json::json!({ "thread_type": "heartbeat" });
            conn.execute(
                "INSERT INTO conversations (id, channel, user_id, metadata, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![id.to_string(), "heartbeat", user_id, metadata.to_string(), now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            Ok(id)
        }
        .await;

        match &result {
            Ok(_) => {
                conn.execute("COMMIT", params![])
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
            Err(_) => {
                let _ = conn.execute("ROLLBACK", params![]).await;
            }
        }
        result
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        // Try to find existing
        let mut rows = conn
            .query(
                r#"
                SELECT id FROM conversations
                WHERE user_id = ?1 AND channel = ?2
                  AND json_extract(metadata, '$.thread_type') = 'assistant'
                LIMIT 1
                "#,
                params![user_id, channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str: String = row.get(0).unwrap_or_default();
            return id_str
                .parse()
                .map_err(|_| DatabaseError::Serialization("Invalid UUID".to_string()));
        }

        // Create new
        let id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        let metadata = serde_json::json!({"thread_type": "assistant", "title": "Assistant"});
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id.to_string(), channel, user_id, metadata.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "INSERT INTO conversations (id, channel, user_id, metadata, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id.to_string(), channel, user_id, metadata.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(id)
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        let conn = self.connect().await?;
        let fetch_limit = limit + 1;
        let cid = conversation_id.to_string();

        let mut rows = if let Some(before_ts) = before {
            conn.query(
                r#"
                    SELECT id, role, content, metadata, created_at
                    FROM conversation_messages
                    WHERE conversation_id = ?1 AND created_at < ?2
                    ORDER BY created_at DESC, rowid DESC
                    LIMIT ?3
                    "#,
                params![cid, fmt_ts(&before_ts), fetch_limit],
            )
            .await
        } else {
            conn.query(
                r#"
                    SELECT id, role, content, metadata, created_at
                    FROM conversation_messages
                    WHERE conversation_id = ?1
                    ORDER BY created_at DESC, rowid DESC
                    LIMIT ?2
                    "#,
                params![cid, fetch_limit],
            )
            .await
        }
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut all = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            all.push(ConversationMessage {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                role: get_text(&row, 1),
                content: get_text(&row, 2),
                metadata: get_json(&row, 3),
                created_at: get_ts(&row, 4),
            });
        }

        let has_more = all.len() as i64 > limit;
        all.truncate(limit as usize);
        all.reverse(); // oldest first
        Ok((all, has_more))
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        // SQLite: use json_patch to merge the key
        let patch = serde_json::json!({ key: value });
        conn.execute(
            "UPDATE conversations SET metadata = json_patch(metadata, ?2) WHERE id = ?1",
            params![id.to_string(), patch.to_string()],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT metadata FROM conversations WHERE id = ?1",
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(get_json(&row, 0))),
            None => Ok(None),
        }
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        self.list_messages_for_conversation(conversation_id).await
    }

    async fn list_conversation_turns(
        &self,
        conversation_id: Uuid,
        include_tool_calls: bool,
    ) -> Result<Vec<ConversationTurnView>, DatabaseError> {
        self.build_turns_for_conversation(conversation_id, include_tool_calls)
            .await
    }

    async fn upsert_conversation_recall_doc(
        &self,
        doc: &ConversationRecallDoc,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let embedding_blob = doc
            .embedding
            .as_deref()
            .map(serialize_embedding)
            .map(libsql::Value::Blob);

        conn.execute(
            r#"
            INSERT INTO conversation_recall_docs (
                doc_id, user_id, conversation_id, channel, thread_id, turn_index,
                user_message_id, assistant_message_id, turn_timestamp, user_text, assistant_text,
                search_text, preview_text, embedding, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(doc_id) DO UPDATE SET
                user_id = excluded.user_id,
                conversation_id = excluded.conversation_id,
                channel = excluded.channel,
                thread_id = excluded.thread_id,
                turn_index = excluded.turn_index,
                user_message_id = excluded.user_message_id,
                assistant_message_id = excluded.assistant_message_id,
                turn_timestamp = excluded.turn_timestamp,
                user_text = excluded.user_text,
                assistant_text = excluded.assistant_text,
                search_text = excluded.search_text,
                preview_text = excluded.preview_text,
                embedding = COALESCE(excluded.embedding, conversation_recall_docs.embedding),
                updated_at = excluded.updated_at
            "#,
            params![
                doc.doc_id.to_string(),
                doc.user_id.clone(),
                doc.conversation_id.to_string(),
                doc.channel.clone(),
                doc.thread_id.clone(),
                doc.turn_index as i64,
                doc.user_message_id.to_string(),
                doc.assistant_message_id.to_string(),
                fmt_ts(&doc.turn_timestamp),
                doc.user_text.clone(),
                doc.assistant_text.clone(),
                doc.search_text.clone(),
                doc.preview_text.clone(),
                embedding_blob,
                fmt_ts(&doc.updated_at),
            ],
        )
        .await
        .map_err(|e| {
            DatabaseError::Query(format!("failed to upsert conversation recall doc: {e}"))
        })?;
        Ok(())
    }

    async fn search_conversation_recall(
        &self,
        user_id: &str,
        query: &str,
        query_embedding: Option<&[f32]>,
        config: &SearchConfig,
        exclude_conversation_id: Option<Uuid>,
    ) -> Result<Vec<ConversationRecallHit>, DatabaseError> {
        let conn = self.connect().await?;
        let pre_limit = config.pre_fusion_limit.max(config.limit) as i64;
        let match_query = sqlite_match_query(query);
        let exclude = exclude_conversation_id.map(|id| id.to_string());

        let fts_results = if config.use_fts {
            let sql = if exclude.is_some() {
                format!(
                    "SELECT {}, bm25(conversation_recall_docs_fts) AS score
                     FROM conversation_recall_docs_fts fts
                     JOIN conversation_recall_docs d ON d.rowid = fts.rowid
                     WHERE d.user_id = ?1 AND d.conversation_id != ?2 AND conversation_recall_docs_fts MATCH ?3
                     ORDER BY score, d.turn_timestamp DESC
                     LIMIT ?4",
                    Self::doc_select_columns()
                )
            } else {
                format!(
                    "SELECT {}, bm25(conversation_recall_docs_fts) AS score
                     FROM conversation_recall_docs_fts fts
                     JOIN conversation_recall_docs d ON d.rowid = fts.rowid
                     WHERE d.user_id = ?1 AND conversation_recall_docs_fts MATCH ?2
                     ORDER BY score, d.turn_timestamp DESC
                     LIMIT ?3",
                    Self::doc_select_columns()
                )
            };

            let mut rows = if let Some(ref exclude_id) = exclude {
                conn.query(
                    &sql,
                    params![user_id, exclude_id.clone(), match_query, pre_limit],
                )
                .await
            } else {
                conn.query(&sql, params![user_id, match_query, pre_limit])
                    .await
            }
            .map_err(|e| DatabaseError::Query(format!("conversation recall FTS failed: {e}")))?;

            let mut results = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?
            {
                let raw_score = row.get::<f64>(14).unwrap_or(0.0) as f32;
                let normalized = 1.0 / (1.0 + raw_score.abs());
                results.push(RankedItem {
                    item_id: get_text(&row, 0).parse().unwrap_or_default(),
                    payload: Self::row_to_recall_hit(
                        &row,
                        normalized,
                        results.len() as u32 + 1,
                        false,
                    ),
                    rank: results.len() as u32 + 1,
                });
            }
            results
        } else {
            Vec::new()
        };

        let vector_results = if let (true, Some(embedding)) = (config.use_vector, query_embedding) {
            let vector_json = format!(
                "[{}]",
                embedding
                    .iter()
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let sql = if exclude.is_some() {
                format!(
                    "SELECT {}
                     FROM vector_top_k('idx_conversation_recall_docs_embedding', vector(?1), ?2) AS top_k
                     JOIN conversation_recall_docs d ON d.rowid = top_k.id
                     WHERE d.user_id = ?3 AND d.conversation_id != ?4",
                    Self::doc_select_columns()
                )
            } else {
                format!(
                    "SELECT {}
                     FROM vector_top_k('idx_conversation_recall_docs_embedding', vector(?1), ?2) AS top_k
                     JOIN conversation_recall_docs d ON d.rowid = top_k.id
                     WHERE d.user_id = ?3",
                    Self::doc_select_columns()
                )
            };

            let vector_attempt: Result<Vec<RankedItem<ConversationRecallHit>>, DatabaseError> =
                async {
                    let mut rows = if let Some(ref exclude_id) = exclude {
                        conn.query(
                            &sql,
                            params![vector_json, pre_limit, user_id, exclude_id.clone()],
                        )
                        .await
                    } else {
                        conn.query(&sql, params![vector_json, pre_limit, user_id])
                            .await
                    }
                    .map_err(|e| {
                        DatabaseError::Query(format!(
                            "conversation recall vector query failed: {e}"
                        ))
                    })?;

                    let mut results = Vec::new();
                    while let Some(row) = rows.next().await.map_err(|e| {
                        DatabaseError::Query(format!(
                            "conversation recall vector row fetch failed: {e}"
                        ))
                    })? {
                        results.push(RankedItem {
                            item_id: get_text(&row, 0).parse().unwrap_or_default(),
                            payload: Self::row_to_recall_hit(
                                &row,
                                1.0 / (results.len() as f32 + 1.0),
                                results.len() as u32 + 1,
                                true,
                            ),
                            rank: results.len() as u32 + 1,
                        });
                    }
                    Ok(results)
                }
                .await;

            match vector_attempt {
                Ok(results) => results,
                Err(error) => {
                    tracing::warn!(
                        "Conversation recall vector search failed, falling back to FTS-only: {}",
                        error
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        let mut hits = Vec::new();
        for item in fuse_results(fts_results, vector_results, config) {
            let mut hit = item.payload;
            hit.score = item.score;
            hit.fts_rank = item.fts_rank;
            hit.vector_rank = item.vector_rank;
            hits.push(hit);
        }
        Ok(hits)
    }

    async fn backfill_conversation_recall_for_user(
        &self,
        user_id: &str,
    ) -> Result<usize, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id FROM conversations WHERE user_id = ?1 ORDER BY started_at ASC",
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut upserted = 0usize;
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let conversation_id = get_text(&row, 0).parse().unwrap_or_default();
            for turn in self
                .build_turns_for_conversation(conversation_id, false)
                .await?
            {
                if let Some(doc) = Self::turn_to_recall_doc(user_id, &turn) {
                    self.upsert_conversation_recall_doc(&doc).await?;
                    upserted += 1;
                }
            }
        }

        Ok(upserted)
    }

    async fn list_conversation_recall_docs_without_embeddings(
        &self,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<ConversationRecallDoc>, DatabaseError> {
        let conn = self.connect().await?;
        let sql = format!(
            "SELECT {}
             FROM conversation_recall_docs d
             WHERE d.user_id = ?1 AND d.embedding IS NULL
             ORDER BY d.updated_at DESC
             LIMIT ?2",
            Self::doc_select_columns()
        );
        let mut rows = conn
            .query(&sql, params![user_id, limit as i64])
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut docs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            docs.push(Self::row_to_recall_doc(&row));
        }
        Ok(docs)
    }

    async fn list_recent_conversation_recall(
        &self,
        user_id: &str,
        limit: usize,
        exclude_conversation_id: Option<Uuid>,
    ) -> Result<Vec<ConversationRecallHit>, DatabaseError> {
        let conn = self.connect().await?;
        let sql = if exclude_conversation_id.is_some() {
            format!(
                "SELECT {}
                 FROM conversation_recall_docs d
                 WHERE d.user_id = ?1 AND d.conversation_id != ?2
                 ORDER BY d.turn_timestamp DESC, d.updated_at DESC
                 LIMIT ?3",
                Self::doc_select_columns()
            )
        } else {
            format!(
                "SELECT {}
                 FROM conversation_recall_docs d
                 WHERE d.user_id = ?1
                 ORDER BY d.turn_timestamp DESC, d.updated_at DESC
                 LIMIT ?2",
                Self::doc_select_columns()
            )
        };

        let mut rows = if let Some(exclude_id) = exclude_conversation_id {
            conn.query(&sql, params![user_id, exclude_id.to_string(), limit as i64])
                .await
        } else {
            conn.query(&sql, params![user_id, limit as i64]).await
        }
        .map_err(|e| {
            DatabaseError::Query(format!(
                "failed to list recent conversation recall docs: {e}"
            ))
        })?;

        let mut hits = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let rank = hits.len() as u32 + 1;
            hits.push(ConversationRecallHit {
                doc_id: get_text(&row, 0).parse().unwrap_or_default(),
                conversation_id: get_text(&row, 2).parse().unwrap_or_default(),
                channel: get_text(&row, 3),
                thread_id: get_text(&row, 4),
                turn_index: get_i64(&row, 5).max(0) as usize,
                user_message_id: get_text(&row, 6).parse().unwrap_or_default(),
                assistant_message_id: get_text(&row, 7).parse().unwrap_or_default(),
                turn_timestamp: get_ts(&row, 8),
                user_text: get_text(&row, 9),
                assistant_text: get_text(&row, 10),
                preview_text: get_text(&row, 12),
                score: 1.0 / rank as f32,
                fts_rank: None,
                vector_rank: None,
                confidence_score: 0.0,
            });
        }
        Ok(hits)
    }

    async fn update_conversation_recall_doc_embedding(
        &self,
        doc_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE conversation_recall_docs SET embedding = ?2, updated_at = ?3 WHERE doc_id = ?1",
            params![
                doc_id.to_string(),
                serialize_embedding(embedding),
                fmt_ts(&Utc::now()),
            ],
        )
        .await
        .map_err(|e| {
            DatabaseError::Query(format!(
                "failed to update conversation recall embedding: {e}"
            ))
        })?;
        Ok(())
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM conversations WHERE id = ?1 AND user_id = ?2",
                libsql::params![conversation_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let found = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(found.is_some())
    }

    async fn delete_conversation(&self, conversation_id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            conn.execute(
                "DELETE FROM llm_calls WHERE conversation_id = ?1 AND job_id IS NULL",
                libsql::params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            conn.execute(
                "UPDATE llm_calls SET conversation_id = NULL WHERE conversation_id = ?1 AND job_id IS NOT NULL",
                libsql::params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            conn.execute(
                "UPDATE agent_jobs SET conversation_id = NULL WHERE conversation_id = ?1",
                libsql::params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            conn.execute(
                "DELETE FROM conversations WHERE id = ?1",
                libsql::params![conversation_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            Ok::<_, DatabaseError>(())
        }
        .await;

        match result {
            Ok(()) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                return Err(error);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::JobContext;
    use crate::db::{Database, JobStore};
    use crate::history::LlmCallRecord;
    use crate::retrieval::SearchConfig;
    use rust_decimal::Decimal;

    fn tool_call_row(name: &str) -> String {
        serde_json::json!({
            "name": name,
            "call_id": format!("call_{name}"),
            "parameters": {"query": "hello"},
            "result_preview": "ok",
            "completed_at": Utc::now().to_rfc3339(),
        })
        .to_string()
    }

    #[tokio::test]
    async fn test_get_or_create_routine_conversation_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_routine_conv.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let routine_id = Uuid::new_v4();
        let user_id = "test_user";

        // First call — creates the conversation
        let id1 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        // Second call — should return the SAME conversation
        let id2 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        assert_eq!(id1, id2, "Expected same conversation ID on repeated calls");

        // Third call — still the same
        let id3 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        assert_eq!(id1, id3);

        // Different routine_id should get a different conversation
        let other_routine_id = Uuid::new_v4();
        let id4 = backend
            .get_or_create_routine_conversation(other_routine_id, "other-routine", user_id)
            .await
            .unwrap();

        assert_ne!(
            id1, id4,
            "Different routines should get different conversations"
        );
    }

    #[tokio::test]
    async fn test_routine_conversation_persists_across_messages() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_routine_persist.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let routine_id = Uuid::new_v4();
        let user_id = "test_user";

        // First invocation: create conversation and add a message
        let id1 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        backend
            .add_conversation_message(id1, "assistant", "[cron] Completed: all good")
            .await
            .unwrap();

        // Second invocation: should find existing conversation
        let id2 = backend
            .get_or_create_routine_conversation(routine_id, "my-routine", user_id)
            .await
            .unwrap();

        assert_eq!(id1, id2, "Second invocation should reuse same conversation");

        backend
            .add_conversation_message(id2, "assistant", "[cron] Completed: still good")
            .await
            .unwrap();

        // Verify only one routine conversation exists (not two)
        let convs = backend
            .list_conversations_all_channels(user_id, 50)
            .await
            .unwrap();

        let routine_convs: Vec<_> = convs.iter().filter(|c| c.channel == "routine").collect();
        assert_eq!(
            routine_convs.len(),
            1,
            "Should have exactly 1 routine conversation, found {}",
            routine_convs.len()
        );
    }

    #[tokio::test]
    async fn test_get_or_create_heartbeat_conversation_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_heartbeat_conv.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let user_id = "test_user";

        let id1 = backend
            .get_or_create_heartbeat_conversation(user_id)
            .await
            .unwrap();

        let id2 = backend
            .get_or_create_heartbeat_conversation(user_id)
            .await
            .unwrap();

        assert_eq!(
            id1, id2,
            "Expected same heartbeat conversation on repeated calls"
        );
    }

    #[tokio::test]
    async fn test_list_conversation_turns_rebuilds_canonical_turns() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("conversation_turns.db");
        let backend = LibSqlBackend::new_local(&db_path).await.expect("db");
        backend.run_migrations().await.expect("migrations");

        let conversation_id = backend
            .create_conversation("desktop", "user-1", Some("thread-1"))
            .await
            .expect("conversation");
        backend
            .add_conversation_message(conversation_id, "user", "Where did we leave off?")
            .await
            .expect("user");
        backend
            .add_conversation_message(conversation_id, "thinking", "internal chain")
            .await
            .expect("thinking");
        backend
            .add_conversation_message(conversation_id, "tool_call", &tool_call_row("search"))
            .await
            .expect("tool call");
        backend
            .add_conversation_message(
                conversation_id,
                "assistant",
                "We stopped after wiring the DB.",
            )
            .await
            .expect("assistant");

        let plain_turns = backend
            .list_conversation_turns(conversation_id, false)
            .await
            .expect("turns");
        assert_eq!(plain_turns.len(), 1);
        assert_eq!(plain_turns[0].user_text, "Where did we leave off?");
        assert_eq!(
            plain_turns[0].assistant_text.as_deref(),
            Some("We stopped after wiring the DB.")
        );
        assert!(plain_turns[0].tool_calls.is_empty());

        let rich_turns = backend
            .list_conversation_turns(conversation_id, true)
            .await
            .expect("turns");
        assert_eq!(rich_turns[0].tool_calls.len(), 1);
        assert_eq!(rich_turns[0].tool_calls[0].name, "search");
    }

    #[tokio::test]
    async fn test_backfill_conversation_recall_only_indexes_completed_turns() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("conversation_recall_backfill.db");
        let backend = LibSqlBackend::new_local(&db_path).await.expect("db");
        backend.run_migrations().await.expect("migrations");

        let conversation_id = backend
            .create_conversation("desktop", "user-1", Some("thread-1"))
            .await
            .expect("conversation");
        backend
            .add_conversation_message(conversation_id, "user", "Remember the release date")
            .await
            .expect("user");
        backend
            .add_conversation_message(conversation_id, "tool_call", &tool_call_row("calendar"))
            .await
            .expect("tool call");
        backend
            .add_conversation_message(
                conversation_id,
                "assistant",
                "The release was on 2026-03-01.",
            )
            .await
            .expect("assistant");
        backend
            .add_conversation_message(conversation_id, "user", "And the follow-up?")
            .await
            .expect("incomplete user");

        let indexed = backend
            .backfill_conversation_recall_for_user("user-1")
            .await
            .expect("backfill");
        assert_eq!(indexed, 1);

        let docs = backend
            .list_conversation_recall_docs_without_embeddings("user-1", 10)
            .await
            .expect("docs");
        assert_eq!(docs.len(), 1);
        assert!(docs[0].search_text.contains("Remember the release date"));
        assert!(!docs[0].search_text.contains("calendar"));
        assert_eq!(docs[0].turn_index, 0);
    }

    #[tokio::test]
    async fn test_search_conversation_recall_fts_respects_exclusion() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("conversation_recall_search.db");
        let backend = LibSqlBackend::new_local(&db_path).await.expect("db");
        backend.run_migrations().await.expect("migrations");

        let conversation_a = backend
            .create_conversation("desktop", "user-1", Some("thread-a"))
            .await
            .expect("conversation a");
        backend
            .add_conversation_message(conversation_a, "user", "We shipped mooncake analytics")
            .await
            .expect("user a");
        backend
            .add_conversation_message(
                conversation_a,
                "assistant",
                "Yes, mooncake analytics launched yesterday.",
            )
            .await
            .expect("assistant a");

        let conversation_b = backend
            .create_conversation("desktop", "user-1", Some("thread-b"))
            .await
            .expect("conversation b");
        backend
            .add_conversation_message(
                conversation_b,
                "user",
                "Draft a recap for mooncake analytics",
            )
            .await
            .expect("user b");
        backend
            .add_conversation_message(
                conversation_b,
                "assistant",
                "The recap mentions mooncake analytics and adoption.",
            )
            .await
            .expect("assistant b");

        backend
            .backfill_conversation_recall_for_user("user-1")
            .await
            .expect("backfill");

        let hits = backend
            .search_conversation_recall(
                "user-1",
                "mooncake analytics",
                None,
                &SearchConfig::default().fts_only().with_limit(10),
                Some(conversation_a),
            )
            .await
            .expect("search");

        assert!(!hits.is_empty());
        assert!(hits.iter().all(|hit| hit.conversation_id != conversation_a));
        assert!(hits.iter().any(|hit| hit.conversation_id == conversation_b));
    }

    #[tokio::test]
    async fn test_search_conversation_recall_falls_back_when_vector_query_fails() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("conversation_recall_vector_fallback.db");
        let backend = LibSqlBackend::new_local(&db_path).await.expect("db");
        backend.run_migrations().await.expect("migrations");

        let conversation_id = backend
            .create_conversation("desktop", "user-1", Some("thread-a"))
            .await
            .expect("conversation");
        backend
            .add_conversation_message(conversation_id, "user", "We discussed launch readiness")
            .await
            .expect("user");
        backend
            .add_conversation_message(
                conversation_id,
                "assistant",
                "Launch readiness review is scheduled for Friday.",
            )
            .await
            .expect("assistant");

        backend
            .backfill_conversation_recall_for_user("user-1")
            .await
            .expect("backfill");

        let hits = backend
            .search_conversation_recall(
                "user-1",
                "launch readiness",
                Some(&[0.1, 0.2, 0.3]),
                &SearchConfig::default().with_limit(5),
                None,
            )
            .await
            .expect("search should fall back to fts");

        assert!(!hits.is_empty());
        assert!(
            hits.iter()
                .any(|hit| hit.preview_text.contains("Launch readiness"))
        );
    }

    #[tokio::test]
    async fn test_delete_conversation_handles_job_and_chat_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("delete_conversation_fk_cleanup.db");
        let backend = LibSqlBackend::new_local(&db_path).await.expect("db");
        backend.run_migrations().await.expect("migrations");

        let conversation_id = backend
            .create_conversation("desktop", "user-1", Some("thread-a"))
            .await
            .expect("conversation");
        backend
            .add_conversation_message(conversation_id, "user", "hello")
            .await
            .expect("user");
        backend
            .add_conversation_message(conversation_id, "assistant", "world")
            .await
            .expect("assistant");

        let mut job = JobContext::with_user("user-1", "Cleanup test", "job linked to conversation");
        job.conversation_id = Some(conversation_id);
        backend.save_job(&job).await.expect("save job");

        backend
            .record_llm_call(&LlmCallRecord {
                job_id: None,
                conversation_id: Some(conversation_id),
                provider: "test",
                model: "stub",
                input_tokens: 10,
                output_tokens: 5,
                cost: Decimal::new(15, 6),
                purpose: Some("chat"),
            })
            .await
            .expect("record chat llm call");

        backend
            .record_llm_call(&LlmCallRecord {
                job_id: Some(job.job_id),
                conversation_id: Some(conversation_id),
                provider: "test",
                model: "stub",
                input_tokens: 20,
                output_tokens: 10,
                cost: Decimal::new(30, 6),
                purpose: Some("job"),
            })
            .await
            .expect("record job llm call");

        backend
            .delete_conversation(conversation_id)
            .await
            .expect("delete conversation");

        assert!(
            backend
                .get_conversation_metadata(conversation_id)
                .await
                .expect("conversation metadata after delete")
                .is_none()
        );

        let stored_job = backend
            .get_job(job.job_id)
            .await
            .expect("get job after delete")
            .expect("job should survive");
        assert_eq!(stored_job.conversation_id, None);

        let conn = backend.connect().await.expect("connect");
        let mut rows = conn
            .query(
                "SELECT job_id, conversation_id, purpose FROM llm_calls ORDER BY purpose ASC",
                (),
            )
            .await
            .expect("query llm calls");

        let mut seen = Vec::new();
        while let Some(row) = rows.next().await.expect("next llm call row") {
            seen.push((
                get_opt_text(&row, 0),
                get_opt_text(&row, 1),
                get_opt_text(&row, 2),
            ));
        }

        assert_eq!(seen.len(), 1, "chat-only llm call should be deleted");
        let job_id_text = job.job_id.to_string();
        assert_eq!(seen[0].0.as_deref(), Some(job_id_text.as_str()));
        assert_eq!(seen[0].1, None, "job-linked llm call should be detached");
        assert_eq!(seen[0].2.as_deref(), Some("job"));
    }

    #[tokio::test]
    async fn test_conversation_recall_vector_query_works_with_matching_dimension() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("conversation_recall_vector_ok.db");
        let backend = LibSqlBackend::new_local(&db_path).await.expect("db");
        backend.run_migrations().await.expect("migrations");
        backend
            .ensure_conversation_recall_vector_index(3)
            .await
            .expect("vector index");

        let conversation_id = backend
            .create_conversation("desktop", "user-1", Some("thread-a"))
            .await
            .expect("conversation");
        let user_message_id = backend
            .add_conversation_message(conversation_id, "user", "Where did we leave off?")
            .await
            .expect("user");
        let assistant_message_id = backend
            .add_conversation_message(
                conversation_id,
                "assistant",
                "We discussed launch readiness.",
            )
            .await
            .expect("assistant");

        let doc = ConversationRecallDoc {
            doc_id: Uuid::new_v4(),
            user_id: "user-1".to_string(),
            conversation_id,
            channel: "desktop".to_string(),
            thread_id: "thread-a".to_string(),
            turn_index: 0,
            user_message_id,
            assistant_message_id,
            turn_timestamp: Utc::now(),
            user_text: "Where did we leave off?".to_string(),
            assistant_text: "We discussed launch readiness.".to_string(),
            search_text: "User: Where did we leave off?\nAssistant: We discussed launch readiness."
                .to_string(),
            preview_text:
                "User: Where did we leave off?\nAssistant: We discussed launch readiness."
                    .to_string(),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            updated_at: Utc::now(),
        };

        backend
            .upsert_conversation_recall_doc(&doc)
            .await
            .expect("upsert doc");

        let hits = backend
            .search_conversation_recall(
                "user-1",
                "launch readiness",
                Some(&[0.1, 0.2, 0.3]),
                &SearchConfig::default().with_limit(5),
                None,
            )
            .await
            .expect("search");

        assert!(!hits.is_empty());
    }
}
