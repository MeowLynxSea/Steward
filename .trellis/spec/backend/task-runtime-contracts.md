# Task Runtime Contracts

> Contracts for thread-driven desktop chat execution, secondary run snapshots, and Ask/Yolo approval behavior.

---

## Scenario: Session-Driven Agent Execution

### 1. Scope / Trigger

- Trigger: any change to session/thread messaging, run snapshot creation, Ask/Yolo checkpoints, Tauri events, or mode switching.
- Trigger: any Tauri IPC work that changes session/thread/task behavior.

This contract exists to keep the product session-first while preserving durable execution records.

### 2. Signatures

#### Desktop IPC Commands

```text
list_sessions
create_session
get_session
send_session_message
delete_session
```

#### Secondary execution commands

```text
list_tasks
get_task
approve_task
reject_task
patch_task_mode
delete_task
```

Rules:

- session/thread IPC is the public desktop contract
- Tauri runtime events are the live update contract for desktop message/status streaming
- task/run records remain secondary execution state, not the product center

#### Core mode enum

```rust
enum RunMode {
    Ask,
    Yolo,
}
```

### 3. Contracts

#### Product model

- `session` is the primary user-facing object.
- `thread` is the primary conversational execution unit inside a session.
- A user message always targets one thread inside one session.
- A `run` or `task` is a durable execution snapshot for that thread when the runtime needs approval, mode tracking, or auditability.
- Predefined workflow CRUD is not part of the core desktop contract.

#### Run record shape

```json
{
  "id": "uuid",
  "session_id": "uuid",
  "mode": "ask",
  "status": "queued|running|waiting_approval|completed|failed|cancelled|rejected",
  "summary": "Organize the selected workspace and produce a report",
  "created_at": "RFC3339 timestamp",
  "updated_at": "RFC3339 timestamp",
  "current_step": {
    "id": "step-id",
    "kind": "planning|tool_call|approval|message|result",
    "title": "Proposed file operations"
  },
  "pending_approval": {
    "id": "approval-id",
    "risk": "file_write|file_delete|network_request|external_side_effect",
    "summary": "Move 12 files and rename 3 files",
    "operations": []
  },
  "last_error": null,
  "result_metadata": null
}
```

#### `get_task`

Response:

```json
{
  "run": {
    "id": "uuid",
    "session_id": "uuid",
    "mode": "ask",
    "status": "running",
    "summary": "Summarize the workspace and identify next actions",
    "created_at": "RFC3339 timestamp",
    "updated_at": "RFC3339 timestamp",
    "current_step": {
      "id": "step-uuid",
      "kind": "planning",
      "title": "Inspecting workspace context"
    },
    "pending_approval": null,
    "last_error": null,
    "result_metadata": null
  },
  "timeline": [
    {
      "sequence": 1,
      "event": "run.created",
      "status": "queued",
      "mode": "ask",
      "created_at": "RFC3339 timestamp"
    }
  ]
}
```

Rules:

- timeline is append-only and ordered
- detail must survive process restart
- run detail is the authoritative reconstruction surface for the UI

#### `send_session_message`

Request:

```json
{
  "content": "整理这个工作区，并给我一个结果摘要",
  "mode": "ask"
}
```

Response:

```json
{
  "accepted": true,
  "session_id": "uuid",
  "active_thread_id": "uuid",
  "active_thread_task_id": "uuid-or-null",
  "active_thread_task": {
    "id": "uuid",
    "template_id": "legacy:session-thread",
    "mode": "ask",
    "status": "queued",
    "title": "整理这个工作区，并给我一个结果摘要",
    "created_at": "RFC3339 timestamp",
    "updated_at": "RFC3339 timestamp",
    "current_step": {
      "id": "task-uuid",
      "kind": "log",
      "title": "Queued"
    },
    "pending_approval": null,
    "last_error": null,
    "result_metadata": null
  }
}
```

Rules:

- `mode` is optional; if omitted, the current thread mode stays in effect and a new thread defaults to `ask`
- the IPC layer must reject unsupported modes
- a thread message should attach to the durable task/run record keyed by the same UUID as the thread when a task record exists
- if a task record is available, the IPC response should expose `active_thread_task_id` and the current thread task snapshot immediately
- the effective mode must be persisted on the attached task record before the agent loop consumes the message

#### `get_session`

Response additions:

```json
{
  "session": {
    "id": "uuid"
  },
  "active_thread_id": "uuid",
  "thread_messages": [],
  "active_thread_task": {
    "id": "uuid",
    "template_id": "legacy:session-thread",
    "mode": "ask",
    "status": "running",
    "title": "整理这个工作区，并给我一个结果摘要"
  }
}
```

Rules:

- `active_thread_task` is nullable
- when present, it is the authoritative task/run record attached to the active thread
- the UI must be able to render current execution state from session detail without guessing thread identity from logs

#### Ask-mode contract

- risky side effects suspend the run before execution
- `pending_approval` must be structured and renderable without parsing plain logs
- approval and rejection decisions are persisted as timeline events

#### Yolo-mode contract

- the same run path executes without extra approval pauses for actions allowed by policy
- Yolo does not bypass sandbox, secret, or network policy constraints

#### Approval payload contract

```json
{
  "id": "approval-id",
  "risk": "file_write",
  "summary": "Write summary.md and move 8 files",
  "operations": [
    {
      "kind": "write_file",
      "tool_name": "write_file",
      "parameters": {
        "path": "/Users/alex/report.md"
      }
    }
  ]
}
```

#### Result metadata contract

```json
{
  "artifacts": [
    {
      "kind": "file",
      "path": "/Users/alex/report.md"
    }
  ],
  "notes": "Summary generated from workspace context"
}
```

### 4. Validation And Error Matrix

| Condition | Expected Behavior | IPC Error |
|-----------|-------------------|-----------|
| invalid mode | reject request | command returns validation error |
| session missing | reject message send | command returns not found error |
| thread missing | reject thread-scoped mutation/read | command returns not found error |
| run missing | reject run mutation/read | command returns not found error |
| approve when no pending approval | reject state transition | command returns conflict error |
| reject when run already completed | reject state transition | command returns conflict error |
| cancel when run already terminal | reject state transition | command returns conflict error |

### 5. Good / Base / Bad

#### Good

- the user chats naturally in a session, each turn lands in a thread, and the UI shows related run progress as needed
- Ask mode exposes real proposed side effects before execution
- a completed run preserves outputs and errors for later review

#### Base

- sessions can exist without currently active runs
- threads remain durable even when no run is currently attached
- future routines may create runs without changing the core run contract

#### Bad

- the product requires users to create predefined workflows before they can use the agent
- approval decisions are only visible as raw log text
- thread or run state is lost on refresh or restart

### 6. Tests Required

- integration test for session creation and message send
- integration test for thread creation/selection under a session
- integration test proving a message can create a durable run record
- integration test for Ask approval lifecycle
- integration test for Yolo execution under the same safety policy
- regression test proving no predefined workflow CRUD is required for core flows
