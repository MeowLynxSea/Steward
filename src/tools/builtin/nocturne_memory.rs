//! Nocturne-compatible memory tools.
//!
//! These provide the Nocturne Memory MCP-style CRUD surface (read/create/update/delete/alias/search)
//! plus trigger management, backed by Steward's native graph memory.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::context::JobContext;
use crate::memory::{
    CreateMemoryAliasInput, MemoryChildEntry, MemoryGlossaryEntry, MemoryIndexEntry, MemoryManager,
    MemoryNodeKind, MemoryVisibility, UpdateMemoryNodeInput,
};
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

fn parse_kind(value: &str) -> Result<MemoryNodeKind, ToolError> {
    match value {
        "boot" => Ok(MemoryNodeKind::Boot),
        "identity" => Ok(MemoryNodeKind::Identity),
        "value" => Ok(MemoryNodeKind::Value),
        "user_profile" => Ok(MemoryNodeKind::UserProfile),
        "directive" => Ok(MemoryNodeKind::Directive),
        "curated" => Ok(MemoryNodeKind::Curated),
        "episode" => Ok(MemoryNodeKind::Episode),
        "procedure" => Ok(MemoryNodeKind::Procedure),
        "reference" => Ok(MemoryNodeKind::Reference),
        other => Err(ToolError::InvalidParameters(format!(
            "unsupported memory kind: {other}"
        ))),
    }
}

fn kind_enum_values() -> Vec<&'static str> {
    vec![
        "boot",
        "identity",
        "value",
        "user_profile",
        "directive",
        "curated",
        "episode",
        "procedure",
        "reference",
    ]
}

fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    let mut last_dash = false;
    for ch in value.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn parse_parent_uri(uri: &str) -> Result<(String, String), ToolError> {
    let (domain, path) = uri.split_once("://").ok_or_else(|| {
        ToolError::InvalidParameters(format!(
            "expected uri in the form domain://path (or domain://), got: {uri}"
        ))
    })?;
    if domain.trim().is_empty() {
        return Err(ToolError::InvalidParameters(format!(
            "uri must include a domain: {uri}"
        )));
    }
    Ok((domain.to_string(), path.trim_matches('/').to_string()))
}

fn parse_uri_strict(uri: &str) -> Result<(String, String), ToolError> {
    let (domain, path) = uri.split_once("://").ok_or_else(|| {
        ToolError::InvalidParameters(format!(
            "expected uri in the form domain://path, got: {uri}"
        ))
    })?;
    if domain.trim().is_empty() || path.trim().is_empty() {
        return Err(ToolError::InvalidParameters(format!(
            "uri must include both a domain and a path: {uri}"
        )));
    }
    Ok((domain.to_string(), path.trim_matches('/').to_string()))
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

fn parse_visibility(params: &serde_json::Value) -> Result<MemoryVisibility, ToolError> {
    match params
        .get("visibility")
        .and_then(|v| v.as_str())
        .unwrap_or("private")
    {
        "private" => Ok(MemoryVisibility::Private),
        "session" => Ok(MemoryVisibility::Session),
        "shared" => Ok(MemoryVisibility::Shared),
        other => Err(ToolError::InvalidParameters(format!(
            "unsupported visibility: {other}"
        ))),
    }
}

fn parse_visibility_value(value: Option<&str>) -> Result<MemoryVisibility, ToolError> {
    match value.unwrap_or("private") {
        "private" => Ok(MemoryVisibility::Private),
        "session" => Ok(MemoryVisibility::Session),
        "shared" => Ok(MemoryVisibility::Shared),
        other => Err(ToolError::InvalidParameters(format!(
            "unsupported visibility: {other}"
        ))),
    }
}

fn parse_priority(params: &serde_json::Value, default_value: i32) -> Result<i32, ToolError> {
    match params.get("priority") {
        Some(v) => v.as_i64().map(|n| n as i32).ok_or_else(|| {
            ToolError::InvalidParameters("'priority' must be an integer".to_string())
        }),
        None => Ok(default_value),
    }
}

fn parse_string_list(params: &serde_json::Value, key: &str) -> Result<Vec<String>, ToolError> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    let array = value.as_array().ok_or_else(|| {
        ToolError::InvalidParameters(format!("'{key}' must be an array of strings"))
    })?;
    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    ToolError::InvalidParameters(format!(
                        "'{key}' must contain only non-empty strings"
                    ))
                })
        })
        .collect()
}

