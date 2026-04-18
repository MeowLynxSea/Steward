# Agent Module

Core agent logic. This is the most complex subsystem — read this before working in `src/agent/`.

## Module Map

| File | Role |
|------|------|
| `agent_loop.rs` | `Agent` struct, `AgentDeps`, main `run()` event loop. Delegates to siblings. |
| `dispatcher.rs` | Agentic loop for conversational turns: LLM call → tool execution → repeat. Injects skill context. Returns `Response` or `NeedApproval`. |
| `thread_ops.rs` | Thread/session operations: `process_user_input`, undo/redo, approval, auth-mode interception, DB hydration, compaction. |
| `commands.rs` | System command handlers (`/help`, `/model`, `/status`, `/skills`, etc.) and job intent handlers. |
| `session.rs` | Data model: `Session` → `Thread` → `Turn`. State machines for threads and turns. |
| `session_manager.rs` | Lifecycle: create/lookup sessions, map external thread IDs to internal UUIDs, manage undo managers. |
| `router.rs` | Routes explicit `/commands` to `MessageIntent`. Natural language bypasses the router entirely. |
| `scheduler.rs` | Parallel job scheduling. Maintains `jobs` map (full LLM-driven) and `subtasks` map (tool-exec/background). |
| *(moved to `src/worker/job.rs`)* | Per-job execution now lives in `src/worker/job.rs` as `JobDelegate`, using the shared `run_agentic_loop()` engine. |
| `agentic_loop.rs` | Shared agentic loop engine: `run_agentic_loop()`, `LoopDelegate` trait, `LoopOutcome`, `LoopSignal`, `TextAction`. All execution paths (chat, job, Claude Code) delegate to this. |
| `compaction.rs` | Context window management: summarize old turns, persist them into episodic memory when available, and trim context. Three strategies. |
| `context_monitor.rs` | Detects memory pressure. Suggests `CompactionStrategy` based on usage level. |
| `../conversation_recall/` | Cross-conversation history recall. Maintains turn-level recall docs derived from persisted conversation history, prompt injection helpers, and explicit history retrieval APIs. |
| `self_repair.rs` | Detects stuck jobs and broken tools, attempts recovery. |
| `heartbeat.rs` | Proactive periodic execution. Reads the heartbeat procedure from native memory graph (fallback to `HEARTBEAT.md`), notifies via channel if findings. |
| `submission.rs` | Parses all user submissions into typed variants before routing. |
| `undo.rs` | Turn-based undo/redo with checkpoints. Checkpoints store message lists (max 20 by default). |
| `routine.rs` | `Routine` types: `Trigger` (cron/event/system_event/manual) + `RoutineAction` (lightweight/full_job) + `RoutineGuardrails`. |
| `routine_engine.rs` | Cron ticker and event matcher. Fires routines when triggers match. Lightweight runs inline; full_job dispatches to `Scheduler`. System-event runs now persist the full trigger payload on `RoutineRun` so lightweight routines can inspect structured event context instead of only a short detail string. |
| `task.rs` | Task types for the scheduler: `Job`, `ToolExec`, `Background`. Used by `spawn_subtask` and `spawn_batch`. |
| `cost_guard.rs` | LLM spend and action-rate enforcement. Tracks daily budget (cents) and hourly call rate. Lives in `AgentDeps`. |
| `job_monitor.rs` | Subscribes to runtime job events and injects Claude Code job output back into the agent loop as `IncomingMessage`. |

## Session / Thread / Turn Model

```
Session (per user)
└── Thread (per conversation — can have many)
    └── Turn (per request/response pair)
        ├── user_input: String
        ├── response: Option<String>
        ├── tool_calls: Vec<ToolCall>
        └── state: TurnState (Pending | Running | Complete | Failed)
```

- A session has one **active thread** at a time; threads can be switched.
- Turns are append-only. Undo rolls back by restoring a prior checkpoint (message list, not a full thread snapshot).
- `UndoManager` is per-thread, stored in `SessionManager`, not on `Session` itself. Max 20 checkpoints (oldest dropped when exceeded).
- Group chat detection: if `metadata.chat_type` is `group`/`channel`/`supergroup`, user-scoped identity prompt files are excluded from the system prompt to prevent leaking personal context.
- Automatic cross-conversation recall follows the same privacy boundary: group/channel chats do not auto-inject user-scoped historical turns unless explicitly enabled in `CONVERSATION_RECALL_ALLOW_GROUP_AUTO_RECALL`.
- **Auth mode**: if a thread has `pending_auth` set (e.g. from `tool_auth` returning `awaiting_token`), the next user message is intercepted before any turn creation, logging, or safety validation and sent directly to the credential store. Any control submission (undo, interrupt, etc.) cancels auth mode.
- `ThreadState` values: `Idle`, `Processing`, `AwaitingApproval`, `Completed`, `Interrupted`.
- `SessionManager` maps `(user_id, channel, external_thread_id)` → internal UUID. Desktop runtime keeps sessions until explicit deletion.

## Agentic Loop (dispatcher.rs)

