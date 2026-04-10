//! Graph-native tools for Steward's native memory system.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use uuid::Uuid;

use crate::context::JobContext;
use crate::memory::{
    CreateMemoryAliasInput, MemoryManager, MemoryNodeKind, MemoryVisibility, UpdateMemoryNodeInput,
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

fn parse_visibility(value: Option<&str>) -> Result<MemoryVisibility, ToolError> {
    match value.unwrap_or("private") {
        "private" => Ok(MemoryVisibility::Private),
        "session" => Ok(MemoryVisibility::Session),
        "shared" => Ok(MemoryVisibility::Shared),
        other => Err(ToolError::InvalidParameters(format!(
            "unsupported memory visibility: {other}"
        ))),
    }
}

fn parse_keywords(params: &serde_json::Value) -> Result<Vec<String>, ToolError> {
    let Some(value) = params.get("keywords") else {
        return Ok(Vec::new());
    };
    let array = value.as_array().ok_or_else(|| {
        ToolError::InvalidParameters("'keywords' must be an array of strings".to_string())
    })?;
    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    ToolError::InvalidParameters(
                        "'keywords' must contain only non-empty strings".to_string(),
                    )
                })
        })
        .collect()
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

fn parse_route_strict(route: &str) -> Result<(String, String), ToolError> {
    let (domain, path) = route.split_once("://").ok_or_else(|| {
        ToolError::InvalidParameters(format!(
            "expected route in the form domain://path, got: {route}"
        ))
    })?;
    if domain.trim().is_empty() || path.trim().is_empty() {
        return Err(ToolError::InvalidParameters(format!(
            "route must include both a domain and a path: {route}"
        )));
    }
    Ok((domain.to_string(), path.trim_matches('/').to_string()))
}

