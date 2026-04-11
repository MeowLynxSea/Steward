//! Shared retrieval fusion utilities for hybrid search.
//!
//! This module is intentionally payload-agnostic so both workspace search and
//! native memory recall can reuse the same fusion logic while keeping their
//! own result metadata.

use std::collections::HashMap;

use uuid::Uuid;

/// Strategy used to fuse FTS and vector search results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FusionStrategy {
    /// Reciprocal Rank Fusion (default). Ignores `fts_weight`/`vector_weight`.
    #[default]
    Rrf,
    /// Weighted score fusion using normalized rank-derived scores.
    WeightedScore,
}

/// Configuration for hybrid retrieval.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Maximum number of results to return.
    pub limit: usize,
    /// RRF constant (typically 60). Higher values flatten rank differences.
    pub rrf_k: u32,
    /// Whether to include FTS results.
    pub use_fts: bool,
    /// Whether to include vector results.
    pub use_vector: bool,
    /// Minimum score threshold after normalization.
    pub min_score: f32,
    /// Maximum results to fetch from each method before fusion.
    pub pre_fusion_limit: usize,
    /// Fusion strategy to use when combining results.
    pub fusion_strategy: FusionStrategy,
    /// Weight for FTS results in `WeightedScore` fusion.
    pub fts_weight: f32,
    /// Weight for vector results in `WeightedScore` fusion.
    pub vector_weight: f32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            limit: 10,
            rrf_k: 60,
            use_fts: true,
            use_vector: true,
            min_score: 0.0,
            pre_fusion_limit: 50,
            fusion_strategy: FusionStrategy::default(),
            fts_weight: 0.5,
            vector_weight: 0.5,
        }
    }
}

impl SearchConfig {
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    pub fn with_rrf_k(mut self, k: u32) -> Self {
        self.rrf_k = k;
        self
    }

    pub fn vector_only(mut self) -> Self {
        self.use_fts = false;
        self.use_vector = true;
        self
    }

    pub fn fts_only(mut self) -> Self {
        self.use_fts = true;
        self.use_vector = false;
        self
    }

    pub fn with_min_score(mut self, score: f32) -> Self {
        self.min_score = score.clamp(0.0, 1.0);
        self
    }

    pub fn with_fusion_strategy(mut self, strategy: FusionStrategy) -> Self {
        self.fusion_strategy = strategy;
        self
    }

    pub fn with_fts_weight(mut self, weight: f32) -> Self {
        if weight.is_finite() && weight >= 0.0 {
            self.fts_weight = weight;
        }
        self
    }

    pub fn with_vector_weight(mut self, weight: f32) -> Self {
        if weight.is_finite() && weight >= 0.0 {
            self.vector_weight = weight;
        }
        self
    }
}

/// Ranked payload produced by one retrieval method.
#[derive(Debug, Clone)]
pub struct RankedItem<T> {
    pub item_id: Uuid,
    pub payload: T,
    pub rank: u32,
}

/// Payload after hybrid fusion.
#[derive(Debug, Clone)]
pub struct FusedItem<T> {
    pub item_id: Uuid,
    pub payload: T,
    pub score: f32,
    pub fts_rank: Option<u32>,
    pub vector_rank: Option<u32>,
}

impl<T> FusedItem<T> {
    pub fn from_fts(&self) -> bool {
        self.fts_rank.is_some()
    }

    pub fn from_vector(&self) -> bool {
        self.vector_rank.is_some()
    }

    pub fn is_hybrid(&self) -> bool {
        self.fts_rank.is_some() && self.vector_rank.is_some()
    }
}

pub fn fuse_results<T: Clone>(
    fts_results: Vec<RankedItem<T>>,
    vector_results: Vec<RankedItem<T>>,
    config: &SearchConfig,
) -> Vec<FusedItem<T>> {
    match config.fusion_strategy {
        FusionStrategy::Rrf => reciprocal_rank_fusion(fts_results, vector_results, config),
        FusionStrategy::WeightedScore => weighted_score_fusion(fts_results, vector_results, config),
    }
}

