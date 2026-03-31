# Task Runtime Contracts

> Contracts for task execution, Ask/Yolo approval, and API v0 behavior.

---

## Scenario: Ask/Yolo Task Execution

### 1. Scope / Trigger

- Trigger: Any change to task execution, template execution, approval checkpoints, SSE task streams, or task-mode switching.
- Trigger: Any API work under `/api/v0/tasks*`, `/api/v0/templates*`, or task-related parts of `/api/v0/settings`.

This scenario requires code-spec depth because the same contract spans the agent loop, storage, API, SSE payloads, and frontend runtime behavior.

### 2. Signatures

#### Task APIs

```http
POST   /api/v0/tasks
GET    /api/v0/tasks
GET    /api/v0/tasks/:id
GET    /api/v0/tasks/:id/stream
POST   /api/v0/tasks/:id/approve
POST   /api/v0/tasks/:id/reject
PATCH  /api/v0/tasks/:id/mode
DELETE /api/v0/tasks/:id
```

#### Template APIs

```http
GET    /api/v0/templates
POST   /api/v0/templates
GET    /api/v0/templates/:id
PUT    /api/v0/templates/:id
DELETE /api/v0/templates/:id
```

#### Template response shape

```json
{
  "id": "builtin:file-archive|uuid",
  "name": "File Archive",
  "description": "Scan a source directory and propose organization actions.",
  "parameter_schema": {
    "type": "object",
    "properties": {}
  },
  "default_mode": "ask|yolo",
  "output_expectations": {},
  "builtin": true,
  "mutable": false,
  "clonable": true,
  "created_at": "RFC3339 timestamp or null",
  "updated_at": "RFC3339 timestamp or null"
}
```

#### Core task mode enum

```rust
enum TaskMode {
    Ask,
    Yolo,
}
```

### 3. Contracts

#### Task record shape

```json
{
  "id": "uuid",
  "template_id": "uuid-or-builtin-id",
  "title": "Archive Downloads",
  "mode": "ask",
  "status": "queued|running|waiting_approval|completed|failed|cancelled|rejected",
  "created_at": "RFC3339 timestamp",
  "updated_at": "RFC3339 timestamp",
  "current_step": {
    "id": "step-id",
    "kind": "tool_call|approval|log|result",
    "title": "Proposed file moves"
  },
  "pending_approval": {
    "id": "approval-id",
    "risk": "file_write|file_delete|network_request|external_side_effect",
    "summary": "Move 42 files into categorized folders",
    "operations": []
  }
}
```

#### `GET /api/v0/tasks/:id`

Response:

```json
{
  "task": {
    "id": "uuid",
    "template_id": "builtin:file-archive",
    "mode": "ask",
    "status": "failed",
    "title": "Archive Downloads",
    "created_at": "RFC3339 timestamp",
    "updated_at": "RFC3339 timestamp",
    "current_step": {
      "id": "failed-uuid",
      "kind": "result",
      "title": "Failed"
    },
    "pending_approval": null,
    "last_error": "disk unavailable",
    "result_metadata": {
      "failure_reason": "disk unavailable"
    }
  },
  "timeline": [
    {
      "sequence": 1,
      "event": "task.created",
      "status": "queued",
      "mode": "ask",
      "current_step": {
        "id": "task-uuid",
        "kind": "log",
        "title": "Queued"
      },
      "pending_approval": null,
      "last_error": null,
      "result_metadata": null,
      "created_at": "RFC3339 timestamp"
    }
  ]
}
```

Rules:

- `GET /tasks/:id` is the task detail API, not only a shallow task header lookup.
- Timeline entries are append-only and ordered by `sequence`.
- Detail responses must survive process restart by loading persisted task records and timeline events from storage.

#### `POST /api/v0/tasks`

Request:

```json
{
  "template_id": "builtin:file-archive",
  "mode": "ask",
  "parameters": {
    "source_path": "/Users/alex/Downloads",
    "target_root": "/Users/alex/Documents"
  }
}
```

Response:

```json
{
  "task_id": "uuid",
  "status": "queued"
}
```

Rules:

- `mode` is required and must be `ask` or `yolo`.
- `parameters` must be validated against the template schema before execution starts.
- Task creation persists the task before agent execution begins.

#### `POST /api/v0/templates`

Request:

```json
{
  "name": "Custom Archive",
  "description": "User-defined archive variant",
  "parameter_schema": {
    "type": "object",
    "properties": {
      "source_path": { "type": "string" }
    },
    "required": ["source_path"]
  },
  "default_mode": "ask",
  "output_expectations": {
    "kind": "file_operation_plan"
  }
}
```

Rules:

- Creates a user-owned template with a generated UUID id.
- `parameter_schema.type` must be `object`.
- `parameter_schema.properties` must be an object.
- `output_expectations` must be an object.
- Validation failures return `422` with `{ "error": "...", "field_errors": { ... } }`.

#### `PUT /api/v0/templates/:id`

Rules:

- Built-in templates are readable but not mutable.
- Updating a built-in template must return `409`.
- Updating a missing user template must return `404`.

#### `DELETE /api/v0/templates/:id`

Rules:

- Built-in templates are readable but not deletable.
- Deleting a built-in template must return `409`.
- Deleting a missing user template must return `404`.

