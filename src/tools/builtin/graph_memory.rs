//! Nocturne-style graph memory tools for Steward's native memory system.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;

use crate::context::JobContext;
use crate::memory::{
    CreateMemoryAliasInput, MemoryManager, MemoryNodeDetail, MemoryNodeKind, UpdateMemoryNodeInput,
};
use crate::tools::tool::ToolDiscoverySummary;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

const DEFAULT_DOMAIN: &str = "core";
const DEFAULT_CREATE_PRIORITY: i32 = 0;
const DEFAULT_SEARCH_LIMIT: usize = 10;

fn parse_uri(uri: &str) -> Result<(String, String), ToolError> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err(ToolError::InvalidParameters(
            "uri must not be empty".to_string(),
        ));
    }

    if let Some((domain, path)) = trimmed.split_once("://") {
        let domain = domain.trim().to_lowercase();
        if domain.is_empty() {
            return Err(ToolError::InvalidParameters(format!(
                "invalid uri '{uri}': missing domain"
            )));
        }
        return Ok((domain, path.trim_matches('/').to_string()));
    }

    Ok((
        DEFAULT_DOMAIN.to_string(),
        trimmed.trim_matches('/').to_string(),
    ))
}

fn parse_non_root_uri(uri: &str) -> Result<(String, String), ToolError> {
    let (domain, path) = parse_uri(uri)?;
    if path.is_empty() {
        return Err(ToolError::InvalidParameters(format!(
            "uri must include a path: {uri}"
        )));
    }
    Ok((domain, path))
}

fn make_uri(domain: &str, path: &str) -> String {
    if path.is_empty() {
        format!("{domain}://")
    } else {
        format!("{domain}://{path}")
    }
}

fn optional_string(params: &serde_json::Value, key: &str) -> Result<Option<String>, ToolError> {
    match params.get(key) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_string()))
            .ok_or_else(|| {
                ToolError::InvalidParameters(format!("'{key}' must be a string when provided"))
            }),
    }
}

fn optional_priority(params: &serde_json::Value) -> Result<Option<i32>, ToolError> {
    match params.get("priority") {
        Some(value) => {
            let value = value.as_i64().ok_or_else(|| {
                ToolError::InvalidParameters("'priority' must be an integer".to_string())
            })?;
            Ok(Some(value as i32))
        }
        None => Ok(None),
    }
}

fn optional_string_list(
    params: &serde_json::Value,
    key: &str,
) -> Result<Option<Vec<String>>, ToolError> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .map(|value| {
                value.as_str().map(|text| text.to_string()).ok_or_else(|| {
                    ToolError::InvalidParameters(format!("'{key}' entries must all be strings"))
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(ToolError::InvalidParameters(format!(
            "'{key}' must be an array of strings when provided"
        ))),
    }
}

fn validate_title(title: &str) -> Result<(), ToolError> {
    if title.is_empty() {
        return Err(ToolError::InvalidParameters(
            "title must not be empty".to_string(),
        ));
    }
    if title
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Ok(())
    } else {
        Err(ToolError::InvalidParameters(
            "title must only contain alphanumeric characters, underscores, or hyphens".to_string(),
        ))
    }
}

fn is_virtual_system_view(uri: &str) -> bool {
    let trimmed = uri.trim();
    trimmed == "system://boot"
        || trimmed == "system://glossary"
        || trimmed == "system://index"
        || trimmed.starts_with("system://index/")
        || trimmed == "system://recent"
        || trimmed.starts_with("system://recent/")
}

fn selected_route_metadata(
    detail: &MemoryNodeDetail,
    requested_uri: &str,
) -> (Option<String>, Option<i32>, Option<String>) {
    let route = detail
        .routes
        .iter()
        .find(|route| route.uri() == requested_uri)
        .or(detail.primary_route.as_ref());

    let disclosure = route
        .and_then(|route| route.edge_id)
        .and_then(|edge_id| detail.edges.iter().find(|edge| edge.id == edge_id));

    (
        route.map(|route| route.uri()),
        disclosure.map(|edge| edge.priority),
        disclosure.and_then(|edge| edge.trigger_text.clone()),
    )
}

fn direct_child_segment(parent_path: &str, uri: &str) -> Option<String> {
    let (_, child_path) = parse_uri(uri).ok()?;
    if parent_path.is_empty() {
        if child_path.contains('/') {
            return None;
        }
        return Some(child_path);
    }

    let prefix = format!("{parent_path}/");
    let rest = child_path.strip_prefix(&prefix)?;
    if rest.contains('/') {
        return None;
    }
    Some(rest.to_string())
}

async fn next_numeric_child_name(
    memory: &MemoryManager,
    owner_id: &str,
    domain: &str,
    parent_path: &str,
) -> Result<String, ToolError> {
    let entries = memory
        .list_index(owner_id, None, Some(domain))
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("memory index failed: {e}")))?;

    let next = entries
        .iter()
        .filter_map(|entry| direct_child_segment(parent_path, &entry.uri))
        .filter_map(|segment| segment.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        + 1;

    Ok(next.to_string())
}