All execution paths now use the shared `run_agentic_loop()` engine in `agentic_loop.rs`, each providing their own `LoopDelegate` implementation:

- **`ChatDelegate`** (`dispatcher.rs`) — conversational turns, tool approval, skill context injection
- **`JobDelegate`** (`src/worker/job.rs`) — background scheduler jobs, planning support, completion detection

```
run_agentic_loop(delegate, reasoning, reason_ctx, config)
  1. Check signals (stop/cancel) via delegate.check_signals()
  2. Pre-LLM hook via delegate.before_llm_call()
  3. LLM call via delegate.call_llm()
  4. If text response → delegate.handle_text_response() → Continue or Return
  5. If tool calls → delegate.execute_tool_calls() → Continue or Return
  6. Post-iteration hook via delegate.after_iteration()
  7. Repeat until LoopOutcome returned or max_iterations reached
```

**Tool approval:** Tools flagged `requires_approval` pause the loop — `ChatDelegate` returns `LoopOutcome::NeedApproval(pending)`. The desktop runtime stores the `PendingApproval` in thread/session state and emits an `approval_needed` runtime event. The user's approval/deny resumes the loop.

**Prompt context assembly:** conversational turns now compose `workspace prompt + native memory prompt + conversation history prompt`. The history block is intentionally light, excludes the current thread by default, and shows absolute timestamps plus conversation identifiers so the model gets time sense without drowning the active chat.

**Turn-complete memory reflection:** completed chat turns emit a structured `agent:turn_completed` system event containing `{thread_id, user_input, assistant_output, timestamp}`. The default `memory_reflection` routine consumes that payload asynchronously, relies on prompt-level interpretation instead of rule-based keyword tagging, and mirrors its summary back into the source thread so the result appears in live desktop UI updates and persisted chat history.

**Conversation history tools:** `search_conversation_history` returns matched canonical turns plus adjacent preview turns. `read_conversation_context` expands a selected `conversation_id` into a slice or full canonical thread. Both default to excluding `thinking`; tool-call summaries are opt-in on context reads.

**Shared tool execution:** `tools/execute.rs` provides `execute_tool_with_safety()` (validate → timeout → execute → serialize) and `process_tool_result()` (sanitize → wrap → ChatMessage), used by all three delegates.

**ChatDelegate vs JobDelegate:** `ChatDelegate` runs for user-initiated conversational turns (holds session lock, tracks turns). `JobDelegate` is spawned by the `Scheduler` for background jobs created via `CreateJob` / `/job` — it runs independently of the session and has planning support (`use_planning` flag).

## Command Routing (router.rs)

The `Router` handles explicit `/commands` (prefix `/`). It parses them into `MessageIntent` variants: `CreateJob`, `CheckJobStatus`, `CancelJob`, `ListJobs`, `HelpJob`, `Command`. Natural language messages bypass the router entirely — they go directly to `dispatcher.rs` via `process_user_input`. Note: most user-facing commands (undo, compact, etc.) are handled by `SubmissionParser` before the router runs, so `Router` only sees unrecognized `/xxx` patterns that haven't already been claimed by `submission.rs`.

## Compaction

Triggered by `ContextMonitor` when token usage approaches the model's context limit.

**Token estimation**: Word-count × 1.3 + 4 overhead per message. Default context limit: 100,000 tokens. Compaction threshold: 80% (configurable).

Three strategies, chosen by `ContextMonitor.suggest_compaction()` based on usage ratio:
- **MoveToWorkspace** — Archives the full turn transcript into native episodic memory when available, otherwise falls back to workspace archival or `Truncate(5)`. Keeps 10 recent turns. Used when usage is 80–85% (moderate).
- **Summarize** (`keep_recent: N`) — LLM generates a summary of old turns, writes it to native episodic memory when available (or workspace daily log as a fallback), removes old turns. Used when usage is 85–95%.
- **Truncate** (`keep_recent: N`) — Removes oldest turns without summarization (fast path). Used when usage >95% (critical).

If the LLM call for summarization fails, the error propagates — turns are **not** truncated on failure.

Manual trigger: user sends `/compact` (parsed by `submission.rs`).

## Scheduler

`Scheduler` maintains two maps under `Arc<RwLock<HashMap>>`:
- `jobs` — full LLM-driven jobs, each with a `Worker` and an `mpsc` channel for `WorkerMessage` (`Start`, `Stop`, `Ping`, `UserMessage`).
- `subtasks` — lightweight `ToolExec` or `Background` tasks spawned via `spawn_subtask()` / `spawn_batch()`.

**Preferred entry point**: `dispatch_job()` — creates context, optionally sets metadata, persists to DB (so FK references from `job_actions`/`llm_calls` are valid immediately), then calls `schedule()`. Don't call `schedule()` directly unless you've already persisted.

Check-insert is done under a single write lock to prevent TOCTOU races. A cleanup task polls every second for job completion and removes the entry from the map.

`spawn_subtask()` returns a `oneshot::Receiver` — callers must await it to get the result. `spawn_batch()` runs all tasks concurrently and returns results in input order.

