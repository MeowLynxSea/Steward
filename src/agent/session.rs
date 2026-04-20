//! Session and thread model for turn-based agent interactions.
//!
//! A Session contains one or more Threads. Each Thread represents a
//! conversation/interaction sequence with the agent. Threads contain
//! Turns, which are request/response pairs.
//!
//! This model supports:
//! - Undo: Roll back to a previous turn
//! - Interrupt: Cancel the current turn mid-execution
//! - Compaction: Summarize old turns to save context
//! - Resume: Continue from a saved checkpoint

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, TimeDelta, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm::{ChatMessage, ToolCall, generate_tool_call_id};
use steward_common::truncate_preview;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnCostInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: String,
}

/// A session containing one or more threads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID.
    pub id: Uuid,
    /// User ID that owns this session.
    pub user_id: String,
    /// Active thread ID.
    pub active_thread: Option<Uuid>,
    /// All threads in this session.
    pub threads: HashMap<Uuid, Thread>,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session was last active.
    pub last_active_at: DateTime<Utc>,
    /// Session metadata.
    pub metadata: serde_json::Value,
    /// Tools that have been auto-approved for this session ("always approve").
    #[serde(default)]
    pub auto_approved_tools: HashSet<String>,
    /// Absolute filesystem prefixes auto-approved for this session.
    ///
    /// Used for path-scoped approvals on host filesystem tools so "always
    /// allow" does not silently approve the entire tool class.
    #[serde(default)]
    pub auto_approved_path_prefixes: HashSet<String>,
}

impl Session {
    /// Create a new session.
    pub fn new(user_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id: user_id.into(),
            active_thread: None,
            threads: HashMap::new(),
            created_at: now,
            last_active_at: now,
            metadata: serde_json::Value::Null,
            auto_approved_tools: HashSet::new(),
            auto_approved_path_prefixes: HashSet::new(),
        }
    }

    /// Check if a tool has been auto-approved for this session.
    pub fn is_tool_auto_approved(&self, tool_name: &str) -> bool {
        self.auto_approved_tools.contains(tool_name)
    }

    /// Add a tool to the auto-approved set.
    pub fn auto_approve_tool(&mut self, tool_name: impl Into<String>) {
        self.auto_approved_tools.insert(tool_name.into());
    }

    /// Check if an absolute filesystem path is covered by a session-scoped approval prefix.
    pub fn is_path_auto_approved(&self, path: &std::path::Path) -> bool {
        self.auto_approved_path_prefixes
            .iter()
            .any(|prefix| path.starts_with(std::path::Path::new(prefix)))
    }

    /// Add an absolute filesystem prefix to the auto-approved set.
    pub fn auto_approve_path_prefix(&mut self, prefix: impl Into<String>) {
        self.auto_approved_path_prefixes.insert(prefix.into());
    }

    /// Create a new thread in this session.
    pub fn create_thread(&mut self) -> &mut Thread {
        let thread = Thread::new(self.id);
        let thread_id = thread.id;
        self.active_thread = Some(thread_id);
        self.last_active_at = Utc::now();
        self.threads.entry(thread_id).or_insert(thread)
    }

    /// Get the active thread.
    pub fn active_thread(&self) -> Option<&Thread> {
        self.active_thread.and_then(|id| self.threads.get(&id))
    }

    /// Get the active thread mutably.
    pub fn active_thread_mut(&mut self) -> Option<&mut Thread> {
        self.active_thread.and_then(|id| self.threads.get_mut(&id))
    }

    /// Get or create the active thread.
    pub fn get_or_create_thread(&mut self) -> &mut Thread {
        match self.active_thread {
            None => self.create_thread(),
            Some(id) => {
                if self.threads.contains_key(&id) {
                    // Entry existence confirmed by contains_key above.
                    // get_mut borrows self.threads mutably, so we can't
                    // combine the check and access into if-let without
                    // conflicting with the self.create_thread() fallback.
                    self.threads.get_mut(&id).unwrap() // safety: contains_key guard above
                } else {
                    // Stale active_thread ID: create a new thread, which
                    // updates self.active_thread to the new thread's ID.
                    self.create_thread()
                }
            }
        }
    }

    /// Switch to a different thread.
    pub fn switch_thread(&mut self, thread_id: Uuid) -> bool {
        if self.threads.contains_key(&thread_id) {
            self.active_thread = Some(thread_id);
            self.last_active_at = Utc::now();
            true
        } else {
            false
        }
    }
}

/// State of a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadState {
    /// Thread is idle, waiting for input.
    Idle,
    /// Thread is processing a turn.
    Processing,
    /// Thread is waiting for user approval.
    AwaitingApproval,
    /// Thread has completed (no more turns expected).
    Completed,
    /// Thread was interrupted.
    Interrupted,
}

/// Pending auth token request.
///
/// Auth mode TTL — must stay in sync with
/// `crate::cli::oauth_defaults::OAUTH_FLOW_EXPIRY` (5 minutes / 300 s).
/// Defined separately to avoid a session→cli module dependency.
const AUTH_MODE_TTL_SECS: i64 = 300;
const AUTH_MODE_TTL: TimeDelta = TimeDelta::seconds(AUTH_MODE_TTL_SECS);

/// When `tool_auth` returns `awaiting_token`, the thread enters auth mode.
/// The next user message is intercepted before entering the normal pipeline
/// (no logging, no turn creation, no history) and routed directly to the
/// credential store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAuth {
    /// Extension name to authenticate.
    pub extension_name: String,
    /// When this auth mode was entered. Used for TTL expiry.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
}

impl PendingAuth {
    /// Returns `true` if this auth mode has exceeded the TTL.
    pub fn is_expired(&self) -> bool {
        Utc::now() - self.created_at > AUTH_MODE_TTL
    }
}

/// Pending tool approval request stored on a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique request ID.
    pub request_id: Uuid,
    /// Tool name requiring approval.
    pub tool_name: String,
    /// Tool parameters (original values, used for execution).
    pub parameters: serde_json::Value,
    /// Redacted tool parameters (sensitive values replaced with `[REDACTED]`).
    /// Used for display in approval UI, logs, and runtime event payloads.
    #[serde(default)]
    pub display_parameters: serde_json::Value,
    /// Description of what the tool will do.
    pub description: String,
    /// Tool call ID from LLM (for proper context continuation).
    pub tool_call_id: String,
    /// Context messages at the time of the request (to resume from).
    pub context_messages: Vec<ChatMessage>,
    /// Remaining tool calls from the same assistant message that were not
    /// executed yet when approval was requested.
    #[serde(default)]
    pub deferred_tool_calls: Vec<ToolCall>,
    /// User timezone at the time the approval was requested, so it persists
    /// through the approval flow even if the approval message lacks timezone.
    #[serde(default)]
    pub user_timezone: Option<String>,
    /// Whether the "always" auto-approve option should be offered to the user.
    /// `false` when the tool returned `ApprovalRequirement::Always` (e.g.
    /// destructive shell commands), meaning every invocation must be confirmed.
    #[serde(default = "default_true")]
    pub allow_always: bool,
}

fn default_true() -> bool {
    true
}

/// A queued user message that arrived while a turn was still processing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingUserAttachment {
    pub id: String,
    pub kind: crate::channels::AttachmentKind,
    pub mime_type: String,
    pub filename: Option<String>,
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u32>,
}

impl PendingUserAttachment {
    pub fn from_incoming_attachment(attachment: &crate::channels::IncomingAttachment) -> Self {
        Self {
            id: attachment.id.clone(),
            kind: attachment.kind.clone(),
            mime_type: attachment.mime_type.clone(),
            filename: attachment.filename.clone(),
            size_bytes: attachment.size_bytes,
            workspace_uri: attachment.storage_key.clone(),
            extracted_text: attachment.extracted_text.clone(),
            duration_secs: attachment.duration_secs,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PendingUserMessageDelivery {
    #[default]
    AfterTurn,
    InjectNextOpportunity,
}

/// A queued user message that arrived while a turn was still processing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingUserMessage {
    pub content: String,
    #[serde(default = "Utc::now")]
    pub received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<PendingUserAttachment>,
    #[serde(default)]
    pub delivery: PendingUserMessageDelivery,
}

/// A user-authored message segment that contributes to a single logical turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimedUserMessageSegment {
    pub content: String,
    pub sent_at: DateTime<Utc>,
}

/// Lightweight persisted metadata for a user-message attachment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnUserAttachment {
    pub id: String,
    pub kind: crate::channels::AttachmentKind,
    pub mime_type: String,
    pub filename: Option<String>,
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u32>,
}

impl TurnUserAttachment {
    pub fn from_incoming_attachment(attachment: &crate::channels::IncomingAttachment) -> Self {
        Self {
            id: attachment.id.clone(),
            kind: attachment.kind.clone(),
            mime_type: attachment.mime_type.clone(),
            filename: attachment.filename.clone(),
            size_bytes: attachment.size_bytes,
            workspace_uri: attachment.storage_key.clone(),
            extracted_text: attachment.extracted_text.clone(),
            duration_secs: attachment.duration_secs,
        }
    }

    pub fn from_pending_attachment(attachment: &PendingUserAttachment) -> Self {
        Self {
            id: attachment.id.clone(),
            kind: attachment.kind.clone(),
            mime_type: attachment.mime_type.clone(),
            filename: attachment.filename.clone(),
            size_bytes: attachment.size_bytes,
            workspace_uri: attachment.workspace_uri.clone(),
            extracted_text: attachment.extracted_text.clone(),
            duration_secs: attachment.duration_secs,
        }
    }
}

/// A conversation thread within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    /// Unique thread ID.
    pub id: Uuid,
    /// Parent session ID.
    pub session_id: Uuid,
    /// Current state.
    pub state: ThreadState,
    /// Turns in this thread.
    pub turns: Vec<Turn>,
    /// When the thread was created.
    pub created_at: DateTime<Utc>,
    /// When the thread was last updated.
    pub updated_at: DateTime<Utc>,
    /// Thread metadata (e.g., title, tags).
    pub metadata: serde_json::Value,
    /// Pending approval request (when state is AwaitingApproval).
    #[serde(default)]
    pub pending_approval: Option<PendingApproval>,
    /// Pending auth token request (thread is in auth mode).
    #[serde(default)]
    pub pending_auth: Option<PendingAuth>,
    /// Messages queued while the thread was processing a turn.
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub pending_messages: VecDeque<PendingUserMessage>,
}

/// Maximum number of messages that can be queued while a thread is processing.
/// 10 merged messages can produce a large combined input for the LLM, but this
/// is acceptable for the personal assistant use case where a single user sends
/// rapid follow-ups. The drain loop processes them as one newline-delimited turn.
pub const MAX_PENDING_MESSAGES: usize = 10;

