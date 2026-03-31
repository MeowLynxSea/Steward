# Desktop-First Architecture

> Executable architecture rules for the IronCowork fork direction.

---

## Scenario: IronCowork Desktop-First Runtime

### 1. Scope / Trigger

- Trigger: Any task that renames, moves, or introduces runtime modules while converting IronClaw into IronCowork.
- Trigger: Any work that adds `src-tauri/`, `ui/`, `crates/ironcowork-api`, or restructures agent/storage/workspace code into new crates.
- Trigger: Any work that removes or replaces gateway, channel, PostgreSQL, or NEAR AI assumptions.

This scenario requires code-spec depth because it changes repository layout, service boundaries, storage contracts, and cross-layer runtime behavior.

### 2. Signatures

#### Runtime boot

```rust
async fn run_api(bind_addr: std::net::SocketAddr, state: AppState) -> anyhow::Result<()>;
async fn launch_tauri(api_base_url: String) -> anyhow::Result<()>;
```

#### Frontend transport

```text
HTTP JSON requests to /api/v0/*
SSE streams from /api/v0/tasks/:id/stream and /api/v0/sessions/:id/stream
```

#### Storage

```rust
async fn open_libsql(path: &std::path::Path) -> anyhow::Result<libsql::Database>;
```

### 3. Contracts

#### Repository ownership

| Area | Responsibility |
|------|----------------|
| `crates/ironcowork-api` | Axum routes, SSE, state wiring, HTTP-facing DTOs |
| `src-tauri` | Native shell only: notifications, tray, drag-and-drop, lifecycle |
| `ui/` | Svelte web UI consuming HTTP/SSE only |
| `ironclaw_core` | Agent loop, scheduler, routing, task orchestration core |
| `ironclaw_tools` | Built-in tools and MCP adapters |
| `ironclaw_safety` | WASM sandbox, injection defense, secret safety |
| `ironclaw_workspace` | Retrieval/indexing logic backed by libSQL |
| `ironclaw_storage` | libSQL-only persistence |

#### Forbidden boundary crossings

- `ui/` must not depend on Tauri IPC for primary business actions.
- `src-tauri/` must not own task state, session state, or approval decisions.
- Business operations must not bypass `ironcowork-api` to talk directly to storage.
- Risky actions must not bypass the tool/safety layer.
- New code must not add PostgreSQL feature flags, connection pools, or migrations.

#### Binding and exposure

- Default bind address is `127.0.0.1`.
- Remote access is externalized to reverse proxy or tunnel tools outside the app.
- The app does not ship built-in public exposure, tunnel management, or LAN discovery.

### 4. Validation & Error Matrix

| Condition | Expected Behavior | Error Shape |
|-----------|-------------------|-------------|
| API tries to bind non-loopback by default | Reject configuration or require explicit override path if ever added later | startup error with clear bind address message |
| Frontend attempts privileged native action in browser-only mode | Degrade gracefully and return capability unavailable | HTTP 409 or capability flag in response |
| Code introduces PostgreSQL-only path | Block in review and tests; do not add mixed storage abstraction again | build/test failure or review rejection |
| Task logic tries to call shell directly for file mutation | Reject design and route through tool layer | implementation blocked before merge |

### 5. Good / Base / Bad Cases

#### Good

- Tauri window starts after Axum is ready and points to the same HTTP base URL that a remote browser could use.
- Svelte writes settings through `PATCH /api/v0/settings` and subscribes to SSE for runtime output.
- File indexing from drag-and-drop becomes an HTTP request to `/api/v0/workspace/index`.

#### Base

- Browser-only access works without notifications, tray, or native dialogs.
- Desktop mode adds native features without changing backend API contracts.

#### Bad

- Tauri command directly mutates a task record in SQLite.
- UI sends approval decisions through custom IPC instead of HTTP.
- Storage layer preserves both PostgreSQL and libSQL implementations "for flexibility".

### 6. Tests Required

- Startup test proving API binds to loopback.
- Integration test proving browser mode can function without Tauri-only APIs.
- Integration test proving desktop shell features call back into HTTP endpoints instead of mutating state locally.
- Regression test proving storage build/test matrix runs without PostgreSQL dependencies.

### 7. Wrong vs Correct

#### Wrong

```rust
#[tauri::command]
async fn approve_task(task_id: String, db: tauri::State<'_, Db>) -> Result<(), String> {
    db.tasks().approve(task_id).await.map_err(|e| e.to_string())
}
```

This makes the desktop shell a second backend and breaks browser parity.

#### Correct

```rust
#[tauri::command]
async fn notify_pending_approval(task_id: String, api_base_url: tauri::State<'_, String>) -> Result<(), String> {
    let _ = task_id;
    let _ = api_base_url;
    Ok(())
}
```

The native shell may assist with user experience, but the authoritative approval action still goes through the HTTP API.

---

## Design Decisions

### Decision: One Web App, Different Access Modes

**Context**: The product needs a local desktop shell without forking the frontend into separate web and desktop stacks.

**Decision**: Keep one Svelte app and one Axum backend contract. Tauri adds native capabilities only when available.

**Why**:

- prevents contract drift between desktop and browser modes
- keeps frontend state/debugging centered on HTTP/SSE
- allows future remote access without re-implementing business flows

### Decision: Delete Legacy Product Surfaces Early

**Context**: Channel and gateway code encode the wrong product worldview.

**Decision**: Prefer deletion over compatibility shims once replacement direction is known.

**Why**:

- avoids carrying chat-centric abstractions into task-centric runtime code
- reduces accidental reuse of the wrong interfaces
- makes the fork direction visible to every contributor