## Self-Repair

`DefaultSelfRepair` runs on `repair_check_interval` (from `AgentConfig`). It:
1. Calls `ContextManager::find_stuck_jobs()` to find jobs in `JobState::Stuck`.
2. Attempts `ctx.attempt_recovery()` (transitions back to `InProgress`).
3. Returns `ManualRequired` if `repair_attempts >= max_repair_attempts`.
4. Detects broken tools via `store.get_broken_tools(5)` (threshold: 5 failures). Requires `with_store()` to be called; returns empty without a store.
5. Attempts to rebuild broken tools via `SoftwareBuilder`. Requires `with_builder()` to be called; returns `ManualRequired` without a builder.

The `stuck_threshold` duration is used for time-based detection of `InProgress` jobs that have been running longer than the threshold. When `detect_stuck_jobs()` finds such jobs, it transitions them to `Stuck` before returning them, enabling the normal `attempt_recovery()` path.

Repair results: `Success`, `Retry`, `Failed`, `ManualRequired`. `Retry` does NOT notify the user (to avoid spam).

## Key Invariants

- Never call `.unwrap()` or `.expect()` — use `?` with proper error mapping.
- All state mutations on `Session`/`Thread` happen under `Arc<Mutex<Session>>` lock.
- The agent loop is single-threaded per thread; parallel execution happens at the job/scheduler level.
- Skills are selected **deterministically** (no LLM call) — see `skills/selector.rs`.
- Skills are loaded from the shared `~/.steward/skills` root and may refresh between turns if the on-disk snapshot changes.
- Tool results pass through `SafetyLayer` before returning to LLM (sanitizer → validator → policy → leak detector).
- `SessionManager` uses double-checked locking for session creation. Read lock first (fast path), then write lock with re-check to prevent duplicate sessions.
- `Scheduler.schedule()` holds the write lock for the entire check-insert sequence — don't hold any other locks when calling it.
- `cheap_llm` in `AgentDeps` is used for heartbeat and other lightweight tasks. Falls back to main `llm` if `None`. Use `agent.cheap_llm()` accessor, not `deps.cheap_llm` directly.
- `CostGuard.check_allowed()` must be called **before** LLM calls; `record_llm_call()` must be called **after**. Both calls are separate — the guard does not auto-record.
- `BeforeInbound` and `BeforeOutbound` hooks run for every user message and agent response respectively. Hooks can modify content or reject. Hook errors are logged but **fail-open** (processing continues).

## Complete Submission Command Reference

All commands parsed by `SubmissionParser::parse()`:

| Input | Variant | Notes |
|-------|---------|-------|
| `/undo` | `Undo` | |
| `/redo` | `Redo` | |
| `/interrupt`, `/stop` | `Interrupt` | |
| `/compact` | `Compact` | |
| `/clear` | `Clear` | |
| `/heartbeat` | `Heartbeat` | |
| `/summarize`, `/summary` | `Summarize` | |
| `/suggest` | `Suggest` | |
| `/new`, `/thread new` | `NewThread` | |
| `/thread <uuid>` | `SwitchThread` | Must be valid UUID |
| `/resume <uuid>` | `Resume` | Must be valid UUID |
| `/status [id]`, `/progress [id]`, `/list` | `JobStatus` | `/list` = all jobs |
| `/cancel <id>` | `JobCancel` | |
| `/quit`, `/exit`, `/shutdown` | `Quit` | |
| `yes/y/approve/ok` and aliases | `ApprovalResponse { approved: true, always: false }` | |
| `always/a` and aliases | `ApprovalResponse { approved: true, always: true }` | |
| `no/n/deny/reject/cancel` and aliases | `ApprovalResponse { approved: false }` | |
| JSON `ExecApproval{...}` | `ExecApproval` | From desktop approval IPC flow |
| `/help`, `/?` | `SystemCommand { "help" }` | Bypasses thread-state checks |
| `/version` | `SystemCommand { "version" }` | |
| `/tools` | `SystemCommand { "tools" }` | |
| `/skills [search <q>]` | `SystemCommand { "skills" }` | |
| `/ping` | `SystemCommand { "ping" }` | |
| `/debug` | `SystemCommand { "debug" }` | |
| `/model [name]` | `SystemCommand { "model" }` | |
| Everything else | `UserInput` | Starts a new agentic turn |

**`SystemCommand` vs control**: `SystemCommand` variants bypass thread-state checks entirely (no session lock, no turn creation). `Quit` returns `Ok(None)` from `handle_message` which breaks the main loop.

## Adding a New Submission Command

Submissions are special messages parsed in `submission.rs` before the agentic loop runs. To add a new one:
1. Add a variant to `Submission` enum in `submission.rs`
2. Add parsing in `SubmissionParser::parse()`
3. Handle in `agent_loop.rs` where `SubmissionResult` is matched (the `match submission { ... }` block in `handle_message`)
4. Implement the handler method (usually in `thread_ops.rs` for session operations, or `commands.rs` for system commands)