impl Thread {
    /// Create a new thread.
    pub fn new(session_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            session_id,
            state: ThreadState::Idle,
            turns: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Null,
            pending_approval: None,
            pending_auth: None,
            pending_messages: VecDeque::new(),
        }
    }

    /// Create a thread with a specific ID (for DB hydration).
    pub fn with_id(id: Uuid, session_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id,
            session_id,
            state: ThreadState::Idle,
            turns: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Null,
            pending_approval: None,
            pending_auth: None,
            pending_messages: VecDeque::new(),
        }
    }

    /// Get the current turn number (1-indexed for display).
    pub fn turn_number(&self) -> usize {
        self.turns.len() + 1
    }

    /// Get the last turn.
    pub fn last_turn(&self) -> Option<&Turn> {
        self.turns.last()
    }

    /// Get the last turn mutably.
    pub fn last_turn_mut(&mut self) -> Option<&mut Turn> {
        self.turns.last_mut()
    }

    /// Queue a message for processing after the current turn completes.
    /// Returns `false` if the queue is at capacity ([`MAX_PENDING_MESSAGES`]).
    pub fn queue_message(&mut self, content: String, received_at: DateTime<Utc>) -> bool {
        self.queue_pending_message_back(PendingUserMessage {
            content,
            received_at,
            attachments: Vec::new(),
            delivery: PendingUserMessageDelivery::AfterTurn,
        })
    }

    /// Queue a message with attachments at the back of the queue.
    pub fn queue_message_with_attachments(
        &mut self,
        content: String,
        received_at: DateTime<Utc>,
        attachments: Vec<PendingUserAttachment>,
    ) -> bool {
        self.queue_pending_message_back(PendingUserMessage {
            content,
            received_at,
            attachments,
            delivery: PendingUserMessageDelivery::AfterTurn,
        })
    }

    /// Queue a pending message at the front of the queue.
    pub fn queue_pending_message_front(&mut self, message: PendingUserMessage) -> bool {
        if self.pending_messages.len() >= MAX_PENDING_MESSAGES {
            return false;
        }
        self.pending_messages.push_front(message);
        self.updated_at = Utc::now();
        true
    }

    /// Queue a pending message at the back of the queue.
    pub fn queue_pending_message_back(&mut self, message: PendingUserMessage) -> bool {
        if self.pending_messages.len() >= MAX_PENDING_MESSAGES {
            return false;
        }
        self.pending_messages.push_back(message);
        self.updated_at = Utc::now();
        true
    }

    /// Queue an injectable pending message ahead of after-turn queue items while
    /// preserving FIFO order among other injectable messages.
    pub fn queue_pending_message_for_next_opportunity(
        &mut self,
        mut message: PendingUserMessage,
    ) -> bool {
        if self.pending_messages.len() >= MAX_PENDING_MESSAGES {
            return false;
        }
        message.delivery = PendingUserMessageDelivery::InjectNextOpportunity;
        let insert_at = self
            .pending_messages
            .iter()
            .take_while(|pending| {
                pending.delivery == PendingUserMessageDelivery::InjectNextOpportunity
            })
            .count();
        self.pending_messages.insert(insert_at, message);
        self.updated_at = Utc::now();
        true
    }

    /// Take the next pending message from the queue.
    pub fn take_pending_message(&mut self) -> Option<PendingUserMessage> {
        self.pending_messages.pop_front()
    }

    /// Take the next pending message that should be injected into the live loop.
    pub fn take_next_injectable_message(&mut self) -> Option<PendingUserMessage> {
        let position = self.pending_messages.iter().position(|message| {
            message.delivery == PendingUserMessageDelivery::InjectNextOpportunity
        })?;
        let message = self.pending_messages.remove(position);
        if message.is_some() {
            self.updated_at = Utc::now();
        }
        message
    }

    /// Estimate tokens for this thread's message history.
    pub fn estimate_messages_tokens(&self) -> u32 {
        const CHARS_PER_TOKEN: u32 = 4;
        let mut total: u32 = 0;
        for turn in &self.turns {
            total = total.saturating_add(turn.user_input.len() as u32 / CHARS_PER_TOKEN);
            if let Some(narrative) = &turn.narrative {
                total = total.saturating_add(narrative.len() as u32 / CHARS_PER_TOKEN);
            }
            for tc in &turn.tool_calls {
                total = total.saturating_add(tc.name.len() as u32 / CHARS_PER_TOKEN);
                total = total.saturating_add(tc.parameters.to_string().len() as u32 / CHARS_PER_TOKEN);
                if let Some(ref result) = tc.result {
                    total = total.saturating_add(result.to_string().len() as u32 / CHARS_PER_TOKEN);
                }
            }
            for segment in &turn.assistant_segments {
                total = total.saturating_add(segment.content.len() as u32 / CHARS_PER_TOKEN);
            }
        }
        total
    }

    /// Drain all pending messages from the queue.
    /// Multiple messages are joined with newlines so the LLM receives
    /// full context from rapid consecutive inputs (#259).
    pub fn drain_pending_messages(&mut self) -> Option<Vec<PendingUserMessage>> {
        if self.pending_messages.is_empty() {
            return None;
        }
        let parts: Vec<PendingUserMessage> = self.pending_messages.drain(..).collect();
        self.updated_at = Utc::now();
        Some(parts)
    }

    /// Re-queue previously drained content at the front of the queue.
    /// Used to preserve user input when the drain loop fails to process
    /// merged messages (soft error, hard error, interrupt).
    ///
    /// This intentionally bypasses [`MAX_PENDING_MESSAGES`] — the content
    /// was already counted against the cap before draining. The overshoot
    /// is bounded to 1 entry (the re-queued merged string) plus any new
    /// messages that arrived during the failed attempt.
    pub fn requeue_drained(&mut self, messages: Vec<PendingUserMessage>) {
        for message in messages.into_iter().rev() {
            self.pending_messages.push_front(message);
        }
        self.updated_at = Utc::now();
    }

    /// Start a new turn with user input.
    pub fn start_turn(&mut self, user_input: impl Into<String>) -> &mut Turn {
        let turn_number = self.turns.len();
        let turn = Turn::new(turn_number, user_input);
        self.turns.push(turn);
        self.state = ThreadState::Processing;
        self.updated_at = Utc::now();
        // turn_number was len() before push, so it's a valid index after push
        &mut self.turns[turn_number]
    }

    /// Start a new turn with user input and an explicit send timestamp.
    pub fn start_turn_at(
        &mut self,
        user_input: impl Into<String>,
        started_at: DateTime<Utc>,
    ) -> &mut Turn {
        let turn_number = self.turns.len();
        let turn = Turn::new_at(turn_number, user_input, started_at);
        self.turns.push(turn);
        self.state = ThreadState::Processing;
        self.updated_at = Utc::now();
        // turn_number was len() before push, so it's a valid index after push
        &mut self.turns[turn_number]
    }

    /// Complete the current turn with a response.
    pub fn complete_turn(&mut self, response: impl Into<String>) {
        if let Some(turn) = self.turns.last_mut() {
            turn.complete(response);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Fail the current turn with an error.
    pub fn fail_turn(&mut self, error: impl Into<String>) {
        if let Some(turn) = self.turns.last_mut() {
            turn.fail(error);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Mark the thread as awaiting approval with pending request details.
    pub fn await_approval(&mut self, pending: PendingApproval) {
        self.state = ThreadState::AwaitingApproval;
        self.pending_approval = Some(pending);
        self.updated_at = Utc::now();
    }

    /// Take the pending approval (clearing it from the thread).
    pub fn take_pending_approval(&mut self) -> Option<PendingApproval> {
        self.pending_approval.take()
    }

    /// Clear pending approval and return to idle state.
    pub fn clear_pending_approval(&mut self) {
        self.pending_approval = None;
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Enter auth mode: next user message will be routed directly to
    /// the credential store, bypassing the normal pipeline entirely.
    pub fn enter_auth_mode(&mut self, extension_name: String) {
        self.pending_auth = Some(PendingAuth {
            extension_name,
            created_at: Utc::now(),
        });
        self.updated_at = Utc::now();
    }

    /// Take the pending auth (clearing auth mode).
    pub fn take_pending_auth(&mut self) -> Option<PendingAuth> {
        self.pending_auth.take()
    }

    /// Interrupt the current turn and discard any queued messages.
    pub fn interrupt(&mut self) {
        if let Some(turn) = self.turns.last_mut() {
            turn.interrupt();
        }
        self.pending_messages.clear();
        self.pending_approval = None;
        self.state = ThreadState::Interrupted;
        self.updated_at = Utc::now();
    }

    /// Resume after interruption.
    pub fn resume(&mut self) {
        if self.state == ThreadState::Interrupted {
            self.state = ThreadState::Idle;
            self.updated_at = Utc::now();
        }
    }

    /// Get all messages for context building, including tool call history.
    ///
    /// Emits the full LLM-compatible message sequence per turn:
    /// `user → [assistant_with_tool_calls → tool_result*] → assistant`
    ///
    /// This ensures the LLM sees prior tool executions and won't re-attempt
    /// completed actions in subsequent turns.
    pub fn messages(&self) -> Vec<ChatMessage> {
        self.build_messages(None)
    }

    /// Get all messages formatted for LLM context, with user-message send times.
    pub fn messages_for_context(&self, user_tz: Tz) -> Vec<ChatMessage> {
        self.build_messages(Some(user_tz))
    }

    fn build_messages(&self, user_tz: Option<Tz>) -> Vec<ChatMessage> {
        let mut messages = Vec::new();
        // We use the enumeration index (`turn_idx`) rather than `turn.turn_number`
        // intentionally: after `truncate_turns()`, the remaining turns are
        // re-numbered starting from 0, so the enumeration index and turn_number
        // are equivalent. Using the index avoids coupling to the field and keeps
        // tool-call ID generation deterministic for the current message window.
        for (turn_idx, turn) in self.turns.iter().enumerate() {
            let user_content = if let Some(tz) = user_tz {
                if turn.user_message_segments.is_empty() {
                    format_user_message_for_context(&turn.user_input, turn.started_at, tz)
                } else {
                    format_user_message_segments_for_context(&turn.user_message_segments, tz)
                }
            } else {
                turn.user_input.clone()
            };
            let user_content =
                format_user_content_with_attachments(user_content, &turn.user_attachments);
            if turn.image_content_parts.is_empty() {
                messages.push(ChatMessage::user(user_content));
            } else {
                messages.push(ChatMessage::user_with_parts(
                    user_content,
                    turn.image_content_parts.clone(),
                ));
            }

            if !turn.tool_calls.is_empty() {
                // Assign synthetic call IDs for this turn's tool calls, so that
                // declarations and results can be consistently correlated.
                let tool_calls_with_ids: Vec<(String, &_)> = turn
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(tc_idx, tc)| {
                        // Use provider-compatible tool call IDs derived from turn/tool indices.
                        (generate_tool_call_id(turn_idx, tc_idx), tc)
                    })
                    .collect();

                // Build ToolCall objects using the synthetic call IDs.
                let tool_calls: Vec<ToolCall> = tool_calls_with_ids
                    .iter()
                    .map(|(call_id, tc)| ToolCall {
                        id: call_id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.parameters.clone(),
                        reasoning: None,
                    })
                    .collect();

                // Assistant message declaring the tool calls (no text content)
                messages.push(ChatMessage::assistant_with_tool_calls(None, tool_calls));

                // Individual tool result messages, truncated to limit context size.
                for (call_id, tc) in tool_calls_with_ids {
                    let content = if let Some(ref err) = tc.error {
                        // .error already contains the full error text;
                        // pass through without wrapping to avoid double-prefix.
                        truncate_preview(err, 1000)
                    } else if let Some(ref res) = tc.result {
                        let raw = match res {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        truncate_preview(&raw, 1000)
                    } else {
                        "OK".to_string()
                    };
                    messages.push(ChatMessage::tool_result(call_id, &tc.name, content));
                }
            }
            if let Some(ref response) = turn.response {
                messages.push(ChatMessage::assistant(response));
            }
        }
        messages
    }

    /// Truncate turns to a specific count (keeping most recent).
    pub fn truncate_turns(&mut self, keep: usize) {
        if self.turns.len() > keep {
            let drain_count = self.turns.len() - keep;
            self.turns.drain(0..drain_count);
            // Re-number remaining turns
            for (i, turn) in self.turns.iter_mut().enumerate() {
                turn.turn_number = i;
            }
        }
    }

    /// Restore thread state from a checkpoint's messages.
    ///
    /// Clears existing turns and rebuilds from the message sequence.
    /// Handles the full message pattern including tool messages:
    /// `user → [assistant_with_tool_calls → tool_result*] → assistant`
    ///
    /// Also supports the legacy pattern (user/assistant pairs only) for
    /// backward compatibility with old checkpoint data.
    pub fn restore_from_messages(&mut self, messages: Vec<ChatMessage>) {
        self.turns.clear();
        self.state = ThreadState::Idle;

        let mut iter = messages.into_iter().peekable();
        let mut turn_number = 0;

        while let Some(msg) = iter.next() {
            if msg.role == crate::llm::Role::User {
                let mut turn = Turn::new(turn_number, &msg.content);

                // Consume tool call sequences (assistant_with_tool_calls + tool_results).
                // A single turn may contain multiple rounds of tool calls, so we
                // track the cumulative base index into turn.tool_calls.
                while let Some(next) = iter.peek() {
                    if next.role == crate::llm::Role::Assistant && next.tool_calls.is_some() {
                        let call_base_idx = turn.tool_calls.len();

                        if let Some(assistant_msg) = iter.next()
                            && let Some(ref tcs) = assistant_msg.tool_calls
                        {
                            if !assistant_msg.content.trim().is_empty() && turn.narrative.is_none()
                            {
                                turn.narrative = Some(assistant_msg.content.clone());
                            }
                            for tc in tcs {
                                turn.record_tool_call_with_reasoning(
                                    &tc.name,
                                    tc.arguments.clone(),
                                    tc.reasoning.clone(),
                                    Some(tc.id.clone()),
                                );
                            }
                        }

                        // Consume the corresponding tool_result messages,
                        // indexing relative to this batch's base offset.
                        let mut pos = 0;
                        while let Some(tr) = iter.peek() {
                            if tr.role != crate::llm::Role::Tool {
                                break;
                            }
                            if let Some(tool_msg) = iter.next() {
                                let idx = call_base_idx + pos;
                                if idx < turn.tool_calls.len() {
                                    // Store as result — the error/success distinction
                                    // is for the live turn only; restored context just
                                    // needs the content the LLM originally saw.
                                    turn.tool_calls[idx].result =
                                        Some(serde_json::Value::String(tool_msg.content.clone()));
                                }
                            }
                            pos += 1;
                        }
                    } else {
                        break;
                    }
                }

                while iter.peek().is_some_and(|next| {
                    next.role == crate::llm::Role::Assistant && next.tool_calls.is_none()
                }) {
                    if let Some(response) = iter.next() {
                        turn.restore_assistant_segment(response.content, Utc::now(), None);
                    }
                }

                self.turns.push(turn);
                turn_number += 1;
            } else {
                // Skip non-user messages that aren't anchored to a turn
                continue;
            }
        }

        self.updated_at = Utc::now();
    }

    /// Restore thread state directly from persisted conversation rows.
    ///
    /// Unlike `restore_from_messages`, this preserves the original user-message
    /// timestamps from storage so LLM context can reflect when each message was sent.
    pub fn restore_from_conversation_messages(
        &mut self,
        messages: &[crate::history::ConversationMessage],
    ) {
        self.turns.clear();
        self.state = ThreadState::Idle;

        let mut idx = 0usize;
        let mut turn_number = 0usize;
        let mut pending_thinking_segments: Vec<String> = Vec::new();

        while idx < messages.len() {
            let msg = &messages[idx];
            if msg.role != "user" {
                idx += 1;
                continue;
            }

            let mut turn = Turn::new_at(turn_number, &msg.content, msg.created_at);
            turn.user_message_id = Some(msg.id);
            turn.user_attachments = user_attachments_from_metadata(&msg.metadata);
            idx += 1;

            while idx < messages.len() {
                let next = &messages[idx];
                match next.role.as_str() {
                    "thinking" => {
                        if !next.content.trim().is_empty() {
                            pending_thinking_segments.push(next.content.clone());
                        }
                        idx += 1;
                    }
                    "tool_call" => {
                        let call = match serde_json::from_str::<serde_json::Value>(&next.content) {
                            Ok(value) => value,
                            Err(_) => {
                                idx += 1;
                                continue;
                            }
                        };

                        let tool_call = ToolCall {
                            id: call["call_id"].as_str().unwrap_or("call_0").to_string(),
                            name: call["name"].as_str().unwrap_or("unknown").to_string(),
                            arguments: call
                                .get("parameters")
                                .cloned()
                                .unwrap_or(serde_json::json!({})),
                            reasoning: call
                                .get("rationale")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                        };

                        if turn.narrative.is_none() && !pending_thinking_segments.is_empty() {
                            turn.narrative = Some(pending_thinking_segments.join(""));
                        }
                        pending_thinking_segments.clear();

                        turn.record_tool_call_with_reasoning(
                            tool_call.name.clone(),
                            tool_call.arguments.clone(),
                            tool_call.reasoning.clone(),
                            Some(tool_call.id.clone()),
                        );

                        if let Some(last_call) = turn.tool_calls.last_mut() {
                            let content = if let Some(err) =
                                call.get("error").and_then(|v| v.as_str())
                            {
                                Some(err.to_string())
                            } else if let Some(res) = call.get("result").and_then(|v| v.as_str()) {
                                Some(res.to_string())
                            } else if let Some(preview) =
                                call.get("result_preview").and_then(|v| v.as_str())
                            {
                                Some(preview.to_string())
                            } else if call.get("completed_at").and_then(|v| v.as_str()).is_some() {
                                Some("OK".to_string())
                            } else {
                                None
                            };
                            last_call.result = content.map(serde_json::Value::String);
                        }
                        idx += 1;
                    }
                    "tool_calls" => {
                        let (calls, wrapper_narrative): (Vec<serde_json::Value>, Option<String>) =
                            match serde_json::from_str::<serde_json::Value>(&next.content) {
                                Ok(serde_json::Value::Array(arr)) => (arr, None),
                                Ok(serde_json::Value::Object(obj)) => (
                                    obj.get("calls")
                                        .and_then(|v| v.as_array())
                                        .cloned()
                                        .unwrap_or_default(),
                                    obj.get("narrative")
                                        .and_then(|v| v.as_str())
                                        .map(str::to_owned)
                                        .filter(|value| !value.trim().is_empty()),
                                ),
                                _ => (Vec::new(), None),
                            };

                        if turn.narrative.is_none() {
                            turn.narrative = if pending_thinking_segments.is_empty() {
                                wrapper_narrative
                            } else {
                                Some(pending_thinking_segments.join(""))
                            };
                        }
                        pending_thinking_segments.clear();

                        for call in calls {
                            let tool_call_id =
                                call["call_id"].as_str().unwrap_or("call_0").to_string();
                            let name = call["name"].as_str().unwrap_or("unknown").to_string();
                            let arguments = call
                                .get("parameters")
                                .cloned()
                                .unwrap_or(serde_json::json!({}));
                            let reasoning = call
                                .get("rationale")
                                .and_then(|v| v.as_str())
                                .map(String::from);

                            turn.record_tool_call_with_reasoning(
                                name.clone(),
                                arguments,
                                reasoning,
                                Some(tool_call_id),
                            );

                            if let Some(last_call) = turn.tool_calls.last_mut() {
                                let content =
                                    if let Some(err) = call.get("error").and_then(|v| v.as_str()) {
                                        Some(err.to_string())
                                    } else if let Some(res) =
                                        call.get("result").and_then(|v| v.as_str())
                                    {
                                        Some(res.to_string())
                                    } else if let Some(preview) =
                                        call.get("result_preview").and_then(|v| v.as_str())
                                    {
                                        Some(preview.to_string())
                                    } else {
                                        None
                                    };
                                last_call.result = content.map(serde_json::Value::String);
                            }
                        }
                        idx += 1;
                    }
                    "assistant" => {
                        turn.restore_assistant_segment(
                            next.content.clone(),
                            next.created_at,
                            Some(next.id),
                        );
                        pending_thinking_segments.clear();
                        idx += 1;
                    }
                    "user" => break,
                    _ => {
                        idx += 1;
                    }
                }
            }

            self.turns.push(turn);
            turn_number += 1;
        }

        self.updated_at = Utc::now();
    }
}

/// State of a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnState {
    /// Turn is being processed.
    Processing,
    /// Turn completed successfully.
    Completed,
    /// Turn failed with an error.
    Failed,
    /// Turn was interrupted.
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnAssistantSegment {
    pub content: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip)]
    pub conversation_message_id: Option<Uuid>,
}

/// A single turn (request/response pair) in a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// Turn number (0-indexed).
    pub turn_number: usize,
    /// User input that started this turn.
    pub user_input: String,
    /// Agent response (if completed).
    pub response: Option<String>,
    /// Tool calls made during this turn.
    pub tool_calls: Vec<TurnToolCall>,
    /// Turn state.
    pub state: TurnState,
    /// When the turn started.
    pub started_at: DateTime<Utc>,
    /// When the turn completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message (if failed).
    pub error: Option<String>,
    /// Agent's reasoning narrative for this turn.
    /// Cleaned via `clean_response` and sanitized through `SafetyLayer` before storage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative: Option<String>,
    /// Persisted cost summary for this user-message turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_cost: Option<TurnCostInfo>,
    /// Persisted conversation row backing the user message in history.
    #[serde(skip)]
    pub user_message_id: Option<Uuid>,
    /// Persisted conversation row backing the assistant response in history.
    #[serde(skip)]
    pub assistant_message_id: Option<Uuid>,
    /// Individual assistant-visible message segments emitted during this turn.
    ///
    /// A single logical turn can contain multiple assistant text segments when
    /// the model streams text, calls tools, then resumes speaking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assistant_segments: Vec<TurnAssistantSegment>,
    /// Cumulative usage snapshot captured at turn start to compute per-turn deltas.
    #[serde(skip)]
    pub cost_baseline: Option<crate::agent::cost_guard::ModelTokens>,
    /// Individual user-authored segments that were merged into this logical turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_message_segments: Vec<TimedUserMessageSegment>,
    /// Persisted attachments associated with the user message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_attachments: Vec<TurnUserAttachment>,
    /// Transient image content parts for multimodal LLM input.
    /// Not serialized — images are only needed for the current LLM call.
    /// The text description in `user_input` persists for compaction/context.
    #[serde(skip)]
    pub image_content_parts: Vec<crate::llm::ContentPart>,
    /// The currently open assistant segment receiving streamed text.
    #[serde(skip)]
    pub live_assistant_segment_index: Option<usize>,
}

impl Turn {
    /// Create a new turn.
    pub fn new(turn_number: usize, user_input: impl Into<String>) -> Self {
        Self::new_at(turn_number, user_input, Utc::now())
    }

    /// Create a new turn with an explicit start time.
    pub fn new_at(
        turn_number: usize,
        user_input: impl Into<String>,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            turn_number,
            user_input: user_input.into(),
            response: None,
            tool_calls: Vec::new(),
            state: TurnState::Processing,
            started_at,
            completed_at: None,
            error: None,
            narrative: None,
            turn_cost: None,
            user_message_id: None,
            assistant_message_id: None,
            assistant_segments: Vec::new(),
            cost_baseline: None,
            user_message_segments: Vec::new(),
            user_attachments: Vec::new(),
            image_content_parts: Vec::new(),
            live_assistant_segment_index: None,
        }
    }

    /// Append a streamed assistant chunk to this turn's in-progress response.
    pub fn append_response_chunk(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        match self.response.as_mut() {
            Some(existing) => existing.push_str(chunk),
            None => self.response = Some(chunk.to_string()),
        }

        let segment_index = match self.live_assistant_segment_index {
            Some(idx) if idx < self.assistant_segments.len() => idx,
            _ => {
                let created_at = self.completed_at.unwrap_or_else(Utc::now);
                self.assistant_segments.push(TurnAssistantSegment {
                    content: String::new(),
                    created_at,
                    conversation_message_id: None,
                });
                let idx = self.assistant_segments.len() - 1;
                self.live_assistant_segment_index = Some(idx);
                idx
            }
        };

        if let Some(segment) = self.assistant_segments.get_mut(segment_index) {
            segment.content.push_str(chunk);
        }
    }

    pub fn append_user_message_segment(
        &mut self,
        content: impl Into<String>,
        sent_at: DateTime<Utc>,
    ) {
        let content = content.into();
        if content.trim().is_empty() {
            return;
        }

        if self.user_message_segments.is_empty() {
            self.user_message_segments.push(TimedUserMessageSegment {
                content: self.user_input.clone(),
                sent_at: self.started_at,
            });
        }

        self.user_message_segments
            .push(TimedUserMessageSegment { content, sent_at });
    }

    pub fn append_user_attachments(
        &mut self,
        attachments: impl IntoIterator<Item = TurnUserAttachment>,
    ) {
        self.user_attachments.extend(attachments);
    }

    /// Complete this turn.
    pub fn complete(&mut self, response: impl Into<String>) {
        let response = response.into();
        self.response = Some(response.clone());
        if self.assistant_segments.is_empty() && !response.is_empty() {
            self.assistant_segments.push(TurnAssistantSegment {
                content: response,
                created_at: Utc::now(),
                conversation_message_id: self.assistant_message_id,
            });
        }
        self.state = TurnState::Completed;
        self.completed_at = Some(Utc::now());
        self.live_assistant_segment_index = None;
        self.assistant_message_id = self
            .assistant_segments
            .last()
            .and_then(|segment| segment.conversation_message_id);
        // Free image data — only needed for the initial LLM call, not subsequent turns
        self.image_content_parts.clear();
    }

    /// Fail this turn.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.state = TurnState::Failed;
        self.completed_at = Some(Utc::now());
        self.live_assistant_segment_index = None;
        self.image_content_parts.clear();
    }

    /// Interrupt this turn.
    pub fn interrupt(&mut self) {
        self.state = TurnState::Interrupted;
        self.completed_at = Some(Utc::now());
        self.live_assistant_segment_index = None;
        self.image_content_parts.clear();
    }

    pub fn seal_response_segment(&mut self) {
        self.live_assistant_segment_index = None;
    }

    pub fn current_assistant_segment_snapshot(&self) -> Option<(usize, Option<Uuid>, String)> {
        let idx = self.live_assistant_segment_index?;
        let segment = self.assistant_segments.get(idx)?;
        Some((
            idx,
            segment.conversation_message_id,
            segment.content.clone(),
        ))
    }

    pub fn restore_assistant_segment(
        &mut self,
        content: impl Into<String>,
        created_at: DateTime<Utc>,
        message_id: Option<Uuid>,
    ) {
        let content = content.into();
        if content.is_empty() {
            return;
        }

        match self.response.as_mut() {
            Some(existing) => existing.push_str(&content),
            None => self.response = Some(content.clone()),
        }

        self.assistant_segments.push(TurnAssistantSegment {
            content,
            created_at,
            conversation_message_id: message_id,
        });
        self.assistant_message_id = message_id;
        self.state = TurnState::Completed;
        self.completed_at = Some(created_at);
        self.live_assistant_segment_index = None;
    }

    pub fn set_assistant_segment_message_id(&mut self, index: usize, message_id: Uuid) {
        if let Some(segment) = self.assistant_segments.get_mut(index) {
            segment.conversation_message_id = Some(message_id);
            self.assistant_message_id = Some(message_id);
        }
    }

    /// Record a tool call.
    pub fn record_tool_call(&mut self, name: impl Into<String>, params: serde_json::Value) {
        self.seal_response_segment();
        self.tool_calls.push(TurnToolCall {
            name: name.into(),
            started_at: Utc::now(),
            completed_at: None,
            parameters: params,
            result: None,
            error: None,
            rationale: None,
            tool_call_id: None,
            conversation_message_id: None,
        });
    }

    /// Record a tool call with reasoning context.
    pub fn record_tool_call_with_reasoning(
        &mut self,
        name: impl Into<String>,
        params: serde_json::Value,
        rationale: Option<String>,
        tool_call_id: Option<String>,
    ) {
        self.seal_response_segment();
        self.tool_calls.push(TurnToolCall {
            name: name.into(),
            started_at: Utc::now(),
            completed_at: None,
            parameters: params,
            result: None,
            error: None,
            rationale,
            tool_call_id,
            conversation_message_id: None,
        });
    }

    /// Mark a tool call as actually started.
    pub fn mark_tool_call_started_for(&mut self, tool_call_id: &str) {
        if let Some(call) = self
            .tool_calls
            .iter_mut()
            .find(|c| c.tool_call_id.as_deref() == Some(tool_call_id))
        {
            call.started_at = Utc::now();
            call.completed_at = None;
        } else if let Some(call) = self
            .tool_calls
            .iter_mut()
            .find(|c| c.result.is_none() && c.error.is_none())
        {
            tracing::debug!(
                tool_call_id = %tool_call_id,
                fallback_tool = %call.name,
                "tool_call_id not found for start, falling back to first pending call"
            );
            call.started_at = Utc::now();
            call.completed_at = None;
        } else {
            tracing::warn!(
                tool_call_id = %tool_call_id,
                "Tool start dropped: no matching or pending tool call"
            );
        }
    }

    /// Record tool call result.
    pub fn record_tool_result(&mut self, result: serde_json::Value) {
        if let Some(call) = self.tool_calls.last_mut() {
            call.result = Some(result);
            call.completed_at = Some(Utc::now());
        }
    }

    /// Record tool call error.
    pub fn record_tool_error(&mut self, error: impl Into<String>) {
        if let Some(call) = self.tool_calls.last_mut() {
            call.error = Some(error.into());
            call.completed_at = Some(Utc::now());
        }
    }

    /// Record a tool result by tool_call_id, with fallback to first pending call.
    pub fn record_tool_result_for(&mut self, tool_call_id: &str, result: serde_json::Value) {
        if let Some(call) = self
            .tool_calls
            .iter_mut()
            .find(|c| c.tool_call_id.as_deref() == Some(tool_call_id))
        {
            call.result = Some(result);
            call.completed_at = Some(Utc::now());
        } else if let Some(call) = self
            .tool_calls
            .iter_mut()
            .find(|c| c.result.is_none() && c.error.is_none())
        {
            tracing::debug!(
                tool_call_id = %tool_call_id,
                fallback_tool = %call.name,
                "tool_call_id not found, falling back to first pending call"
            );
            call.result = Some(result);
            call.completed_at = Some(Utc::now());
        } else {
            tracing::warn!(
                tool_call_id = %tool_call_id,
                "Tool result dropped: no matching or pending tool call"
            );
        }
    }

    /// Record a tool error by tool_call_id, with fallback to first pending call.
    pub fn record_tool_error_for(&mut self, tool_call_id: &str, error: impl Into<String>) {
        if let Some(call) = self
            .tool_calls
            .iter_mut()
            .find(|c| c.tool_call_id.as_deref() == Some(tool_call_id))
        {
            call.error = Some(error.into());
            call.completed_at = Some(Utc::now());
        } else if let Some(call) = self
            .tool_calls
            .iter_mut()
            .find(|c| c.result.is_none() && c.error.is_none())
        {
            tracing::debug!(
                tool_call_id = %tool_call_id,
                fallback_tool = %call.name,
                "tool_call_id not found, falling back to first pending call"
            );
            call.error = Some(error.into());
            call.completed_at = Some(Utc::now());
        } else {
            tracing::warn!(
                tool_call_id = %tool_call_id,
                "Tool error dropped: no matching or pending tool call"
            );
        }
    }
}

