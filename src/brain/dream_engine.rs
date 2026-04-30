use std::sync::Arc;

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::config::BrainConfig;
use crate::db::Database;
use crate::error::DatabaseError;

use super::associative::AssociativeNetwork;
use super::episodic::EpisodicMemory;

/// Consolidation report returned after a dream cycle.
#[derive(Debug, Clone)]
pub struct DreamReport {
    pub nodes_decayed: usize,
    pub edges_reinforced: usize,
    pub episodes_replayed: usize,
}

/// The "sleep" phase of the brain — periodically consolidates memory.
///
/// 1. Decay all current activations (simulates forgetting without reinforcement)
/// 2. Replay recent episodes and reinforce associative edges (Hebbian learning)
pub struct DreamEngine {
    db: Arc<dyn Database>,
    config: BrainConfig,
    associative: AssociativeNetwork,
    episodic: EpisodicMemory,
}

impl DreamEngine {
    pub fn new(db: Arc<dyn Database>, config: BrainConfig) -> Self {
        let associative = AssociativeNetwork::new(db.clone(), config.clone());
        let episodic = EpisodicMemory::new(db.clone());
        Self {
            db,
            config,
            associative,
            episodic,
        }
    }

    pub async fn run_consolidation(
        &self,
        space_id: Uuid,
    ) -> Result<DreamReport, DatabaseError> {
        let mut report = DreamReport {
            nodes_decayed: 0,
            edges_reinforced: 0,
            episodes_replayed: 0,
        };

        // 1. Decay all current activations
        self.db
            .decay_all_activations(space_id, self.config.dream_decay_factor)
            .await?;
        report.nodes_decayed = self
            .db
            .list_top_activated(space_id, 1)
            .await
            .map(|_| 1)
            .unwrap_or(0);

        // 2. Episode Replay — group by 5-minute windows, reinforce co-activated pairs
        let since = Utc::now() - Duration::hours(24);
        let episodes = self
            .episodic
            .episodes_in_window(space_id, since, Utc::now())
            .await?;

        let groups = EpisodicMemory::group_by_time_window(&episodes, 5);
        for group in &groups {
            if group.len() >= 2 {
                let edges = self
                    .associative
                    .reinforce_all_pairs(space_id, group, "dream_replay")
                    .await?;
                report.edges_reinforced += edges.len();
            }
        }
        report.episodes_replayed = episodes.len();

        Ok(report)
    }
}