// (Intentionally no system-uri split helper: system:// URIs are handled inline.)

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
        "Read a memory URI from Steward's graph memory. Supports virtual system:// URIs (boot, index, recent, glossary)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" }
            },
            "required": ["uri"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let uri = require_str(&params, "uri")?;

        // Virtual system URIs
        if uri == "system://boot" {
            let boot = self
                .memory
                .boot_set(&ctx.user_id, None, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("boot read failed: {e}")))?;
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "uri": uri,
                    "kind": "system_boot",
                    "items": boot,
                }),
                start.elapsed(),
            ));
        }

        if uri == "system://glossary" {
            let glossary: Vec<MemoryGlossaryEntry> = self
                .memory
                .glossary(&ctx.user_id, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("glossary failed: {e}")))?;
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "uri": uri,
                    "kind": "system_glossary",
                    "glossary": glossary,
                }),
                start.elapsed(),
            ));
        }

        if uri.starts_with("system://index") {
            let domain = uri.strip_prefix("system://index").and_then(|rest| {
                let rest = rest.trim_matches('/');
                if rest.is_empty() {
                    None
                } else {
                    Some(rest)
                }
            });
            let index: Vec<MemoryIndexEntry> = self
                .memory
                .list_index(&ctx.user_id, None, domain)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("index failed: {e}")))?;
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "uri": uri,
                    "kind": "system_index",
                    "domain": domain,
                    "entries": index,
                }),
                start.elapsed(),
            ));
        }

        if uri.starts_with("system://recent") {
            let limit = uri
                .strip_prefix("system://recent")
                .and_then(|rest| rest.trim_matches('/').parse::<usize>().ok())
                .unwrap_or(10)
                .clamp(1, 200);
            let recent = self
                .memory
                .list_recent(&ctx.user_id, None, limit, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("recent failed: {e}")))?;
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "uri": uri,
                    "kind": "system_recent",
                    "limit": limit,
                    "entries": recent,
                }),
                start.elapsed(),
            ));
        }

        // Normal node read
        let detail = self
            .memory
            .open(&ctx.user_id, None, uri)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory open failed: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("memory not found: {uri}")))?;

        let children: Vec<MemoryChildEntry> = self
            .memory
            .children(&ctx.user_id, None, uri, 50)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("children failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "uri": uri,
                "node": detail.node,
                "version": detail.active_version,
                "content": detail.active_version.content,
                "routes": detail.routes,
                "edges": detail.edges,
                "keywords": detail.keywords,
                "children": children,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
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
        "Create a new memory node under a parent URI (Nocturne-compatible)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let kind_values = kind_enum_values();
        serde_json::json!({
            "type": "object",
            "properties": {
                "parent_uri": { "type": "string", "description": "Parent URI to derive a child uri from (domain://path or domain://). Provide this OR 'uri'." },
                "uri": { "type": "string", "description": "Explicit root uri to create (domain://path). Provide this OR 'parent_uri'." },
                "content": { "type": "string" },
                "priority": { "type": "integer", "default": 50 },
                "title": { "type": "string" },
                "disclosure": { "type": ["string", "null"] },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] },
                "keywords": { "type": "array", "items": { "type": "string" } },
                "kind": { "type": "string", "enum": kind_values, "description": "Optional node kind. Defaults to 'reference'. Use 'user_profile' for stable user facts like the user's name." }
            },
            "anyOf": [
                { "required": ["uri"] },
                { "required": ["parent_uri"] }
            ],
            "required": ["content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let content = require_str(&params, "content")?;
        let priority = parse_priority(&params, 50)?;
        let kind = params
            .get("kind")
            .and_then(|v| v.as_str())
            .map(parse_kind)
            .transpose()?
            .unwrap_or(MemoryNodeKind::Reference);
        let disclosure = optional_string(&params, "disclosure")?;
        let visibility = parse_visibility(&params)?;
        let keywords = parse_string_list(&params, "keywords")?;

        let title = optional_string(&params, "title")?
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| Utc::now().format("memory-%Y%m%d-%H%M%S").to_string());

        let (parent_uri, domain, path) = if let Some(uri) = optional_string(&params, "uri")? {
            let (domain, path) = parse_uri_strict(&uri)?;
            (None, domain, path)
        } else {
            let parent_uri = require_str(&params, "parent_uri")?;
            let (domain, parent_path) = parse_parent_uri(parent_uri)?;
            let slug = slugify(&title);
            let path = if parent_path.is_empty() {
                slug
            } else {
                format!("{parent_path}/{slug}")
            };
            (Some(parent_uri.to_string()), domain, path)
        };

        let (detail, changeset) = self
            .memory
            .create(
                &ctx.user_id,
                None,
                parent_uri.as_deref(),
                &title,
                kind,
                content,
                &domain,
                &path,
                priority,
                disclosure,
                visibility,
                keywords,
                serde_json::json!({
                    "source": "tool:create_memory",
                    "job_id": ctx.job_id,
                }),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("create_memory failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "uri": format!("{domain}://{path}"),
                "node": detail,
                "changeset_id": changeset.id,
                "status": changeset.status,
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
        "Update a memory via patch/append (Nocturne-compatible)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let kind_values = kind_enum_values();
        serde_json::json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "expected_version_id": { "type": "string", "description": "If provided, the update will fail unless the current active version id matches." },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "append": { "type": "string" },
                "priority": { "type": "integer" },
                "disclosure": { "type": ["string", "null"] },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] },
                "keywords": { "type": "array", "items": { "type": "string" } },
                "title": { "type": "string" },
                "kind": { "type": "string", "enum": kind_values }
            },
            "required": ["uri"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let uri = require_str(&params, "uri")?;

        if uri.starts_with("system://") {
            return Err(ToolError::InvalidParameters(
                "system:// URIs are virtual entry points (read-only). Update a concrete memory URI like system://boot/memory_protocol (or a core://... node) instead."
                    .to_string(),
            ));
        }

        let detail = self
            .memory
            .open(&ctx.user_id, None, uri)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory open failed: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("memory not found: {uri}")))?;

        if let Some(expected) = params.get("expected_version_id").and_then(|v| v.as_str()) {
            let expected = Uuid::parse_str(expected).map_err(|_| {
                ToolError::InvalidParameters(
                    "'expected_version_id' must be a valid UUID string".to_string(),
                )
            })?;
            if expected != detail.active_version.id {
                return Err(ToolError::ExecutionFailed(format!(
                    "version mismatch: expected {expected}, current {}",
                    detail.active_version.id
                )));
            }
        }

        let old_string = optional_string(&params, "old_string")?;
        let new_string = optional_string(&params, "new_string")?;
        let append = optional_string(&params, "append")?;

        let content = match (old_string, new_string, append) {
            (Some(old), Some(new), None) => {
                let current = &detail.active_version.content;
                let count = current.matches(&old).count();
                if count != 1 {
                    return Err(ToolError::ExecutionFailed(format!(
                        "patch requires old_string to match exactly once (matched {count} times)"
                    )));
                }
                Some(current.replacen(&old, &new, 1))
            }
            (Some(_), None, _) => {
                return Err(ToolError::InvalidParameters(
                    "patch mode requires both old_string and new_string".to_string(),
                ));
            }
            (None, None, Some(text)) => {
                let mut current = detail.active_version.content.clone();
                if !current.ends_with('\n') {
                    current.push('\n');
                }
                current.push('\n');
                current.push_str(&text);
                Some(current)
            }
            (None, None, None) => None,
            _ => {
                return Err(ToolError::InvalidParameters(
                    "update modes are mutually exclusive: use patch (old_string/new_string) or append".to_string(),
                ));
            }
        };

        let input = UpdateMemoryNodeInput {
            route_or_node: uri.to_string(),
            content,
            priority: params.get("priority").and_then(|v| v.as_i64()).map(|v| v as i32),
            title: optional_string(&params, "title")?,
            kind: params
                .get("kind")
                .and_then(|v| v.as_str())
                .map(parse_kind)
                .transpose()?,
            trigger_text: if params.get("disclosure").is_some() {
                match params.get("disclosure") {
                    Some(serde_json::Value::Null) => Some(None),
                    Some(_) => Some(optional_string(&params, "disclosure")?),
                    None => None,
                }
            } else {
                None
            },
            visibility: params
                .get("visibility")
                .and_then(|v| v.as_str())
                .map(|v| parse_visibility_value(Some(v)))
                .transpose()?,
            keywords: if params.get("keywords").is_some() {
                Some(parse_string_list(&params, "keywords")?)
            } else {
                None
            },
            metadata: Some(serde_json::json!({
                "source": "tool:update_memory",
                "job_id": ctx.job_id,
            })),
            ..Default::default()
        };

        let (updated, changeset) = self
            .memory
            .update(&ctx.user_id, None, &input)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("update_memory failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "uri": uri,
                "node": updated,
                "changeset_id": changeset.id,
                "status": changeset.status,
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
        "Delete a memory URI path (Nocturne-compatible path deletion)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" }
            },
            "required": ["uri"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let uri = require_str(&params, "uri")?;
        let _ = parse_uri_strict(uri)?;

        let changeset = self
            .memory
            .delete(&ctx.user_id, None, uri)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("delete_memory failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "uri": uri,
                "changeset_id": changeset.id,
                "status": changeset.status,
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
        "Add an alias route for a target memory URI (Nocturne-compatible)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "new_uri": { "type": "string" },
                "target_uri": { "type": "string" },
                "priority": { "type": "integer" },
                "disclosure": { "type": ["string", "null"] },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] }
            },
            "required": ["new_uri", "target_uri"]
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
        let (domain, path) = parse_uri_strict(new_uri)?;
        let priority = parse_priority(&params, 50)?;
        let disclosure = optional_string(&params, "disclosure")?;
        let visibility = parse_visibility(&params)?;

        let input = CreateMemoryAliasInput {
            space_id: Uuid::nil(),
            target_route_or_node: target_uri.to_string(),
            domain,
            path,
            visibility,
            priority,
            trigger_text: disclosure,
            changeset_id: None,
        };

        let (route, changeset) = self
            .memory
            .alias(&ctx.user_id, None, &input)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("add_alias failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "new_uri": new_uri,
                "target_uri": target_uri,
                "route": route,
                "changeset_id": changeset.id,
                "status": changeset.status,
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
        "Search memory by keyword (FTS), not semantic similarity (Nocturne-compatible)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "domain": { "type": "string" },
                "limit": { "type": "integer", "default": 10, "minimum": 1, "maximum": 50 }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let query = require_str(&params, "query")?;
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .clamp(1, 50) as usize;
        let domains = params
            .get("domain")
            .and_then(|v| v.as_str())
            .map(|d| vec![d.to_string()])
            .unwrap_or_default();

        let results = self
            .memory
            .search(&ctx.user_id, None, query, limit, &domains)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("search_memory failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "query": query,
                "limit": limit,
                "results": results,
                "result_count": results.len(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
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
        "Bind or unbind trigger keywords to a memory node (Nocturne-compatible)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "add": { "type": "array", "items": { "type": "string" } },
                "remove": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["uri"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let uri = require_str(&params, "uri")?;

        let detail = self
            .memory
            .open(&ctx.user_id, None, uri)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory open failed: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("memory not found: {uri}")))?;

        let add = parse_string_list(&params, "add")?;
        let remove = parse_string_list(&params, "remove")?;

        let mut keywords = detail
            .keywords
            .iter()
            .map(|k| k.keyword.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect::<HashSet<_>>();

        for k in add {
            keywords.insert(k);
        }
        for k in remove {
            keywords.remove(&k);
        }

        let mut list = keywords.into_iter().collect::<Vec<_>>();
        list.sort();

        let input = UpdateMemoryNodeInput {
            route_or_node: uri.to_string(),
            keywords: Some(list.clone()),
            metadata: Some(serde_json::json!({
                "source": "tool:manage_triggers",
                "job_id": ctx.job_id,
            })),
            ..Default::default()
        };

        let (_updated, changeset) = self
            .memory
            .update(&ctx.user_id, None, &input)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("manage_triggers failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "uri": uri,
                "keywords": list,
                "changeset_id": changeset.id,
                "status": changeset.status,
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
