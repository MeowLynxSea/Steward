# IronCowork Desktop-First Transformation Plan

**Date:** 2026-03-30
**Status:** Proposed
**Goal:** Rebuild this fork into IronCowork, a local-first desktop automation product for knowledge workers, while retaining the Rust agent engine, safety controls, MCP support, and workspace retrieval capabilities from IronClaw.

---

## Product Direction

### Why This Fork Exists

The target product is not a cheaper wrapper around existing coding CLIs and it is not a channel-first chat assistant. IronCowork is intended to become a native-feeling automation environment for knowledge work such as file organization, recurring reports, and information synthesis.

### Product Repositioning

| Dimension | Current IronClaw Shape | IronCowork Target |
|-----------|------------------------|-------------------|
| Primary interaction | Message-driven chat across channels | Task/template-driven execution with session support |
| Core user | Chat users, channel users, operator workflows | Knowledge workers on local desktop environments |
| Frontend | Web gateway plus messaging surfaces | One web UI, accessed via Tauri locally or browser remotely |
| Risk control | Tool execution inside existing agent flow | Explicit Ask/Yolo execution mode with approval checkpoints |
| Storage | PostgreSQL or libSQL feature variants | libSQL only |
| Identity | NEAR AI-oriented onboarding | Local config and environment only |

### Non-Negotiable Decisions

- The fork does not optimize for upstream mergeability.
- The service binds to `127.0.0.1` by default.
- Tauri is a native bridge only. Business logic remains in the Axum backend.
- libSQL is the only supported storage backend.
- High-risk actions must flow through Ask/Yolo approval logic and remain constrained by the existing Rust safety model.

---

## Current Baseline

The current repository still reflects the old product model:

- `README.md` and package metadata still identify the project as IronClaw.
- `Cargo.toml` still enables both PostgreSQL and libSQL and keeps PostgreSQL in the default feature set.
- `src/channels/`, `channels-src/`, `deploy/`, and channel/webhook-oriented logic remain present.
- Existing onboarding and docs still reference NEAR AI login and database bootstrap workflows.
- The runtime already contains useful building blocks worth preserving:
  - Rust agent loop and orchestration
  - WASM safety and tool isolation
  - MCP and tool registry infrastructure
  - Workspace/search modules with libSQL support already partially present

This means the fork should be treated as a selective extraction of a good engine from the wrong product shell.

---

## Target Architecture

### Runtime Layout

```
+------------------------+      HTTP/SSE      +------------------------+
|  Svelte UI             | <----------------> |  ironcowork-api        |
|  - tasks               |                    |  Axum on 127.0.0.1     |
|  - sessions            |                    |  settings/tasks/stream  |
|  - templates           |                    +-----------+------------+
+-----------+------------+                                |
            |                                             |
            | Tauri native bridge (optional)              |
            v                                             v
+------------------------+                    +------------------------+
|  src-tauri             |                    |  core runtime crates   |
|  notifications         |                    |  agent / scheduler     |
|  tray                  |                    |  tools / MCP / safety  |
|  drag-and-drop         |                    |  workspace / storage   |
+------------------------+                    +------------------------+
                                                         |
                                                         v
                                              +------------------------+
                                              |  libSQL                |
                                              |  FTS5 + DiskANN        |
                                              +------------------------+
```

### Repository Shape After Migration

```text
IronCowork/
├── src-tauri/                  # New desktop shell
├── ui/                         # New Svelte front-end
├── crates/
│   ├── ironcowork-api/         # New Axum HTTP/SSE API
│   ├── ironclaw_core/          # Retained agent loop, scheduler, routing core
│   ├── ironclaw_tools/         # Retained built-in tools and MCP client code
│   ├── ironclaw_safety/        # Retained WASM sandbox and prompt safety
│   ├── ironclaw_workspace/     # Retained retrieval logic, adapted to libSQL only
│   ├── ironclaw_llm/           # Retained provider adapters
│   └── ironclaw_storage/       # Rebuilt as libSQL-only storage layer
├── migrations/                 # libSQL-only migrations
├── static/                     # Built UI assets
├── channels-src/               # Deleted
├── deploy/                     # Deleted
└── Cargo.toml                  # Workspace root for desktop-first architecture
```

### Architectural Boundaries