fn create_memory_examples() -> Vec<serde_json::Value> {
    vec![
        json!({
            "parent_uri": "core://user",
            "content": "Template: The user's name is <user_name>.",
            "priority": 0,
            "title": "name",
            "disclosure": "Template: When I need to address the user correctly"
        }),
        json!({
            "parent_uri": "core://assistant/identity",
            "content": "Template: My name is <assistant_name>.",
            "priority": 0,
            "title": "name",
            "disclosure": "Template: When the user asks who I am, or I need to introduce myself"
        }),
        json!({
            "parent_uri": "core://assistant/style",
            "content": "Template: Status updates should be concise and high-signal.",
            "priority": 1,
            "title": "status_updates",
            "disclosure": "Template: When I am about to send a progress update"
        }),
    ]
}

fn create_memory_tool_summary() -> ToolDiscoverySummary {
    ToolDiscoverySummary {
        always_required: vec!["parent_uri".into(), "content".into(), "priority".into()],
        conditional_requirements: vec![
            "Ordinary durable memories should usually include a title; omit it only when you explicitly want numbered siblings like 1/2/3...".into(),
            "User facts, self-model facts, important lessons, preferences, and agreements should usually include disclosure so recall knows when to surface them.".into(),
        ],
        notes: vec![
            "URI answers What; disclosure answers When.".into(),
            "Do not put ordinary facts directly under core:// unless you are intentionally creating a new semantic root. Pick a parent_uri that names the real concept.".into(),
            "If you are unsure where something belongs, use search_memory to find the existing concept instead of dumping it at core://.".into(),
            "Avoid vague containers like logs, misc, or history. Parent nodes and titles should express the actual concept.".into(),
            "The examples below are placeholders only, not real facts from the current conversation.".into(),
        ],
        examples: create_memory_examples(),
    }
}

pub struct SearchMemoryTool {
    memory: Arc<MemoryManager>,
}

impl SearchMemoryTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for SearchMemoryTool {
    fn name(&self) -> &str {
        "search_memory"
    }