fn parse_parent_route(route: &str) -> Result<(String, String), ToolError> {
    let (domain, path) = route.split_once("://").ok_or_else(|| {
        ToolError::InvalidParameters(format!(
            "expected route in the form domain://path (or domain://), got: {route}"
        ))
    })?;
    if domain.trim().is_empty() {
        return Err(ToolError::InvalidParameters(format!(
            "route must include a domain: {route}"
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

fn derive_title(content: &str, kind: MemoryNodeKind, explicit: Option<String>) -> String {
    explicit
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            content
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| format!("{kind:?} Memory"))
}

pub struct MemoryRecallTool {
    memory: Arc<MemoryManager>,
}

impl MemoryRecallTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryRecallTool {
    fn name(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Recall relevant graph-native long-term memory nodes by query. Use this for Steward's durable memory graph, not workspace files or legacy MEMORY.md/daily logs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "reason": { "type": "string" },
                "limit": { "type": "integer", "default": 5, "minimum": 1, "maximum": 20 },
                "domains": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            },
            "required": ["query", "reason"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let query = require_str(&params, "query")?;
        let reason = require_str(&params, "reason")?;
        let limit = params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(5)
            .clamp(1, 20) as usize;
        let domains = params
            .get("domains")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let results = self
            .memory
            .recall(&ctx.user_id, None, query, limit, &domains)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory recall failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "query": query,
                "reason": reason,
                "results": results,
                "result_count": results.len(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct MemoryOpenTool {
    memory: Arc<MemoryManager>,
}

impl MemoryOpenTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryOpenTool {
    fn name(&self) -> &str {
        "memory_open"
    }

    fn description(&self) -> &str {
        "Open a memory node by route or node id from Steward's graph-native memory system."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "route_or_node_id": { "type": "string" }
            },
            "required": ["route_or_node_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let key = require_str(&params, "route_or_node_id")?;

        // Virtual system URIs (Nocturne-style entry points).
        if key == "system://boot" {
            let boot = self
                .memory
                .boot_set(&ctx.user_id, None, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("boot read failed: {e}")))?;
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "key": key,
                    "detail": {
                        "kind": "system_boot",
                        "items": boot,
                    }
                }),
                start.elapsed(),
            ));
        }
        if key == "system://glossary" {
            let glossary = self
                .memory
                .glossary(&ctx.user_id, None)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("glossary failed: {e}")))?;
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "key": key,
                    "detail": {
                        "kind": "system_glossary",
                        "glossary": glossary,
                    }
                }),
                start.elapsed(),
            ));
        }
        if key.starts_with("system://index") {
            let domain = key.strip_prefix("system://index").and_then(|rest| {
                let rest = rest.trim_matches('/');
                if rest.is_empty() { None } else { Some(rest) }
            });
            let index = self
                .memory
                .list_index(&ctx.user_id, None, domain)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("index failed: {e}")))?;
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "key": key,
                    "detail": {
                        "kind": "system_index",
                        "domain": domain,
                        "entries": index,
                    }
                }),
                start.elapsed(),
            ));
        }
        if key.starts_with("system://recent") {
            let limit = key
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
                    "key": key,
                    "detail": {
                        "kind": "system_recent",
                        "limit": limit,
                        "entries": recent,
                    }
                }),
                start.elapsed(),
            ));
        }

        let detail = self
            .memory
            .open(&ctx.user_id, None, key)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory open failed: {e}")))?;
        Ok(ToolOutput::success(
            serde_json::json!({
                "key": key,
                "detail": detail,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct MemoryCreateTool {
    memory: Arc<MemoryManager>,
}

impl MemoryCreateTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryCreateTool {
    fn name(&self) -> &str {
        "memory_create"
    }

    fn description(&self) -> &str {
        "Create a new graph-native memory node under a parent route. Use this instead of writing MEMORY.md, HEARTBEAT.md, or daily/*.md. This writes directly to Steward's native memory graph and records a changeset snapshot (auto-applied)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let kind_values = kind_enum_values();
        serde_json::json!({
            "type": "object",
            "properties": {
                "parent_route": { "type": "string", "description": "Parent URI to derive a child route from (domain://path or domain://). Provide this OR 'route'. (Alias: parent_uri)" },
                "parent_uri": { "type": "string", "description": "Alias for parent_route." },
                "content": { "type": "string" },
                "kind": { "type": "string", "enum": kind_values, "description": "Optional memory node kind. Defaults to 'reference'. Use 'user_profile' for stable user facts like the user's name." },
                "priority": { "type": "integer", "default": 50 },
                "trigger_text": { "type": ["string", "null"], "description": "Legacy name for disclosure." },
                "disclosure": { "type": ["string", "null"], "description": "When this memory should be recalled." },
                "title": { "type": "string" },
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] },
                "route": { "type": "string", "description": "Root creation URI (domain://path). Required when parent_route is omitted. (Alias: uri)" },
                "uri": { "type": "string", "description": "Alias for route." }
            },
            "anyOf": [
                { "required": ["route"] },
                { "required": ["uri"] },
                { "required": ["parent_route"] },
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
        let parent_route = optional_string(&params, "parent_route")?
            .or(optional_string(&params, "parent_uri")?);
        let content = require_str(&params, "content")?;
        let kind = optional_string(&params, "kind")?
            .map(|value| parse_kind(&value))
            .transpose()?
            .unwrap_or(MemoryNodeKind::Reference);
        let title = derive_title(content, kind, optional_string(&params, "title")?);
        let priority = optional_priority(&params)?.unwrap_or(50);
        let trigger_text = if params.get("disclosure").is_some() {
            optional_string(&params, "disclosure")?
        } else {
            optional_string(&params, "trigger_text")?
        };
        let visibility = parse_visibility(params.get("visibility").and_then(|v| v.as_str()))?;
        let keywords = parse_keywords(&params)?;

        let route = if let Some(route) = optional_string(&params, "route")?
            .or(optional_string(&params, "uri")?)
        {
            route
        } else if let Some(ref parent_route) = parent_route {
            let (domain, parent_path) = parse_parent_route(parent_route)?;
            let slug = slugify(&title);
            if parent_path.is_empty() {
                format!("{domain}://{slug}")
            } else {
                format!("{domain}://{parent_path}/{slug}")
            }
        } else {
            return Err(ToolError::InvalidParameters(
                "memory_create requires either 'route' (for root creation) or 'parent_route' (to derive a child route)".to_string(),
            ));
        };
        let (domain, path) = parse_route_strict(&route)?;

        let (detail, changeset) = self
            .memory
            .create(
                &ctx.user_id,
                None,
                parent_route.as_deref(),
                &title,
                kind,
                content,
                &domain,
                &path,
                priority,
                trigger_text,
                visibility,
                keywords,
                serde_json::json!({
                    "source": "tool:memory_create",
                    "job_id": ctx.job_id,
                }),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory create failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
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

pub struct MemoryUpdateTool {
    memory: Arc<MemoryManager>,
}

impl MemoryUpdateTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryUpdateTool {
    fn name(&self) -> &str {
        "memory_update"
    }

    fn description(&self) -> &str {
        "Update an existing graph-native memory node or route metadata. Use this instead of editing legacy workspace memory files such as MEMORY.md, HEARTBEAT.md, or daily/*.md. Records the change in a changeset snapshot (auto-applied)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let kind_values = kind_enum_values();
        serde_json::json!({
            "type": "object",
            "properties": {
                "route_or_node_id": { "type": "string", "description": "Node id or route (domain://path). Alias: uri." },
                "uri": { "type": "string", "description": "Alias for route_or_node_id." },
                "old_string": { "type": "string", "description": "Patch mode: uniquely-matching substring to replace." },
                "new_string": { "type": "string", "description": "Patch mode: replacement text (can be empty)." },
                "append": { "type": "string", "description": "Append mode: text to append to the end of content." },
                "expected_version_id": { "type": "string", "description": "Optional optimistic concurrency check; must match the active version id." },
                "content": { "type": "string", "description": "Unsafe full replacement. Only allowed when replace_content=true." },
                "replace_content": { "type": "boolean", "default": false },
                "priority": { "type": "integer" },
                "trigger_text": { "type": ["string", "null"], "description": "Legacy name for disclosure." },
                "disclosure": { "type": ["string", "null"], "description": "When this memory should be recalled." },
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "title": { "type": "string" },
                "kind": { "type": "string", "enum": kind_values, "description": "Optional node kind update (e.g. promote to 'boot')." },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] }
            },
            "anyOf": [
                { "required": ["route_or_node_id"] },
                { "required": ["uri"] }
            ]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let key = if let Some(value) = params.get("route_or_node_id").and_then(|v| v.as_str()) {
            value
        } else {
            require_str(&params, "uri")?
        };

        if key.starts_with("system://") {
            return Err(ToolError::InvalidParameters(
                "system:// URIs are virtual entry points (read-only). Use memory_open/read_memory to inspect them, and update a concrete URI like system://boot/memory_protocol instead."
                    .to_string(),
            ));
        }

        let detail = self
            .memory
            .open(&ctx.user_id, None, key)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory open failed: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("memory node not found: {key}")))?;

        if let Some(expected) = params.get("expected_version_id").and_then(|v| v.as_str()) {
            let expected = Uuid::parse_str(expected).map_err(|_| {
                ToolError::InvalidParameters(format!("invalid expected_version_id: {expected}"))
            })?;
            if expected != detail.active_version.id {
                return Err(ToolError::ExecutionFailed(format!(
                    "version mismatch for '{key}': expected {expected}, found {}",
                    detail.active_version.id
                )));
            }
        }

        let old_string = optional_string(&params, "old_string")?;
        let new_string = optional_string(&params, "new_string")?;
        let append = optional_string(&params, "append")?;
        let replace_content = params
            .get("replace_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let full_content = optional_string(&params, "content")?;

        let content = match (old_string, new_string, append, full_content) {
            (Some(old), Some(new), None, None) => {
                let current = &detail.active_version.content;
                let count = current.matches(&old).count();
                if count != 1 {
                    return Err(ToolError::ExecutionFailed(format!(
                        "patch requires old_string to match exactly once (matched {count} times)"
                    )));
                }
                Some(current.replacen(&old, &new, 1))
            }
            (Some(_), None, _, _) => {
                return Err(ToolError::InvalidParameters(
                    "patch mode requires both 'old_string' and 'new_string'".to_string(),
                ));
            }
            (None, None, Some(text), None) => {
                let mut current = detail.active_version.content.clone();
                if !current.ends_with('\n') {
                    current.push('\n');
                }
                current.push('\n');
                current.push_str(&text);
                Some(current)
            }
            (None, None, None, Some(text)) => {
                if !replace_content {
                    return Err(ToolError::InvalidParameters(
                        "full content replacement is disabled; use patch/append, or pass replace_content=true".to_string(),
                    ));
                }
                Some(text)
            }
            (None, None, None, None) => None,
            _ => {
                return Err(ToolError::InvalidParameters(
                    "update modes are mutually exclusive: use either patch (old_string/new_string), append, or (content+replace_content=true)".to_string(),
                ));
            }
        };

        let visibility = match params.get("visibility") {
            Some(_) => Some(parse_visibility(
                params.get("visibility").and_then(|v| v.as_str()),
            )?),
            None => None,
        };
        let keywords = if params.get("keywords").is_some() {
            Some(parse_keywords(&params)?)
        } else {
            None
        };

        let kind = match params.get("kind").and_then(|v| v.as_str()) {
            Some(value) => Some(parse_kind(value)?),
            None => None,
        };

        let trigger_text = if params.get("disclosure").is_some() {
            match params.get("disclosure") {
                Some(serde_json::Value::Null) => Some(None),
                Some(_) => Some(optional_string(&params, "disclosure")?),
                None => None,
            }
        } else {
            match params.get("trigger_text") {
                Some(serde_json::Value::Null) => Some(None),
                Some(_) => Some(optional_string(&params, "trigger_text")?),
                None => None,
            }
        };

        let input = UpdateMemoryNodeInput {
            route_or_node: key.to_string(),
            title: optional_string(&params, "title")?,
            content,
            priority: optional_priority(&params)?,
            trigger_text,
            visibility,
            metadata: Some(serde_json::json!({
                "source": "tool:memory_update",
                "job_id": ctx.job_id,
            })),
            keywords,
            changeset_id: None,
            kind,
        };
        let (detail, changeset) = self
            .memory
            .update(&ctx.user_id, None, &input)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory update failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
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

pub struct MemoryAliasTool {
    memory: Arc<MemoryManager>,
}

impl MemoryAliasTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryAliasTool {
    fn name(&self) -> &str {
        "memory_alias"
    }

    fn description(&self) -> &str {
        "Create an alias route for an existing memory node without copying its content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "new_route": { "type": "string" },
                "target_route": { "type": "string" },
                "priority": { "type": "integer", "default": 50 },
                "trigger_text": { "type": ["string", "null"], "description": "Legacy name for disclosure." },
                "disclosure": { "type": ["string", "null"], "description": "When this alias should be recalled." },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] }
            },
            "required": ["new_route", "target_route"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let new_route = require_str(&params, "new_route")?;
        let target_route = require_str(&params, "target_route")?;
        let (domain, path) = parse_route_strict(new_route)?;
        let input = CreateMemoryAliasInput {
            space_id: Uuid::nil(),
            target_route_or_node: target_route.to_string(),
            domain,
            path,
            visibility: parse_visibility(params.get("visibility").and_then(|v| v.as_str()))?,
            priority: optional_priority(&params)?.unwrap_or(50),
            trigger_text: if params.get("disclosure").is_some() {
                optional_string(&params, "disclosure")?
            } else {
                optional_string(&params, "trigger_text")?
            },
            changeset_id: None,
        };
        let (route, changeset) = self
            .memory
            .alias(&ctx.user_id, None, &input)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory alias failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "route": route,
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

pub struct MemoryDeleteTool {
    memory: Arc<MemoryManager>,
}

impl MemoryDeleteTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryDeleteTool {
    fn name(&self) -> &str {
        "memory_delete"
    }

    fn description(&self) -> &str {
        "Delete a memory route or node from the native memory graph. This is a high-impact operation and always requires approval."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "route_or_node_id": { "type": "string", "description": "Route/URI to delete (path-only delete). Node ids are not accepted." }
            },
            "required": ["route_or_node_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let key = require_str(&params, "route_or_node_id")?;
        // Nocturne-style "forgetting": delete only a route/path, not the underlying node id.
        let _ = parse_route_strict(key)?;
        let changeset = self
            .memory
            .delete(&ctx.user_id, None, key)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory delete failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "deleted": key,
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

pub struct MemoryReviewTool {
    memory: Arc<MemoryManager>,
}

impl MemoryReviewTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemoryReviewTool {
    fn name(&self) -> &str {
        "memory_review"
    }

    fn description(&self) -> &str {
        "Apply or request rollback for a pending memory changeset."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "changeset_id": { "type": "string" },
                "action": { "type": "string", "enum": ["accept", "rollback"] }
            },
            "required": ["changeset_id", "action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let changeset_id = require_str(&params, "changeset_id")?;
        let action = require_str(&params, "action")?;
        let id = Uuid::parse_str(changeset_id).map_err(|_| {
            ToolError::InvalidParameters(format!("invalid changeset id: {changeset_id}"))
        })?;
        self.memory
            .review(&_ctx.user_id, None, id, action)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory review failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "changeset_id": id,
                "action": action,
                "status": if action == "rollback" {
                    "rollback_requested"
                } else {
                    "applied"
                }
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        let _ = params;
        ApprovalRequirement::Never
    }
}