#### `PATCH /api/v0/tasks/:id/mode`

Request:

```json
{
  "mode": "yolo"
}
```

Rules:

- Mode changes are allowed while a task is `queued`, `running`, or `waiting_approval`.
- Switching from `ask` to `yolo` while waiting approval resumes execution from the current checkpoint once the checkpoint is explicitly approved or auto-resolved by defined policy.
- Mode changes must be persisted and emitted on the task SSE stream.
- Mode changes must append a timeline entry visible from `GET /tasks/:id`.

#### Approval endpoints

`POST /api/v0/tasks/:id/approve`

```json
{
  "approval_id": "approval-id"
}
```

`POST /api/v0/tasks/:id/reject`

```json
{
  "approval_id": "approval-id",
  "reason": "optional user-visible message"
}
```

Rules:

- Approval/rejection must target the currently pending approval only.
- Approve resumes execution.
- Reject transitions task to `rejected` unless future rollback semantics are explicitly implemented.
- Approval checkpoints and rejection outcomes must be persisted in task timeline history.

#### SSE event envelope

```json
{
  "event": "task.updated",
  "task_id": "uuid",
  "sequence": 12,
  "timestamp": "RFC3339 timestamp",
  "payload": {}
}
```

Required task event types:

- `task.created`
- `task.log`
- `task.step.started`
- `task.waiting_approval`
- `task.mode_changed`
- `task.completed`
- `task.failed`
- `task.cancelled`
- `task.rejected`

### 4. Validation & Error Matrix

| Endpoint / Condition | Expected Behavior | Status |
|----------------------|-------------------|--------|
| `POST /tasks` with unknown template | reject before enqueue | `404` |
| `POST /tasks` with invalid parameters | return field validation errors | `422` |
| `POST /templates` with invalid schema | reject with field-level validation errors | `422` |
| `PUT /templates/:id` for built-in template | reject mutation | `409` |
| `DELETE /templates/:id` for built-in template | reject mutation | `409` |
| `PATCH /tasks/:id/mode` with invalid mode | reject | `422` |
| `POST /tasks/:id/approve` when no approval is pending | reject | `409` |
| `POST /tasks/:id/approve` with stale `approval_id` | reject as stale checkpoint | `409` |
| `POST /tasks/:id/reject` when task already finished | reject | `409` |
| `DELETE /tasks/:id` for missing task | reject | `404` |
| `GET /tasks/:id/stream` for missing task | reject | `404` |

### 5. Good / Base / Bad Cases

#### Good

- File archive task in `ask` mode pauses before file writes and emits a structured preview payload.
- User flips a running task from `ask` to `yolo`, and the stream emits `task.mode_changed`.
- Periodic briefing task in `yolo` mode completes and writes markdown output without opening any approval checkpoint for low-risk local writes that policy has classified as safe.
- Template library returns both built-in and user-defined templates in one typed list, with built-ins marked read-only.
- Restarting the runtime still allows `GET /tasks/:id` to return the terminal task state and ordered timeline.

#### Base

- Session/chat features may exist, but they do not replace template-backed task execution.
- A task may emit plain logs and still be valid as long as status and approval contracts are preserved.

#### Bad

- Approval is represented as a free-form chat message instead of a typed event.
- UI assumes any paused task means approval is pending.
- Tools decide on their own whether they are in Ask or Yolo without consulting persisted task mode.
- Template schema validation relies on opaque `400` text without field-level keys the UI can render.
- Task detail is reconstructed from logs instead of a persisted task record plus explicit timeline rows.

### 6. Tests Required

- Integration test for `POST /tasks` with valid and invalid template parameters.
- Integration test for template CRUD covering built-in read-only semantics and `422` field errors.
- Integration test proving task detail and timeline survive runtime restart against the same libSQL database.
- Agent-loop test proving risky tool calls pause in `ask` mode and continue in `yolo` mode.
- API test for approve/reject race handling with stale `approval_id`.
- SSE test asserting `task.waiting_approval` and `task.mode_changed` events are emitted in order.
- UI integration test asserting an approval preview is driven by typed API/SSE payloads, not log scraping.

### 7. Wrong vs Correct

#### Wrong

```json
{
  "status": "paused",
  "message": "Please confirm this."
}
```

This is not enough for the UI or runtime to reason about the approval checkpoint.

#### Correct

```json
{
  "status": "waiting_approval",
  "pending_approval": {
    "id": "approval-123",
    "risk": "file_delete",
    "summary": "Delete 3 duplicate files",
    "operations": [
      { "kind": "delete", "path": "/tmp/a.txt" }
    ]
  }
}
```

The task state is explicit, typed, and resumable.

---

## Conventions

### Convention: Task State Is Authoritative

**What**: The backend task record is the source of truth for mode, status, pending approval, and execution history.

**Why**: UI refreshes, Tauri restarts, and browser clients must all recover the same task state.

### Convention: Approval Is a Structured Operation Preview

**What**: Approval checkpoints expose typed operations and risk class, not only log text.

**Why**: The UI needs deterministic rendering and future policy engines need machine-readable payloads.

### Convention: SSE Complements REST, It Does Not Replace It

**What**: REST gives the current resource snapshot; SSE gives ordered updates.

**Why**: Clients must be able to reconnect and rebuild task state without replaying the entire runtime from logs.
