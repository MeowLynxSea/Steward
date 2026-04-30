use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::db::Database;
use crate::error::DatabaseError;
use crate::memory::{MemoryEpisode, WmSnapshotEntry};

/// Manages episodic memory traces — time-stamped records of node activations
/// with Working Memory snapshots.
pub struct EpisodicMemory {
    db: Arc<dyn Database>,
}

impl EpisodicMemory {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }

    /// Record an activation episode for a node.
    pub async fn record_activation(
        &self,
        space_id: Uuid,
        node_id: Uuid,
        trigger_uri: Option<&str>,
        trigger_text: Option<&str>,
        wm_snapshot: &[WmSnapshotEntry],
        activation_strength: f64,
    ) -> Result<MemoryEpisode, DatabaseError> {
        self.db
            .create_episode(
                space_id,
                node_id,
                "activation",
                trigger_uri,
                trigger_text,
                wm_snapshot,
                activation_strength,
            )
            .await
    }

    /// Record a conversation episode (user mentions a node naturally).
    pub async fn record_conversation(
        &self,
        space_id: Uuid,
        node_id: Uuid,
        trigger_text: &str,
        wm_snapshot: &[WmSnapshotEntry],
        activation_strength: f64,
    ) -> Result<MemoryEpisode, DatabaseError> {
        self.db
            .create_episode(
                space_id,
                node_id,
                "conversation",
                None,
                Some(trigger_text),
                wm_snapshot,
                activation_strength,
            )
            .await
    }

    /// Record a dream/consolidation episode.
    pub async fn record_dream(
        &self,
        space_id: Uuid,
        node_id: Uuid,
        wm_snapshot: &[WmSnapshotEntry],
        activation_strength: f64,
    ) -> Result<MemoryEpisode, DatabaseError> {
        self.db
            .create_episode(
                space_id,
                node_id,
                "dream",
                None,
                Some("dream consolidation replay"),
                wm_snapshot,
                activation_strength,
            )
            .await
    }

    /// List recent episodes for a node.
    pub async fn list_episodes(
        &self,
        node_id: Uuid,
        limit: usize,
    ) -> Result<Vec<MemoryEpisode>, DatabaseError> {
        self.db.list_episodes(node_id, limit).await
    }

    /// List episodes within a time window (used by DreamEngine for replay).
    pub async fn episodes_in_window(
        &self,
        space_id: Uuid,
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Result<Vec<MemoryEpisode>, DatabaseError> {
        self.db
            .list_recent_episodes(space_id, window_start)
            .await
            .map(|episodes| {
                episodes
                    .into_iter()
                    .filter(|ep| ep.created_at <= window_end)
                    .collect()
            })
    }

    /// Group episodes into time buckets for Hebbian replay.
    ///
    /// Returns groups of node IDs that co-activated within each bucket.
    pub fn group_by_time_window(
        episodes: &[MemoryEpisode],
        bucket_minutes: i64,
    ) -> Vec<Vec<Uuid>> {
        if episodes.is_empty() {
            return Vec::new();
        }

        let mut sorted = episodes.to_vec();
        sorted.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let mut groups: Vec<Vec<Uuid>> = Vec::new();
        let mut current_group: Vec<Uuid> = Vec::new();
        let mut current_start: Option<DateTime<Utc>> = None;

        for ep in sorted {
            match current_start {
                None => {
                    current_start = Some(ep.created_at);
                    current_group.push(ep.node_id);
                }
                Some(start) => {
                    if ep.created_at - start <= Duration::minutes(bucket_minutes) {
                        current_group.push(ep.node_id);
                    } else {
                        if !current_group.is_empty() {
                            groups.push(current_group);
                        }
                        current_group = vec![ep.node_id];
                        current_start = Some(ep.created_at);
                    }
                }
            }
        }

        if !current_group.is_empty() {
            groups.push(current_group);
        }

        groups
    }
}