fn format_user_message_for_context(content: &str, sent_at: DateTime<Utc>, user_tz: Tz) -> String {
    let local = sent_at.with_timezone(&user_tz);
    format!(
        "<user-message-time sent_at_local=\"{}\" timezone=\"{}\" sent_at_utc=\"{}\" />\n{}",
        local.to_rfc3339(),
        user_tz.name(),
        sent_at.to_rfc3339(),
        content
    )
}

fn format_user_message_segments_for_context(
    segments: &[TimedUserMessageSegment],
    user_tz: Tz,
) -> String {
    segments
        .iter()
        .map(|segment| format_user_message_for_context(&segment.content, segment.sent_at, user_tz))
        .collect::<Vec<_>>()
        .join("\n")
}

fn user_attachments_from_metadata(metadata: &serde_json::Value) -> Vec<TurnUserAttachment> {
    metadata
        .get("attachments")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<TurnUserAttachment>>(value).ok())
        .unwrap_or_default()
}

fn format_user_content_with_attachments(
    content: String,
    attachments: &[TurnUserAttachment],
) -> String {
    if attachments.is_empty() {
        return content;
    }

    let mut text = content;
    if !text.trim().is_empty() {
        text.push_str("\n\n");
    }
    text.push_str("<attachments>");
    for (index, attachment) in attachments.iter().enumerate() {
        text.push('\n');
        text.push_str(&format_turn_attachment(index + 1, attachment));
    }
    text.push_str("\n</attachments>");
    text
}

