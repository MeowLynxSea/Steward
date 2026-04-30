use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::config::BrainConfig;
use crate::db::Database;
use crate::error::DatabaseError;
use crate::memory::{MemorySearchHit, NodeActivation};
use crate::workspace::EmbeddingProvider;

/// Computes composite activation scores for memory nodes.
#[derive(Clone)]
pub struct ActivationEngine {
    db: Arc<dyn Database>,
    config: BrainConfig,
}

impl ActivationEngine {
    pub fn new(db: Arc<dyn Database>, config: BrainConfig) -> Self {
        Self { db, config }
    }

    /// Compute activation scores for all candidate nodes given a user query.
    ///
    /// Returns candidates sorted by activation score descending.
    pub async fn compute_activations(
        &self,
        space_id: Uuid,
        query: &str,
        semantic_hits: &[MemorySearchHit],
        keyword_hits: &[(String, Vec<String>)], // (uri, matched_keywords)
        _embeddings: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Result<Vec<ActivationCandidate>, DatabaseError> {
        let now = Utc::now();

        // Collect all unique node IDs from both semantic and keyword hits
        let mut node_scores: HashMap<Uuid, ActivationCandidate> = HashMap::new();

        // 1. Semantic similarity component (35% default)
        for hit in semantic_hits {
            let semantic_score = if let (Some(fts), Some(vec)) = (hit.fts_rank, hit.vector_rank) {
                // Hybrid match: normalize score
                let hybrid = 1.0 / (1.0 + (fts as f64 + vec as f64) / 2.0);
                hybrid.min(1.0)
            } else if hit.fts_rank.is_some() {
                0.6
            } else if hit.vector_rank.is_some() {
                0.7
            } else {
                0.5
            };

            let candidate = node_scores.entry(hit.node_id).or_insert_with(|| ActivationCandidate {
                node_id: hit.node_id,
                uri: hit.uri.clone(),
                title: hit.title.clone(),
                semantic_score: 0.0,
                keyword_score: 0.0,
                neighbor_score: 0.0,
                recency_score: 0.0,
                baseline_score: 0.0,
                final_score: 0.0,
            });
            candidate.semantic_score = semantic_score.max(candidate.semantic_score);
        }

        // 2. Keyword match component (25% default)
        let query_terms: HashSet<String> = query
            .to_ascii_lowercase()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let query_term_count = query_terms.len().max(1);

        for (uri, matched) in keyword_hits {
            // We need to find the node_id for this URI. For now, use a heuristic:
            // keyword hits are usually produced alongside semantic hits, so the node
            // should already be in node_scores. If not, we'll skip.
            // In practice, `MemoryManager::collect_triggered_recall` returns candidates
            // with node_ids, so this path is mainly for future extensibility.
            let _ = uri;
            let keyword_score = (matched.len() as f64 / query_term_count as f64).min(1.0);
            // Apply to all existing candidates as a global query-level boost
            for candidate in node_scores.values_mut() {
                if matched.iter().any(|kw| candidate.title.to_ascii_lowercase().contains(kw)) {
                    candidate.keyword_score = candidate.keyword_score.max(keyword_score);
                }
            }
        }

        // Also check which semantic hits had matched keywords
        for hit in semantic_hits {
            if !hit.matched_keywords.is_empty() {
                if let Some(candidate) = node_scores.get_mut(&hit.node_id) {
                    let score = (hit.matched_keywords.len() as f64 / query_term_count as f64).min(1.0);
                    candidate.keyword_score = candidate.keyword_score.max(score);
                }
            }
        }

        // 3. Fetch activation states for recency and baseline
        let node_ids: Vec<Uuid> = node_scores.keys().copied().collect();
        let mut activation_map: HashMap<Uuid, NodeActivation> = HashMap::new();
        for node_id in &node_ids {
            if let Some(activation) = self.db.get_node_activation(space_id, *node_id).await? {
                activation_map.insert(*node_id, activation);
            }
        }

        // 4. Neighbor propagation component (20% default)
        // Collect top seeds and propagate activation through associative edges
        let seeds: Vec<(Uuid, f64)> = node_scores
            .values()
            .map(|c| {
                let raw = c.semantic_score * self.config.activation_semantic_weight
                    + c.keyword_score * self.config.activation_keyword_weight;
                (c.node_id, raw)
            })
            .collect();

        let mut propagated: HashMap<Uuid, f64> = HashMap::new();
        for (seed_id, seed_score) in &seeds {
            let neighbors = self
                .db
                .list_associative_neighbors(space_id, *seed_id, 10)
                .await?;
            for (edge, neighbor_id) in neighbors {
                let contribution = seed_score * edge.weight * self.config.neighbor_decay;
                *propagated.entry(neighbor_id).or_insert(0.0) += contribution;
            }
        }

        // Apply propagation scores to candidates
        for candidate in node_scores.values_mut() {
            if let Some(prop) = propagated.get(&candidate.node_id) {
                candidate.neighbor_score = (*prop).min(1.0);
            }
        }

        // 5. Recency and baseline components
        for candidate in node_scores.values_mut() {
            if let Some(activation) = activation_map.get(&candidate.node_id) {
                // Recency: exponential decay with 24h half-life
                if let Some(last_activated) = activation.last_activated_at {
                    let hours_ago = (now - last_activated).num_seconds() as f64 / 3600.0;
                    candidate.recency_score = (-hours_ago / self.config.recency_half_life_hours).exp();
                }
                // Baseline importance
                candidate.baseline_score = activation.baseline_activation.min(1.0);
            }
        }

        // Compose final scores
        let mut results: Vec<ActivationCandidate> = node_scores.into_values().collect();
        for candidate in &mut results {
            candidate.compute_final(&self.config);
        }

        results.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap());
        Ok(results)
    }
}

/// A candidate node with per-component activation scores.
#[derive(Debug, Clone)]
pub struct ActivationCandidate {
    pub node_id: Uuid,
    pub uri: String,
    pub title: String,
    pub semantic_score: f64,
    pub keyword_score: f64,
    pub neighbor_score: f64,
    pub recency_score: f64,
    pub baseline_score: f64,
    pub final_score: f64,
}

impl ActivationCandidate {
    fn compute_final(&mut self, config: &BrainConfig) {
        self.final_score = self.semantic_score * config.activation_semantic_weight
            + self.keyword_score * config.activation_keyword_weight
            + self.neighbor_score * config.activation_neighbor_weight
            + self.recency_score * config.activation_recency_weight
            + self.baseline_score * config.activation_baseline_weight;
        self.final_score = self.final_score.min(1.0);
    }
}
