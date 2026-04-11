use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::ConversationRecallConfig;
use crate::db::Database;
use crate::error::DatabaseError;
use crate::retrieval::SearchConfig;
use crate::workspace::EmbeddingProvider;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationToolCallSummary {
    pub name: String,
    pub call_id: Option<String>,
    pub status: String,
    pub parameters: serde_json::Value,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub rationale: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurnView {
    pub conversation_id: Uuid,
    pub channel: String,
    pub thread_id: String,
    pub turn_index: usize,
    pub user_message_id: Uuid,
    pub assistant_message_id: Option<Uuid>,
    pub timestamp: DateTime<Utc>,
    pub user_text: String,
    pub assistant_text: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ConversationToolCallSummary>,
}

#[derive(Debug, Clone)]
pub struct ConversationRecallDoc {
    pub doc_id: Uuid,
    pub user_id: String,
    pub conversation_id: Uuid,
    pub channel: String,
    pub thread_id: String,
    pub turn_index: usize,
    pub user_message_id: Uuid,
    pub assistant_message_id: Uuid,
    pub turn_timestamp: DateTime<Utc>,
    pub user_text: String,
    pub assistant_text: String,
    pub search_text: String,
    pub preview_text: String,
    pub embedding: Option<Vec<f32>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecallHit {
    pub doc_id: Uuid,
    pub conversation_id: Uuid,
    pub channel: String,
    pub thread_id: String,
    pub turn_index: usize,
    pub user_message_id: Uuid,
    pub assistant_message_id: Uuid,
    pub turn_timestamp: DateTime<Utc>,
    pub user_text: String,
    pub assistant_text: String,
    pub preview_text: String,
    pub score: f32,
    pub fts_rank: Option<u32>,
    pub vector_rank: Option<u32>,
    pub confidence_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecallPromptEntry {
    pub conversation_id: Uuid,
    pub channel: String,
    pub thread_id: String,
    pub turn_index: usize,
    pub turn_timestamp: DateTime<Utc>,
    pub preview_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecallSelection {
    pub entries: Vec<ConversationRecallPromptEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationContextSlice {
    pub conversation_id: Uuid,
    pub anchor_turn_index: Option<usize>,
    pub total_turns: usize,
    pub turns: Vec<ConversationTurnView>,
}

#[derive(Clone)]
pub struct ConversationRecallManager {
    db: Arc<dyn Database>,
    embeddings: Arc<RwLock<Option<Arc<dyn EmbeddingProvider>>>>,
    config: ConversationRecallConfig,
}

impl ConversationRecallManager {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self {
            db,
            embeddings: Arc::new(RwLock::new(None)),
            config: ConversationRecallConfig::default(),
        }
    }

    pub fn with_embeddings(self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.set_embeddings(Some(provider));
        self
    }

    pub fn with_config(mut self, config: ConversationRecallConfig) -> Self {
        self.config = config;
        self
    }

    pub fn set_embeddings(&self, provider: Option<Arc<dyn EmbeddingProvider>>) {
        *self.embeddings.write().unwrap_or_else(|e| e.into_inner()) = provider;
    }

    fn current_embeddings(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.embeddings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn retrieval_config(&self, limit: usize) -> SearchConfig {
        SearchConfig {
            limit,
            rrf_k: self.config.rrf_k,
            use_fts: true,
            use_vector: self.current_embeddings().is_some(),
            min_score: 0.0,
            pre_fusion_limit: self.config.pre_fusion_limit.max(limit),
            fusion_strategy: self.config.fusion_strategy,
            fts_weight: self.config.fts_weight,
            vector_weight: self.config.vector_weight,
        }
    }

    pub async fn backfill_for_user(&self, user_id: &str) -> Result<usize, DatabaseError> {
        let count = self
            .db
            .backfill_conversation_recall_for_user(user_id)
            .await?;
        if self.current_embeddings().is_some() {
            self.backfill_embeddings(user_id, self.config.pre_fusion_limit.max(100))
                .await?;
        }
        Ok(count)
    }

    pub async fn backfill_embeddings(
        &self,
        user_id: &str,
        batch_size: usize,
    ) -> Result<usize, DatabaseError> {
        let Some(provider) = self.current_embeddings() else {
            return Ok(0);
        };

        let mut updated = 0usize;
        loop {
            let docs = self
                .db
                .list_conversation_recall_docs_without_embeddings(user_id, batch_size)
                .await?;
            if docs.is_empty() {
                break;
            }

            let texts = docs
                .iter()
                .map(|doc| doc.search_text.clone())
                .collect::<Vec<_>>();
            let embeddings = provider.embed_batch(&texts).await.map_err(|error| {
                DatabaseError::Query(format!(
                    "conversation recall embedding backfill failed: {error}"
                ))
            })?;

            for (doc, embedding) in docs.iter().zip(embeddings) {
                self.db
                    .update_conversation_recall_doc_embedding(doc.doc_id, &embedding)
                    .await?;
                updated += 1;
            }
        }

        Ok(updated)
    }

    pub async fn upsert_completed_turn(
        &self,
        user_id: &str,
        turn: &ConversationTurnView,
    ) -> Result<(), DatabaseError> {
        let Some(assistant_message_id) = turn.assistant_message_id else {
            return Ok(());
        };
        let Some(assistant_text) = turn.assistant_text.clone() else {
            return Ok(());
        };

        let search_text = Self::build_search_text(&turn.user_text, &assistant_text);
        let preview_text = Self::build_preview_text(&turn.user_text, &assistant_text);
        let embedding = if let Some(provider) = self.current_embeddings() {
            match provider.embed(&search_text).await {
                Ok(embedding) => Some(embedding),
                Err(error) => {
                    tracing::warn!("conversation recall embedding failed: {}", error);
                    None
                }
            }
        } else {
            None
        };

        let doc = ConversationRecallDoc {
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
            assistant_text,
            search_text,
            preview_text,
            embedding,
            updated_at: Utc::now(),
        };

        self.db.upsert_conversation_recall_doc(&doc).await
    }

    pub async fn search(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
        exclude_conversation_id: Option<Uuid>,
    ) -> Result<Vec<ConversationRecallHit>, DatabaseError> {
        let embedding = if let Some(provider) = self.current_embeddings() {
            match provider.embed(query).await {
                Ok(embedding) => Some(embedding),
                Err(error) => {
                    tracing::warn!("conversation recall query embedding failed: {}", error);
                    None
                }
            }
        } else {
            None
        };

        let mut hits = self
            .db
            .search_conversation_recall(
                user_id,
                query,
                embedding.as_deref(),
                &self.retrieval_config(limit),
                exclude_conversation_id,
            )
            .await?;

        let top_score = hits
            .iter()
            .map(|hit| hit.score)
            .fold(0.0f32, f32::max)
            .max(1.0);
        for hit in &mut hits {
            hit.confidence_score = Self::confidence_score(hit, top_score);
        }
        Ok(hits)
    }

    pub async fn build_prompt_context(
        &self,
        user_id: &str,
        current_conversation_id: Option<Uuid>,
        query: &str,
        is_group_chat: bool,
    ) -> Result<String, DatabaseError> {
        if is_group_chat && !self.config.allow_group_auto_recall {
            return Ok(String::new());
        }

        let hits = self
            .search(
                user_id,
                query,
                self.config.seed_limit,
                current_conversation_id,
            )
            .await?;
        let selection = self.select_for_prompt(&hits);
        if selection.entries.is_empty() {
            return Ok(String::new());
        }

        let block = selection
            .entries
            .iter()
            .map(|entry| {
                format!(
                    "### {} [{}]\nConversation: {}\nThread: {}\nTurn: {}\n{}\n",
                    entry.turn_timestamp.to_rfc3339(),
                    entry.channel,
                    entry.conversation_id,
                    entry.thread_id,
                    entry.turn_index,
                    entry.preview_text
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(format!("## Cross-Conversation Recall\n\n{block}"))
    }

    pub async fn search_with_preview(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
        current_conversation_id: Option<Uuid>,
        include_current_thread: bool,
    ) -> Result<Vec<(ConversationRecallHit, ConversationContextSlice)>, DatabaseError> {
        let exclude = if include_current_thread {
            None
        } else {
            current_conversation_id
        };

        let mut hits = self.search(user_id, query, limit, exclude).await?;
        if hits.is_empty() {
            hits = self
                .db
                .list_recent_conversation_recall(user_id, limit, exclude)
                .await?;
        }
        let mut results = Vec::with_capacity(hits.len());
        for hit in hits {
            let preview = self
                .read_context(
                    user_id,
                    hit.conversation_id,
                    Some(hit.turn_index),
                    1,
                    1,
                    false,
                    false,
                )
                .await?;
            results.push((hit, preview));
        }
        Ok(results)
    }

    pub async fn read_context(
        &self,
        user_id: &str,
        conversation_id: Uuid,
        anchor_turn_index: Option<usize>,
        before_turns: usize,
        after_turns: usize,
        full_thread: bool,
        include_tool_calls: bool,
    ) -> Result<ConversationContextSlice, DatabaseError> {
        if !self
            .db
            .conversation_belongs_to_user(conversation_id, user_id)
            .await?
        {
            return Err(DatabaseError::NotFound {
                entity: "conversation".to_string(),
                id: conversation_id.to_string(),
            });
        }

        let turns = self
            .db
            .list_conversation_turns(conversation_id, include_tool_calls)
            .await?;
        let total_turns = turns.len();
        if full_thread {
            return Ok(ConversationContextSlice {
                conversation_id,
                anchor_turn_index,
                total_turns,
                turns,
            });
        }

        if turns.is_empty() {
            return Ok(ConversationContextSlice {
                conversation_id,
                anchor_turn_index,
                total_turns: 0,
                turns: Vec::new(),
            });
        }

        let anchor =
            anchor_turn_index.unwrap_or_else(|| turns.last().map(|t| t.turn_index).unwrap_or(0));
        let anchor_pos = turns
            .iter()
            .position(|turn| turn.turn_index == anchor)
            .unwrap_or(total_turns.saturating_sub(1));
        let start = anchor_pos.saturating_sub(before_turns);
        let end = (anchor_pos + after_turns + 1).min(total_turns);

        Ok(ConversationContextSlice {
            conversation_id,
            anchor_turn_index: Some(anchor),
            total_turns,
            turns: turns[start..end].to_vec(),
        })
    }

    fn select_for_prompt(&self, hits: &[ConversationRecallHit]) -> ConversationRecallSelection {
        let mut recent = Vec::new();
        let mut mid = Vec::new();
        let mut far = Vec::new();
        let now = Utc::now();

        for hit in hits.iter().cloned() {
            let age_days = now.signed_duration_since(hit.turn_timestamp).num_days();
            if age_days <= self.config.recent_bucket_days {
                recent.push(hit);
            } else if age_days <= self.config.mid_bucket_days {
                mid.push(hit);
            } else {
                far.push(hit);
            }
        }

        let mut selected = Vec::new();
        let mut seen = HashSet::new();

        Self::take_bucket(
            &mut selected,
            &mut seen,
            &mut recent,
            self.config.recent_base_quota,
        );
        Self::take_bucket(
            &mut selected,
            &mut seen,
            &mut mid,
            self.config.mid_base_quota,
        );
        Self::take_bucket(
            &mut selected,
            &mut seen,
            &mut far,
            self.config.far_base_quota,
        );

        let mut remaining = recent.into_iter().chain(mid).chain(far).collect::<Vec<_>>();
        remaining.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.turn_timestamp.cmp(&a.turn_timestamp))
        });

        while selected.len() < self.config.auto_prompt_base_limit {
            let Some(next) = remaining.first().cloned() else {
                break;
            };
            remaining.remove(0);
            if seen.insert(next.doc_id) {
                selected.push(next);
            }
        }

        let selected_ids = selected
            .iter()
            .map(|hit| hit.doc_id)
            .collect::<HashSet<_>>();
        let mut high_conf_recent = Vec::new();
        let mut high_conf_mid = Vec::new();
        let mut high_conf_far = Vec::new();
        let now = Utc::now();

        for hit in hits.iter().cloned() {
            if selected_ids.contains(&hit.doc_id)
                || hit.confidence_score < self.config.expand_threshold
            {
                continue;
            }
            let age_days = now.signed_duration_since(hit.turn_timestamp).num_days();
            if age_days <= self.config.recent_bucket_days {
                high_conf_recent.push(hit);
            } else if age_days <= self.config.mid_bucket_days {
                high_conf_mid.push(hit);
            } else {
                high_conf_far.push(hit);
            }
        }

        let recent_selected = selected
            .iter()
            .filter(|hit| {
                now.signed_duration_since(hit.turn_timestamp).num_days()
                    <= self.config.recent_bucket_days
            })
            .count();
        let mid_selected = selected
            .iter()
            .filter(|hit| {
                let age_days = now.signed_duration_since(hit.turn_timestamp).num_days();
                age_days > self.config.recent_bucket_days && age_days <= self.config.mid_bucket_days
            })
            .count();
        let far_selected = selected
            .len()
            .saturating_sub(recent_selected + mid_selected);

        Self::take_bucket(
            &mut selected,
            &mut seen,
            &mut high_conf_recent,
            self.config.recent_max_quota.saturating_sub(recent_selected),
        );
        Self::take_bucket(
            &mut selected,
            &mut seen,
            &mut high_conf_mid,
            self.config.mid_max_quota.saturating_sub(mid_selected),
        );
        Self::take_bucket(
            &mut selected,
            &mut seen,
            &mut high_conf_far,
            self.config.far_max_quota.saturating_sub(far_selected),
        );

        let mut high_conf_remaining = high_conf_recent
            .into_iter()
            .chain(high_conf_mid)
            .chain(high_conf_far)
            .collect::<Vec<_>>();
        high_conf_remaining.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.turn_timestamp.cmp(&a.turn_timestamp))
        });
        for hit in high_conf_remaining {
            if selected.len() >= self.config.auto_prompt_max_limit {
                break;
            }
            if seen.insert(hit.doc_id) {
                selected.push(hit);
            }
        }

        selected.sort_by(|a, b| b.turn_timestamp.cmp(&a.turn_timestamp));

        ConversationRecallSelection {
            entries: selected
                .into_iter()
                .take(self.config.auto_prompt_max_limit)
                .map(|hit| ConversationRecallPromptEntry {
                    conversation_id: hit.conversation_id,
                    channel: hit.channel,
                    thread_id: hit.thread_id,
                    turn_index: hit.turn_index,
                    turn_timestamp: hit.turn_timestamp,
                    preview_text: hit.preview_text,
                })
                .collect(),
        }
    }

    fn take_bucket(
        selected: &mut Vec<ConversationRecallHit>,
        seen: &mut HashSet<Uuid>,
        bucket: &mut Vec<ConversationRecallHit>,
        count: usize,
    ) {
        bucket.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.turn_timestamp.cmp(&a.turn_timestamp))
        });
        for hit in bucket.iter().take(count).cloned() {
            if seen.insert(hit.doc_id) {
                selected.push(hit);
            }
        }
        let taken_ids = selected
            .iter()
            .map(|hit| hit.doc_id)
            .collect::<HashSet<_>>();
        bucket.retain(|hit| !taken_ids.contains(&hit.doc_id));
    }

    fn build_search_text(user_text: &str, assistant_text: &str) -> String {
        format!("User: {user_text}\nAssistant: {assistant_text}")
    }

    fn build_preview_text(user_text: &str, assistant_text: &str) -> String {
        let user = Self::truncate_preview(user_text, 220);
        let assistant = Self::truncate_preview(assistant_text, 260);
        format!("User: {user}\nAssistant: {assistant}")
    }

    fn truncate_preview(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            return text.trim().to_string();
        }
        let byte_index = text
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(text.len());
        format!("{}...", text[..byte_index].trim())
    }

    fn confidence_score(hit: &ConversationRecallHit, top_score: f32) -> f32 {
        let modality_score = match (hit.fts_rank, hit.vector_rank) {
            (Some(_), Some(_)) => 0.45,
            (Some(_), None) | (None, Some(_)) => 0.18,
            (None, None) => 0.0,
        };

        let min_rank = hit.fts_rank.into_iter().chain(hit.vector_rank).min();
        let rank_score = match min_rank {
            Some(1..=2) => 0.35,
            Some(3..=5) => 0.22,
            Some(6..=10) => 0.12,
            Some(_) | None => 0.0,
        };

        let relative_score = if top_score <= 0.0 {
            0.0
        } else {
            (hit.score / top_score).clamp(0.0, 1.0) * 0.20
        };

        (modality_score + rank_score + relative_score).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "libsql")]
    use crate::db::{ConversationStore, Database};
    use chrono::Duration;

    #[cfg(feature = "libsql")]
    async fn test_manager() -> ConversationRecallManager {
        let backend = crate::db::libsql::LibSqlBackend::new_memory()
            .await
            .expect("memory db");
        backend.run_migrations().await.expect("migrations");
        ConversationRecallManager::new(Arc::new(backend))
    }

    fn make_hit(
        turn_index: usize,
        age_days: i64,
        score: f32,
        confidence_score: f32,
    ) -> ConversationRecallHit {
        let now = Utc::now();
        ConversationRecallHit {
            doc_id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            channel: "desktop".to_string(),
            thread_id: format!("thread-{turn_index}"),
            turn_index,
            user_message_id: Uuid::new_v4(),
            assistant_message_id: Uuid::new_v4(),
            turn_timestamp: now - Duration::days(age_days),
            user_text: format!("user {turn_index}"),
            assistant_text: format!("assistant {turn_index}"),
            preview_text: format!("preview {turn_index}"),
            score,
            fts_rank: Some(turn_index as u32 + 1),
            vector_rank: Some(turn_index as u32 + 1),
            confidence_score,
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn select_for_prompt_uses_time_buckets_before_expansion() {
        let manager = test_manager().await;
        let hits = vec![
            make_hit(0, 1, 0.98, 0.98),
            make_hit(1, 3, 0.94, 0.96),
            make_hit(2, 10, 0.90, 0.91),
            make_hit(3, 40, 0.88, 0.93),
            make_hit(4, 120, 0.86, 0.92),
        ];

        let selection = manager.select_for_prompt(&hits);
        assert_eq!(selection.entries.len(), 5);

        let now = Utc::now();
        let recent = selection
            .entries
            .iter()
            .filter(|entry| {
                now.signed_duration_since(entry.turn_timestamp).num_days()
                    <= manager.config.recent_bucket_days
            })
            .count();
        let mid = selection
            .entries
            .iter()
            .filter(|entry| {
                let age = now.signed_duration_since(entry.turn_timestamp).num_days();
                age > manager.config.recent_bucket_days && age <= manager.config.mid_bucket_days
            })
            .count();
        let far = selection.entries.len() - recent - mid;

        assert!(recent >= manager.config.recent_base_quota);
        assert!(mid >= manager.config.mid_base_quota);
        assert!(far >= manager.config.far_base_quota);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn select_for_prompt_does_not_expand_with_low_confidence_fillers() {
        let manager = test_manager().await;
        let hits = vec![
            make_hit(0, 1, 0.98, 0.60),
            make_hit(1, 2, 0.96, 0.59),
            make_hit(2, 30, 0.94, 0.40),
            make_hit(3, 120, 0.93, 0.35),
            make_hit(4, 9, 0.90, 0.10),
            make_hit(5, 80, 0.89, 0.12),
            make_hit(6, 300, 0.88, 0.11),
        ];

        let selection = manager.select_for_prompt(&hits);
        assert_eq!(
            selection.entries.len(),
            manager.config.auto_prompt_base_limit
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn search_with_preview_falls_back_to_recent_cross_thread_turns() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("conversation_recall_recent_fallback.db");
        let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .expect("db");
        backend.run_migrations().await.expect("migrations");

        let older = backend
            .create_conversation("desktop", "user-1", Some("thread-old"))
            .await
            .expect("older conversation");
        backend
            .add_conversation_message(older, "user", "上次我们讨论了论文计划")
            .await
            .expect("older user");
        backend
            .add_conversation_message(older, "assistant", "我们当时定了论文提纲和时间表。")
            .await
            .expect("older assistant");

        let current = backend
            .create_conversation("desktop", "user-1", Some("thread-current"))
            .await
            .expect("current conversation");
        backend
            .add_conversation_message(current, "user", "这是当前线程")
            .await
            .expect("current user");
        backend
            .add_conversation_message(current, "assistant", "当前线程内容")
            .await
            .expect("current assistant");

        let manager = ConversationRecallManager::new(Arc::new(backend));
        manager.backfill_for_user("user-1").await.expect("backfill");

        let results = manager
            .search_with_preview("user-1", "最近对话内容", 5, Some(current), false)
            .await
            .expect("search with fallback");

        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|(hit, _)| hit.conversation_id != current)
        );
        assert!(
            results
                .iter()
                .any(|(hit, _)| hit.preview_text.contains("论文提纲"))
        );
    }
}
