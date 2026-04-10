//! Native graph-based memory system for Steward.
//!
//! This module models agent memory as durable graph entities rather than
//! workspace documents. The workspace remains responsible for mounted files
//! and ad-hoc content indexing; long-term agent memory lives here.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::Database;
use crate::error::DatabaseError;

pub const PRIMARY_SPACE_SLUG: &str = "primary";

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
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryNodeDetail {
    pub node: MemoryNode,
    pub active_version: MemoryVersion,
    pub primary_route: Option<MemoryRoute>,
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

#[derive(Debug, Clone)]
pub struct MemoryRecallCandidate {
    pub hit: MemorySearchHit,
    pub source: &'static str,
}

#[derive(Debug, Clone)]
pub struct MemoryRecallPlan {
    pub boot: Vec<MemoryNodeDetail>,
    pub triggered: Vec<MemoryRecallCandidate>,
    pub recent: Vec<MemoryTimelineEntry>,
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
}

impl MemoryManager {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
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
        self.db
            .search_memory_graph(space.id, query, limit, domains)
            .await
    }

    pub async fn recall(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        limit: usize,
        domains: &[String],
    ) -> Result<Vec<MemorySearchHit>, DatabaseError> {
        self.search(owner_id, agent_id, query, limit, domains).await
    }

    pub async fn open(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
    ) -> Result<Option<MemoryNodeDetail>, DatabaseError> {
        self.get_node(owner_id, agent_id, route_or_node).await
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

    pub async fn list_reviews(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryChangeSet>, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        self.db.list_memory_reviews(space.id).await
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
        let parent_node_id = if let Some(parent_route) = parent_route {
            self.get_node(owner_id, agent_id, parent_route)
                .await?
                .map(|detail| detail.node.id)
        } else {
            None
        };
        let changeset = self
            .db
            .create_memory_changeset(space.id, "tool:memory_create", Some(title))
            .await?;
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
                relation_kind: if parent_node_id.is_some() {
                    MemoryRelationKind::Contains
                } else {
                    MemoryRelationKind::RelatesTo
                },
                visibility,
                priority,
                trigger_text,
                metadata,
                keywords,
                changeset_id: Some(changeset.id),
            })
            .await?;
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
        let changeset = self
            .db
            .create_memory_changeset(space.id, "tool:memory_update", Some(label))
            .await?;
        let mut update = input.clone();
        update.changeset_id = Some(changeset.id);
        let detail = self.db.update_memory_node(space.id, &update).await?;
        Ok((detail, changeset))
    }

    pub async fn alias(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        input: &CreateMemoryAliasInput,
    ) -> Result<(MemoryRoute, MemoryChangeSet), DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let summary = format!("{}://{}", input.domain, input.path);
        let changeset = self
            .db
            .create_memory_changeset(space.id, "tool:memory_alias", Some(&summary))
            .await?;
        let mut alias = input.clone();
        alias.space_id = space.id;
        alias.changeset_id = Some(changeset.id);
        let route = self.db.create_memory_alias(&alias).await?;
        Ok((route, changeset))
    }

    pub async fn delete(
        &self,
        owner_id: &str,
        agent_id: Option<Uuid>,
        route_or_node: &str,
    ) -> Result<MemoryChangeSet, DatabaseError> {
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let changeset = self
            .db
            .create_memory_changeset(space.id, "tool:memory_delete", Some(route_or_node))
            .await?;
        self.db
            .delete_memory_node(space.id, route_or_node, Some(changeset.id))
            .await?;
        Ok(changeset)
    }

    pub async fn review(
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
        let space = self.ensure_primary_space(owner_id, agent_id).await?;
        let boot_items = self
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
        let triggered_hits = self
            .db
            .search_memory_graph(space.id, user_input, 5, &[])
            .await?
            .into_iter()
            .filter(|hit| {
                !is_group_chat
                    || !matches!(
                        hit.kind,
                        MemoryNodeKind::Curated
                            | MemoryNodeKind::UserProfile
                            | MemoryNodeKind::Episode
                    )
            })
            .collect::<Vec<_>>();
        let recent = self.db.list_memory_timeline(space.id, 3).await?;

        let mut parts = Vec::new();
        if !boot_items.is_empty() {
            let block = boot_items
                .iter()
                .map(|item| format!("### {}\n\n{}", item.node.title, item.active_version.content))
                .collect::<Vec<_>>()
                .join("\n\n");
            parts.push(format!("## Memory Boot Set\n\n{block}"));
        }
        if !triggered_hits.is_empty() {
            let block = triggered_hits
                .iter()
                .map(|hit| {
                    let trigger = hit
                        .trigger_text
                        .as_deref()
                        .map(|text| format!("\nTrigger: {text}"))
                        .unwrap_or_default();
                    format!(
                        "### {}\nURI: {}\n{}\n{}",
                        hit.title, hit.uri, hit.content_snippet, trigger
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            parts.push(format!("## Relevant Memory Recall\n\n{block}"));
        }
        if !recent.is_empty() {
            let block = recent
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

fn legacy_import_plan(path: &str, content: &str) -> Option<LegacyImportPlan> {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let default_title = file_name.trim_end_matches(".md").replace('-', " ");
    match path {
        "AGENTS.md" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "instructions/agent".to_string(),
            title: "Agent Instructions".to_string(),
            kind: MemoryNodeKind::Directive,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 5,
            trigger_text: Some("When deciding how Steward should behave in a session".to_string()),
            keywords: vec!["agent".to_string(), "instructions".to_string()],
        }),
        "SOUL.md" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "identity/values".to_string(),
            title: "Core Values".to_string(),
            kind: MemoryNodeKind::Value,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 5,
            trigger_text: Some("When tone or values feel uncertain".to_string()),
            keywords: vec!["values".to_string()],
        }),
        "IDENTITY.md" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "identity/self".to_string(),
            title: "Identity".to_string(),
            kind: MemoryNodeKind::Identity,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 5,
            trigger_text: Some("When Steward needs to re-anchor its role".to_string()),
            keywords: vec!["identity".to_string()],
        }),
        "USER.md" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "identity/user".to_string(),
            title: "User Context".to_string(),
            kind: MemoryNodeKind::UserProfile,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Private,
            priority: 10,
            trigger_text: Some("When tailoring responses to the user".to_string()),
            keywords: vec!["user".to_string()],
        }),
        "TOOLS.md" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "instructions/tools".to_string(),
            title: "Tool Notes".to_string(),
            kind: MemoryNodeKind::Directive,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 15,
            trigger_text: Some("When choosing how to use tools in this environment".to_string()),
            keywords: vec!["tools".to_string()],
        }),
        "HEARTBEAT.md" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "procedures/heartbeat".to_string(),
            title: "Heartbeat Checklist".to_string(),
            kind: MemoryNodeKind::Procedure,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Private,
            priority: 20,
            trigger_text: Some("When running periodic proactive checks".to_string()),
            keywords: vec!["heartbeat".to_string()],
        }),
        "MEMORY.md" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "curated/main".to_string(),
            title: "Curated Memory".to_string(),
            kind: MemoryNodeKind::Curated,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Private,
            priority: 15,
            trigger_text: Some("When prior decisions or lessons may matter".to_string()),
            keywords: vec!["memory".to_string()],
        }),
        "context/profile.json" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "profile/psychographic".to_string(),
            title: "Psychographic Profile".to_string(),
            kind: MemoryNodeKind::UserProfile,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Private,
            priority: 20,
            trigger_text: Some("When interaction style should adapt to the user".to_string()),
            keywords: vec!["profile".to_string()],
        }),
        "context/assistant-directives.md" => Some(LegacyImportPlan {
            domain: "core".to_string(),
            path: "instructions/assistant-directives".to_string(),
            title: "Assistant Directives".to_string(),
            kind: MemoryNodeKind::Directive,
            relation_kind: MemoryRelationKind::Contains,
            visibility: MemoryVisibility::Session,
            priority: 10,
            trigger_text: Some("When resolving response style and behavior".to_string()),
            keywords: vec!["directives".to_string()],
        }),
        _ if path.starts_with("daily/") => Some(LegacyImportPlan {
            domain: "timeline".to_string(),
            path: format!("daily/{}", file_name.trim_end_matches(".md")),
            title: format!("Daily {}", file_name.trim_end_matches(".md")),
            kind: MemoryNodeKind::Episode,
            relation_kind: MemoryRelationKind::Timeline,
            visibility: MemoryVisibility::Private,
            priority: 30,
            trigger_text: Some("When recalling recent activity and context".to_string()),
            keywords: vec!["daily".to_string()],
        }),
        _ if !content.trim().is_empty() => Some(LegacyImportPlan {
            domain: "imported".to_string(),
            path: path
                .trim_matches('/')
                .trim_end_matches(".md")
                .replace('.', "/"),
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
                .open(OWNER_ID, None, "core://tests/create")
                .await
                .expect("open created node")
                .is_some()
        );

        manager
            .review(OWNER_ID, None, changeset.id, "rollback")
            .await
            .expect("rollback create");

        assert!(
            manager
                .open(OWNER_ID, None, "core://tests/create")
                .await
                .expect("open rolled back node")
                .is_none()
        );
        assert!(
            manager
                .list_reviews(OWNER_ID, None)
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
            .review(OWNER_ID, None, create_changeset.id, "accept")
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
            .review(OWNER_ID, None, update_changeset.id, "rollback")
            .await
            .expect("rollback update");

        let restored = manager
            .open(OWNER_ID, None, "core://tests/update")
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
            .review(OWNER_ID, None, create_changeset.id, "accept")
            .await
            .expect("accept create changeset");

        let primary_route = created.primary_route.as_ref().expect("primary route").uri();

        let (alias_route, alias_changeset) = manager
            .alias(
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
                .open(OWNER_ID, None, &alias_route.uri())
                .await
                .expect("open alias")
                .is_some()
        );

        manager
            .review(OWNER_ID, None, alias_changeset.id, "rollback")
            .await
            .expect("rollback alias");

        assert!(
            manager
                .open(OWNER_ID, None, &alias_route.uri())
                .await
                .expect("open rolled back alias")
                .is_none()
        );
        assert!(
            manager
                .open(OWNER_ID, None, &primary_route)
                .await
                .expect("open primary route")
                .is_some()
        );
    }

    #[tokio::test]
    async fn rollback_delete_restores_route_and_active_version() {
        let (manager, _dir) = test_manager().await;
        let (created, create_changeset) = create_sample_node(&manager, "tests/delete").await;
        manager
            .review(OWNER_ID, None, create_changeset.id, "accept")
            .await
            .expect("accept create changeset");

        let primary_route = created.primary_route.as_ref().expect("primary route").uri();
        let version_id = created.active_version.id;

        let delete_changeset = manager
            .delete(OWNER_ID, None, &primary_route)
            .await
            .expect("delete primary route");

        assert!(
            manager
                .open(OWNER_ID, None, &primary_route)
                .await
                .expect("open deleted route")
                .is_none()
        );

        manager
            .review(OWNER_ID, None, delete_changeset.id, "rollback")
            .await
            .expect("rollback delete");

        let restored = manager
            .open(OWNER_ID, None, &primary_route)
            .await
            .expect("open restored route")
            .expect("restored route exists");
        assert_eq!(restored.active_version.id, version_id);
        assert_eq!(restored.active_version.content, "Initial content");
    }
}
