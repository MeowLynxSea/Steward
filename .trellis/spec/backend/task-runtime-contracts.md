# Task Runtime Contracts

> Contracts for session-driven execution, persisted runs, and Ask/Yolo approval behavior.

---

## Scenario: Session-Driven Agent Execution

### 1. Scope / Trigger

- Trigger: any change to session messaging, run creation, Ask/Yolo checkpoints, SSE streams, or run-mode switching.
- Trigger: any API work under `/api/v0/sessions*`, `/api/v0/runs*`, or task/run-related settings behavior.

This contract exists to keep the product session-first while preserving durable execution records.

### 2. Signatures

#### Session APIs

```http
POST /api/v0/sessions
GET  /api/v0/sessions
GET  /api/v0/sessions/:id
POST /api/v0/sessions/:id/messages
GET  /api/v0/sessions/:id/stream
```

#### Run APIs

```http
GET    /api/v0/runs
GET    /api/v0/runs/:id
GET    /api/v0/runs/:id/stream
POST   /api/v0/runs/:id/approve
POST   /api/v0/runs/:id/reject
PATCH  /api/v0/runs/:id/mode
DELETE /api/v0/runs/:id
```

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
- A user message may create, continue, or attach to one or more `runs`.
- A `run` is the durable execution record for agent work.
- Predefined workflow CRUD is not part of the core v0 contract.

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

#### `GET /api/v0/runs/:id`

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

#### `POST /api/v0/sessions/:id/messages`

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
  "task_id": "uuid-or-null",
  "task": {
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

- `mode` is optional; if omitted, the current task mode stays in effect and a new session defaults to `ask`
- the API must reject unsupported modes with `422`
- a session-thread message should attach to the durable task/run record keyed by the same UUID as the session thread
- if a task record is available, the API should expose `task_id` and the current task snapshot immediately
- the effective mode must be persisted on the attached task record before the agent loop consumes the message

#### `GET /api/v0/sessions/:id`

Response additions:

```json
{
  "session": {
    "id": "uuid"
  },
  "messages": [],
  "current_task": {
    "id": "uuid",
    "template_id": "legacy:session-thread",
    "mode": "ask",
    "status": "running",
    "title": "整理这个工作区，并给我一个结果摘要"
  }
}
```

Rules:

- `current_task` is nullable
- when present, it is the authoritative task/run record attached to that session thread
- the UI must be able to render current execution state from session detail without guessing task identity from logs

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

| Condition | Expected Behavior | HTTP |
|-----------|-------------------|------|
| invalid mode | reject request | `422` |
| session missing | reject message send | `404` |
| run missing | reject run mutation/read | `404` |
| approve when no pending approval | reject state transition | `409` |
| reject when run already completed | reject state transition | `409` |

### 5. Good / Base / Bad

#### Good

- the user chats naturally in a session, and the UI shows related run progress as needed
- Ask mode exposes real proposed side effects before execution
- a completed run preserves outputs and errors for later review

#### Base

- sessions can exist without currently active runs
- future routines may create runs without changing the core run contract

#### Bad

- the product requires users to create predefined workflows before they can use the agent
- approval decisions are only visible as raw log text
- run state is lost on refresh or restart

### 6. Tests Required

- integration test for session creation and message send
- integration test proving a message can create a durable run record
- integration test for Ask approval lifecycle
- integration test for Yolo execution under the same safety policy
- regression test proving no predefined workflow CRUD is required for core flows