    fn description(&self) -> &str {
        "Search graph memory by topic, phrase, or keyword. Use this when you know what you are looking for but are unsure which URI to read."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "domain": { "type": "string", "description": "Optional single domain filter such as 'core'." },
                "limit": { "type": "integer", "default": DEFAULT_SEARCH_LIMIT, "minimum": 1, "maximum": 100 }
            },
            "required": ["query"],
            "additionalProperties": false,
            "examples": [
                { "query": "what is the user's name" },
                { "query": "incident response", "domain": "core", "limit": 5 }
            ]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let query = require_str(&params, "query")?;
        let domain = optional_string(&params, "domain")?;
        let limit = params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_SEARCH_LIMIT as u64)
            .clamp(1, 100) as usize;

        let domains = domain.into_iter().collect::<Vec<_>>();
        let results = self
            .memory
            .search(&ctx.user_id, None, query, limit, &domains)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory search failed: {e}")))?;
        let rendered_results = results
            .iter()
            .map(|hit| {
                json!({
                    "node_id": hit.node_id,
                    "route_id": hit.route_id,
                    "version_id": hit.version_id,
                    "uri": hit.uri,
                    "title": hit.title,
                    "kind": hit.kind,
                    "content_snippet": hit.content_snippet,
                    "priority": hit.priority,
                    "trigger_text": hit.trigger_text,
                    "score": hit.score,
                    "fts_rank": hit.fts_rank,
                    "vector_rank": hit.vector_rank,
                    "is_hybrid_match": hit.fts_rank.is_some() && hit.vector_rank.is_some(),
                    "matched_keywords": hit.matched_keywords,
                    "updated_at": hit.updated_at,
                })
            })
            .collect::<Vec<_>>();

        Ok(ToolOutput::success(
            json!({
                "query": query,
                "domain": domains.first(),
                "limit": limit,
                "results": rendered_results,
                "count": results.len(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct ReadMemoryTool {
    memory: Arc<MemoryManager>,
}

impl ReadMemoryTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for ReadMemoryTool {
    fn name(&self) -> &str {
        "read_memory"
    }

    fn description(&self) -> &str {
        "Read a memory by URI. Supports special system URIs like system://boot, system://index, system://recent, and system://glossary."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" }
            },
            "required": ["uri"],
            "additionalProperties": false,
            "examples": [
                { "uri": "system://boot" },
                { "uri": "core://agent" }
            ]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let requested_uri = require_str(&params, "uri")?;

        if requested_uri == "system://boot" {
            let items = self
                .memory
                .boot_set(&ctx.user_id, None, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("boot read failed: {e}")))?;
            let recent_changes = self
                .memory
                .list_recent(&ctx.user_id, None, 5, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("boot recent read failed: {e}")))?;
            return Ok(ToolOutput::success(
                json!({
                    "uri": requested_uri,
                    "kind": "system_boot",
                    "items": items,
                    "recent_changes": recent_changes,
                }),
                start.elapsed(),
            ));
        }

        if requested_uri == "system://glossary" {
            let glossary = self
                .memory
                .glossary(&ctx.user_id, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("glossary read failed: {e}")))?;
            return Ok(ToolOutput::success(
                json!({
                    "uri": requested_uri,
                    "kind": "system_glossary",
                    "entries": glossary,
                }),
                start.elapsed(),
            ));
        }

        if requested_uri == "system://index" || requested_uri.starts_with("system://index/") {
            let domain = requested_uri
                .strip_prefix("system://index")
                .map(|suffix| suffix.trim_matches('/'))
                .filter(|suffix| !suffix.is_empty());
            let entries = self
                .memory
                .list_index(&ctx.user_id, None, domain)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("memory index failed: {e}")))?;
            return Ok(ToolOutput::success(
                json!({
                    "uri": requested_uri,
                    "kind": "system_index",
                    "domain": domain,
                    "entries": entries,
                }),
                start.elapsed(),
            ));
        }

        if requested_uri == "system://recent" || requested_uri.starts_with("system://recent/") {
            let limit = requested_uri
                .strip_prefix("system://recent")
                .map(|suffix| suffix.trim_matches('/'))
                .filter(|suffix| !suffix.is_empty())
                .and_then(|suffix| suffix.parse::<usize>().ok())
                .unwrap_or(10)
                .clamp(1, 100);

            let entries = self
                .memory
                .list_recent(&ctx.user_id, None, limit, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("recent read failed: {e}")))?;
            return Ok(ToolOutput::success(
                json!({
                    "uri": requested_uri,
                    "kind": "system_recent",
                    "limit": limit,
                    "entries": entries,
                }),
                start.elapsed(),
            ));
        }

        let (domain, path) = parse_non_root_uri(requested_uri)?;
        let canonical_uri = make_uri(&domain, &path);
        let detail = self
            .memory
            .get_node(&ctx.user_id, None, &canonical_uri)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory read failed: {e}")))?;
        let detail = detail.ok_or_else(|| {
            ToolError::ExecutionFailed(format!("memory at '{canonical_uri}' not found"))
        })?;

        let children = self
            .memory
            .children(&ctx.user_id, None, &canonical_uri, 200)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory child read failed: {e}")))?;
        let (selected_uri, priority, disclosure) = selected_route_metadata(&detail, &canonical_uri);
        let keywords = detail
            .keywords
            .iter()
            .map(|keyword| keyword.keyword.clone())
            .collect::<Vec<_>>();

        Ok(ToolOutput::success(
            json!({
                "uri": canonical_uri,
                "selected_uri": selected_uri,
                "memory_id": detail.node.id,
                "version_id": detail.active_version.id,
                "title": detail.node.title,
                "kind": detail.node.kind,
                "content": detail.active_version.content,
                "priority": priority,
                "disclosure": disclosure,
                "routes": detail.routes,
                "children": children,
                "keywords": keywords,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct CreateMemoryTool {
    memory: Arc<MemoryManager>,
}

impl CreateMemoryTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for CreateMemoryTool {
    fn name(&self) -> &str {
        "create_memory"
    }

    fn description(&self) -> &str {
        "Create a new memory under a parent node. Usually provide a semantic parent_uri, title, and disclosure; omit title only when you explicitly want numbered sibling nodes. Parent nodes should name the real concept, not vague containers like logs, misc, or history."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "parent_uri": { "type": "string", "description": "Parent URI. Use core:// for a domain root. The parent should express the real concept, not a time bucket or junk drawer." },
                "content": { "type": "string" },
                "priority": { "type": "integer", "description": "Priority (0 = highest; smaller numbers win)." },
                "title": { "type": "string", "description": "Optional route segment name (letters, numbers, '_' or '-'). Ordinary durable memory should almost always provide one; if omitted, a numeric sibling is assigned." },
                "disclosure": { "type": "string", "description": "Recall condition: describe when this memory should surface. User facts, self-model facts, agreements, preferences, and important lessons should usually include it." },
                "keywords": {
                    "type": "array",
                    "description": "Optional lateral recall keywords. Use these to cover paraphrases that disclosure or path names may miss.",
                    "items": { "type": "string" }
                }
            },
            "required": ["parent_uri", "content", "priority"],
            "additionalProperties": false,
            "examples": create_memory_examples()
        })
    }

    fn discovery_summary(&self) -> Option<ToolDiscoverySummary> {
        Some(create_memory_tool_summary())
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let parent_uri = require_str(&params, "parent_uri")?;
        let content = require_str(&params, "content")?;
        let priority = params
            .get("priority")
            .and_then(|value| value.as_i64())
            .ok_or_else(|| {
                ToolError::InvalidParameters("'priority' must be an integer".to_string())
            })? as i32;
        let title = optional_string(&params, "title")?;
        let disclosure = optional_string(&params, "disclosure")?;
        let keywords = optional_string_list(&params, "keywords")?.unwrap_or_default();

        if content.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "content must not be empty".to_string(),
            ));
        }

        let (domain, parent_path) = parse_uri(parent_uri)?;
        let canonical_parent = make_uri(&domain, &parent_path);

        let child_name = if let Some(title) = title.as_deref() {
            validate_title(title)?;
            title.to_string()
        } else {
            next_numeric_child_name(&self.memory, &ctx.user_id, &domain, &parent_path).await?
        };

        let path = if parent_path.is_empty() {
            child_name.clone()
        } else {
            format!("{parent_path}/{child_name}")
        };
        let uri = make_uri(&domain, &path);

        let (detail, changeset) = self
            .memory
            .create(
                &ctx.user_id,
                None,
                Some(&canonical_parent),
                &child_name,
                MemoryNodeKind::Reference,
                content,
                &domain,
                &path,
                priority,
                disclosure.clone(),
                crate::memory::MemoryVisibility::Private,
                keywords.clone(),
                json!({
                    "source": "tool:create_memory",
                    "job_id": ctx.job_id,
                }),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory create failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "uri": uri,
                "node": detail,
                "priority": priority,
                "disclosure": disclosure,
                "keywords": keywords,
                "changeset_id": changeset.id,
                "review_status": changeset.status,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct UpdateMemoryTool {
    memory: Arc<MemoryManager>,
}

impl UpdateMemoryTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for UpdateMemoryTool {
    fn name(&self) -> &str {
        "update_memory"
    }

    fn description(&self) -> &str {
        "Update an existing memory. Supports patch mode, append mode, and metadata updates such as priority, disclosure, and keywords. There is no full replace mode."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "old_string": { "type": "string", "description": "Patch mode: the exact text to replace. It must match uniquely inside the current content." },
                "new_string": { "type": "string", "description": "Patch mode: replacement text. Set to \"\" to delete the matched fragment." },
                "append": { "type": "string", "description": "Append mode: text to add to the end of the memory." },
                "priority": { "type": "integer", "description": "New priority." },
                "disclosure": { "type": "string", "description": "New disclosure / recall condition." },
                "keywords": {
                    "type": "array",
                    "description": "Optional full keyword set. If provided, it replaces the memory's existing keywords.",
                    "items": { "type": "string" }
                }
            },
            "required": ["uri"],
            "additionalProperties": false,
            "examples": [
                {
                    "uri": "core://agent/my_user",
                    "old_string": "old paragraph",
                    "new_string": "new paragraph"
                },
                {
                    "uri": "core://agent",
                    "append": "\\n## New Section\\nNew content..."
                }
            ]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let uri = require_str(&params, "uri")?;
        if is_virtual_system_view(uri) {
            return Err(ToolError::InvalidParameters(
                "virtual system views are read-only; update a concrete memory URI instead"
                    .to_string(),
            ));
        }

        let (domain, path) = parse_non_root_uri(uri)?;
        let canonical_uri = make_uri(&domain, &path);
        let old_string = optional_string(&params, "old_string")?;
        let new_string = optional_string(&params, "new_string")?;
        let append = optional_string(&params, "append")?;
        let priority = optional_priority(&params)?;
        let disclosure = optional_string(&params, "disclosure")?;
        let keywords = optional_string_list(&params, "keywords")?;

        if old_string.is_some() && append.is_some() {
            return Err(ToolError::InvalidParameters(
                "cannot use patch mode and append mode at the same time".to_string(),
            ));
        }
        if old_string.is_some() && new_string.is_none() {
            return Err(ToolError::InvalidParameters(
                "old_string provided without new_string".to_string(),
            ));
        }
        if new_string.is_some() && old_string.is_none() {
            return Err(ToolError::InvalidParameters(
                "new_string provided without old_string".to_string(),
            ));
        }

        let current = self
            .memory
            .get_node(&ctx.user_id, None, &canonical_uri)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory read failed: {e}")))?;
        let current = current.ok_or_else(|| {
            ToolError::ExecutionFailed(format!("memory at '{canonical_uri}' not found"))
        })?;

        let content = if let (Some(old), Some(new)) = (old_string.as_deref(), new_string.as_deref())
        {
            if old == new {
                return Err(ToolError::InvalidParameters(
                    "old_string and new_string are identical".to_string(),
                ));
            }
            let count = current.active_version.content.matches(old).count();
            if count == 0 {
                return Err(ToolError::ExecutionFailed(format!(
                    "old_string not found in memory content at '{canonical_uri}'"
                )));
            }
            if count > 1 {
                return Err(ToolError::ExecutionFailed(format!(
                    "old_string found {count} times in memory content at '{canonical_uri}'; provide more surrounding context"
                )));
            }
            Some(current.active_version.content.replacen(old, new, 1))
        } else if let Some(append) = append.as_deref() {
            if append.is_empty() {
                return Err(ToolError::InvalidParameters(
                    "append must not be empty".to_string(),
                ));
            }
            Some(format!("{}{}", current.active_version.content, append))
        } else {
            None
        };

        if content.is_none() && priority.is_none() && disclosure.is_none() && keywords.is_none() {
            return Err(ToolError::InvalidParameters(
                "no update fields provided; use patch mode, append mode, priority, disclosure, or keywords"
                    .to_string(),
            ));
        }

        let input = UpdateMemoryNodeInput {
            route_or_node: canonical_uri.clone(),
            content,
            priority,
            trigger_text: disclosure.clone().map(Some),
            keywords,
            metadata: Some(json!({
                "source": "tool:update_memory",
                "job_id": ctx.job_id,
            })),
            ..Default::default()
        };
        let (detail, changeset) = self
            .memory
            .update(&ctx.user_id, None, &input)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory update failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "uri": canonical_uri,
                "node": detail,
                "changeset_id": changeset.id,
                "review_status": changeset.status,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct DeleteMemoryTool {
    memory: Arc<MemoryManager>,
}

impl DeleteMemoryTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for DeleteMemoryTool {
    fn name(&self) -> &str {
        "delete_memory"
    }

    fn description(&self) -> &str {
        "Delete a memory route and its descendant routes without deleting the underlying content body. Read it first so you know what you are removing."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" }
            },
            "required": ["uri"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let uri = require_str(&params, "uri")?;
        if is_virtual_system_view(uri) {
            return Err(ToolError::InvalidParameters(
                "virtual system views cannot be deleted".to_string(),
            ));
        }

        let (domain, path) = parse_non_root_uri(uri)?;
        let canonical_uri = make_uri(&domain, &path);
        let changeset = self
            .memory
            .delete_memory(&ctx.user_id, None, &canonical_uri)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory delete failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "uri": canonical_uri,
                "changeset_id": changeset.id,
                "review_status": changeset.status,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}

pub struct AddAliasTool {
    memory: Arc<MemoryManager>,
}

impl AddAliasTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for AddAliasTool {
    fn name(&self) -> &str {
        "add_alias"
    }

    fn description(&self) -> &str {
        "Create an alias route for an existing memory. This is not a copy; it is a new access path to the same content. The new parent path must already exist."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "new_uri": { "type": "string" },
                "target_uri": { "type": "string" },
                "priority": { "type": "integer", "default": DEFAULT_CREATE_PRIORITY, "description": "Independent priority for this alias route." },
                "disclosure": { "type": "string", "description": "Independent disclosure / recall condition for this alias route." }
            },
            "required": ["new_uri", "target_uri"],
            "additionalProperties": false,
            "examples": [
                {
                    "new_uri": "core://timeline/2024/05/20",
                    "target_uri": "core://agent/my_user/first_meeting",
                    "priority": 1,
                    "disclosure": "When I want to remember how we started"
                }
            ]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let new_uri = require_str(&params, "new_uri")?;
        let target_uri = require_str(&params, "target_uri")?;
        let priority = optional_priority(&params)?.unwrap_or(DEFAULT_CREATE_PRIORITY);
        let disclosure = optional_string(&params, "disclosure")?;

        if is_virtual_system_view(new_uri) || is_virtual_system_view(target_uri) {
            return Err(ToolError::InvalidParameters(
                "virtual system views cannot be used as alias endpoints".to_string(),
            ));
        }

        let (new_domain, new_path) = parse_non_root_uri(new_uri)?;
        let (target_domain, target_path) = parse_non_root_uri(target_uri)?;
        let input = CreateMemoryAliasInput {
            space_id: uuid::Uuid::nil(),
            target_route_or_node: make_uri(&target_domain, &target_path),
            domain: new_domain.clone(),
            path: new_path.clone(),
            visibility: crate::memory::MemoryVisibility::Private,
            priority,
            trigger_text: disclosure.clone(),
            changeset_id: None,
        };

        let (route, changeset) = self
            .memory
            .add_alias(&ctx.user_id, None, &input)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory alias failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "uri": route.uri(),
                "target_uri": make_uri(&target_domain, &target_path),
                "priority": priority,
                "disclosure": disclosure,
                "changeset_id": changeset.id,
                "review_status": changeset.status,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct ManageBootTool {
    memory: Arc<MemoryManager>,
}

impl ManageBootTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for ManageBootTool {
    fn name(&self) -> &str {
        "manage_boot"
    }

    fn description(&self) -> &str {
        "Add or remove a durable memory from the explicit boot set, and optionally control its boot load priority."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "action": {
                    "type": "string",
                    "enum": ["add", "remove"]
                },
                "priority": {
                    "type": "integer",
                    "description": "Used only when action=add. Boot load priority (0 = highest)."
                }
            },
            "required": ["uri", "action"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let uri = require_str(&params, "uri")?;
        let action = require_str(&params, "action")?;
        let priority = optional_priority(&params)?.unwrap_or(DEFAULT_CREATE_PRIORITY);

        let result = match action {
            "add" => {
                let route = self
                    .memory
                    .add_to_boot(&ctx.user_id, None, uri, priority)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("boot update failed: {e}")))?;
                json!({
                    "uri": route.uri(),
                    "action": "add",
                    "priority": priority,
                })
            }
            "remove" => {
                self.memory
                    .remove_from_boot(&ctx.user_id, None, uri)
                    .await
                    .map_err(|e| ToolError::ExecutionFailed(format!("boot update failed: {e}")))?;
                json!({
                    "uri": uri,
                    "action": "remove",
                })
            }
            _ => {
                return Err(ToolError::InvalidParameters(
                    "'action' must be 'add' or 'remove'".to_string(),
                ));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct ManageTriggersTool {
    memory: Arc<MemoryManager>,
}

impl ManageTriggersTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for ManageTriggersTool {
    fn name(&self) -> &str {
        "manage_triggers"
    }

    fn description(&self) -> &str {
        "Add or remove recall keywords for a memory node, and optionally update its disclosure to strengthen lateral recall coverage."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "add": { "type": "array", "items": { "type": "string" } },
                "remove": { "type": "array", "items": { "type": "string" } },
                "disclosure": {
                    "description": "Optional new disclosure. Pass null to clear it.",
                    "anyOf": [
                        { "type": "string" },
                        { "type": "null" }
                    ]
                }
            },
            "required": ["uri"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let uri = require_str(&params, "uri")?;
        let add = optional_string_list(&params, "add")?.unwrap_or_default();
        let remove = optional_string_list(&params, "remove")?.unwrap_or_default();
        let disclosure = match params.get("disclosure") {
            Some(serde_json::Value::Null) => Some(None),
            Some(_) => optional_string(&params, "disclosure")?.map(Some),
            None => None,
        };

        if add.is_empty() && remove.is_empty() && disclosure.is_none() {
            return Err(ToolError::InvalidParameters(
                "provide at least one of add, remove, or disclosure".to_string(),
            ));
        }

        let detail = self
            .memory
            .manage_triggers(&ctx.user_id, None, uri, &add, &remove, disclosure)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("trigger update failed: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "uri": uri,
                "node": detail,
                "added": add,
                "removed": remove,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct ExplainMemoryRecallTool {
    memory: Arc<MemoryManager>,
}

impl ExplainMemoryRecallTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for ExplainMemoryRecallTool {
    fn name(&self) -> &str {
        "explain_memory_recall"
    }

    fn description(&self) -> &str {
        "Explain how a query matched across boot, trigger hits, hybrid search, graph expansion, and recent episodes."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "group_chat": { "type": "boolean", "default": false }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let query = require_str(&params, "query")?;
        let group_chat = params
            .get("group_chat")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let explanation = self
            .memory
            .explain_recall(&ctx.user_id, None, query, group_chat)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("recall explanation failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::to_value(explanation).map_err(|e| {
                ToolError::ExecutionFailed(format!("recall explanation serialize failed: {e}"))
            })?,
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestHarnessBuilder;
    use crate::tools::Tool;

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn create_memory_schema_matches_nocturne_shape() {
        let harness = TestHarnessBuilder::new().build().await;
        let tool = CreateMemoryTool::new(Arc::new(MemoryManager::new(Arc::clone(&harness.db))));
        let schema = tool.parameters_schema();
        let errors = crate::tools::tool::validate_tool_schema(&schema, "create_memory");
        assert!(
            errors.is_empty(),
            "create_memory schema should validate cleanly: {errors:?}",
        );

        let properties = schema["properties"]
            .as_object()
            .expect("create_memory schema should have properties");
        assert!(properties.contains_key("parent_uri"));
        assert!(properties.contains_key("content"));
        assert!(properties.contains_key("priority"));
        assert!(properties.contains_key("title"));
        assert!(properties.contains_key("disclosure"));
        assert!(properties.contains_key("keywords"));
        assert!(!properties.contains_key("route"));
        assert!(!properties.contains_key("kind"));
        assert!(!properties.contains_key("visibility"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn create_memory_without_title_uses_numeric_child_route() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "Root parent",
                    "priority": 0,
                    "title": "agent"
                }),
                &ctx,
            )
            .await
            .expect("parent creation should succeed");

        let result = create
            .execute(
                json!({
                    "parent_uri": "core://agent",
                    "content": "The user's name is 梦凌汐.",
                    "priority": 1
                }),
                &ctx,
            )
            .await
            .expect("titleless child creation should succeed");

        assert_eq!(result.result["uri"], json!("core://agent/1"));

        let detail = memory
            .get_node("user1", None, "core://agent/1")
            .await
            .expect("open numeric route")
            .expect("numeric route should exist");
        assert!(detail.active_version.content.contains("梦凌汐"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn update_memory_supports_patch_and_append_only() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let update = UpdateMemoryTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "Original content",
                    "priority": 0,
                    "title": "agent"
                }),
                &ctx,
            )
            .await
            .expect("create should succeed");

        update
            .execute(
                json!({
                    "uri": "core://agent",
                    "old_string": "Original",
                    "new_string": "Updated"
                }),
                &ctx,
            )
            .await
            .expect("patch should succeed");

        update
            .execute(
                json!({
                    "uri": "core://agent",
                    "append": "\nExtra note"
                }),
                &ctx,
            )
            .await
            .expect("append should succeed");

        let detail = memory
            .get_node("user1", None, "core://agent")
            .await
            .expect("open updated node")
            .expect("node should exist");
        assert!(detail.active_version.content.contains("Updated content"));
        assert!(detail.active_version.content.contains("Extra note"));

        let err = update
            .execute(json!({ "uri": "core://agent" }), &ctx)
            .await
            .expect_err("empty update should fail");
        assert!(format!("{err}").contains("no update fields provided"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn create_memory_duplicate_path_fails() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "First version",
                    "priority": 0,
                    "title": "user_name"
                }),
                &ctx,
            )
            .await
            .expect("first create should succeed");

        let err = create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "Second version",
                    "priority": 0,
                    "title": "user_name"
                }),
                &ctx,
            )
            .await
            .expect_err("duplicate create should fail");
        assert!(format!("{err}").contains("already exists"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn create_memory_auto_scaffolds_missing_parent_chain() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://user/profile",
                    "content": "模板：用户名字是<user_name>。",
                    "priority": 0,
                    "title": "name",
                    "disclosure": "模板：当我需要正确称呼用户时"
                }),
                &ctx,
            )
            .await
            .expect("create with missing semantic parents should succeed");

        let user_root = memory
            .get_node("user1", None, "core://user")
            .await
            .expect("open scaffolded user root")
            .expect("user root should exist");
        assert_eq!(user_root.node.kind, MemoryNodeKind::Curated);
        assert_eq!(
            user_root
                .node
                .metadata
                .get("source")
                .and_then(|value| value.as_str()),
            Some("tool:auto_scaffold_parent")
        );

        let profile_root = memory
            .get_node("user1", None, "core://user/profile")
            .await
            .expect("open scaffolded profile root")
            .expect("profile root should exist");
        assert_eq!(profile_root.node.kind, MemoryNodeKind::Curated);

        let detail = memory
            .get_node("user1", None, "core://user/profile/name")
            .await
            .expect("open final leaf")
            .expect("leaf should exist");
        assert!(detail.active_version.content.contains("<user_name>"));
    }

    #[test]
    fn create_memory_examples_use_placeholders_not_real_facts() {
        let rendered =
            serde_json::to_string(&create_memory_examples()).expect("serialize examples");
        assert!(!rendered.contains("梦凌汐"));
        assert!(!rendered.contains("钦灵"));
        assert!(rendered.contains("<user_name>"));
        assert!(rendered.contains("<assistant_name>"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn add_alias_requires_parent_and_cascades_descendants() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let add_alias = AddAliasTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "Agent root",
                    "priority": 0,
                    "title": "agent"
                }),
                &ctx,
            )
            .await
            .expect("create root should succeed");
        create
            .execute(
                json!({
                    "parent_uri": "core://agent",
                    "content": "Profile root",
                    "priority": 0,
                    "title": "profile"
                }),
                &ctx,
            )
            .await
            .expect("create child should succeed");
        create
            .execute(
                json!({
                    "parent_uri": "core://agent/profile",
                    "content": "用户名字是梦凌汐。",
                    "priority": 0,
                    "title": "name"
                }),
                &ctx,
            )
            .await
            .expect("create leaf should succeed");

        let err = add_alias
            .execute(
                json!({
                    "new_uri": "core://my_user/name",
                    "target_uri": "core://agent/profile"
                }),
                &ctx,
            )
            .await
            .expect_err("missing alias parent should fail");
        assert!(format!("{err}").contains("core://my_user"));

        create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "Mirror root",
                    "priority": 0,
                    "title": "my_user"
                }),
                &ctx,
            )
            .await
            .expect("create alias parent should succeed");

        add_alias
            .execute(
                json!({
                    "new_uri": "core://my_user/profile",
                    "target_uri": "core://agent/profile"
                }),
                &ctx,
            )
            .await
            .expect("alias should succeed");

        let aliased_child = memory
            .get_node("user1", None, "core://my_user/profile/name")
            .await
            .expect("open cascaded alias child")
            .expect("cascaded alias child should exist");
        assert!(aliased_child.active_version.content.contains("梦凌汐"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn delete_memory_removes_path_subtree_only() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let add_alias = AddAliasTool::new(Arc::clone(&memory));
        let delete = DeleteMemoryTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "Agent root",
                    "priority": 0,
                    "title": "agent"
                }),
                &ctx,
            )
            .await
            .expect("create root should succeed");
        create
            .execute(
                json!({
                    "parent_uri": "core://agent",
                    "content": "Profile root",
                    "priority": 0,
                    "title": "profile"
                }),
                &ctx,
            )
            .await
            .expect("create child should succeed");
        create
            .execute(
                json!({
                    "parent_uri": "core://agent/profile",
                    "content": "用户名字是梦凌汐。",
                    "priority": 0,
                    "title": "name"
                }),
                &ctx,
            )
            .await
            .expect("create leaf should succeed");
        create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "Mirror root",
                    "priority": 0,
                    "title": "my_user"
                }),
                &ctx,
            )
            .await
            .expect("create alias parent should succeed");
        add_alias
            .execute(
                json!({
                    "new_uri": "core://my_user/profile",
                    "target_uri": "core://agent/profile"
                }),
                &ctx,
            )
            .await
            .expect("alias should succeed");

        delete
            .execute(json!({ "uri": "core://my_user/profile" }), &ctx)
            .await
            .expect("delete subtree should succeed");

        assert!(
            memory
                .get_node("user1", None, "core://my_user/profile")
                .await
                .expect("read deleted alias")
                .is_none()
        );
        assert!(
            memory
                .get_node("user1", None, "core://my_user/profile/name")
                .await
                .expect("read deleted alias child")
                .is_none()
        );
        assert!(
            memory
                .get_node("user1", None, "core://agent/profile/name")
                .await
                .expect("read original child")
                .is_some()
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn search_and_read_memory_use_nocturne_names() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let read = ReadMemoryTool::new(Arc::clone(&memory));
        let search = SearchMemoryTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://",
                    "content": "The user's name is 梦凌汐.",
                    "priority": 0,
                    "title": "user_name"
                }),
                &ctx,
            )
            .await
            .expect("create should succeed");

        let read_result = read
            .execute(json!({ "uri": "core://user_name" }), &ctx)
            .await
            .expect("read should succeed");
        assert_eq!(read_result.result["uri"], json!("core://user_name"));

        let search_result = search
            .execute(json!({ "query": "梦凌汐" }), &ctx)
            .await
            .expect("search should succeed");
        assert_eq!(search_result.result["count"], json!(1));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn manage_boot_and_manage_triggers_update_recall_structure() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let manage_boot = ManageBootTool::new(Arc::clone(&memory));
        let manage_triggers = ManageTriggersTool::new(Arc::clone(&memory));
        let read = ReadMemoryTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://user",
                    "content": "用户名字是梦凌汐。",
                    "priority": 0,
                    "title": "name"
                }),
                &ctx,
            )
            .await
            .expect("create should succeed");

        manage_triggers
            .execute(
                json!({
                    "uri": "core://user/name",
                    "add": ["梦凌汐", "名字"],
                    "disclosure": "当我需要正确称呼用户时"
                }),
                &ctx,
            )
            .await
            .expect("manage triggers should succeed");

        manage_boot
            .execute(
                json!({
                    "uri": "core://user/name",
                    "action": "add",
                    "priority": 1
                }),
                &ctx,
            )
            .await
            .expect("manage boot should succeed");

        let glossary = read
            .execute(json!({ "uri": "system://glossary" }), &ctx)
            .await
            .expect("read glossary should succeed");
        assert!(
            glossary.result["entries"]
                .as_array()
                .expect("glossary entries")
                .iter()
                .any(|entry| entry["keyword"] == "梦凌汐")
        );

        let boot = read
            .execute(json!({ "uri": "system://boot" }), &ctx)
            .await
            .expect("read boot should succeed");
        assert!(
            boot.result["items"]
                .as_array()
                .expect("boot items")
                .iter()
                .any(|entry| entry["primary_route"]["path"] == "user/name")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn explain_memory_recall_reports_stage_breakdown() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let create = CreateMemoryTool::new(Arc::clone(&memory));
        let explain = ExplainMemoryRecallTool::new(Arc::clone(&memory));
        let manage_boot = ManageBootTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        create
            .execute(
                json!({
                    "parent_uri": "core://user",
                    "content": "用户名字是梦凌汐。",
                    "priority": 0,
                    "title": "name",
                    "keywords": ["梦凌汐", "名字"]
                }),
                &ctx,
            )
            .await
            .expect("create should succeed");

        manage_boot
            .execute(
                json!({
                    "uri": "core://user/name",
                    "action": "add",
                    "priority": 1
                }),
                &ctx,
            )
            .await
            .expect("manage boot should succeed");

        let result = explain
            .execute(json!({ "query": "你记得我叫什么吗" }), &ctx)
            .await
            .expect("explain recall should succeed");

        assert_eq!(result.result["query"], json!("你记得我叫什么吗"));
        assert!(
            result.result["boot"]
                .as_array()
                .expect("boot entries")
                .iter()
                .any(|entry| entry["uri"] == "core://user/name")
        );
        let in_relevant = result.result["relevant"]
            .as_array()
            .expect("relevant entries")
            .iter()
            .any(|entry| entry["uri"] == "core://user/name");
        assert!(
            in_relevant
                || result.result["boot"]
                    .as_array()
                    .expect("boot entries")
                    .iter()
                    .any(|entry| entry["uri"] == "core://user/name")
        );
    }
}