fn format_turn_attachment(index: usize, attachment: &TurnUserAttachment) -> String {
    let filename = escape_xml_attr(attachment.filename.as_deref().unwrap_or("unknown"));
    let mime = escape_xml_attr(&attachment.mime_type);

    match &attachment.kind {
        crate::channels::AttachmentKind::Audio => {
            let duration_attr = attachment
                .duration_secs
                .map(|secs| format!(" duration=\"{secs}s\""))
                .unwrap_or_default();
            let body = match attachment.extracted_text.as_deref() {
                Some(text) => format!("Transcript: {}", escape_xml_text(text)),
                None => "Audio transcript unavailable.".to_string(),
            };

            format!(
                "<attachment index=\"{index}\" type=\"audio\" filename=\"{filename}\"{duration_attr}>\n\
                 {body}\n\
                 </attachment>"
            )
        }
        crate::channels::AttachmentKind::Image => {
            let size_attr = attachment
                .size_bytes
                .map(|size| format!(" size=\"{}\"", format_attachment_size(size)))
                .unwrap_or_default();

            format!(
                "<attachment index=\"{index}\" type=\"image\" filename=\"{filename}\" mime=\"{mime}\"{size_attr}>\n\
                 [Image attached — sent as visual content]\n\
                 </attachment>"
            )
        }
        crate::channels::AttachmentKind::Document => {
            let size_attr = attachment
                .size_bytes
                .map(|size| format!(" size=\"{}\"", format_attachment_size(size)))
                .unwrap_or_default();

            match attachment.extracted_text.as_deref() {
                Some(text) => format!(
                    "<attachment index=\"{index}\" type=\"document\" filename=\"{filename}\" mime=\"{mime}\"{size_attr}>\n\
                     {}\n\
                     </attachment>",
                    escape_xml_text(text)
                ),
                None => format!(
                    "<attachment index=\"{index}\" type=\"document\" filename=\"{filename}\" mime=\"{mime}\"{size_attr}>\n\
                     [Document attached — text extraction unavailable]\n\
                     </attachment>"
                ),
            }
        }
    }
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn format_attachment_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Record of a tool call made during a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnToolCall {
    /// Tool name.
    pub name: String,
    /// When the tool call started.
    #[serde(default = "Utc::now")]
    pub started_at: DateTime<Utc>,
    /// When the tool call completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Parameters passed to the tool.
    pub parameters: serde_json::Value,
    /// Result from the tool (if successful).
    pub result: Option<serde_json::Value>,
    /// Error from the tool (if failed).
    pub error: Option<String>,
    /// Agent's reasoning for choosing this tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// The tool_call_id from the LLM, for identity-based result matching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Persisted conversation row backing this tool call in history.
    #[serde(skip)]
    pub conversation_message_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued_message(content: &str) -> PendingUserMessage {
        PendingUserMessage {
            content: content.to_string(),
            received_at: Utc::now(),
            attachments: Vec::new(),
            delivery: PendingUserMessageDelivery::AfterTurn,
        }
    }

    #[test]
    fn test_session_creation() {
        let mut session = Session::new("user-123");
        assert!(session.active_thread.is_none());

        session.create_thread();
        assert!(session.active_thread.is_some());
    }

    #[test]
    fn test_thread_turns() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Hello");
        assert_eq!(thread.state, ThreadState::Processing);
        assert_eq!(thread.turns.len(), 1);

        thread.complete_turn("Hi there!");
        assert_eq!(thread.state, ThreadState::Idle);
        assert_eq!(thread.turns[0].response, Some("Hi there!".to_string()));
    }

    #[test]
    fn test_thread_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("First message");
        thread.complete_turn("First response");
        thread.start_turn("Second message");
        thread.complete_turn("Second response");

        let messages = thread.messages();
        assert_eq!(messages.len(), 4);
    }

    #[test]
    fn test_messages_for_context_includes_user_send_time() {
        let mut thread = Thread::new(Uuid::new_v4());
        let sent_at = chrono::DateTime::parse_from_rfc3339("2026-04-13T07:08:09Z")
            .unwrap()
            .with_timezone(&Utc);

        thread.start_turn_at("现在几点了？", sent_at);
        thread.complete_turn("现在是下午。");

        let messages = thread.messages_for_context(chrono_tz::Asia::Shanghai);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, crate::llm::Role::User);
        assert!(
            messages[0]
                .content
                .contains("sent_at_local=\"2026-04-13T15:08:09+08:00\"")
        );
        assert!(messages[0].content.contains("timezone=\"Asia/Shanghai\""));
        assert!(
            messages[0]
                .content
                .contains("sent_at_utc=\"2026-04-13T07:08:09+00:00\"")
        );
        assert!(messages[0].content.ends_with("现在几点了？"));
    }

    #[test]
    fn test_messages_for_context_marks_each_queued_user_segment() {
        let mut thread = Thread::new(Uuid::new_v4());
        let first = chrono::DateTime::parse_from_rfc3339("2026-04-13T07:08:09Z")
            .unwrap()
            .with_timezone(&Utc);
        let second = chrono::DateTime::parse_from_rfc3339("2026-04-13T07:09:10Z")
            .unwrap()
            .with_timezone(&Utc);

        let turn = thread.start_turn_at("第一条\n第二条", first);
        turn.user_message_segments = vec![
            TimedUserMessageSegment {
                content: "第一条".to_string(),
                sent_at: first,
            },
            TimedUserMessageSegment {
                content: "第二条".to_string(),
                sent_at: second,
            },
        ];
        thread.complete_turn("收到。");

        let messages = thread.messages_for_context(chrono_tz::Asia::Shanghai);
        assert!(
            messages[0]
                .content
                .contains("sent_at_local=\"2026-04-13T15:08:09+08:00\"")
        );
        assert!(
            messages[0]
                .content
                .contains("sent_at_local=\"2026-04-13T15:09:10+08:00\"")
        );
        assert!(messages[0].content.contains("第一条"));
        assert!(messages[0].content.contains("第二条"));
    }

    #[test]
    fn test_messages_for_context_appends_attachment_context() {
        let mut thread = Thread::new(Uuid::new_v4());
        let turn = thread.start_turn("帮我总结附件");
        turn.user_attachments = vec![TurnUserAttachment {
            id: "att-1".to_string(),
            kind: crate::channels::AttachmentKind::Document,
            mime_type: "text/plain".to_string(),
            filename: Some("notes.txt".to_string()),
            size_bytes: Some(42),
            workspace_uri: Some("workspace://default/attachments/notes.txt".to_string()),
            extracted_text: Some("第一行\n第二行".to_string()),
            duration_secs: None,
        }];
        thread.complete_turn("收到");

        let messages = thread.messages();
        assert!(messages[0].content.contains("<attachments>"));
        assert!(messages[0].content.contains("filename=\"notes.txt\""));
        assert!(messages[0].content.contains("第一行"));
    }

    #[test]
    fn test_turn_tool_calls() {
        let mut turn = Turn::new(0, "Test input");
        turn.record_tool_call("echo", serde_json::json!({"message": "test"}));
        turn.record_tool_result(serde_json::json!("test"));

        assert_eq!(turn.tool_calls.len(), 1);
        assert!(turn.tool_calls[0].result.is_some());
    }

    #[test]
    fn streamed_chunks_accumulate_on_turn_response() {
        let mut turn = Turn::new(0, "Test input");
        turn.append_response_chunk("Hel");
        turn.append_response_chunk("lo");

        assert_eq!(turn.response.as_deref(), Some("Hello"));
    }

    #[test]
    fn streamed_chunks_split_into_assistant_segments_across_tool_boundaries() {
        let mut turn = Turn::new(0, "Test input");
        turn.append_response_chunk("先查一下");
        turn.record_tool_call_with_reasoning(
            "search",
            serde_json::json!({ "q": "test" }),
            None,
            Some("call_1".to_string()),
        );
        turn.append_response_chunk("查到了");

        assert_eq!(turn.response.as_deref(), Some("先查一下查到了"));
        assert_eq!(turn.assistant_segments.len(), 2);
        assert_eq!(turn.assistant_segments[0].content, "先查一下");
        assert_eq!(turn.assistant_segments[1].content, "查到了");
    }

    #[test]
    fn test_restore_from_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        // First add some turns
        thread.start_turn("Original message");
        thread.complete_turn("Original response");

        // Now restore from different messages
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
            ChatMessage::user("How are you?"),
            ChatMessage::assistant("I'm good!"),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].user_input, "Hello");
        assert_eq!(thread.turns[0].response, Some("Hi there!".to_string()));
        assert_eq!(thread.turns[1].user_input, "How are you?");
        assert_eq!(thread.turns[1].response, Some("I'm good!".to_string()));
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_restore_from_conversation_messages_preserves_started_at() {
        let mut thread = Thread::new(Uuid::new_v4());
        let user_created_at = chrono::DateTime::parse_from_rfc3339("2026-04-13T07:08:09Z")
            .unwrap()
            .with_timezone(&Utc);
        let assistant_created_at = chrono::DateTime::parse_from_rfc3339("2026-04-13T07:08:30Z")
            .unwrap()
            .with_timezone(&Utc);

        let db_messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "帮我看一下今天的安排".to_string(),
                metadata: serde_json::json!({}),
                created_at: user_created_at,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "今天有两个会议。".to_string(),
                metadata: serde_json::json!({}),
                created_at: assistant_created_at,
            },
        ];

        thread.restore_from_conversation_messages(&db_messages);

        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].started_at, user_created_at);

        let messages = thread.messages_for_context(chrono_tz::Asia::Shanghai);
        assert!(
            messages[0]
                .content
                .contains("sent_at_local=\"2026-04-13T15:08:09+08:00\"")
        );
        assert!(messages[0].content.ends_with("帮我看一下今天的安排"));
    }

    #[test]
    fn test_restore_from_conversation_messages_preserves_multiple_assistant_segments() {
        let mut thread = Thread::new(Uuid::new_v4());
        let user_message_id = Uuid::new_v4();
        let first_assistant_id = Uuid::new_v4();
        let tool_call_id = Uuid::new_v4();
        let second_assistant_id = Uuid::new_v4();

        let db_messages = vec![
            crate::history::ConversationMessage {
                id: user_message_id,
                role: "user".to_string(),
                content: "帮我看看".to_string(),
                metadata: serde_json::json!({}),
                created_at: chrono::DateTime::parse_from_rfc3339("2026-04-13T07:08:09Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
            crate::history::ConversationMessage {
                id: first_assistant_id,
                role: "assistant".to_string(),
                content: "我先查一下。".to_string(),
                metadata: serde_json::json!({}),
                created_at: chrono::DateTime::parse_from_rfc3339("2026-04-13T07:08:10Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
            crate::history::ConversationMessage {
                id: tool_call_id,
                role: "tool_call".to_string(),
                content: serde_json::json!({
                    "call_id": "call_1",
                    "name": "search",
                    "parameters": { "q": "安排" },
                    "result_preview": "done"
                })
                .to_string(),
                metadata: serde_json::json!({}),
                created_at: chrono::DateTime::parse_from_rfc3339("2026-04-13T07:08:11Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
            crate::history::ConversationMessage {
                id: second_assistant_id,
                role: "assistant".to_string(),
                content: "查到了两个事项。".to_string(),
                metadata: serde_json::json!({}),
                created_at: chrono::DateTime::parse_from_rfc3339("2026-04-13T07:08:12Z")
                    .unwrap()
                    .with_timezone(&Utc),
            },
        ];

        thread.restore_from_conversation_messages(&db_messages);

        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].assistant_segments.len(), 2);
        assert_eq!(
            thread.turns[0].assistant_segments[0].content,
            "我先查一下。"
        );
        assert_eq!(
            thread.turns[0].assistant_segments[1].content,
            "查到了两个事项。"
        );
        assert_eq!(
            thread.turns[0].assistant_message_id,
            Some(second_assistant_id)
        );
    }

    #[test]
    fn test_restore_from_conversation_messages_restores_user_attachments() {
        let mut thread = Thread::new(Uuid::new_v4());
        let message_id = Uuid::new_v4();
        let db_messages = vec![crate::history::ConversationMessage {
            id: message_id,
            role: "user".to_string(),
            content: "看看这个文件".to_string(),
            metadata: serde_json::json!({
                "attachments": [{
                    "id": "att-1",
                    "kind": "Document",
                    "mime_type": "application/pdf",
                    "filename": "report.pdf",
                    "size_bytes": 2048,
                    "workspace_uri": "workspace://default/attachments/report.pdf",
                    "extracted_text": "Quarterly report",
                    "duration_secs": null
                }]
            }),
            created_at: Utc::now(),
        }];

        thread.restore_from_conversation_messages(&db_messages);

        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].user_message_id, Some(message_id));
        assert_eq!(thread.turns[0].user_attachments.len(), 1);
        assert_eq!(
            thread.turns[0].user_attachments[0].filename.as_deref(),
            Some("report.pdf")
        );
    }

    #[test]
    fn test_restore_from_messages_incomplete_turn() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Messages with incomplete last turn (no assistant response)
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
            ChatMessage::user("How are you?"),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[1].user_input, "How are you?");
        assert!(thread.turns[1].response.is_none());
    }

    #[test]
    fn test_enter_auth_mode() {
        let before = Utc::now();
        let mut thread = Thread::new(Uuid::new_v4());
        assert!(thread.pending_auth.is_none());

        thread.enter_auth_mode("desktop-auth".to_string());
        assert!(thread.pending_auth.is_some());
        let pending = thread.pending_auth.as_ref().unwrap();
        assert_eq!(pending.extension_name, "desktop-auth");
        assert!(pending.created_at >= before);
        assert!(!pending.is_expired());
    }

    #[test]
    fn test_take_pending_auth() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.enter_auth_mode("notion".to_string());

        let pending = thread.take_pending_auth();
        assert!(pending.is_some());
        let pending = pending.unwrap();
        assert_eq!(pending.extension_name, "notion");
        assert!(!pending.is_expired());
        // Should be cleared after take
        assert!(thread.pending_auth.is_none());
        assert!(thread.take_pending_auth().is_none());
    }

    #[test]
    fn test_pending_auth_serialization() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.enter_auth_mode("openai".to_string());

        let json = serde_json::to_string(&thread).expect("should serialize");
        assert!(json.contains("pending_auth"));
        assert!(json.contains("openai"));
        assert!(json.contains("created_at"));

        let restored: Thread = serde_json::from_str(&json).expect("should deserialize");
        assert!(restored.pending_auth.is_some());
        let pending = restored.pending_auth.unwrap();
        assert_eq!(pending.extension_name, "openai");
        assert!(!pending.is_expired());
    }

    #[test]
    fn test_pending_auth_expiry() {
        let mut pending = PendingAuth {
            extension_name: "test".to_string(),
            created_at: Utc::now(),
        };
        assert!(!pending.is_expired());
        // Backdate beyond the TTL
        pending.created_at = Utc::now() - AUTH_MODE_TTL - TimeDelta::seconds(1);
        assert!(pending.is_expired());
    }

    #[test]
    fn test_pending_auth_default_none() {
        // Deserialization of old data without pending_auth should default to None
        let mut thread = Thread::new(Uuid::new_v4());
        thread.pending_auth = None;
        let json = serde_json::to_string(&thread).expect("serialize");

        // Remove the pending_auth field to simulate old data
        let json = json.replace(",\"pending_auth\":null", "");
        let restored: Thread = serde_json::from_str(&json).expect("should deserialize");
        assert!(restored.pending_auth.is_none());
    }

    #[test]
    fn test_thread_with_id() {
        let specific_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let thread = Thread::with_id(specific_id, session_id);

        assert_eq!(thread.id, specific_id);
        assert_eq!(thread.session_id, session_id);
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_thread_with_id_restore_messages() {
        let thread_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut thread = Thread::with_id(thread_id, session_id);

        let messages = vec![
            ChatMessage::user("Hello from DB"),
            ChatMessage::assistant("Restored response"),
        ];
        thread.restore_from_messages(messages);

        assert_eq!(thread.id, thread_id);
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].user_input, "Hello from DB");
        assert_eq!(
            thread.turns[0].response,
            Some("Restored response".to_string())
        );
    }

    #[test]
    fn test_restore_from_messages_empty() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Add a turn first, then restore with empty vec
        thread.start_turn("hello");
        thread.complete_turn("hi");
        assert_eq!(thread.turns.len(), 1);

        thread.restore_from_messages(Vec::new());

        // Should clear all turns and stay idle
        assert!(thread.turns.is_empty());
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_restore_from_messages_only_assistant_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Only assistant messages (no user messages to anchor turns)
        let messages = vec![
            ChatMessage::assistant("I'm here"),
            ChatMessage::assistant("Still here"),
        ];

        thread.restore_from_messages(messages);

        // Assistant-only messages have no user turn to attach to, so
        // they should be skipped entirely.
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_restore_from_messages_multiple_user_messages_in_a_row() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Two user messages with no assistant response between them
        let messages = vec![
            ChatMessage::user("first"),
            ChatMessage::user("second"),
            ChatMessage::assistant("reply to second"),
        ];

        thread.restore_from_messages(messages);

        // First user message becomes a turn with no response,
        // second user message pairs with the assistant response.
        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].user_input, "first");
        assert!(thread.turns[0].response.is_none());
        assert_eq!(thread.turns[1].user_input, "second");
        assert_eq!(
            thread.turns[1].response,
            Some("reply to second".to_string())
        );
    }

    #[test]
    fn test_thread_switch() {
        let mut session = Session::new("user-1");

        let t1_id = session.create_thread().id;
        let t2_id = session.create_thread().id;

        // After creating two threads, active should be the last one
        assert_eq!(session.active_thread, Some(t2_id));

        // Switch back to the first
        assert!(session.switch_thread(t1_id));
        assert_eq!(session.active_thread, Some(t1_id));

        // Switching to a nonexistent thread should fail
        let fake_id = Uuid::new_v4();
        assert!(!session.switch_thread(fake_id));
        // Active thread should remain unchanged
        assert_eq!(session.active_thread, Some(t1_id));
    }

    #[test]
    fn test_get_or_create_thread_idempotent() {
        let mut session = Session::new("user-1");

        let tid1 = session.get_or_create_thread().id;
        let tid2 = session.get_or_create_thread().id;

        // Should return the same thread (not create a new one each time)
        assert_eq!(tid1, tid2);
        assert_eq!(session.threads.len(), 1);
    }

    #[test]
    fn test_truncate_turns() {
        let mut thread = Thread::new(Uuid::new_v4());

        for i in 0..5 {
            thread.start_turn(format!("msg-{}", i));
            thread.complete_turn(format!("resp-{}", i));
        }
        assert_eq!(thread.turns.len(), 5);

        thread.truncate_turns(3);
        assert_eq!(thread.turns.len(), 3);

        // Should keep the most recent turns
        assert_eq!(thread.turns[0].user_input, "msg-2");
        assert_eq!(thread.turns[1].user_input, "msg-3");
        assert_eq!(thread.turns[2].user_input, "msg-4");

        // Turn numbers should be re-indexed
        assert_eq!(thread.turns[0].turn_number, 0);
        assert_eq!(thread.turns[1].turn_number, 1);
        assert_eq!(thread.turns[2].turn_number, 2);
    }

    #[test]
    fn test_truncate_turns_noop_when_fewer() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("only one");
        thread.complete_turn("response");

        thread.truncate_turns(10);
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].user_input, "only one");
    }

    #[test]
    fn test_thread_interrupt_and_resume() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("do something");
        assert_eq!(thread.state, ThreadState::Processing);

        thread.interrupt();
        assert_eq!(thread.state, ThreadState::Interrupted);

        let last_turn = thread.last_turn().unwrap();
        assert_eq!(last_turn.state, TurnState::Interrupted);
        assert!(last_turn.completed_at.is_some());

        thread.resume();
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_resume_only_from_interrupted() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Idle thread: resume should be a no-op
        assert_eq!(thread.state, ThreadState::Idle);
        thread.resume();
        assert_eq!(thread.state, ThreadState::Idle);

        // Processing thread: resume should not change state
        thread.start_turn("work");
        assert_eq!(thread.state, ThreadState::Processing);
        thread.resume();
        assert_eq!(thread.state, ThreadState::Processing);
    }

    #[test]
    fn test_turn_fail() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("risky operation");
        thread.fail_turn("connection timed out");

        assert_eq!(thread.state, ThreadState::Idle);

        let turn = thread.last_turn().unwrap();
        assert_eq!(turn.state, TurnState::Failed);
        assert_eq!(turn.error, Some("connection timed out".to_string()));
        assert!(turn.response.is_none());
        assert!(turn.completed_at.is_some());
    }

    #[test]
    fn test_messages_with_incomplete_last_turn() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("first");
        thread.complete_turn("first reply");
        thread.start_turn("second (in progress)");

        let messages = thread.messages();
        // Should have 3 messages: user, assistant, user (no assistant for in-progress)
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].content, "first reply");
        assert_eq!(messages[2].content, "second (in progress)");
    }

    #[test]
    fn test_thread_serialization_round_trip() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("hello");
        thread.complete_turn("world");

        let json = serde_json::to_string(&thread).unwrap();
        let restored: Thread = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.id, thread.id);
        assert_eq!(restored.session_id, thread.session_id);
        assert_eq!(restored.turns.len(), 1);
        assert_eq!(restored.turns[0].user_input, "hello");
        assert_eq!(restored.turns[0].response, Some("world".to_string()));
    }

    #[test]
    fn test_session_serialization_round_trip() {
        let mut session = Session::new("user-ser");
        session.create_thread();
        session.auto_approve_tool("echo");

        let json = serde_json::to_string(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.user_id, "user-ser");
        assert_eq!(restored.threads.len(), 1);
        assert!(restored.is_tool_auto_approved("echo"));
        assert!(!restored.is_tool_auto_approved("shell"));
    }

    #[test]
    fn test_auto_approved_tools() {
        let mut session = Session::new("user-1");

        assert!(!session.is_tool_auto_approved("shell"));
        session.auto_approve_tool("shell");
        assert!(session.is_tool_auto_approved("shell"));

        // Idempotent
        session.auto_approve_tool("shell");
        assert_eq!(session.auto_approved_tools.len(), 1);
    }

    #[test]
    fn test_auto_approved_path_prefixes() {
        let mut session = Session::new("user-1");
        let root = std::env::temp_dir().join("cowork-approval-root");
        let nested = root.join("nested/file.txt");
        let outside = std::env::temp_dir().join("cowork-other/file.txt");

        assert!(!session.is_path_auto_approved(&nested));
        session.auto_approve_path_prefix(root.display().to_string());
        assert!(session.is_path_auto_approved(&nested));
        assert!(!session.is_path_auto_approved(&outside));
    }

    #[test]
    fn test_turn_tool_call_error() {
        let mut turn = Turn::new(0, "test");
        turn.record_tool_call("http", serde_json::json!({"url": "example.com"}));
        turn.record_tool_error("timeout");

        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].error, Some("timeout".to_string()));
        assert!(turn.tool_calls[0].result.is_none());
    }

    #[test]
    fn test_turn_number_increments() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Before any turns, turn_number() is 1 (1-indexed for display)
        assert_eq!(thread.turn_number(), 1);

        thread.start_turn("first");
        thread.complete_turn("done");
        assert_eq!(thread.turn_number(), 2);

        thread.start_turn("second");
        assert_eq!(thread.turn_number(), 3);
    }

    #[test]
    fn test_complete_turn_on_empty_thread() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Completing a turn when there are no turns should be a safe no-op
        thread.complete_turn("phantom response");
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_fail_turn_on_empty_thread() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Failing a turn when there are no turns should be a safe no-op
        thread.fail_turn("phantom error");
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_pending_approval_flow() {
        let mut thread = Thread::new(Uuid::new_v4());

        let approval = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"command": "rm -rf /"}),
            display_parameters: serde_json::json!({"command": "rm -rf /"}),
            description: "dangerous command".to_string(),
            tool_call_id: "call_123".to_string(),
            context_messages: vec![ChatMessage::user("do it")],
            deferred_tool_calls: vec![],
            user_timezone: None,
            allow_always: false,
        };

        thread.await_approval(approval);
        assert_eq!(thread.state, ThreadState::AwaitingApproval);
        assert!(thread.pending_approval.is_some());

        let taken = thread.take_pending_approval();
        assert!(taken.is_some());
        assert_eq!(taken.unwrap().tool_name, "shell");
        assert!(thread.pending_approval.is_none());
    }

    #[test]
    fn test_clear_pending_approval() {
        let mut thread = Thread::new(Uuid::new_v4());

        let approval = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "http".to_string(),
            parameters: serde_json::json!({}),
            display_parameters: serde_json::json!({}),
            description: "test".to_string(),
            tool_call_id: "call_456".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![],
            user_timezone: None,
            allow_always: true,
        };

        thread.await_approval(approval);
        thread.clear_pending_approval();

        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.pending_approval.is_none());
    }

    #[test]
    fn test_active_thread_accessors() {
        let mut session = Session::new("user-1");

        assert!(session.active_thread().is_none());
        assert!(session.active_thread_mut().is_none());

        let tid = session.create_thread().id;

        assert!(session.active_thread().is_some());
        assert_eq!(session.active_thread().unwrap().id, tid);

        // Mutably modify through accessor
        session.active_thread_mut().unwrap().start_turn("test");
        assert_eq!(
            session.active_thread().unwrap().state,
            ThreadState::Processing
        );
    }

    // Regression tests for #568: tool call history must survive hydration.

    #[test]
    fn test_messages_includes_tool_calls() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Search for X");
        {
            let turn = thread.turns.last_mut().unwrap();
            turn.record_tool_call("workspace_search", serde_json::json!({"query": "X"}));
            turn.record_tool_result(serde_json::json!("Found X in doc.md"));
        }
        thread.complete_turn("I found X in doc.md.");

        let messages = thread.messages();
        // user + assistant_with_tool_calls + tool_result + assistant = 4
        assert_eq!(messages.len(), 4);

        assert_eq!(messages[0].role, crate::llm::Role::User);
        assert_eq!(messages[0].content, "Search for X");

        assert_eq!(messages[1].role, crate::llm::Role::Assistant);
        assert!(messages[1].tool_calls.is_some());
        let tcs = messages[1].tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].name, "workspace_search");

        assert_eq!(messages[2].role, crate::llm::Role::Tool);
        assert!(messages[2].content.contains("Found X"));

        assert_eq!(messages[3].role, crate::llm::Role::Assistant);
        assert_eq!(messages[3].content, "I found X in doc.md.");
    }

    #[test]
    fn test_messages_multiple_tool_calls_per_turn() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Do two things");
        {
            let turn = thread.turns.last_mut().unwrap();
            turn.record_tool_call("echo", serde_json::json!({"msg": "a"}));
            turn.record_tool_result(serde_json::json!("a"));
            turn.record_tool_call("time", serde_json::json!({}));
            turn.record_tool_error("timeout");
        }
        thread.complete_turn("Done.");

        let messages = thread.messages();
        // user + assistant_with_calls(2) + tool_result + tool_result + assistant = 5
        assert_eq!(messages.len(), 5);

        let tcs = messages[1].tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 2);

        // First tool: success
        assert_eq!(messages[2].content, "a");
        // Second tool: error (passed through directly, no wrapping)
        assert!(messages[3].content.contains("timeout"));
    }

    #[test]
    fn test_restore_from_messages_with_tool_calls() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Build a message sequence with tool calls
        let tc = ToolCall {
            id: "call_0".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "test"}),
            reasoning: None,
        };
        let messages = vec![
            ChatMessage::user("Find test"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
            ChatMessage::tool_result("call_0", "search", "result: found"),
            ChatMessage::assistant("Found it."),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 1);
        let turn = &thread.turns[0];
        assert_eq!(turn.user_input, "Find test");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "search");
        assert_eq!(
            turn.tool_calls[0].result,
            Some(serde_json::Value::String("result: found".to_string()))
        );
        assert_eq!(turn.response, Some("Found it.".to_string()));
    }

    #[test]
    fn test_restore_from_messages_with_tool_error() {
        let mut thread = Thread::new(Uuid::new_v4());

        let tc = ToolCall {
            id: "call_0".to_string(),
            name: "http".to_string(),
            arguments: serde_json::json!({}),
            reasoning: None,
        };
        let messages = vec![
            ChatMessage::user("Fetch URL"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
            ChatMessage::tool_result("call_0", "http", "Error: timeout"),
            ChatMessage::assistant("The request timed out."),
        ];

        thread.restore_from_messages(messages);

        // restore_from_messages stores all tool content as result (not error),
        // because it can't reliably distinguish errors from results that happen
        // to start with "Error: ". The content is preserved for LLM context.
        let turn = &thread.turns[0];
        assert_eq!(
            turn.tool_calls[0].result,
            Some(serde_json::Value::String("Error: timeout".to_string()))
        );
    }

    #[test]
    fn test_messages_round_trip_with_tools() {
        // Build a thread with tool calls, get messages(), restore, get messages() again
        // The two message sequences should be equivalent.
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Do search");
        {
            let turn = thread.turns.last_mut().unwrap();
            turn.record_tool_call("search", serde_json::json!({"q": "test"}));
            turn.record_tool_result(serde_json::json!("found"));
        }
        thread.complete_turn("Here are results.");

        let messages_original = thread.messages();

        // Restore into a new thread
        let mut thread2 = Thread::new(Uuid::new_v4());
        thread2.restore_from_messages(messages_original.clone());

        let messages_restored = thread2.messages();

        // Same number of messages
        assert_eq!(messages_original.len(), messages_restored.len());

        // Same roles
        for (orig, rest) in messages_original.iter().zip(messages_restored.iter()) {
            assert_eq!(orig.role, rest.role);
        }

        // Same final response
        assert_eq!(
            messages_original.last().unwrap().content,
            messages_restored.last().unwrap().content
        );
    }

    #[test]
    fn test_restore_multi_stage_tool_calls() {
        let mut thread = Thread::new(Uuid::new_v4());

        let tc1 = ToolCall {
            id: "call_a".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "data"}),
            reasoning: None,
        };
        let tc2 = ToolCall {
            id: "call_b".to_string(),
            name: "write".to_string(),
            arguments: serde_json::json!({"path": "out.txt"}),
            reasoning: None,
        };
        let messages = vec![
            ChatMessage::user("Find and save"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc1]),
            ChatMessage::tool_result("call_a", "search", "found data"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc2]),
            ChatMessage::tool_result("call_b", "write", "written"),
            ChatMessage::assistant("Done, saved to out.txt"),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 1);
        let turn = &thread.turns[0];
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].name, "search");
        assert_eq!(turn.tool_calls[1].name, "write");
        assert_eq!(
            turn.tool_calls[0].result,
            Some(serde_json::Value::String("found data".to_string()))
        );
        assert_eq!(
            turn.tool_calls[1].result,
            Some(serde_json::Value::String("written".to_string()))
        );
        assert_eq!(turn.response, Some("Done, saved to out.txt".to_string()));
    }

    #[test]
    fn test_messages_truncates_large_tool_results() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Read big file");
        {
            let turn = thread.turns.last_mut().unwrap();
            turn.record_tool_call("read_file", serde_json::json!({"path": "big.txt"}));
            let big_result = "x".repeat(2000);
            turn.record_tool_result(serde_json::json!(big_result));
        }
        thread.complete_turn("Here's the file content.");

        let messages = thread.messages();
        let tool_result_content = &messages[2].content;
        assert!(
            tool_result_content.len() <= 1010,
            "Tool result should be truncated, got {} chars",
            tool_result_content.len()
        );
        assert!(tool_result_content.ends_with("..."));
    }

    #[test]
    fn test_thread_message_queue() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Queue is initially empty
        assert!(thread.pending_messages.is_empty());
        assert!(thread.take_pending_message().is_none());

        // Queue messages and verify FIFO ordering
        assert!(thread.queue_message("first".to_string(), Utc::now()));
        assert!(thread.queue_message("second".to_string(), Utc::now()));
        assert!(thread.queue_message("third".to_string(), Utc::now()));
        assert_eq!(thread.pending_messages.len(), 3);

        assert_eq!(
            thread.take_pending_message().map(|msg| msg.content),
            Some("first".to_string())
        );
        assert_eq!(
            thread.take_pending_message().map(|msg| msg.content),
            Some("second".to_string())
        );
        assert_eq!(
            thread.take_pending_message().map(|msg| msg.content),
            Some("third".to_string())
        );
        assert!(thread.take_pending_message().is_none());

        // Fill to capacity — all 10 should succeed
        for i in 0..MAX_PENDING_MESSAGES {
            assert!(thread.queue_message(format!("msg-{}", i), Utc::now()));
        }
        assert_eq!(thread.pending_messages.len(), MAX_PENDING_MESSAGES);

        // 11th message rejected by queue_message itself
        assert!(!thread.queue_message("overflow".to_string(), Utc::now()));
        assert_eq!(thread.pending_messages.len(), MAX_PENDING_MESSAGES);

        // Drain and verify order
        for i in 0..MAX_PENDING_MESSAGES {
            assert_eq!(
                thread.take_pending_message().map(|msg| msg.content),
                Some(format!("msg-{}", i))
            );
        }
        assert!(thread.take_pending_message().is_none());
    }

    #[test]
    fn test_injectable_pending_messages_preempt_after_turn_queue() {
        let mut thread = Thread::new(Uuid::new_v4());

        assert!(thread.queue_message("queued-1".to_string(), Utc::now()));
        assert!(thread.queue_pending_message_for_next_opportunity(queued_message("sheer-1")));
        assert!(thread.queue_pending_message_for_next_opportunity(queued_message("sheer-2")));
        assert!(thread.queue_message("queued-2".to_string(), Utc::now()));

        assert_eq!(
            thread.take_next_injectable_message().map(|msg| msg.content),
            Some("sheer-1".to_string())
        );
        assert_eq!(
            thread.take_next_injectable_message().map(|msg| msg.content),
            Some("sheer-2".to_string())
        );
        assert_eq!(
            thread.take_pending_message().map(|msg| msg.content),
            Some("queued-1".to_string())
        );
        assert_eq!(
            thread.take_pending_message().map(|msg| msg.content),
            Some("queued-2".to_string())
        );
    }

    #[test]
    fn test_thread_message_queue_serialization() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Empty queue should not appear in serialization (skip_serializing_if)
        let json = serde_json::to_string(&thread).unwrap();
        assert!(!json.contains("pending_messages"));

        // Non-empty queue should serialize and deserialize
        thread.queue_message("queued msg".to_string(), Utc::now());
        let json = serde_json::to_string(&thread).unwrap();
        assert!(json.contains("pending_messages"));
        assert!(json.contains("queued msg"));

        let restored: Thread = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.pending_messages.len(), 1);
        assert_eq!(restored.pending_messages[0].content, "queued msg");
    }

    #[test]
    fn test_thread_message_queue_default_on_old_data() {
        // Deserialization of old data without pending_messages should default to empty
        let thread = Thread::new(Uuid::new_v4());
        let json = serde_json::to_string(&thread).unwrap();

        // The field is absent (skip_serializing_if), simulating old data
        assert!(!json.contains("pending_messages"));
        let restored: Thread = serde_json::from_str(&json).unwrap();
        assert!(restored.pending_messages.is_empty());
    }

    #[test]
    fn test_interrupt_clears_pending_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Start a turn so there's something to interrupt
        thread.start_turn("initial input");

        // Queue several messages while "processing"
        thread.queue_message("queued-1".to_string(), Utc::now());
        thread.queue_message("queued-2".to_string(), Utc::now());
        thread.queue_message("queued-3".to_string(), Utc::now());
        assert_eq!(thread.pending_messages.len(), 3);

        // Interrupt should clear the queue
        thread.interrupt();
        assert!(thread.pending_messages.is_empty());
        assert_eq!(thread.state, ThreadState::Interrupted);
    }

    #[test]
    fn test_thread_state_idle_after_full_drain() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Simulate a full drain cycle: start turn, queue messages, complete turn,
        // then drain all queued messages as a single merged turn (#259).
        thread.start_turn("turn 1");
        assert_eq!(thread.state, ThreadState::Processing);

        thread.queue_message("queued-a".to_string(), Utc::now());
        thread.queue_message("queued-b".to_string(), Utc::now());

        // Complete the turn (simulates process_user_input finishing)
        thread.complete_turn("response 1");
        assert_eq!(thread.state, ThreadState::Idle);

        // Drain: merge all queued messages and process as a single turn
        let merged = thread.drain_pending_messages().unwrap();
        let merged_content = merged
            .iter()
            .map(|msg| msg.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(merged_content, "queued-a\nqueued-b");
        thread.start_turn(&merged_content);
        thread.complete_turn("response for merged");

        // Queue is fully drained, thread is idle
        assert!(thread.drain_pending_messages().is_none());
        assert!(thread.pending_messages.is_empty());
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_drain_pending_messages_merges_with_newlines() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Empty queue returns None
        assert!(thread.drain_pending_messages().is_none());

        // Single message returned as-is (no trailing newline)
        thread.queue_message("only one".to_string(), Utc::now());
        assert_eq!(
            thread
                .drain_pending_messages()
                .map(|msgs| msgs.into_iter().map(|msg| msg.content).collect::<Vec<_>>()),
            Some(vec!["only one".to_string()]),
        );
        assert!(thread.pending_messages.is_empty());

        // Multiple messages joined with newlines
        thread.queue_message("hey".to_string(), Utc::now());
        thread.queue_message("can you check the server".to_string(), Utc::now());
        thread.queue_message("it started 10 min ago".to_string(), Utc::now());
        assert_eq!(
            thread
                .drain_pending_messages()
                .map(|msgs| msgs.into_iter().map(|msg| msg.content).collect::<Vec<_>>()),
            Some(vec![
                "hey".to_string(),
                "can you check the server".to_string(),
                "it started 10 min ago".to_string(),
            ]),
        );
        assert!(thread.pending_messages.is_empty());

        // Queue is empty after drain
        assert!(thread.drain_pending_messages().is_none());
    }

    #[test]
    fn test_requeue_drained_preserves_content_at_front() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Re-queue into empty queue
        thread.requeue_drained(vec![queued_message("failed batch")]);
        assert_eq!(thread.pending_messages.len(), 1);
        assert_eq!(thread.pending_messages[0].content, "failed batch");

        // New messages go behind the re-queued content
        thread.queue_message("new msg".to_string(), Utc::now());
        assert_eq!(thread.pending_messages.len(), 2);

        // Drain should return re-queued content first (front of queue)
        let merged = thread
            .drain_pending_messages()
            .unwrap()
            .into_iter()
            .map(|msg| msg.content)
            .collect::<Vec<_>>();
        assert_eq!(
            merged,
            vec!["failed batch".to_string(), "new msg".to_string()]
        );
    }

    #[test]
    fn test_record_tool_result_for_by_id() {
        let mut turn = Turn::new(0, "test");
        turn.record_tool_call_with_reasoning(
            "tool_a",
            serde_json::json!({}),
            None,
            Some("id_a".into()),
        );
        turn.record_tool_call_with_reasoning(
            "tool_b",
            serde_json::json!({}),
            None,
            Some("id_b".into()),
        );

        // Record result for second tool by ID
        turn.record_tool_result_for("id_b", serde_json::json!("result_b"));
        assert!(turn.tool_calls[0].result.is_none());
        assert_eq!(
            turn.tool_calls[1].result.as_ref().unwrap(),
            &serde_json::json!("result_b")
        );
    }

    #[test]
    fn test_record_tool_error_for_by_id() {
        let mut turn = Turn::new(0, "test");
        turn.record_tool_call_with_reasoning(
            "tool_a",
            serde_json::json!({}),
            None,
            Some("id_a".into()),
        );
        turn.record_tool_call_with_reasoning(
            "tool_b",
            serde_json::json!({}),
            None,
            Some("id_b".into()),
        );

        turn.record_tool_error_for("id_a", "failed");
        assert_eq!(turn.tool_calls[0].error.as_deref(), Some("failed"));
        assert!(turn.tool_calls[1].error.is_none());
    }

    #[test]
    fn test_record_tool_result_for_fallback_to_pending() {
        let mut turn = Turn::new(0, "test");
        turn.record_tool_call_with_reasoning(
            "tool_a",
            serde_json::json!({}),
            None,
            Some("id_a".into()),
        );
        turn.record_tool_call_with_reasoning(
            "tool_b",
            serde_json::json!({}),
            None,
            Some("id_b".into()),
        );

        // First tool already has a result
        turn.tool_calls[0].result = Some(serde_json::json!("done"));

        // Unknown ID should fall back to first pending (tool_b)
        turn.record_tool_result_for("unknown_id", serde_json::json!("fallback"));
        assert_eq!(
            turn.tool_calls[0].result.as_ref().unwrap(),
            &serde_json::json!("done")
        );
        assert_eq!(
            turn.tool_calls[1].result.as_ref().unwrap(),
            &serde_json::json!("fallback")
        );
    }

    #[test]
    fn test_record_tool_result_for_no_pending_is_noop() {
        let mut turn = Turn::new(0, "test");
        turn.record_tool_call_with_reasoning(
            "tool_a",
            serde_json::json!({}),
            None,
            Some("id_a".into()),
        );
        turn.tool_calls[0].result = Some(serde_json::json!("done"));

        // No pending calls, unknown ID — should be a no-op
        turn.record_tool_result_for("unknown_id", serde_json::json!("lost"));
        assert_eq!(
            turn.tool_calls[0].result.as_ref().unwrap(),
            &serde_json::json!("done")
        );
    }
}
