//! Workspace-facing wrappers around the shared retrieval fusion core.

use uuid::Uuid;

use crate::retrieval::{FusedItem, RankedItem};
pub use crate::retrieval::{FusionStrategy, SearchConfig};

/// A search result with hybrid scoring.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub document_id: Uuid,
    pub document_path: String,
    pub chunk_id: Uuid,
    pub content: String,
    pub score: f32,
    pub fts_rank: Option<u32>,
    pub vector_rank: Option<u32>,
}

impl SearchResult {
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

/// Raw result from a single workspace retrieval method.
#[derive(Debug, Clone)]
pub struct RankedResult {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub document_path: String,
    pub content: String,
    pub rank: u32,
}

#[derive(Debug, Clone)]
struct WorkspacePayload {
    document_id: Uuid,
    document_path: String,
    content: String,
}

fn to_ranked_items(results: Vec<RankedResult>) -> Vec<RankedItem<WorkspacePayload>> {
    results
        .into_iter()
        .map(|result| RankedItem {
            item_id: result.chunk_id,
            payload: WorkspacePayload {
                document_id: result.document_id,
                document_path: result.document_path,
                content: result.content,
            },
            rank: result.rank,
        })
        .collect()
}

fn from_fused_items(results: Vec<FusedItem<WorkspacePayload>>) -> Vec<SearchResult> {
    results
        .into_iter()
        .map(|result| SearchResult {
            document_id: result.payload.document_id,
            document_path: result.payload.document_path,
            chunk_id: result.item_id,
            content: result.payload.content,
            score: result.score,
            fts_rank: result.fts_rank,
            vector_rank: result.vector_rank,
        })
        .collect()
}

pub fn fuse_results(
    fts_results: Vec<RankedResult>,
    vector_results: Vec<RankedResult>,
    config: &SearchConfig,
) -> Vec<SearchResult> {
    from_fused_items(crate::retrieval::fuse_results(
        to_ranked_items(fts_results),
        to_ranked_items(vector_results),
        config,
    ))
}

pub fn reciprocal_rank_fusion(
    fts_results: Vec<RankedResult>,
    vector_results: Vec<RankedResult>,
    config: &SearchConfig,
) -> Vec<SearchResult> {
    from_fused_items(crate::retrieval::reciprocal_rank_fusion(
        to_ranked_items(fts_results),
        to_ranked_items(vector_results),
        config,
    ))
}

#[allow(dead_code)]
pub fn weighted_score_fusion(
    fts_results: Vec<RankedResult>,
    vector_results: Vec<RankedResult>,
    config: &SearchConfig,
) -> Vec<SearchResult> {
    from_fused_items(crate::retrieval::weighted_score_fusion(
        to_ranked_items(fts_results),
        to_ranked_items(vector_results),
        config,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(chunk_id: Uuid, doc_id: Uuid, rank: u32) -> RankedResult {
        RankedResult {
            chunk_id,
            document_id: doc_id,
            document_path: format!("docs/{}.md", doc_id),
            content: format!("content for chunk {}", chunk_id),
            rank,
        }
    }

    fn make_result_with_path(chunk_id: Uuid, doc_id: Uuid, path: &str, rank: u32) -> RankedResult {
        RankedResult {
            chunk_id,
            document_id: doc_id,
            document_path: path.to_string(),
            content: format!("content for chunk {}", chunk_id),
            rank,
        }
    }

    #[test]
    fn test_rrf_propagates_document_path() {
        let config = SearchConfig::default().with_limit(10);

        let doc_a = Uuid::new_v4();
        let doc_b = Uuid::new_v4();
        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let chunk3 = Uuid::new_v4();

        let fts_results = vec![
            make_result_with_path(chunk1, doc_a, "notes/todo.md", 1),
            make_result_with_path(chunk2, doc_b, "journal/2024-01-15.md", 2),
        ];
        let vector_results = vec![
            make_result_with_path(chunk1, doc_a, "notes/todo.md", 1),
            make_result_with_path(chunk3, doc_b, "journal/2024-01-15.md", 2),
        ];

        let results = reciprocal_rank_fusion(fts_results, vector_results, &config);
        assert_eq!(
            results
                .iter()
                .find(|r| r.chunk_id == chunk1)
                .unwrap()
                .document_path,
            "notes/todo.md"
        );
        assert!(
            results
                .iter()
                .all(|r| Uuid::parse_str(&r.document_path).is_err())
        );
    }

    #[test]
    fn test_rrf_hybrid_match_boosted() {
        let config = SearchConfig::default().with_limit(10);

        let chunk1 = Uuid::new_v4();
        let chunk2 = Uuid::new_v4();
        let chunk3 = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let results = reciprocal_rank_fusion(
            vec![make_result(chunk1, doc, 1), make_result(chunk2, doc, 2)],
            vec![make_result(chunk1, doc, 1), make_result(chunk3, doc, 2)],
            &config,
        );

        assert_eq!(results[0].chunk_id, chunk1);
        assert!(results[0].is_hybrid());
    }

    #[test]
    fn test_weighted_fusion_fts_boost() {
        let config = SearchConfig::default()
            .with_fusion_strategy(FusionStrategy::WeightedScore)
            .with_fts_weight(2.0)
            .with_vector_weight(0.5)
            .with_limit(10);

        let chunk_fts = Uuid::new_v4();
        let chunk_vec = Uuid::new_v4();
        let doc = Uuid::new_v4();

        let results = weighted_score_fusion(
            vec![make_result(chunk_fts, doc, 2)],
            vec![make_result(chunk_vec, doc, 2)],
            &config,
        );

        assert_eq!(results[0].chunk_id, chunk_fts);
        assert!(results[0].from_fts());
    }
}