pub fn reciprocal_rank_fusion<T: Clone>(
    fts_results: Vec<RankedItem<T>>,
    vector_results: Vec<RankedItem<T>>,
    config: &SearchConfig,
) -> Vec<FusedItem<T>> {
    let k = config.rrf_k as f32;
    let mut scores: HashMap<Uuid, (T, f32, Option<u32>, Option<u32>)> = HashMap::new();

    for result in fts_results {
        let rrf_score = 1.0 / (k + result.rank as f32);
        scores
            .entry(result.item_id)
            .and_modify(|info| {
                info.1 += rrf_score;
                info.2 = Some(result.rank);
            })
            .or_insert((result.payload, rrf_score, Some(result.rank), None));
    }

    for result in vector_results {
        let rrf_score = 1.0 / (k + result.rank as f32);
        scores
            .entry(result.item_id)
            .and_modify(|info| {
                info.1 += rrf_score;
                info.3 = Some(result.rank);
            })
            .or_insert((result.payload, rrf_score, None, Some(result.rank)));
    }

    normalize_sort_and_limit(scores, config)
}

pub fn weighted_score_fusion<T: Clone>(
    fts_results: Vec<RankedItem<T>>,
    vector_results: Vec<RankedItem<T>>,
    config: &SearchConfig,
) -> Vec<FusedItem<T>> {
    let mut scores: HashMap<Uuid, (T, f32, Option<u32>, Option<u32>)> = HashMap::new();

    for result in fts_results {
        let score = config.fts_weight * (1.0 / result.rank as f32);
        scores
            .entry(result.item_id)
            .and_modify(|info| {
                info.1 += score;
                info.2 = Some(result.rank);
            })
            .or_insert((result.payload, score, Some(result.rank), None));
    }

    for result in vector_results {
        let score = config.vector_weight * (1.0 / result.rank as f32);
        scores
            .entry(result.item_id)
            .and_modify(|info| {
                info.1 += score;
                info.3 = Some(result.rank);
            })
            .or_insert((result.payload, score, None, Some(result.rank)));
    }

    normalize_sort_and_limit(scores, config)
}

fn normalize_sort_and_limit<T: Clone>(
    scores: HashMap<Uuid, (T, f32, Option<u32>, Option<u32>)>,
    config: &SearchConfig,
) -> Vec<FusedItem<T>> {
    let mut results: Vec<FusedItem<T>> = scores
        .into_iter()
        .map(|(item_id, info)| FusedItem {
            item_id,
            payload: info.0,
            score: info.1,
            fts_rank: info.2,
            vector_rank: info.3,
        })
        .collect();

    if let Some(max_score) = results.iter().map(|r| r.score).reduce(f32::max)
        && max_score > 0.0
    {
        for result in &mut results {
            result.score /= max_score;
        }
    }

    if config.min_score > 0.0 {
        results.retain(|result| result.score >= config.min_score);
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(config.limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(item_id: Uuid, rank: u32) -> RankedItem<String> {
        RankedItem {
            item_id,
            payload: format!("payload-{item_id}"),
            rank,
        }
    }

    #[test]
    fn rrf_boosts_hybrid_matches() {
        let config = SearchConfig::default().with_limit(10);
        let shared = Uuid::new_v4();
        let only_fts = Uuid::new_v4();
        let only_vec = Uuid::new_v4();

        let results = reciprocal_rank_fusion(
            vec![make_result(shared, 1), make_result(only_fts, 2)],
            vec![make_result(shared, 1), make_result(only_vec, 2)],
            &config,
        );

        assert_eq!(results[0].item_id, shared);
        assert!(results[0].is_hybrid());
    }

    #[test]
    fn weighted_respects_weights() {
        let config = SearchConfig::default()
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_fts_weight(2.0)
            .with_vector_weight(0.5);
        let only_fts = Uuid::new_v4();
        let only_vec = Uuid::new_v4();

        let results = weighted_score_fusion(
            vec![make_result(only_fts, 2)],
            vec![make_result(only_vec, 2)],
            &config,
        );

        assert_eq!(results[0].item_id, only_fts);
    }

    #[test]
    fn config_builders_work() {
        let config = SearchConfig::default()
            .with_limit(20)
            .with_rrf_k(30)
            .with_min_score(0.2)
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_fts_weight(0.7)
            .with_vector_weight(0.3);

        assert_eq!(config.limit, 20);
        assert_eq!(config.rrf_k, 30);
        assert!((config.min_score - 0.2).abs() < 0.001);
        assert_eq!(config.fusion_strategy, FusionStrategy::WeightedScore);
        assert!((config.fts_weight - 0.7).abs() < 0.001);
        assert!((config.vector_weight - 0.3).abs() < 0.001);
    }
}