- `src-tauri/` may expose native affordances only: notifications, tray, dialogs, drag-and-drop, startup lifecycle.
- All application state and business logic live behind HTTP/SSE in `ironcowork-api`.
- The same HTTP service must support both Tauri-hosted local use and plain browser access.
- Ask/Yolo is part of task runtime state, not a UI-only flag.
- File mutation, deletion, remote requests, and similar risky actions must go through the tool/safety layer instead of ad hoc shell execution.

---

## Keep / Remove / Add

### Keep

- Rust agent loop and scheduler concepts
- WASM sandbox and credential leak protections
- Tool registry and MCP support
- Retrieval/workspace ranking logic where independent from PostgreSQL assumptions
- Multi-provider LLM adapter code

### Remove Early

- `channels-src/`
- `deploy/`
- old web gateway code paths
- message channel product flows (Telegram, Slack, relay/web channel assumptions)
- NEAR AI OAuth onboarding requirement
- PostgreSQL dependencies, feature flags, migrations, and tests

### Add

- `crates/ironcowork-api`
- `src-tauri/`
- `ui/`
- task/template data model
- Ask/Yolo approval event API and SSE stream events
- desktop shell integration points for notifications and drag-and-drop indexing

---

## Execution Model Shift

### 1. From Chat-Driven to Task/Template-Driven

The product center moves from "user sends a message" to "user executes a task template with parameters". Sessions still exist, but they are secondary. The primary lifecycle becomes:

1. Select or create template
2. Fill parameters
3. Choose execution mode (`ask` or `yolo`)
4. Run task
5. Inspect logs, approvals, outputs, and history

### 2. Ask/Yolo as a First-Class Runtime Contract

Ask/Yolo cannot be an optional UI convenience. It must be enforced in the agent loop before risky tool calls commit. The API surface must expose:

- current task mode
- current approval checkpoint
- preview payload for proposed operations
- approve/reject endpoints
- mode switching during execution

### 3. One Web App, Two Access Modes

Local desktop mode:
- Axum binds to loopback
- Tauri hosts the UI and enables native capabilities

Remote browser mode:
- User reaches the same HTTP service through their own tunnel or reverse proxy
- Native-only features degrade gracefully

---

## Phase Plan

### Phase 0: Core Purification

**Goal:** Start the fork locally without PostgreSQL, channel modules, or NEAR account dependency.

#### Deliverables

- Rename and fork identity updated toward IronCowork
- `channels-src`, `deploy`, and old build helpers removed
- PostgreSQL code removed from storage/runtime paths
- libSQL is the default and only backend
- NEAR AI login requirement removed from onboarding/config loading
- libSQL FTS5 + DiskANN retrieval verified by integration test

#### Issues

##### Issue 0.1: Remove perimeter modules

- Delete `channels-src/`, `deploy/`, `scripts/build-all.sh`
- Remove web gateway, REPL, and channel features from `Cargo.toml`
- Identify and delete dead code paths under `src/channels/`, `src/webhooks/`, and related docs

##### Issue 0.2: Collapse storage to libSQL

- Remove PostgreSQL store implementations and feature flags
- Delete `tokio-postgres`, `deadpool-postgres`, `pgvector`, and related test setup
- Normalize migrations to libSQL-compatible DDL only

##### Issue 0.3: Remove NEAR AI dependency

- Replace `ironclaw onboard` assumptions with local config/env loading
- Support `LLM_BACKEND` and `LLM_API_KEY` from config file or environment
- Remove browser OAuth and account bootstrap references

##### Issue 0.4: Verify retrieval stack

- Add integration test for insert -> FTS5 query -> vector ANN query -> RRF result
- Assert workspace module still works against libSQL-only storage

### Phase 1: API + UI + Desktop Shell Skeleton

**Goal:** A minimal but real desktop-first shell running on Axum + Svelte + Tauri.

#### Deliverables

- `ironcowork-api` crate listens on `127.0.0.1`
- settings endpoints and health endpoint exist
- task SSE stream exists
- Ask/Yolo approval checkpoints work at runtime
- Svelte shell can read/write settings and consume streams
- Tauri can notify, show tray actions, and forward dropped folders

#### Issues

##### Issue 1.1: Create `ironcowork-api`

- `GET /api/v0/health`
- `GET/PATCH /api/v0/settings`
- shared app state
- SSE plumbing for tasks and sessions

##### Issue 1.2: Add Ask/Yolo interceptor

- intercept risky tool operations before execution
- create suspend/resume state machine
- expose pending approval payload via API and SSE

##### Issue 1.3: Create Svelte shell

- left navigation + content pane
- `apiClient` for HTTP and SSE
- settings page for provider/key setup

