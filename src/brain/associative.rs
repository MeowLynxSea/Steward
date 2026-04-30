use std::sync::Arc;

use uuid::Uuid;

use crate::config::BrainConfig;
use crate::db::Database;
use crate::error::DatabaseError;
use crate::memory::AssociativeEdge;

/// Manages associative (Hebbian-learned) edges between memory nodes.
#[derive(Clone)]
pub struct AssociativeNetwork {
    db: Arc<dyn Database>,
    config: BrainConfig,
}

impl AssociativeNetwork {
    pub fn new(db: Arc<dyn Database>, config: BrainConfig) -> Self {
        Self { db, config }
    }

    /// Reinforce the associative edge between two nodes that co-activated.
    ///
    /// If the edge doesn't exist, creates it with the delta as initial weight.
    /// If it exists, updates weight using Hebbian averaging + boost.
    pub async fn reinforce_coactivation(
        &self,
        space_id: Uuid,
        source: Uuid,
        target: Uuid,
        evidence: &str,
    ) -> Result<AssociativeEdge, DatabaseError> {
        if source == target {
            return Err(DatabaseError::Query(
                "cannot create associative edge from a node to itself".to_string(),
            ));
        }

        self.db
            .reinforce_associative_edge(
                space_id,
                source,
                target,
                "association",
                self.config.hebbian_delta,
                self.config.hebbian_boost,
                evidence,
            )
            .await
    }

    /// Batch reinforce all co-occurring pairs in a set of activated nodes.
    pub async fn reinforce_all_pairs(
        &self,
        space_id: Uuid,
        node_ids: &[Uuid],
        evidence: &str,
    ) -> Result<Vec<AssociativeEdge>, DatabaseError> {
        let mut results = Vec::new();
        for i in 0..node_ids.len() {
            for j in (i + 1)..node_ids.len() {
                match self
                    .reinforce_coactivation(space_id, node_ids[i], node_ids[j], evidence)
                    .await
                {
                    Ok(edge) => results.push(edge),
                    Err(e) => {
                        tracing::warn!(
                            "failed to reinforce edge {:?} -> {:?}: {}",
                            node_ids[i],
                            node_ids[j],
                            e
                        );
                    }
                }
            }
        }
        Ok(results)
    }

    /// Propagate activation from seed nodes through associative edges.
    ///
    /// Returns (neighbor_node_id, propagated_score) pairs.
    pub async fn propagate_activation(
        &self,
        space_id: Uuid,
        seeds: &[(Uuid, f64)],
        depth: usize,
        decay: f64,
    ) -> Result<Vec<(Uuid, f64)>, DatabaseError> {
        let mut scores: std::collections::HashMap<Uuid, f64> = std::collections::HashMap::new();
        let mut current_seeds = seeds.to_vec();

        for _ in 0..depth.max(1) {
            let mut next_seeds = Vec::new();
            for (node_id, score) in &current_seeds {
                let neighbors = self
                    .db
                    .list_associative_neighbors(space_id, *node_id, 20)
                    .await?;
                for (edge, neighbor_id) in neighbors {
                    let contribution = score * edge.weight * decay;
                    let entry = scores.entry(neighbor_id).or_insert(0.0);
                    *entry += contribution;
                    next_seeds.push((neighbor_id, contribution));
                }
            }
            current_seeds = next_seeds;
        }

        let mut results: Vec<(Uuid, f64)> = scores.into_iter().collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        Ok(results)
    }
}
