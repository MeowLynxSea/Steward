//! Native graph-based memory system for Steward.
//!
//! This module models agent memory as durable graph entities rather than
//! workspace documents. The workspace remains responsible for mounted files
//! and ad-hoc content indexing; long-term agent memory lives here.

pub(crate) mod search_terms;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use aho_corasick::AhoCorasickBuilder;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::MemoryRecallConfig;
use crate::db::Database;
use crate::error::DatabaseError;
use crate::retrieval::{FusedItem, RankedItem, SearchConfig};
use crate::workspace::EmbeddingProvider;

pub const PRIMARY_SPACE_SLUG: &str = "primary";
const NOCTURNE_NATIVE_SYSTEM_PROMPT: &str = include_str!("nocturne_system_prompt_native.md");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryNodeKind {
    Boot,
    Identity,
    Value,
    UserProfile,
    Directive,
    Curated,
    Episode,
    Procedure,
    Reference,
}

impl MemoryNodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::Identity => "identity",
            Self::Value => "value",
            Self::UserProfile => "user_profile",
            Self::Directive => "directive",
            Self::Curated => "curated",
            Self::Episode => "episode",
            Self::Procedure => "procedure",
            Self::Reference => "reference",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "boot" => Self::Boot,
            "identity" => Self::Identity,
            "value" => Self::Value,
            "user_profile" => Self::UserProfile,
            "directive" => Self::Directive,
            "curated" => Self::Curated,
            "episode" => Self::Episode,
            "procedure" => Self::Procedure,
            _ => Self::Reference,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryVersionStatus {
    Active,
    Deprecated,
    Orphaned,
}

impl MemoryVersionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Orphaned => "orphaned",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "deprecated" => Self::Deprecated,
            "orphaned" => Self::Orphaned,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRelationKind {
    Contains,
    RelatesTo,
    Timeline,
    Trigger,
}

impl MemoryRelationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::RelatesTo => "relates_to",
            Self::Timeline => "timeline",
            Self::Trigger => "trigger",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "relates_to" => Self::RelatesTo,
            "timeline" => Self::Timeline,
            "trigger" => Self::Trigger,
            _ => Self::Contains,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryVisibility {
    Private,
    Session,
    Shared,
}

