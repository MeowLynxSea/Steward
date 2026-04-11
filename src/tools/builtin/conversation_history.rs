use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::context::JobContext;
use crate::conversation_recall::ConversationRecallManager;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

fn optional_usize(params: &serde_json::Value, key: &str) -> Result<Option<usize>, ToolError> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(|number| Some(number as usize))
            .ok_or_else(|| {
                ToolError::InvalidParameters(format!("'{key}' must be a positive integer"))
            }),
    }
}

pub struct SearchConversationHistoryTool {
    recall: Arc<ConversationRecallManager>,
}

impl SearchConversationHistoryTool {
    pub fn new(recall: Arc<ConversationRecallManager>) -> Self {
        Self { recall }
    }
}

#[async_trait]
impl Tool for SearchConversationHistoryTool {
    fn name(&self) -> &str {
        "search_conversation_history"
    }

    fn description(&self) -> &str {
        "Search the user's historical conversation turns across past threads and return matched turns with nearby canonical context."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to recall from historical conversations."
                },
                "limit": {
                    "type": "integer",
                    "description": "How many matched turns to return. Defaults to 8 and is capped at 12."
                },
                "include_current_thread": {
                    "type": "boolean",
                    "description": "Whether to include matches from the current conversation thread."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?;
        let requested_limit = optional_usize(&params, "limit")?.unwrap_or(8);
        let limit = requested_limit.clamp(1, 12);
        let include_current_thread = params
            .get("include_current_thread")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let results = self
            .recall
            .search_with_preview(
                &ctx.user_id,
                query,
                limit,
                ctx.conversation_id,
                include_current_thread,
            )
            .await
            .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "query": query,
                "limit": limit,
                "results": results.into_iter().map(|(hit, preview)| {
                    serde_json::json!({
                        "conversation_id": hit.conversation_id,
                        "turn_index": hit.turn_index,
                        "channel": hit.channel,
                        "timestamp": hit.turn_timestamp,
                        "matched_turn": {
                            "user": hit.user_text,
                            "assistant": hit.assistant_text,
                        },
                        "context_preview": preview.turns,
                        "score": hit.score,
                        "is_current_thread": ctx.conversation_id == Some(hit.conversation_id),
                    })
                }).collect::<Vec<_>>(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct ReadConversationContextTool {
    recall: Arc<ConversationRecallManager>,
}

impl ReadConversationContextTool {
    pub fn new(recall: Arc<ConversationRecallManager>) -> Self {
        Self { recall }
    }
}

#[async_trait]
impl Tool for ReadConversationContextTool {
    fn name(&self) -> &str {
        "read_conversation_context"
    }

    fn description(&self) -> &str {
        "Read canonical turn context for a historical conversation, either around an anchor turn or for the full thread."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "conversation_id": {
                    "type": "string",
                    "description": "Conversation UUID to inspect."
                },
                "anchor_turn_index": {
                    "type": "integer",
                    "description": "Optional anchor turn index when reading a slice."
                },
                "before_turns": {
                    "type": "integer",
                    "description": "How many turns before the anchor to include. Defaults to 2."
                },
                "after_turns": {
                    "type": "integer",
                    "description": "How many turns after the anchor to include. Defaults to 2."
                },
                "full_thread": {
                    "type": "boolean",
                    "description": "Return the full canonical thread instead of an anchored slice."
                },
                "include_tool_calls": {
                    "type": "boolean",
                    "description": "Include tool call summaries inside the returned turn window."
                }
            },
            "required": ["conversation_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let conversation_id =
            Uuid::parse_str(require_str(&params, "conversation_id")?).map_err(|_| {
                ToolError::InvalidParameters("invalid 'conversation_id' UUID".to_string())
            })?;
        let anchor_turn_index = optional_usize(&params, "anchor_turn_index")?;
        let before_turns = optional_usize(&params, "before_turns")?
            .unwrap_or(2)
            .min(20);
        let after_turns = optional_usize(&params, "after_turns")?.unwrap_or(2).min(20);
        let full_thread = params
            .get("full_thread")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let include_tool_calls = params
            .get("include_tool_calls")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let slice = self
            .recall
            .read_context(
                &ctx.user_id,
                conversation_id,
                anchor_turn_index,
                before_turns,
                after_turns,
                full_thread,
                include_tool_calls,
            )
            .await
            .map_err(|error| ToolError::ExecutionFailed(error.to_string()))?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "conversation_id": slice.conversation_id,
                "anchor_turn_index": slice.anchor_turn_index,
                "total_turns": slice.total_turns,
                "turns": slice.turns,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}
