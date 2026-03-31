# Desktop-First Architecture

> Executable architecture rules for IronCowork as a desktop autonomous agent.

---

## Scenario: IronCowork Desktop Runtime

### 1. Scope / Trigger

- Trigger: any work that changes runtime/module boundaries while converting IronClaw into IronCowork.
- Trigger: any work that adds or edits `src-tauri/`, `ui/`, `crates/ironcowork-api`, session/run APIs, or workspace/runtime wiring.
- Trigger: any work that removes old channel, gateway, PostgreSQL, NEAR AI, or predefined-workflow-first assumptions.

### 2. Signatures

#### Runtime boot

```rust
async fn run_api(bind_addr: std::net::SocketAddr, state: AppState) -> anyhow::Result<()>;
async fn launch_tauri(api_base_url: String) -> anyhow::Result<()>;
```

#### Frontend transport

```text
HTTP JSON requests to /api/v0/*
SSE streams from /api/v0/sessions/:id/stream and /api/v0/runs/:id/stream
```

#### Storage

```rust
async fn open_libsql(path: &std::path::Path) -> anyhow::Result<libsql::Database>;
```

### 3. Contracts

#### Repository ownership

| Area | Responsibility |
|------|----------------|
| `crates/ironcowork-api` | Axum routes, SSE, shared state, HTTP DTOs |
| `src-tauri` | Notifications, tray, drag-and-drop, desktop lifecycle |
| `ui/` | Svelte app over HTTP/SSE only |
| runtime crates | Agent loop, runs, approvals, tools, workspace, storage |
| storage layer | libSQL-only persistence |

#### Product-center contract

- `session` is the primary user-facing object.
- `run` or `task` is a persisted execution artifact created by a session or future background routine.
- The product must not require predefined workflows as a first-class concept.
- Future routines or presets may exist, but they must layer on top of the session/run model.

#### Forbidden boundary crossings

- `ui/` must not depend on Tauri IPC for primary business flows.
- `src-tauri/` must not own session state, run state, or approval decisions.
- Business operations must not bypass `ironcowork-api` to talk directly to storage.
- Risky actions must not bypass the tool/safety layer.
- New code must not reintroduce PostgreSQL or predefined-workflow-first assumptions.

#### Binding and exposure

- Default bind address is `127.0.0.1`.
- Remote access, if any, is managed externally by the user.
- The app does not ship built-in tunnel or public exposure features.

### 4. Validation And Error Matrix

| Condition | Expected Behavior | Error Shape |
|-----------|-------------------|-------------|
| API tries to bind non-loopback by default | reject or require explicit future opt-in | startup error |
| Browser-only mode lacks desktop capabilities | degrade gracefully | capability unavailable signal or no-op |
| New code reintroduces predefined workflow CRUD as a core API | reject in review | design rejection |
| Task/run logic tries to mutate files outside the tool layer | block the implementation | review/test failure |

### 5. Good / Base / Bad

#### Good

- Tauri starts after Axum is ready and points at the same HTTP base URL a browser can use.
- Svelte writes settings via HTTP, subscribes to session/run SSE, and renders approvals from structured payloads.
- Folder drops become HTTP requests to workspace endpoints.

#### Base

- Browser mode works without notifications, tray, or native dialogs.
- Desktop mode adds convenience without changing backend authority.

#### Bad

- Tauri command writes run status directly into SQLite.
- UI approval goes through custom IPC instead of HTTP.
- A new feature requires predefined workflow CRUD before a user can talk to the agent.

### 6. Tests Required

- startup test proving loopback bind behavior
- browser-mode integration test without Tauri APIs
- integration test proving desktop shell callbacks still flow through HTTP
- regression test proving session/run flows work without predefined workflow endpoints

### 7. Wrong vs Correct

#### Wrong

```rust
#[tauri::command]
async fn approve_run(run_id: String, db: tauri::State<'_, Db>) -> Result<(), String> {
    db.runs().approve(run_id).await.map_err(|e| e.to_string())
}
```

#### Correct

```rust
#[tauri::command]
async fn notify_pending_approval(run_id: String, api_base_url: tauri::State<'_, String>) -> Result<(), String> {
    let _ = run_id;
    let _ = api_base_url;
    Ok(())
}
```

Native affordances may help the user, but the authoritative action still goes through the HTTP API.