##### Issue 1.4: Add Tauri shell

- start Axum service first, then open window
- system notifications for completion and approval requests
- tray menu for show/quit
- drag-and-drop folder index trigger

##### Issue 1.5: Connect sessions and task views

- session list + conversation stream
- task list + task status view

### Phase 2: MVP Task Scenarios

**Goal:** Prove the architecture with two task-template workflows that generate user value.

#### Deliverables

- task template CRUD
- file archive template
- periodic briefing template
- approval UI and execution log UI

#### Issues

##### Issue 2.1: Task template model

- template schema
- built-in vs user template distinction
- CRUD API and persistence

##### Issue 2.2: File archive workflow

- directory scan toolchain
- proposal preview for rename/move actions
- Ask preview and Yolo background execution

##### Issue 2.3: Periodic briefing workflow

- cron scheduling
- MCP fetch + local note aggregation
- markdown report output to selected path

##### Issue 2.4: Execution UX

- step timeline
- approval modal for Ask checkpoints
- runtime mode toggle

---

## HTTP API v0

### Settings

- `GET /api/v0/settings`
- `PATCH /api/v0/settings`

### Templates

- `GET /api/v0/templates`
- `POST /api/v0/templates`
- `GET /api/v0/templates/:id`
- `PUT /api/v0/templates/:id`
- `DELETE /api/v0/templates/:id`

### Tasks

- `POST /api/v0/tasks`
- `GET /api/v0/tasks`
- `GET /api/v0/tasks/:id`
- `GET /api/v0/tasks/:id/stream`
- `POST /api/v0/tasks/:id/approve`
- `POST /api/v0/tasks/:id/reject`
- `PATCH /api/v0/tasks/:id/mode`
- `DELETE /api/v0/tasks/:id`

### Sessions

- `POST /api/v0/sessions`
- `GET /api/v0/sessions`
- `POST /api/v0/sessions/:id/messages`
- `GET /api/v0/sessions/:id/stream`

### Workspace

- `POST /api/v0/workspace/index`
- `GET /api/v0/workspace/tree`
- `POST /api/v0/workspace/search`

### Tools and MCP

- `GET /api/v0/tools`
- `POST /api/v0/mcp/servers`
- `DELETE /api/v0/mcp/servers/:name`

---

## Risks and Mitigations

| Risk | Why It Matters | Mitigation |
|------|----------------|------------|
| Half-migrated architecture | Old chat/gateway assumptions can leak into new code | Treat new API/task runtime as source of truth and delete legacy surfaces early |
| Storage drag | Keeping PostgreSQL compatibility will slow every future change | Remove feature flags and dead implementations in Phase 0 instead of abstracting them further |
| Tauri overreach | IPC-heavy designs will couple frontend and desktop runtime | Limit Tauri to native affordances; all business actions go through HTTP/SSE |
| Approval model drift | Ask/Yolo may become inconsistent across tools | Enforce approval checkpoints at the agent/tool boundary, not in individual UIs |
| Remote access confusion | Users may expect built-in public exposure | Keep loopback-only binding explicit; document tunnel/reverse-proxy responsibility as external |

---

## Acceptance Checks Per Phase

### Phase 0

- `cargo test` passes without PostgreSQL services
- libSQL retrieval integration tests cover FTS5 + ANN + RRF
- repository no longer builds or documents removed channels/gateway paths

### Phase 1

- backend listens only on `127.0.0.1`
- settings page can save config and validate connectivity
- task stream emits logs and approval events over SSE
- Tauri shell can surface notifications without owning business logic

### Phase 2

- file archive template can preview and execute changes in Ask and Yolo modes
- periodic briefing template can run on schedule and write markdown output
- task history and session history are visible in the UI

---

## Explicit Red Lines

- Do not preserve upstream-compatible architecture for its own sake.
- Do not reintroduce public network exposure features inside the app.
- Do not execute arbitrary user commands outside the safety/tool boundary.
- Do not let Tauri IPC become the main application API.
- Do not spend Phase 1 effort on high-polish visual design before the task runtime works.

---

## Recommended Immediate Next Work

1. Start with Issue 0.2 and 0.4 together because storage simplification is the hardest architectural blocker.
2. Delete channel and gateway surfaces aggressively rather than leaving compatibility shims.
3. Introduce the new API crate before building the UI so the frontend contract stabilizes early.
4. Treat Ask/Yolo as a runtime state machine with tests before building the approval modal.
