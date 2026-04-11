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

const AUTO_MEMORY_DOMAIN: &str = "memory";

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

fn reject_legacy_param_aliases(
    params: &serde_json::Value,
    aliases: &[(&str, &str)],
) -> Result<(), ToolError> {
    for (legacy, canonical) in aliases {
        if params.get(*legacy).is_some() {
            return Err(ToolError::InvalidParameters(format!(
                "'{legacy}' is no longer supported; use '{canonical}' instead"
            )));
        }
    }
    Ok(())
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

fn build_save_route(
    params: &serde_json::Value,
    explicit_title: Option<&str>,
    content: Option<&str>,
) -> Result<(String, Option<String>), ToolError> {
    let explicit_route = optional_string(params, "route")?;
    let parent_route = optional_string(params, "parent_route")?;
    if explicit_route.is_some() && parent_route.is_some() {
        return Err(ToolError::InvalidParameters(
            "memory_save accepts either 'route' or 'parent_route', but not both".to_string(),
        ));
    }

    if let Some(route) = explicit_route {
        return Ok((route, None));
    }

    let has_append = params.get("append").is_some();
    let has_patch = params.get("old_string").is_some() || params.get("new_string").is_some();

    if parent_route.is_none() {
        if has_append {
            return Err(ToolError::InvalidParameters(
                "append mode requires an existing memory route".to_string(),
            ));
        }
        if has_patch {
            return Err(ToolError::InvalidParameters(
                "patch mode requires an existing memory route".to_string(),
            ));
        }
    }

    let title_source = explicit_title
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            content.and_then(|text| {
                text.lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(ToOwned::to_owned)
            })
        })
        .ok_or_else(|| {
            ToolError::InvalidParameters(
                "memory_save needs 'title' or non-empty 'content' to derive a route"
                    .to_string(),
            )
        })?;

    let slug = slugify(&title_source);
    if slug.is_empty() {
        return Err(ToolError::InvalidParameters(
            "memory_save could not derive a route slug from the provided title/content".to_string(),
        ));
    }

    if let Some(parent_route) = parent_route {
        let (domain, parent_path) = parse_parent_route(&parent_route)?;
        let route = if parent_path.is_empty() {
            format!("{domain}://{slug}")
        } else {
            format!("{domain}://{parent_path}/{slug}")
        };
        return Ok((route, Some(parent_route)));
    }

    Ok((format!("{AUTO_MEMORY_DOMAIN}://{slug}"), None))
}

fn resolve_save_content(
    params: &serde_json::Value,
    current_content: Option<&str>,
    explicit_title: Option<&str>,
) -> Result<Option<String>, ToolError> {
    let old_string = optional_string(params, "old_string")?;
    let new_string = optional_string(params, "new_string")?;
    let append = optional_string(params, "append")?;
    let full_content = optional_string(params, "content")?;

    match (old_string, new_string, append, full_content, current_content) {
        (Some(old), Some(new), None, None, Some(current)) => {
            let count = current.matches(&old).count();
            if count != 1 {
                return Err(ToolError::ExecutionFailed(format!(
                    "patch requires old_string to match exactly once (matched {count} times)"
                )));
            }
            Ok(Some(current.replacen(&old, &new, 1)))
        }
        (Some(_), None, _, _, _) => Err(ToolError::InvalidParameters(
            "patch mode requires both 'old_string' and 'new_string'".to_string(),
        )),
        (None, None, Some(text), None, Some(current)) => {
            let mut updated = current.to_string();
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push('\n');
            updated.push_str(&text);
            Ok(Some(updated))
        }
        (None, None, None, Some(text), Some(_)) => Ok(Some(text)),
        (None, None, None, None, Some(_)) => Ok(None),
        (None, None, None, Some(text), None) => Ok(Some(text)),
        (None, None, None, None, None) => explicit_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| {
                ToolError::InvalidParameters(
                    "memory_save needs semantic content when creating a new memory".to_string(),
                )
            }),
        (_, _, Some(_), Some(_), _) | (Some(_), Some(_), _, Some(_), _) => Err(
            ToolError::InvalidParameters(
                "save modes are mutually exclusive: use either patch (old_string/new_string), append, or full 'content'".to_string(),
            ),
        ),
        (_, _, Some(_), _, None) => Err(ToolError::InvalidParameters(
            "append mode requires an existing memory route".to_string(),
        )),
        (Some(_), Some(_), _, _, None) => Err(ToolError::InvalidParameters(
            "patch mode requires an existing memory route".to_string(),
        )),
        _ => Err(ToolError::InvalidParameters(
            "invalid memory_save parameter combination".to_string(),
        )),
    }
}

pub struct MemorySaveTool {
    memory: Arc<MemoryManager>,
}

impl MemorySaveTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl Tool for MemorySaveTool {
    fn name(&self) -> &str {
        "memory_save"
    }

