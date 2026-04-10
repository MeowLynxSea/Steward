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

fn parse_route(route: &str) -> Result<(String, String), ToolError> {
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
        "Create a new graph-native memory node under a parent route. Use this instead of writing MEMORY.md, HEARTBEAT.md, or daily/*.md. This writes directly to Steward's native memory graph and records a pending review changeset."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "parent_route": { "type": "string" },
                "content": { "type": "string" },
                "kind": { "type": "string" },
                "priority": { "type": "integer", "default": 50 },
                "trigger_text": { "type": ["string", "null"] },
                "title": { "type": "string" },
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] },
                "route": { "type": "string" }
            },
            "required": ["parent_route", "content", "kind"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let parent_route = require_str(&params, "parent_route")?;
        let content = require_str(&params, "content")?;
        let kind = parse_kind(require_str(&params, "kind")?)?;
        let title = derive_title(content, kind, optional_string(&params, "title")?);
        let priority = optional_priority(&params)?.unwrap_or(50);
        let trigger_text = optional_string(&params, "trigger_text")?;
        let visibility = parse_visibility(params.get("visibility").and_then(|v| v.as_str()))?;
        let keywords = parse_keywords(&params)?;

        let route = if let Some(route) = optional_string(&params, "route")? {
            route
        } else {
            let (domain, parent_path) = parse_route(parent_route)?;
            let slug = slugify(&title);
            format!("{domain}://{parent_path}/{slug}")
        };
        let (domain, path) = parse_route(&route)?;

        let (detail, changeset) = self
            .memory
            .create(
                &ctx.user_id,
                None,
                Some(parent_route),
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
        ApprovalRequirement::UnlessAutoApproved
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
        "Update an existing graph-native memory node or route metadata. Use this instead of editing legacy workspace memory files such as MEMORY.md, HEARTBEAT.md, or daily/*.md. Records the change in a pending review changeset."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "route_or_node_id": { "type": "string" },
                "content": { "type": "string" },
                "priority": { "type": "integer" },
                "trigger_text": { "type": ["string", "null"] },
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "title": { "type": "string" },
                "visibility": { "type": "string", "enum": ["private", "session", "shared"] }
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
        let input = UpdateMemoryNodeInput {
            route_or_node: key.to_string(),
            title: optional_string(&params, "title")?,
            content: optional_string(&params, "content")?,
            priority: optional_priority(&params)?,
            trigger_text: match params.get("trigger_text") {
                Some(serde_json::Value::Null) => Some(None),
                Some(_) => Some(optional_string(&params, "trigger_text")?),
                None => None,
            },
            visibility,
            metadata: Some(serde_json::json!({
                "source": "tool:memory_update",
                "job_id": ctx.job_id,
            })),
            keywords,
            changeset_id: None,
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
        ApprovalRequirement::UnlessAutoApproved
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
                "trigger_text": { "type": ["string", "null"] },
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
        let (domain, path) = parse_route(new_route)?;
        let input = CreateMemoryAliasInput {
            space_id: Uuid::nil(),
            target_route_or_node: target_route.to_string(),
            domain,
            path,
            visibility: parse_visibility(params.get("visibility").and_then(|v| v.as_str()))?,
            priority: optional_priority(&params)?.unwrap_or(50),
            trigger_text: optional_string(&params, "trigger_text")?,
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
        ApprovalRequirement::UnlessAutoApproved
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
        ApprovalRequirement::Always
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
        match params.get("action").and_then(|value| value.as_str()) {
            Some("rollback") => ApprovalRequirement::Always,
            _ => ApprovalRequirement::UnlessAutoApproved,
        }
    }
}