impl MemoryVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Session => "session",
            Self::Shared => "shared",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "shared" => Self::Shared,
            "session" => Self::Session,
            _ => Self::Private,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySpace {
    pub id: Uuid,
    pub owner_id: String,
    pub agent_id: Option<Uuid>,
    pub slug: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryNode {
    pub id: Uuid,
    pub space_id: Uuid,
    pub kind: MemoryNodeKind,
    pub title: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryVersion {
    pub id: Uuid,
    pub node_id: Uuid,
    pub supersedes_version_id: Option<Uuid>,
    pub status: MemoryVersionStatus,
    pub content: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEdge {
    pub id: Uuid,
    pub space_id: Uuid,
    pub parent_node_id: Option<Uuid>,
    pub child_node_id: Uuid,
    pub relation_kind: MemoryRelationKind,
    pub visibility: MemoryVisibility,
    pub priority: i32,
    pub trigger_text: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRoute {
    pub id: Uuid,
    pub space_id: Uuid,
    pub edge_id: Option<Uuid>,
    pub node_id: Uuid,
    pub domain: String,
    pub path: String,
    pub is_primary: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MemoryRoute {
    pub fn uri(&self) -> String {
        format!("{}://{}", self.domain, self.path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryKeyword {
    pub id: Uuid,
    pub space_id: Uuid,
    pub node_id: Uuid,
    pub keyword: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChangeSet {
    pub id: Uuid,
    pub space_id: Uuid,
    pub origin: String,
    pub summary: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChangeSetRow {
    pub id: Uuid,
    pub changeset_id: Uuid,
    pub node_id: Option<Uuid>,
    pub route_id: Option<Uuid>,
    pub operation: String,
    pub before_json: serde_json::Value,
    pub after_json: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchDoc {
    pub route_id: Uuid,
    pub space_id: Uuid,
    pub node_id: Uuid,
    pub version_id: Uuid,
    pub uri: String,
    pub title: String,
    pub kind: MemoryNodeKind,
    pub content: String,
    pub trigger_text: Option<String>,
    pub keywords: Vec<String>,
    pub search_terms: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchHit {
    pub node_id: Uuid,
    pub route_id: Uuid,
    pub version_id: Uuid,
    pub uri: String,
    pub title: String,
    pub kind: MemoryNodeKind,
    pub content_snippet: String,
    pub priority: i32,
    pub trigger_text: Option<String>,
    pub score: f32,
    pub fts_rank: Option<u32>,
    pub vector_rank: Option<u32>,
    pub matched_keywords: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryNodeDetail {
    pub node: MemoryNode,
    pub active_version: MemoryVersion,
    pub primary_route: Option<MemoryRoute>,
    #[serde(default)]
    pub selected_route: Option<MemoryRoute>,
    #[serde(default)]
    pub selected_uri: Option<String>,
    pub routes: Vec<MemoryRoute>,
    pub edges: Vec<MemoryEdge>,
    pub keywords: Vec<MemoryKeyword>,
    pub related_nodes: Vec<MemorySearchHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySidebarItem {
    pub node_id: Uuid,
    pub route_id: Option<Uuid>,
    pub uri: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub kind: MemoryNodeKind,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySidebarSection {
    pub key: String,
    pub title: String,
    pub items: Vec<MemorySidebarItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryTimelineEntry {
    pub node_id: Uuid,
    pub route_id: Option<Uuid>,
    pub uri: Option<String>,
    pub title: String,
    pub content_snippet: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryIndexEntry {
    pub uri: String,
    pub title: String,
    pub kind: MemoryNodeKind,
    pub priority: i32,
    pub disclosure: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChildEntry {
    pub uri: String,
    pub title: String,
    pub kind: MemoryNodeKind,
    pub priority: i32,
    pub disclosure: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryGlossaryEntry {
    pub keyword: String,
    pub uris: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MemoryRecallCandidate {
    pub hit: MemorySearchHit,
    pub source: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct MemoryRecallPlan {
    pub boot: Vec<MemoryNodeDetail>,
    pub triggered: Vec<MemoryRecallCandidate>,
    pub relevant: Vec<MemoryRecallCandidate>,
    pub expanded: Vec<MemoryRecallCandidate>,
    pub recent: Vec<MemoryTimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallExplanation {
    pub query: String,
    pub boot: Vec<MemoryRecallExplanationEntry>,
    pub triggered: Vec<MemoryRecallExplanationEntry>,
    pub relevant: Vec<MemoryRecallExplanationEntry>,
    pub expanded: Vec<MemoryRecallExplanationEntry>,
    pub recent: Vec<MemoryTimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallExplanationEntry {
    pub node_id: Uuid,
    pub route_id: Uuid,
    pub uri: String,
    pub title: String,
    pub source: String,
    pub reason: String,
    pub score: Option<f32>,
    pub fts_rank: Option<u32>,
    pub vector_rank: Option<u32>,
    pub matched_keywords: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NewMemoryNodeInput {
    pub space_id: Uuid,
    pub parent_node_id: Option<Uuid>,
    pub domain: String,
    pub path: String,
    pub title: String,
    pub kind: MemoryNodeKind,
    pub content: String,
    pub relation_kind: MemoryRelationKind,
    pub visibility: MemoryVisibility,
    pub priority: i32,
    pub trigger_text: Option<String>,
    pub metadata: serde_json::Value,
    pub keywords: Vec<String>,
    pub changeset_id: Option<Uuid>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateMemoryNodeInput {
    pub route_or_node: String,
    pub title: Option<String>,
    pub kind: Option<MemoryNodeKind>,
    pub content: Option<String>,
    pub priority: Option<i32>,
    pub trigger_text: Option<Option<String>>,
    pub visibility: Option<MemoryVisibility>,
    pub metadata: Option<serde_json::Value>,
    pub keywords: Option<Vec<String>>,
    pub changeset_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct CreateMemoryAliasInput {
    pub space_id: Uuid,
    pub target_route_or_node: String,
    pub domain: String,
    pub path: String,
    pub visibility: MemoryVisibility,
    pub priority: i32,
    pub trigger_text: Option<String>,
    pub changeset_id: Option<Uuid>,
}

#[derive(Clone)]
pub struct MemoryManager {
    db: Arc<dyn Database>,
    embeddings: Arc<RwLock<Option<Arc<dyn EmbeddingProvider>>>>,
    recall_config: MemoryRecallConfig,
}

impl MemoryManager {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self {
            db,
            embeddings: Arc::new(RwLock::new(None)),
            recall_config: MemoryRecallConfig::default(),
        }
    }

    pub fn with_embeddings(self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.set_embeddings(Some(provider));
        self
    }

    pub fn with_recall_config(mut self, config: MemoryRecallConfig) -> Self {
        self.recall_config = config;
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
            rrf_k: self.recall_config.rrf_k,
            use_fts: true,
            use_vector: self.current_embeddings().is_some(),
            min_score: 0.0,
            pre_fusion_limit: self.recall_config.pre_fusion_limit.max(limit),
            fusion_strategy: self.recall_config.fusion_strategy,
            fts_weight: self.recall_config.fts_weight,
            vector_weight: self.recall_config.vector_weight,
        }
    }

    fn recall_allows_kind(&self, kind: MemoryNodeKind, is_group_chat: bool) -> bool {
        !is_group_chat
            || !matches!(
                kind,
                MemoryNodeKind::Curated | MemoryNodeKind::UserProfile | MemoryNodeKind::Episode
            )
    }

    fn candidate_from_hit(
        &self,
        hit: MemorySearchHit,
        source: impl Into<String>,
        reason: impl Into<String>,
    ) -> MemoryRecallCandidate {
        MemoryRecallCandidate {
            hit,
            source: source.into(),
            reason: reason.into(),
        }
    }

    fn explanation_entry(candidate: &MemoryRecallCandidate) -> MemoryRecallExplanationEntry {
        MemoryRecallExplanationEntry {
            node_id: candidate.hit.node_id,
            route_id: candidate.hit.route_id,
            uri: candidate.hit.uri.clone(),
            title: candidate.hit.title.clone(),
            source: candidate.source.clone(),
            reason: candidate.reason.clone(),
            score: Some(candidate.hit.score),
            fts_rank: candidate.hit.fts_rank,
            vector_rank: candidate.hit.vector_rank,
            matched_keywords: candidate.hit.matched_keywords.clone(),
        }
    }

    fn boot_explanation_entry(detail: &MemoryNodeDetail) -> MemoryRecallExplanationEntry {
        let route = detail
            .selected_route
            .as_ref()
            .or(detail.primary_route.as_ref())
            .or_else(|| detail.routes.first());
        MemoryRecallExplanationEntry {
            node_id: detail.node.id,
            route_id: route.map(|route| route.id).unwrap_or_default(),
            uri: route.map(MemoryRoute::uri).unwrap_or_default(),
            title: detail.node.title.clone(),
            source: "boot".to_string(),
            reason: "Explicit boot membership".to_string(),
            score: None,
            fts_rank: None,
            vector_rank: None,
            matched_keywords: Vec::new(),
        }
    }

    fn format_search_doc_for_embedding(doc: &MemorySearchDoc) -> String {
        let mut parts = vec![format!("URI: {}", doc.uri), format!("Title: {}", doc.title)];
        if let Some(trigger) = &doc.trigger_text
            && !trigger.trim().is_empty()
        {
            parts.push(format!("Disclosure: {trigger}"));
        }
        if !doc.keywords.is_empty() {
            parts.push(format!("Keywords: {}", doc.keywords.join(", ")));
        }
        parts.push(doc.content.clone());
        parts.join("\n")
    }

    async fn backfill_embeddings_for_space(
        &self,
        space_id: Uuid,
        limit: usize,
    ) -> Result<usize, DatabaseError> {
        let Some(provider) = self.current_embeddings() else {
            return Ok(0);
        };
        let docs = self
            .db
            .list_memory_search_docs_without_embeddings(space_id, limit)
            .await?;
        if docs.is_empty() {
            return Ok(0);
        }

        let texts = docs
            .iter()
            .map(Self::format_search_doc_for_embedding)
            .collect::<Vec<_>>();
        let embeddings = provider
            .embed_batch(&texts)
            .await
            .map_err(|e| DatabaseError::Query(format!("memory embedding backfill failed: {e}")))?;

        let mut updated = 0usize;
        for (doc, embedding) in docs.iter().zip(embeddings.iter()) {
            self.db
                .update_memory_search_doc_embedding(doc.route_id, embedding)
                .await?;
            updated += 1;
        }
        Ok(updated)
    }

    pub async fn backfill_embeddings(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<usize, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.backfill_embeddings_for_space(space.id, limit).await
    }

    async fn hybrid_search_space(
        &self,
        space_id: Uuid,
        query: &str,
        limit: usize,
        domains: &[String],
    ) -> Result<Vec<MemorySearchHit>, DatabaseError> {
        let config = self.retrieval_config(limit);
        let fts_hits = self
            .db
            .search_memory_graph(space_id, query, config.pre_fusion_limit, domains)
            .await?;

        if self.current_embeddings().is_none() {
            return Ok(fts_hits.into_iter().take(limit).collect());
        }

        let _ = self
            .backfill_embeddings_for_space(space_id, config.pre_fusion_limit)
            .await;
        let provider = self.current_embeddings().expect("checked above");
        let query_embedding = match provider.embed(query).await {
            Ok(embedding) => embedding,
            Err(error) => {
                tracing::warn!("native memory query embedding failed: {}", error);
                return Ok(fts_hits.into_iter().take(limit).collect());
            }
        };

        let vector_hits = match self
            .db
            .vector_search_memory_graph(
                space_id,
                &query_embedding,
                config.pre_fusion_limit,
                domains,
            )
            .await
        {
            Ok(hits) => hits,
            Err(error) => {
                tracing::warn!("native memory vector search failed: {}", error);
                return Ok(fts_hits.into_iter().take(limit).collect());
            }
        };

        if vector_hits.is_empty() {
            return Ok(fts_hits.into_iter().take(limit).collect());
        }

        Ok(Self::fuse_memory_hits(fts_hits, vector_hits, &config))
    }

    fn fuse_memory_hits(
        fts_hits: Vec<MemorySearchHit>,
        vector_hits: Vec<MemorySearchHit>,
        config: &SearchConfig,
    ) -> Vec<MemorySearchHit> {
        let fts_ranked = fts_hits
            .into_iter()
            .enumerate()
            .map(|(index, hit)| RankedItem {
                item_id: hit.route_id,
                payload: hit,
                rank: (index + 1) as u32,
            })
            .collect::<Vec<_>>();
        let vector_ranked = vector_hits
            .into_iter()
            .enumerate()
            .map(|(index, hit)| RankedItem {
                item_id: hit.route_id,
                payload: hit,
                rank: (index + 1) as u32,
            })
            .collect::<Vec<_>>();

        Self::map_fused_hits(crate::retrieval::fuse_results(
            fts_ranked,
            vector_ranked,
            config,
        ))
    }

    fn map_fused_hits(results: Vec<FusedItem<MemorySearchHit>>) -> Vec<MemorySearchHit> {
        results
            .into_iter()
            .map(|result| {
                let mut hit = result.payload;
                hit.score = result.score;
                hit.fts_rank = result.fts_rank;
                hit.vector_rank = result.vector_rank;
                hit
            })
            .collect()
    }

    async fn collect_triggered_recall(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        space_id: Uuid,
        user_input: &str,
        is_group_chat: bool,
    ) -> Result<Vec<MemoryRecallCandidate>, DatabaseError> {
        let glossary = self.db.list_memory_glossary(space_id).await?;
        let index = self.db.list_memory_index(space_id, None).await?;
        let mut keyword_hits: HashMap<String, Vec<String>> = HashMap::new();
        let mut disclosure_hits: HashMap<String, String> = HashMap::new();

        let keywords = glossary
            .iter()
            .map(|entry| entry.keyword.as_str())
            .collect::<Vec<_>>();
        if !keywords.is_empty() {
            let matcher = AhoCorasickBuilder::new()
                .ascii_case_insensitive(true)
                .build(&keywords)
                .map_err(|error| {
                    DatabaseError::Query(format!("trigger matcher failed: {error}"))
                })?;

            for matched in matcher.find_iter(user_input) {
                let keyword = glossary[matched.pattern().as_usize()].keyword.clone();
                let uris = &glossary[matched.pattern().as_usize()].uris;
                for uri in uris {
                    keyword_hits
                        .entry(uri.clone())
                        .or_default()
                        .push(keyword.clone());
                }
            }
        }

        let disclosures = index
            .iter()
            .filter_map(|entry| {
                entry
                    .disclosure
                    .as_ref()
                    .filter(|text| !text.trim().is_empty())
                    .map(|text| (entry.uri.clone(), text.clone()))
            })
            .collect::<Vec<_>>();
        if !disclosures.is_empty() {
            let patterns = disclosures
                .iter()
                .map(|(_, disclosure)| disclosure.as_str())
                .collect::<Vec<_>>();
            let matcher = AhoCorasickBuilder::new()
                .ascii_case_insensitive(true)
                .build(&patterns)
                .map_err(|error| {
                    DatabaseError::Query(format!("disclosure matcher failed: {error}"))
                })?;
            for matched in matcher.find_iter(user_input) {
                let (uri, disclosure) = &disclosures[matched.pattern().as_usize()];
                disclosure_hits.insert(uri.clone(), disclosure.clone());
            }
        }

        let mut candidates = Vec::new();
        let mut seen_nodes = HashSet::new();
        for uri in keyword_hits
            .keys()
            .chain(disclosure_hits.keys())
            .cloned()
            .collect::<Vec<_>>()
        {
            let Some(detail) = self.get_node(owner_id, agent_id, &uri).await? else {
                continue;
            };
            if !self.recall_allows_kind(detail.node.kind, is_group_chat)
                || !seen_nodes.insert(detail.node.id)
            {
                continue;
            }

            let route = detail
                .selected_route
                .as_ref()
                .or(detail.primary_route.as_ref())
                .or_else(|| detail.routes.first());
            let Some(route) = route else {
                continue;
            };

            let keywords = keyword_hits.get(&uri).cloned().unwrap_or_default();
            let reason = if let Some(disclosure) = disclosure_hits.get(&uri) {
                format!("Disclosure matched user input: {disclosure}")
            } else if !keywords.is_empty() {
                format!("Matched glossary keywords: {}", keywords.join(", "))
            } else {
                "Triggered recall".to_string()
            };

            candidates.push(
                self.candidate_from_hit(
                    MemorySearchHit {
                        node_id: detail.node.id,
                        route_id: route.id,
                        version_id: detail.active_version.id,
                        uri: route.uri(),
                        title: detail.node.title.clone(),
                        kind: detail.node.kind,
                        content_snippet: detail.active_version.content.clone(),
                        priority: detail
                            .edges
                            .iter()
                            .find(|edge| Some(edge.id) == route.edge_id)
                            .map(|edge| edge.priority)
                            .unwrap_or(100),
                        trigger_text: detail
                            .edges
                            .iter()
                            .find(|edge| Some(edge.id) == route.edge_id)
                            .and_then(|edge| edge.trigger_text.clone()),
                        score: 1.0,
                        fts_rank: None,
                        vector_rank: None,
                        matched_keywords: keywords,
                        updated_at: detail.node.updated_at,
                    },
                    "trigger",
                    reason,
                ),
            );
        }

        candidates.sort_by(|a, b| {
            a.hit
                .priority
                .cmp(&b.hit.priority)
                .then_with(|| b.hit.updated_at.cmp(&a.hit.updated_at))
        });
        candidates.truncate(self.recall_config.trigger_limit);
        Ok(candidates)
    }

    async fn expand_recall_graph(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        seeds: &[MemoryRecallCandidate],
        is_group_chat: bool,
    ) -> Result<Vec<MemoryRecallCandidate>, DatabaseError> {
        let mut expanded = Vec::new();
        let mut seen_nodes = seeds
            .iter()
            .map(|seed| seed.hit.node_id)
            .collect::<HashSet<_>>();

        for seed in seeds.iter().take(3) {
            let Some(detail) = self.get_node(owner_id, agent_id, &seed.hit.uri).await? else {
                continue;
            };

            if let Some(primary_route) = detail.primary_route.as_ref()
                && seen_nodes.insert(detail.node.id)
                && self.recall_allows_kind(detail.node.kind, is_group_chat)
            {
                expanded.push(
                    self.candidate_from_hit(
                        MemorySearchHit {
                            node_id: detail.node.id,
                            route_id: primary_route.id,
                            version_id: detail.active_version.id,
                            uri: primary_route.uri(),
                            title: detail.node.title.clone(),
                            kind: detail.node.kind,
                            content_snippet: detail.active_version.content.clone(),
                            priority: detail
                                .edges
                                .iter()
                                .find(|edge| Some(edge.id) == primary_route.edge_id)
                                .map(|edge| edge.priority)
                                .unwrap_or(100),
                            trigger_text: detail
                                .edges
                                .iter()
                                .find(|edge| Some(edge.id) == primary_route.edge_id)
                                .and_then(|edge| edge.trigger_text.clone()),
                            score: seed.hit.score,
                            fts_rank: seed.hit.fts_rank,
                            vector_rank: seed.hit.vector_rank,
                            matched_keywords: seed.hit.matched_keywords.clone(),
                            updated_at: detail.node.updated_at,
                        },
                        "graph_primary",
                        format!("Primary route for seed {}", seed.hit.uri),
                    ),
                );
            }

            let Some(parent_edge) = detail
                .edges
                .iter()
                .find(|edge| edge.child_node_id == detail.node.id)
            else {
                continue;
            };
            if let Some(parent_id) = parent_edge.parent_node_id
                && let Some(parent_detail) = self
                    .get_node(owner_id, agent_id, &parent_id.to_string())
                    .await?
                && seen_nodes.insert(parent_detail.node.id)
                && self.recall_allows_kind(parent_detail.node.kind, is_group_chat)
                && let Some(parent_route) = parent_detail
                    .primary_route
                    .as_ref()
                    .or_else(|| parent_detail.routes.first())
            {
                expanded.push(
                    self.candidate_from_hit(
                        MemorySearchHit {
                            node_id: parent_detail.node.id,
                            route_id: parent_route.id,
                            version_id: parent_detail.active_version.id,
                            uri: parent_route.uri(),
                            title: parent_detail.node.title.clone(),
                            kind: parent_detail.node.kind,
                            content_snippet: parent_detail.active_version.content.clone(),
                            priority: parent_detail
                                .edges
                                .iter()
                                .find(|edge| Some(edge.id) == parent_route.edge_id)
                                .map(|edge| edge.priority)
                                .unwrap_or(100),
                            trigger_text: parent_detail
                                .edges
                                .iter()
                                .find(|edge| Some(edge.id) == parent_route.edge_id)
                                .and_then(|edge| edge.trigger_text.clone()),
                            score: seed.hit.score,
                            fts_rank: None,
                            vector_rank: None,
                            matched_keywords: Vec::new(),
                            updated_at: parent_detail.node.updated_at,
                        },
                        "graph_parent",
                        format!("Direct parent of seed {}", seed.hit.uri),
                    ),
                );
            }

            let children = self
                .children(owner_id, agent_id, &seed.hit.uri, 2)
                .await?
                .into_iter()
                .take(2)
                .collect::<Vec<_>>();
            for child in children {
                let Some(child_detail) = self.get_node(owner_id, agent_id, &child.uri).await?
                else {
                    continue;
                };
                if !seen_nodes.insert(child_detail.node.id)
                    || !self.recall_allows_kind(child_detail.node.kind, is_group_chat)
                {
                    continue;
                }
                let route = child_detail
                    .primary_route
                    .as_ref()
                    .or_else(|| child_detail.routes.first());
                let Some(route) = route else {
                    continue;
                };
                expanded.push(self.candidate_from_hit(
                    MemorySearchHit {
                        node_id: child_detail.node.id,
                        route_id: route.id,
                        version_id: child_detail.active_version.id,
                        uri: route.uri(),
                        title: child_detail.node.title.clone(),
                        kind: child_detail.node.kind,
                        content_snippet: child_detail.active_version.content.clone(),
                        priority: child.priority,
                        trigger_text: child.disclosure.clone(),
                        score: seed.hit.score,
                        fts_rank: None,
                        vector_rank: None,
                        matched_keywords: Vec::new(),
                        updated_at: child_detail.node.updated_at,
                    },
                    "graph_child",
                    format!("High-priority child of seed {}", seed.hit.uri),
                ));
            }
        }

        expanded.sort_by(|a, b| {
            a.hit
                .priority
                .cmp(&b.hit.priority)
                .then_with(|| b.hit.updated_at.cmp(&a.hit.updated_at))
        });
        expanded.truncate(self.recall_config.expansion_limit);
        Ok(expanded)
    }

    async fn build_recall_plan(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        user_input: &str,
        is_group_chat: bool,
    ) -> Result<MemoryRecallPlan, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let mut boot = self
            .db
            .list_memory_boot_nodes(
                space.id,
                if is_group_chat {
                    Some(MemoryVisibility::Session)
                } else {
                    None
                },
            )
            .await?;
        boot.truncate(self.recall_config.boot_limit);

        let triggered = self
            .collect_triggered_recall(owner_id, agent_id, space.id, user_input, is_group_chat)
            .await?;
        let boot_node_ids = boot
            .iter()
            .map(|detail| detail.node.id)
            .collect::<HashSet<_>>();
        let triggered_node_ids = triggered
            .iter()
            .map(|candidate| candidate.hit.node_id)
            .collect::<HashSet<_>>();

        let mut relevant = self
            .hybrid_search_space(space.id, user_input, self.recall_config.seed_limit, &[])
            .await?
            .into_iter()
            .filter(|hit| {
                self.recall_allows_kind(hit.kind, is_group_chat)
                    && !boot_node_ids.contains(&hit.node_id)
                    && !triggered_node_ids.contains(&hit.node_id)
            })
            .map(|hit| {
                let reason = match (hit.fts_rank, hit.vector_rank) {
                    (Some(fts), Some(vector)) => {
                        format!("Hybrid recall hit (fts #{fts}, vector #{vector})")
                    }
                    (Some(fts), None) => format!("Lexical recall hit (fts #{fts})"),
                    (None, Some(vector)) => format!("Semantic recall hit (vector #{vector})"),
                    (None, None) => "Recall hit".to_string(),
                };
                self.candidate_from_hit(hit, "search", reason)
            })
            .collect::<Vec<_>>();
        relevant.truncate(self.recall_config.seed_limit);

        let existing_node_ids = boot_node_ids
            .iter()
            .copied()
            .chain(triggered_node_ids.iter().copied())
            .chain(relevant.iter().map(|candidate| candidate.hit.node_id))
            .collect::<HashSet<_>>();
        let expanded = self
            .expand_recall_graph(owner_id, agent_id, &relevant, is_group_chat)
            .await?
            .into_iter()
            .filter(|candidate| !existing_node_ids.contains(&candidate.hit.node_id))
            .collect::<Vec<_>>();
        let recent = self
            .db
            .list_memory_timeline(space.id, self.recall_config.recent_limit)
            .await?;

        Ok(MemoryRecallPlan {
            boot,
            triggered,
            relevant,
            expanded,
            recent,
        })
    }

    pub async fn ensure_primary_space(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<MemorySpace, DatabaseError> {
        self.db
            .ensure_memory_space(owner_id, agent_id, PRIMARY_SPACE_SLUG, "Primary Memory")
            .await
    }

    pub async fn import_legacy_workspace(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<(), DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let existing = self
            .db
            .list_memory_sidebar(space.id, 1)
            .await?
            .into_iter()
            .flat_map(|section| section.items)
            .count();
        if existing > 0 {
            return Ok(());
        }

        let docs = self
            .db
            .list_documents(owner_id, agent_id)
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if docs.is_empty() {
            return Ok(());
        }

        let changeset = self
            .db
            .create_memory_changeset(
                space.id,
                "migration",
                Some("Imported legacy workspace memory documents"),
            )
            .await?;

        let mut known_routes = HashSet::new();
        for doc in docs {
            let Some(import_plan) = legacy_import_plan(&doc.path, &doc.content) else {
                continue;
            };
            if !known_routes.insert((import_plan.domain.clone(), import_plan.path.clone())) {
                continue;
            }
            let _ = self
                .db
                .create_memory_node(&NewMemoryNodeInput {
                    space_id: space.id,
                    parent_node_id: None,
                    domain: import_plan.domain,
                    path: import_plan.path,
                    title: import_plan.title,
                    kind: import_plan.kind,
                    content: doc.content,
                    relation_kind: import_plan.relation_kind,
                    visibility: import_plan.visibility,
                    priority: import_plan.priority,
                    trigger_text: import_plan.trigger_text,
                    metadata: serde_json::json!({
                        "legacy_path": doc.path,
                        "imported_at": Utc::now(),
                    }),
                    keywords: import_plan.keywords,
                    changeset_id: Some(changeset.id),
                })
                .await?;
        }

        self.db
            .complete_memory_changeset(changeset.id, "applied")
            .await?;
        Ok(())
    }

    /// Ensure a minimal Boot node exists that documents the memory operating protocol.
    ///
    /// This is the smallest "system core" for Nocturne-style behavior without
    /// seeding a fixed identity tree (identity/value/user_profile, etc.).
    pub async fn ensure_boot_protocol(&self, owner_id: &str) -> Result<(), DatabaseError> {
        let _space = self.ensure_primary_space(owner_id, None).await?;
        let existing = self
            .get_node(owner_id, None, "system://boot/memory_protocol")
            .await?;

        let protocol = NOCTURNE_NATIVE_SYSTEM_PROMPT.trim();

        if let Some(existing) = existing {
            // If this node was seeded by us, keep it updated across versions.
            // Never overwrite user-modified boot protocol content.
            let seeded = existing
                .node
                .metadata
                .get("source")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s == "seed:memory_protocol");

            if seeded && existing.active_version.content.trim() != protocol {
                let input = UpdateMemoryNodeInput {
                    route_or_node: "system://boot/memory_protocol".to_string(),
                    content: Some(protocol.to_string()),
                    metadata: Some(serde_json::json!({
                        "source": "seed:memory_protocol",
                        "updated_at": Utc::now(),
                    })),
                    ..Default::default()
                };
                let _ = self.update(owner_id, None, &input).await?;
            }
            let _ = self
                .add_to_boot(owner_id, None, "system://boot/memory_protocol", 0)
                .await;
            return Ok(());
        }

        // Use the regular create path so we get the same durable invariants
        // (routes, versions, edges, search projections).
        let (_detail, _changeset) = self
            .create(
                owner_id,
                None,
                // `system://boot` is a virtual entry point; the boot node itself
                // lives at `system://boot/memory_protocol` as a normal route.
                Some("system://"),
                "记忆系统 (The Native Memory System)",
                MemoryNodeKind::Boot,
                protocol,
                "system",
                "boot/memory_protocol",
                0,
                Some("当这是一个新会话开始时，或我感觉自己不再像自己时".to_string()),
                MemoryVisibility::Session,
                vec![
                    "memory".to_string(),
                    "boot".to_string(),
                    "protocol".to_string(),
                    "disclosure".to_string(),
                ],
                serde_json::json!({
                    "source": "seed:memory_protocol",
                }),
            )
            .await?;
        let _ = self
            .add_to_boot(owner_id, None, "system://boot/memory_protocol", 0)
            .await;

        Ok(())
    }

    pub async fn list_sidebar(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemorySidebarSection>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db.list_memory_sidebar(space.id, 8).await
    }

    pub async fn get_node(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
    ) -> Result<Option<MemoryNodeDetail>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db.get_memory_node(space.id, route_or_node).await
    }

    pub async fn search(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        limit: usize,
        domains: &[String],
    ) -> Result<Vec<MemorySearchHit>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.hybrid_search_space(space.id, query, limit, domains)
            .await
    }

    pub async fn list_timeline(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryTimelineEntry>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db.list_memory_timeline(space.id, limit).await
    }

    pub async fn list_index(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        domain: Option<&str>,
    ) -> Result<Vec<MemoryIndexEntry>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db.list_memory_index(space.id, domain).await
    }

    pub async fn list_recent(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
        domain: Option<&str>,
    ) -> Result<Vec<MemoryIndexEntry>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db.list_memory_recent(space.id, limit, domain).await
    }

    pub async fn glossary(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryGlossaryEntry>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db.list_memory_glossary(space.id).await
    }

    pub async fn children(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
        limit: usize,
    ) -> Result<Vec<MemoryChildEntry>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let Some(detail) = self.db.get_memory_node(space.id, route_or_node).await? else {
            return Ok(Vec::new());
        };
        self.db
            .list_memory_children(space.id, detail.node.id, limit)
            .await
    }

    pub async fn list_review_changesets(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryChangeSet>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db.list_memory_reviews(space.id).await
    }

    pub async fn boot_set(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        max_visibility: Option<MemoryVisibility>,
    ) -> Result<Vec<MemoryNodeDetail>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db
            .list_memory_boot_nodes(space.id, max_visibility)
            .await
    }

    pub async fn add_to_boot(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
        load_priority: i32,
    ) -> Result<MemoryRoute, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db
            .upsert_memory_boot_route(space.id, route_or_node, load_priority)
            .await
    }

    pub async fn remove_from_boot(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
    ) -> Result<(), DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db
            .delete_memory_boot_route(space.id, route_or_node)
            .await
    }

    pub async fn manage_triggers(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
        add: &[String],
        remove: &[String],
        disclosure: Option<Option<String>>,
    ) -> Result<MemoryNodeDetail, DatabaseError> {
        let Some(existing) = self.get_node(owner_id, agent_id, route_or_node).await? else {
            return Err(DatabaseError::NotFound {
                entity: "memory_node".to_string(),
                id: route_or_node.to_string(),
            });
        };

        let mut keywords = existing
            .keywords
            .iter()
            .map(|keyword| keyword.keyword.clone())
            .collect::<Vec<_>>();
        keywords.retain(|keyword| {
            !remove
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(keyword))
        });
        for keyword in add {
            if !keywords
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(keyword))
            {
                keywords.push(keyword.clone());
            }
        }
        keywords.sort();

        let input = UpdateMemoryNodeInput {
            route_or_node: route_or_node.to_string(),
            trigger_text: disclosure,
            keywords: Some(keywords),
            metadata: Some(serde_json::json!({
                "source": "tool:manage_triggers",
                "updated_at": Utc::now(),
            })),
            ..Default::default()
        };
        let (detail, _changeset) = self.update(owner_id, agent_id, &input).await?;
        Ok(detail)
    }

    pub async fn explain_recall(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        is_group_chat: bool,
    ) -> Result<MemoryRecallExplanation, DatabaseError> {
        let plan = self
            .build_recall_plan(owner_id, agent_id, query, is_group_chat)
            .await?;

        Ok(MemoryRecallExplanation {
            query: query.to_string(),
            boot: plan.boot.iter().map(Self::boot_explanation_entry).collect(),
            triggered: plan.triggered.iter().map(Self::explanation_entry).collect(),
            relevant: plan.relevant.iter().map(Self::explanation_entry).collect(),
            expanded: plan.expanded.iter().map(Self::explanation_entry).collect(),
            recent: plan.recent,
        })
    }

    pub async fn get_versions(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
    ) -> Result<Vec<MemoryVersion>, DatabaseError> {
        let Some(detail) = self.get_node(owner_id, agent_id, route_or_node).await? else {
            return Ok(Vec::new());
        };
        self.db.get_memory_versions(detail.node.id).await
    }

    pub async fn record_episode(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        title: &str,
        content: &str,
        trigger_text: Option<String>,
        metadata: serde_json::Value,
    ) -> Result<MemoryNodeDetail, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let timestamp = Utc::now().format("%Y-%m-%d/%H-%M-%S").to_string();
        let changeset = self
            .db
            .create_memory_changeset(space.id, "runtime", Some(title))
            .await?;
        let detail = self
            .db
            .create_memory_node(&NewMemoryNodeInput {
                space_id: space.id,
                parent_node_id: None,
                domain: "timeline".to_string(),
                path: format!("episodes/{timestamp}"),
                title: title.to_string(),
                kind: MemoryNodeKind::Episode,
                content: content.to_string(),
                relation_kind: MemoryRelationKind::Timeline,
                visibility: MemoryVisibility::Private,
                priority: 25,
                trigger_text,
                metadata,
                keywords: Vec::new(),
                changeset_id: Some(changeset.id),
            })
            .await?;
        self.db
            .complete_memory_changeset(changeset.id, "applied")
            .await?;
        Ok(detail)
    }

    async fn ensure_parent_path_chain(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        space_id: Uuid,
        domain: &str,
        parent_path: &str,
        changeset_id: Uuid,
    ) -> Result<Option<Uuid>, DatabaseError> {
        if parent_path.trim().is_empty() {
            return Ok(None);
        }

        let mut current_path = String::new();
        let mut current_parent_node_id = None;

        for segment in parent_path.split('/').filter(|segment| !segment.is_empty()) {
            if current_path.is_empty() {
                current_path = segment.to_string();
            } else {
                current_path = format!("{current_path}/{segment}");
            }

            let current_uri = format!("{domain}://{current_path}");
            if let Some(existing) = self.get_node(owner_id, agent_id, &current_uri).await? {
                current_parent_node_id = Some(existing.node.id);
                continue;
            }

            let detail = self
                .db
                .create_memory_node(&NewMemoryNodeInput {
                    space_id,
                    parent_node_id: current_parent_node_id,
                    domain: domain.to_string(),
                    path: current_path.clone(),
                    title: segment.to_string(),
                    kind: MemoryNodeKind::Curated,
                    content: format!("Semantic parent node for memories under `{current_uri}`."),
                    relation_kind: MemoryRelationKind::Contains,
                    visibility: MemoryVisibility::Private,
                    priority: 100,
                    trigger_text: None,
                    metadata: serde_json::json!({
                        "source": "tool:auto_scaffold_parent",
                        "scaffold": true,
                    }),
                    keywords: Vec::new(),
                    changeset_id: Some(changeset_id),
                })
                .await?;
            current_parent_node_id = Some(detail.node.id);
        }

        Ok(current_parent_node_id)
    }

    pub async fn create(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        parent_route: Option<&str>,
        title: &str,
        kind: MemoryNodeKind,
        content: &str,
        domain: &str,
        path: &str,
        priority: i32,
        trigger_text: Option<String>,
        visibility: MemoryVisibility,
        keywords: Vec<String>,
        metadata: serde_json::Value,
    ) -> Result<(MemoryNodeDetail, MemoryChangeSet), DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let mut changeset = self
            .db
            .create_memory_changeset(space.id, "tool:create_memory", Some(title))
            .await?;
        let parent_node_id = if let Some(parent_route) = parent_route {
            let is_domain_root = parent_route
                .split_once("://")
                .is_some_and(|(_, path)| path.trim_matches('/').is_empty());
            if is_domain_root {
                None
            } else {
                let (parent_domain, parent_path) = parent_route
                    .split_once("://")
                    .map(|(domain, path)| (domain, path.trim_matches('/')))
                    .ok_or_else(|| DatabaseError::NotFound {
                        entity: "memory_node".to_string(),
                        id: parent_route.to_string(),
                    })?;
                self.ensure_parent_path_chain(
                    owner_id,
                    agent_id,
                    space.id,
                    parent_domain,
                    parent_path,
                    changeset.id,
                )
                .await?
            }
        } else {
            None
        };
        let detail = self
            .db
            .create_memory_node(&NewMemoryNodeInput {
                space_id: space.id,
                parent_node_id,
                domain: domain.to_string(),
                path: path.to_string(),
                title: title.to_string(),
                kind,
                content: content.to_string(),
                // Nocturne-style memory treats hierarchy as a filesystem-like tree.
                // Root nodes are still "contained" in the conceptual domain root.
                relation_kind: MemoryRelationKind::Contains,
                visibility,
                priority,
                trigger_text,
                metadata,
                keywords,
                changeset_id: Some(changeset.id),
            })
            .await?;
        self.db
            .complete_memory_changeset(changeset.id, "applied")
            .await?;
        changeset.status = "applied".to_string();
        Ok((detail, changeset))
    }

    pub async fn update(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        input: &UpdateMemoryNodeInput,
    ) -> Result<(MemoryNodeDetail, MemoryChangeSet), DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let label = input.title.as_deref().unwrap_or(&input.route_or_node);
        let mut changeset = self
            .db
            .create_memory_changeset(space.id, "tool:update_memory", Some(label))
            .await?;
        let mut update = input.clone();
        update.changeset_id = Some(changeset.id);
        let detail = self.db.update_memory_node(space.id, &update).await?;
        self.db
            .complete_memory_changeset(changeset.id, "applied")
            .await?;
        changeset.status = "applied".to_string();
        Ok((detail, changeset))
    }

    pub async fn add_alias(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        input: &CreateMemoryAliasInput,
    ) -> Result<(MemoryRoute, MemoryChangeSet), DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let summary = format!("{}://{}", input.domain, input.path);
        let mut changeset = self
            .db
            .create_memory_changeset(space.id, "tool:add_alias", Some(&summary))
            .await?;
        let mut alias = input.clone();
        alias.space_id = space.id;
        alias.changeset_id = Some(changeset.id);
        let route = self.db.create_memory_alias(&alias).await?;
        self.db
            .complete_memory_changeset(changeset.id, "applied")
            .await?;
        changeset.status = "applied".to_string();
        Ok((route, changeset))
    }

    pub async fn delete_memory(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
    ) -> Result<MemoryChangeSet, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let mut changeset = self
            .db
            .create_memory_changeset(space.id, "tool:delete_memory", Some(route_or_node))
            .await?;
        self.db
            .delete_memory_node(space.id, route_or_node, Some(changeset.id))
            .await?;
        self.db
            .complete_memory_changeset(changeset.id, "applied")
            .await?;
        changeset.status = "applied".to_string();
        Ok(changeset)
    }

    pub async fn review_changeset(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        changeset_id: Uuid,
        action: &str,
    ) -> Result<(), DatabaseError> {
        let _space = self.ensure_primary_space(owner_id, agent_id).await?;
        match action {
            "rollback" | "rollback_requested" => {
                self.db.rollback_memory_changeset(changeset_id).await
            }
            "accept" | "approve" | "applied" => {
                self.db
                    .complete_memory_changeset(changeset_id, "applied")
                    .await
            }
            other => self.db.complete_memory_changeset(changeset_id, other).await,
        }
    }

    pub async fn build_prompt_context(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        user_input: &str,
        is_group_chat: bool,
    ) -> Result<String, DatabaseError> {
        let plan = self
            .build_recall_plan(owner_id, agent_id, user_input, is_group_chat)
            .await?;

        let mut parts = Vec::new();
        if plan.boot.is_empty() {
            parts.push(NOCTURNE_NATIVE_SYSTEM_PROMPT.trim().to_string());
        } else {
            let block = plan
                .boot
                .iter()
                .map(|item| {
                    let uri = item
                        .selected_route
                        .as_ref()
                        .or(item.primary_route.as_ref())
                        .or_else(|| item.routes.first())
                        .map(MemoryRoute::uri)
                        .unwrap_or_else(|| format!("node://{}", item.node.id));
                    format!(
                        "### {}\nURI: {}\n\n{}",
                        item.node.title, uri, item.active_version.content
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            parts.push(format!("## system://boot\n\n{block}"));
        }

        if !plan.triggered.is_empty() {
            let block = plan
                .triggered
                .iter()
                .map(|candidate| {
                    let trigger = candidate
                        .hit
                        .trigger_text
                        .as_deref()
                        .map(|text| format!("\nDisclosure: {text}"))
                        .unwrap_or_default();
                    let matched_keywords = if candidate.hit.matched_keywords.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "\nMatched Keywords: {}",
                            candidate.hit.matched_keywords.join(", ")
                        )
                    };
                    format!(
                        "### {}\nURI: {}\nReason: {}\n{}{}{}",
                        candidate.hit.title,
                        candidate.hit.uri,
                        candidate.reason,
                        candidate.hit.content_snippet,
                        trigger,
                        matched_keywords
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            parts.push(format!("## Triggered Recall\n\n{block}"));
        }

        let mut relevant = plan.relevant;
        relevant.extend(plan.expanded);
        if !relevant.is_empty() {
            let block = relevant
                .iter()
                .map(|candidate| {
                    let retrieval = match (candidate.hit.fts_rank, candidate.hit.vector_rank) {
                        (Some(fts), Some(vector)) => {
                            format!(
                                "fts #{fts}, vector #{vector}, score {:.3}",
                                candidate.hit.score
                            )
                        }
                        (Some(fts), None) => {
                            format!("fts #{fts}, score {:.3}", candidate.hit.score)
                        }
                        (None, Some(vector)) => {
                            format!("vector #{vector}, score {:.3}", candidate.hit.score)
                        }
                        (None, None) => format!("score {:.3}", candidate.hit.score),
                    };
                    format!(
                        "### {}\nURI: {}\nReason: {}\nRetrieval: {}\n{}",
                        candidate.hit.title,
                        candidate.hit.uri,
                        candidate.reason,
                        retrieval,
                        candidate.hit.content_snippet
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            parts.push(format!("## Relevant Memory Recall\n\n{block}"));
        }
        if !plan.recent.is_empty() {
            let block = plan
                .recent
                .iter()
                .map(|item| {
                    format!(
                        "### {}\n{}\n{}",
                        item.title, item.updated_at, item.content_snippet
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            parts.push(format!("## Recent Episodes\n\n{block}"));
        }

        Ok(parts.join("\n\n"))
    }
}

struct LegacyImportPlan {
    domain: String,
    path: String,
    title: String,
    kind: MemoryNodeKind,
    relation_kind: MemoryRelationKind,
    visibility: MemoryVisibility,
    priority: i32,
    trigger_text: Option<String>,
    keywords: Vec<String>,
}

fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    let mut last_was_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn legacy_import_route(path: &str) -> (String, String) {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let slug = slugify(file_name);
    let route_path = if slug.is_empty() {
        "imported-memory".to_string()
    } else {
        slug
    };
    ("legacy".to_string(), route_path)
}

fn legacy_import_plan(path: &str, content: &str) -> Option<LegacyImportPlan> {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let default_title = file_name.trim_end_matches(".md").replace('-', " ");
    let (domain, import_path) = legacy_import_route(path);
    match path {
        "AGENTS.md" => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: "Agent Instructions".to_string(),
            kind: MemoryNodeKind::Directive,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 5,
            trigger_text: Some("When deciding how Steward should behave in a session".to_string()),
            keywords: vec!["agent".to_string(), "instructions".to_string()],
        }),
        "SOUL.md" => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: "Core Values".to_string(),
            kind: MemoryNodeKind::Value,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 5,
            trigger_text: Some("When tone or values feel uncertain".to_string()),
            keywords: vec!["values".to_string()],
        }),
        "IDENTITY.md" => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: "Identity".to_string(),
            kind: MemoryNodeKind::Identity,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 5,
            trigger_text: Some("When Steward needs to re-anchor its role".to_string()),
            keywords: vec!["identity".to_string()],
        }),
        "USER.md" => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: "User Context".to_string(),
            kind: MemoryNodeKind::UserProfile,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Private,
            priority: 10,
            trigger_text: Some("When tailoring responses to the user".to_string()),
            keywords: vec!["user".to_string()],
        }),
        "TOOLS.md" => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: "Tool Notes".to_string(),
            kind: MemoryNodeKind::Directive,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 15,
            trigger_text: Some("When choosing how to use tools in this environment".to_string()),
            keywords: vec!["tools".to_string()],
        }),
        "HEARTBEAT.md" => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: "Heartbeat Checklist".to_string(),
            kind: MemoryNodeKind::Procedure,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Private,
            priority: 20,
            trigger_text: Some("When running periodic proactive checks".to_string()),
            keywords: vec!["heartbeat".to_string()],
        }),
        "MEMORY.md" => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: "Curated Memory".to_string(),
            kind: MemoryNodeKind::Curated,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Private,
            priority: 15,
            trigger_text: Some("When prior decisions or lessons may matter".to_string()),
            keywords: vec!["memory".to_string()],
        }),
        _ if path.starts_with("daily/") => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: format!("Daily {}", file_name.trim_end_matches(".md")),
            kind: MemoryNodeKind::Episode,
            relation_kind: MemoryRelationKind::Timeline,
            visibility: MemoryVisibility::Private,
            priority: 30,
            trigger_text: Some("When recalling recent activity and context".to_string()),
            keywords: vec!["daily".to_string()],
        }),
        _ if !content.trim().is_empty() => Some(LegacyImportPlan {
            domain,
            path: import_path,
            title: default_title,
            kind: MemoryNodeKind::Reference,
            relation_kind: MemoryRelationKind::RelatesTo,
            visibility: MemoryVisibility::Private,
            priority: 50,
            trigger_text: Some("When imported legacy context may be relevant".to_string()),
            keywords: Vec::new(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OWNER_ID: &str = "memory-test-owner";

    async fn test_manager() -> (MemoryManager, tempfile::TempDir) {
        let (db, dir) = crate::testing::test_db().await;
        (MemoryManager::new(db), dir)
    }

    async fn create_sample_node(
        manager: &MemoryManager,
        path: &str,
    ) -> (MemoryNodeDetail, MemoryChangeSet) {
        manager
            .create(
                OWNER_ID,
                None,
                None,
                "Sample Memory",
                MemoryNodeKind::Curated,
                "Initial content",
                "core",
                path,
                12,
                Some("initial trigger".to_string()),
                MemoryVisibility::Private,
                vec!["alpha".to_string()],
                json!({"source": "test"}),
            )
            .await
            .expect("create memory node")
    }

    #[tokio::test]
    async fn rollback_create_removes_pending_node() {
        let (manager, _dir) = test_manager().await;
        let (_detail, changeset) = create_sample_node(&manager, "tests/create").await;

        assert!(
            manager
                .get_node(OWNER_ID, None, "core://tests/create")
                .await
                .expect("open created node")
                .is_some()
        );

        manager
            .review_changeset(OWNER_ID, None, changeset.id, "rollback")
            .await
            .expect("rollback create");

        assert!(
            manager
                .get_node(OWNER_ID, None, "core://tests/create")
                .await
                .expect("open rolled back node")
                .is_none()
        );
        assert!(
            manager
                .list_review_changesets(OWNER_ID, None)
                .await
                .expect("list reviews")
                .iter()
                .all(|item| item.id != changeset.id)
        );
    }

    #[tokio::test]
    async fn rollback_update_restores_prior_version_and_keywords() {
        let (manager, _dir) = test_manager().await;
        let (created, create_changeset) = create_sample_node(&manager, "tests/update").await;
        manager
            .review_changeset(OWNER_ID, None, create_changeset.id, "accept")
            .await
            .expect("accept create changeset");

        let original_version_id = created.active_version.id;
        let original_title = created.node.title.clone();
        let original_keywords = created
            .keywords
            .iter()
            .map(|item| item.keyword.clone())
            .collect::<Vec<_>>();
        let original_edge = created.edges.first().cloned().expect("primary edge");

        let (updated, update_changeset) = manager
            .update(
                OWNER_ID,
                None,
                &UpdateMemoryNodeInput {
                    route_or_node: "core://tests/update".to_string(),
                    title: Some("Updated Memory".to_string()),
                    content: Some("Updated content".to_string()),
                    priority: Some(3),
                    trigger_text: Some(Some("updated trigger".to_string())),
                    keywords: Some(vec!["beta".to_string(), "gamma".to_string()]),
                    ..Default::default()
                },
            )
            .await
            .expect("update memory node");

        assert_eq!(updated.node.title, "Updated Memory");
        assert_eq!(updated.active_version.content, "Updated content");
        assert_ne!(updated.active_version.id, original_version_id);

        manager
            .review_changeset(OWNER_ID, None, update_changeset.id, "rollback")
            .await
            .expect("rollback update");

        let restored = manager
            .get_node(OWNER_ID, None, "core://tests/update")
            .await
            .expect("open restored node")
            .expect("restored node exists");

        assert_eq!(restored.node.title, original_title);
        assert_eq!(restored.active_version.id, original_version_id);
        assert_eq!(restored.active_version.content, "Initial content");
        assert_eq!(
            restored
                .keywords
                .iter()
                .map(|item| item.keyword.clone())
                .collect::<Vec<_>>(),
            original_keywords
        );
        let restored_edge = restored.edges.first().expect("restored edge");
        assert_eq!(restored_edge.priority, original_edge.priority);
        assert_eq!(restored_edge.trigger_text, original_edge.trigger_text);
    }

    #[tokio::test]
    async fn rollback_alias_removes_alias_only() {
        let (manager, _dir) = test_manager().await;
        let (created, create_changeset) = create_sample_node(&manager, "tests/alias").await;
        manager
            .review_changeset(OWNER_ID, None, create_changeset.id, "accept")
            .await
            .expect("accept create changeset");

        let primary_route = created.primary_route.as_ref().expect("primary route").uri();

        let (alias_route, alias_changeset) = manager
            .add_alias(
                OWNER_ID,
                None,
                &CreateMemoryAliasInput {
                    space_id: Uuid::nil(),
                    target_route_or_node: primary_route.clone(),
                    domain: "lookup".to_string(),
                    path: "sample-memory".to_string(),
                    visibility: MemoryVisibility::Session,
                    priority: 20,
                    trigger_text: Some("alias trigger".to_string()),
                    changeset_id: None,
                },
            )
            .await
            .expect("create alias");

        assert_eq!(alias_route.uri(), "lookup://sample-memory");
        assert!(
            manager
                .get_node(OWNER_ID, None, &alias_route.uri())
                .await
                .expect("open alias")
                .is_some()
        );

        manager
            .review_changeset(OWNER_ID, None, alias_changeset.id, "rollback")
            .await
            .expect("rollback alias");

        assert!(
            manager
                .get_node(OWNER_ID, None, &alias_route.uri())
                .await
                .expect("open rolled back alias")
                .is_none()
        );
        assert!(
            manager
                .get_node(OWNER_ID, None, &primary_route)
                .await
                .expect("open primary route")
                .is_some()
        );
    }

    #[tokio::test]
    async fn rollback_alias_removes_cascaded_alias_subtree() {
        let (manager, _dir) = test_manager().await;
        let (_root, root_changeset) = manager
            .create(
                OWNER_ID,
                None,
                Some("core://"),
                "agent",
                MemoryNodeKind::Reference,
                "Agent root",
                "core",
                "agent",
                0,
                None,
                MemoryVisibility::Private,
                Vec::new(),
                serde_json::json!({ "source": "test" }),
            )
            .await
            .expect("create agent root");
        manager
            .review_changeset(OWNER_ID, None, root_changeset.id, "accept")
            .await
            .expect("accept root changeset");

        let (_profile, profile_changeset) = manager
            .create(
                OWNER_ID,
                None,
                Some("core://agent"),
                "profile",
                MemoryNodeKind::Reference,
                "Profile root",
                "core",
                "agent/profile",
                0,
                None,
                MemoryVisibility::Private,
                Vec::new(),
                serde_json::json!({ "source": "test" }),
            )
            .await
            .expect("create profile root");
        manager
            .review_changeset(OWNER_ID, None, profile_changeset.id, "accept")
            .await
            .expect("accept profile changeset");

        let (_name, name_changeset) = manager
            .create(
                OWNER_ID,
                None,
                Some("core://agent/profile"),
                "name",
                MemoryNodeKind::Reference,
                "用户名字是梦凌汐。",
                "core",
                "agent/profile/name",
                0,
                None,
                MemoryVisibility::Private,
                Vec::new(),
                serde_json::json!({ "source": "test" }),
            )
            .await
            .expect("create leaf");
        manager
            .review_changeset(OWNER_ID, None, name_changeset.id, "accept")
            .await
            .expect("accept leaf changeset");

        let (_mirror, mirror_changeset) = manager
            .create(
                OWNER_ID,
                None,
                Some("core://"),
                "my_user",
                MemoryNodeKind::Reference,
                "Mirror root",
                "core",
                "my_user",
                0,
                None,
                MemoryVisibility::Private,
                Vec::new(),
                serde_json::json!({ "source": "test" }),
            )
            .await
            .expect("create mirror root");
        manager
            .review_changeset(OWNER_ID, None, mirror_changeset.id, "accept")
            .await
            .expect("accept mirror changeset");

        let (alias_route, alias_changeset) = manager
            .add_alias(
                OWNER_ID,
                None,
                &CreateMemoryAliasInput {
                    space_id: Uuid::nil(),
                    target_route_or_node: "core://agent/profile".to_string(),
                    domain: "core".to_string(),
                    path: "my_user/profile".to_string(),
                    visibility: MemoryVisibility::Private,
                    priority: 0,
                    trigger_text: None,
                    changeset_id: None,
                },
            )
            .await
            .expect("create alias subtree");

        assert_eq!(alias_route.uri(), "core://my_user/profile");
        assert!(
            manager
                .get_node(OWNER_ID, None, "core://my_user/profile/name")
                .await
                .expect("open cascaded alias child")
                .is_some()
        );

        manager
            .review_changeset(OWNER_ID, None, alias_changeset.id, "rollback")
            .await
            .expect("rollback alias subtree");

        assert!(
            manager
                .get_node(OWNER_ID, None, "core://my_user/profile")
                .await
                .expect("open rolled back alias root")
                .is_none()
        );
        assert!(
            manager
                .get_node(OWNER_ID, None, "core://my_user/profile/name")
                .await
                .expect("open rolled back alias child")
                .is_none()
        );
        assert!(
            manager
                .get_node(OWNER_ID, None, "core://agent/profile/name")
                .await
                .expect("open original child")
                .is_some()
        );
    }

    #[tokio::test]
    async fn rollback_delete_restores_route_and_active_version() {
        let (manager, _dir) = test_manager().await;
        let (created, create_changeset) = create_sample_node(&manager, "tests/delete").await;
        manager
            .review_changeset(OWNER_ID, None, create_changeset.id, "accept")
            .await
            .expect("accept create changeset");

        let primary_route = created.primary_route.as_ref().expect("primary route").uri();
        let version_id = created.active_version.id;

        let delete_changeset = manager
            .delete_memory(OWNER_ID, None, &primary_route)
            .await
            .expect("delete primary route");

        assert!(
            manager
                .get_node(OWNER_ID, None, &primary_route)
                .await
                .expect("open deleted route")
                .is_none()
        );

        manager
            .review_changeset(OWNER_ID, None, delete_changeset.id, "rollback")
            .await
            .expect("rollback delete");

        let restored = manager
            .get_node(OWNER_ID, None, &primary_route)
            .await
            .expect("open restored route")
            .expect("restored route exists");
        assert_eq!(restored.active_version.id, version_id);
        assert_eq!(restored.active_version.content, "Initial content");
    }

    #[tokio::test]
    async fn get_node_preserves_selected_alias_route_context() {
        let (manager, _dir) = test_manager().await;
        let (created, create_changeset) = create_sample_node(&manager, "tests/context").await;
        manager
            .review_changeset(OWNER_ID, None, create_changeset.id, "accept")
            .await
            .expect("accept create changeset");

        let primary_route = created.primary_route.as_ref().expect("primary route").uri();
        let (alias_route, alias_changeset) = manager
            .add_alias(
                OWNER_ID,
                None,
                &CreateMemoryAliasInput {
                    space_id: Uuid::nil(),
                    target_route_or_node: primary_route,
                    domain: "lookup".to_string(),
                    path: "context-memory".to_string(),
                    visibility: MemoryVisibility::Private,
                    priority: 0,
                    trigger_text: Some("When opening through the alias".to_string()),
                    changeset_id: None,
                },
            )
            .await
            .expect("create alias");
        manager
            .review_changeset(OWNER_ID, None, alias_changeset.id, "accept")
            .await
            .expect("accept alias changeset");

        let detail = manager
            .get_node(OWNER_ID, None, &alias_route.uri())
            .await
            .expect("open alias route")
            .expect("alias detail should exist");

        assert_eq!(
            detail.selected_uri.as_deref(),
            Some("lookup://context-memory")
        );
        assert_eq!(
            detail.selected_route.as_ref().map(|route| route.uri()),
            Some("lookup://context-memory".to_string())
        );
    }

    #[tokio::test]
    async fn search_query_expansion_recalls_user_identity_without_embeddings() {
        let (manager, _dir) = test_manager().await;
        let (_user_root, user_root_changeset) = manager
            .create(
                OWNER_ID,
                None,
                Some("core://"),
                "user",
                MemoryNodeKind::Curated,
                "User root",
                "core",
                "user",
                0,
                Some("When recalling stable user identity".to_string()),
                MemoryVisibility::Private,
                vec!["user".to_string()],
                json!({"source": "test"}),
            )
            .await
            .expect("create user root");
        manager
            .review_changeset(OWNER_ID, None, user_root_changeset.id, "accept")
            .await
            .expect("accept user root changeset");

        let (_name_detail, name_changeset) = manager
            .create(
                OWNER_ID,
                None,
                Some("core://user"),
                "name",
                MemoryNodeKind::Reference,
                "用户名字是梦凌汐。",
                "core",
                "user/name",
                0,
                Some("当需要正确称呼用户时".to_string()),
                MemoryVisibility::Private,
                vec!["梦凌汐".to_string(), "名字".to_string()],
                json!({"source": "test"}),
            )
            .await
            .expect("create name memory");
        manager
            .review_changeset(OWNER_ID, None, name_changeset.id, "accept")
            .await
            .expect("accept name changeset");

        let hits = manager
            .search(OWNER_ID, None, "我是谁", 5, &[])
            .await
            .expect("search should succeed");

        assert!(
            hits.iter().any(|hit| hit.uri == "core://user/name"),
            "expanded recall should find user name memory for identity query: {hits:#?}"
        );
    }

    #[tokio::test]
    async fn explain_recall_reports_boot_trigger_and_recent_sections() {
        let (manager, _dir) = test_manager().await;
        manager
            .ensure_boot_protocol(OWNER_ID)
            .await
            .expect("seed boot protocol");

        let (_detail, changeset) = manager
            .create(
                OWNER_ID,
                None,
                Some("core://user"),
                "name",
                MemoryNodeKind::Reference,
                "用户名字是梦凌汐。",
                "core",
                "user/name",
                0,
                Some("当需要正确称呼用户时".to_string()),
                MemoryVisibility::Private,
                vec!["梦凌汐".to_string(), "名字".to_string()],
                json!({"source": "test"}),
            )
            .await
            .expect("create name memory");
        manager
            .review_changeset(OWNER_ID, None, changeset.id, "accept")
            .await
            .expect("accept create changeset");
        manager
            .add_to_boot(OWNER_ID, None, "core://user/name", 1)
            .await
            .expect("add boot entry");
        manager
            .record_episode(
                OWNER_ID,
                None,
                "User introduced themself",
                "用户说自己叫梦凌汐。",
                Some("When checking recent episodes".to_string()),
                json!({"source": "test"}),
            )
            .await
            .expect("record episode");

        let explanation = manager
            .explain_recall(OWNER_ID, None, "你记得我叫什么吗", false)
            .await
            .expect("explain recall");

        assert!(
            explanation
                .boot
                .iter()
                .any(|entry| entry.uri == "core://user/name"),
            "boot explanation should include explicit boot members: {explanation:#?}"
        );
        assert!(
            explanation
                .boot
                .iter()
                .chain(explanation.relevant.iter())
                .any(|entry| entry.uri == "core://user/name"),
            "boot or relevant explanation should include the retrieved memory: {explanation:#?}"
        );
        assert!(
            !explanation.recent.is_empty(),
            "recent episodes should be present in recall explanation"
        );
    }
}