    fn description(&self) -> &str {
        "Create or update a graph-native memory node. Use this instead of writing MEMORY.md, HEARTBEAT.md, or daily/*.md. If the target route already exists, Steward updates it; otherwise it creates a new memory there."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let kind_values = kind_enum_values();
        serde_json::json!({
            "type": "object",
            "properties": {
                "route": { "type": "string", "description": "Exact route to create or update (domain://path). Optional for new memories; if omitted, Steward derives one from the title/content." },
                "parent_route": { "type": "string", "description": "Optional parent route to derive a child route from when creating a new memory." },
                "content": { "type": "string", "description": "Full content for a new memory, or full replacement content for an existing one. For new memories, Steward can fall back to the title when content is omitted." },
                "old_string": { "type": "string", "description": "Patch mode: uniquely-matching substring to replace in an existing memory. Requires route." },
                "new_string": { "type": "string", "description": "Patch mode: replacement text (can be empty). Requires route." },
                "append": { "type": "string", "description": "Append mode: text to append to the end of an existing memory. Requires route." },
                "expected_version_id": { "type": "string", "description": "Optional optimistic concurrency check for existing memories." },
                "kind": { "type": "string", "enum": kind_values, "description": "Optional memory node kind. Defaults to 'reference' for new memories." },
                "priority": { "type": "integer", "default": 50 },
                "disclosure": { "type": ["string", "null"], "description": "When this memory should be recalled." },
                "title": { "type": "string", "description": "Optional human-readable title. Also used for route/content derivation when creating a new memory." },
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] }
            },
            "additionalProperties": false,
            "examples": [
                {
                    "title": "Dreamer Profile",
                    "content": "The user's name is 梦凌汐.",
                    "kind": "user_profile"
                },
                {
                    "route": "memory://dreamer-profile",
                    "old_string": "梦凌汐",
                    "new_string": "梦凌汐（确认）"
                },
                {
                    "parent_route": "memory://profiles",
                    "content": "Prefers concise status updates.",
                    "kind": "user_profile",
                    "title": "Dreamer Profile"
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
        reject_legacy_param_aliases(
            &params,
            &[
                ("parent_uri", "parent_route"),
                ("uri", "route"),
                ("trigger_text", "disclosure"),
            ],
        )?;

        let explicit_title = optional_string(&params, "title")?;
        let requested_content = optional_string(&params, "content")?;
        let (route, parent_route) =
            build_save_route(&params, explicit_title.as_deref(), requested_content.as_deref())?;

        if route.starts_with("system://") {
            return Err(ToolError::InvalidParameters(
                "system:// URIs are virtual entry points (read-only). Use memory_open to inspect them, and save to a concrete non-system route instead.".to_string(),
            ));
        }

        let existing = self
            .memory
            .open(&ctx.user_id, None, &route)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory open failed: {e}")))?;

        if let (Some(detail), Some(expected)) = (
            existing.as_ref(),
            params.get("expected_version_id").and_then(|v| v.as_str()),
        ) {
            let expected = Uuid::parse_str(expected).map_err(|_| {
                ToolError::InvalidParameters(format!("invalid expected_version_id: {expected}"))
            })?;
            if expected != detail.active_version.id {
                return Err(ToolError::ExecutionFailed(format!(
                    "version mismatch for '{route}': expected {expected}, found {}",
                    detail.active_version.id
                )));
            }
        }

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
        let parsed_kind = match params.get("kind").and_then(|v| v.as_str()) {
            Some(value) => Some(parse_kind(value)?),
            None => None,
        };
        let disclosure = match params.get("disclosure") {
            Some(serde_json::Value::Null) => Some(None),
            Some(_) => Some(optional_string(&params, "disclosure")?),
            None => None,
        };

        if let Some(existing) = existing {
            let content = resolve_save_content(
                &params,
                Some(&existing.active_version.content),
                explicit_title.as_deref(),
            )?;
            let input = UpdateMemoryNodeInput {
                route_or_node: route.clone(),
                title: explicit_title,
                content,
                priority: optional_priority(&params)?,
                trigger_text: disclosure,
                visibility,
                metadata: Some(serde_json::json!({
                    "source": "tool:memory_save",
                    "job_id": ctx.job_id,
                })),
                keywords,
                changeset_id: None,
                kind: parsed_kind,
            };
            let (detail, changeset) = self
                .memory
                .update(&ctx.user_id, None, &input)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("memory save failed: {e}")))?;

            return Ok(ToolOutput::success(
                serde_json::json!({
                    "mode": "updated",
                    "node": detail,
                    "changeset_id": changeset.id,
                    "review_status": changeset.status,
                }),
                start.elapsed(),
            ));
        }

        let content = resolve_save_content(&params, None, explicit_title.as_deref())?;
        let content = content.ok_or_else(|| {
            ToolError::InvalidParameters(
                "memory_save needs semantic content when creating a new memory".to_string(),
            )
        })?;
        let kind = parsed_kind.unwrap_or(MemoryNodeKind::Reference);
        let title = derive_title(&content, kind, explicit_title);
        let priority = optional_priority(&params)?.unwrap_or(50);
        let visibility = visibility.unwrap_or(MemoryVisibility::Private);
        let (domain, path) = parse_route_strict(&route)?;

        let (detail, changeset) = self
            .memory
            .create(
                &ctx.user_id,
                None,
                parent_route.as_deref(),
                &title,
                kind,
                &content,
                &domain,
                &path,
                priority,
                disclosure.flatten(),
                visibility,
                keywords.unwrap_or_default(),
                serde_json::json!({
                    "source": "tool:memory_save",
                    "job_id": ctx.job_id,
                }),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("memory save failed: {e}")))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "mode": "created",
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
        reject_legacy_param_aliases(&params, &[("trigger_text", "disclosure")])?;
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
            trigger_text: optional_string(&params, "disclosure")?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestHarnessBuilder;
    use crate::tools::Tool;

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn memory_save_schema_exposes_only_canonical_fields() {
        let harness = TestHarnessBuilder::new().build().await;
        let tool = MemorySaveTool::new(Arc::new(MemoryManager::new(Arc::clone(
            &harness.db,
        ))));
        let schema = tool.parameters_schema();
        let properties = schema["properties"]
            .as_object()
            .expect("memory_save schema should have properties");

        assert!(properties.contains_key("route"));
        assert!(properties.contains_key("parent_route"));
        assert!(properties.contains_key("disclosure"));
        assert!(!properties.contains_key("uri"));
        assert!(!properties.contains_key("parent_uri"));
        assert!(!properties.contains_key("trigger_text"));
        assert!(schema.get("oneOf").is_none());
        assert!(schema.get("not").is_none());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn memory_save_rejects_legacy_aliases_and_mutually_exclusive_routes() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let tool = MemorySaveTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        let legacy_err = tool
            .execute(
                serde_json::json!({
                    "uri": "people://alex-work-style",
                    "content": "Alex is a backend engineer"
                }),
                &ctx,
            )
            .await
            .expect_err("legacy uri alias should be rejected");
        assert!(format!("{legacy_err}").contains("use 'route' instead"));

        let both_err = tool
            .execute(
                serde_json::json!({
                    "route": "people://alex-work-style",
                    "parent_route": "people://profiles",
                    "content": "Alex is a backend engineer"
                }),
                &ctx,
            )
            .await
            .expect_err("route and parent_route together should be rejected");
        assert!(format!("{both_err}").contains("but not both"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn memory_save_supports_route_and_parent_route_paths() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let tool = MemorySaveTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        tool.execute(
            serde_json::json!({
                "route": "people://alex-work-style",
                "content": "Alex is a backend engineer",
                "kind": "user_profile",
                "title": "Alex Work Style"
            }),
            &ctx,
        )
        .await
        .expect("route-based creation should succeed");

        let explicit = memory
            .open("user1", None, "people://alex-work-style")
            .await
            .expect("open explicit route")
            .expect("explicit route should exist");
        assert!(explicit.active_version.content.contains("backend engineer"));

        tool.execute(
            serde_json::json!({
                "route": "people://profiles",
                "content": "Directory of people profiles.",
                "kind": "reference",
                "title": "Profiles"
            }),
            &ctx,
        )
        .await
        .expect("parent route should exist before deriving a child route");

        tool.execute(
            serde_json::json!({
                "parent_route": "people://profiles",
                "content": "Prefers concise status updates.",
                "kind": "user_profile",
                "title": "Dreamer Profile"
            }),
            &ctx,
        )
        .await
        .expect("parent-route creation should succeed");

        let derived = memory
            .open("user1", None, "people://profiles/dreamer-profile")
            .await
            .expect("open derived route")
            .expect("derived route should exist");
        assert!(derived.active_version.content.contains("concise status updates"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn memory_save_can_auto_derive_route_for_new_memory() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let tool = MemorySaveTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        tool.execute(
            serde_json::json!({
                "title": "Dreamer Profile",
                "content": "The user's name is 梦凌汐.",
                "kind": "user_profile"
            }),
            &ctx,
        )
        .await
        .expect("route-less creation should succeed");

        let derived = memory
            .open("user1", None, "memory://dreamer-profile")
            .await
            .expect("open auto-derived route")
            .expect("auto-derived route should exist");
        assert!(derived.active_version.content.contains("梦凌汐"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn memory_save_patch_mode_still_requires_existing_route() {
        let harness = TestHarnessBuilder::new().build().await;
        let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));
        let tool = MemorySaveTool::new(Arc::clone(&memory));
        let ctx = JobContext::with_user("user1", "thread", "desktop");

        let err = tool
            .execute(
                serde_json::json!({
                    "old_string": "梦凌汐",
                    "new_string": "梦凌汐（确认）"
                }),
                &ctx,
            )
            .await
            .expect_err("patch without route should fail");

        assert!(format!("{err}").contains("patch mode requires an existing memory route"));
    }
}
