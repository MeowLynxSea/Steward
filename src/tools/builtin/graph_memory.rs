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
            "content": "模板：用户名字是<user_name>。",
            "priority": 0,
            "title": "name",
            "disclosure": "模板：当我需要正确称呼用户时"
        }),
        json!({
            "parent_uri": "core://assistant/identity",
            "content": "模板：我的名字是<assistant_name>。",
            "priority": 0,
            "title": "name",
            "disclosure": "模板：当用户询问我是谁，或我需要自我介绍时"
        }),
        json!({
            "parent_uri": "core://assistant/style",
            "content": "模板：状态更新应简洁且高信息密度。",
            "priority": 1,
            "title": "status_updates",
            "disclosure": "模板：当我准备发送进度更新时"
        }),
    ]
}

fn create_memory_tool_summary() -> ToolDiscoverySummary {
    ToolDiscoverySummary {
        always_required: vec!["parent_uri".into(), "content".into(), "priority".into()],
        conditional_requirements: vec![
            "普通 durable memory 通常也应该提供 title；只有你明确想要按 1/2/3... 编号的顺序子节点时，才省略 title。".into(),
            "用户资料、自我设定、重要教训、偏好和约定通常也应该提供 disclosure，用来写明什么时候该想起它。".into(),
        ],
        notes: vec![
            "URI 负责 What，disclosure 负责 When。".into(),
            "除非你是在创建新的语义根节点，否则不要把普通事实直接挂在 core:// 根下。先选一个表达主题联想的 parent_uri。".into(),
            "如果你不确定该挂在哪个父节点，先用 search_memory 找现有概念，不要直接写进 core://。".into(),
            "不要用 logs、misc、history 这类容器名；父节点和 title 都应该表达实际概念。".into(),
            "以下 examples 都只是模板占位，不是当前对话里的真实事实，绝不能把示例值当成已知信息。".into(),
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
        "Search graph memory by keyword. Use this when you know the topic but are unsure which URI to read."
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
                { "query": "梦凌汐" },
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

        Ok(ToolOutput::success(
            json!({
                "query": query,
                "domain": domains.first(),
                "limit": limit,
                "results": results,
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
            return Ok(ToolOutput::success(
                json!({
                    "uri": requested_uri,
                    "kind": "system_boot",
                    "items": items,
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
        "在指定父节点下创建新记忆。通常应给出语义化的 parent_uri、title 和 disclosure；只有在你明确想要数字序号子节点时才省略 title。父节点要强调联想相关性（What/主题），不要使用 logs、misc、history 一类无意义的容器。"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "parent_uri": { "type": "string", "description": "父节点 URI。用 core:// 表示域根。父节点应该表达主题联想，而不是时间桶或垃圾桶。" },
                "content": { "type": "string" },
                "priority": { "type": "integer", "description": "优先级（0=最高，数字越小越优先）。" },
                "title": { "type": "string", "description": "可选路径名称（仅限字母、数字、'_'、'-'）。普通 durable memory 几乎总该填写；不填则自动分配序号。" },
                "disclosure": { "type": "string", "description": "触发条件：描述什么时候该想起这条记忆。用户资料、自我设定、约定、偏好和重要教训通常都应该填写。" }
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
                Vec::new(),
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
        "更新已有记忆。支持 Patch 模式、Append 模式，以及 priority/disclosure 元数据更新。没有全量替换模式。"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "old_string": { "type": "string", "description": "Patch 模式：要替换的原文，必须在内容中唯一匹配。" },
                "new_string": { "type": "string", "description": "Patch 模式：替换后的文本。设为 \"\" 可删除匹配片段。" },
                "append": { "type": "string", "description": "Append 模式：追加到内容末尾的文本。" },
                "priority": { "type": "integer", "description": "新的优先级。" },
                "disclosure": { "type": "string", "description": "新的触发条件。" }
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

        if content.is_none() && priority.is_none() && disclosure.is_none() {
            return Err(ToolError::InvalidParameters(
                "no update fields provided; use patch mode, append mode, priority, or disclosure"
                    .to_string(),
            ));
        }

        let input = UpdateMemoryNodeInput {
            route_or_node: canonical_uri.clone(),
            content,
            priority,
            trigger_text: disclosure.clone().map(Some),
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
        "删除一条记忆路径及其子路径，不伤及记忆正文。删除前先 read_memory 确认你知道自己在删什么。"
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
        "为已有记忆创建别名路径。不是复制，而是同一段内容的新入口；新路径的父节点必须已经存在。"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "new_uri": { "type": "string" },
                "target_uri": { "type": "string" },
                "priority": { "type": "integer", "default": DEFAULT_CREATE_PRIORITY, "description": "此别名的独立优先级。" },
                "disclosure": { "type": "string", "description": "此别名的独立触发条件。" }
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
        assert!(!properties.contains_key("route"));
        assert!(!properties.contains_key("kind"));
        assert!(!properties.contains_key("visibility"));
        assert!(!properties.contains_key("keywords"));
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
}
