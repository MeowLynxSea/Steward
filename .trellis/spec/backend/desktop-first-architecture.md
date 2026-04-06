# Desktop-First Architecture

> Executable architecture rules for Steward as a desktop autonomous agent.

---

## Scenario: Steward Desktop Runtime

### 1. Scope / Trigger

- Trigger: any work that changes runtime/module boundaries while converting Steward into Steward.
- Trigger: any work that adds or edits `src-tauri/`, `ui/`, Tauri IPC contracts, session/thread/run APIs, or workspace/runtime wiring.
- Trigger: any work that removes old channel, gateway, PostgreSQL, NEAR AI, or predefined-workflow-first assumptions.

### 2. Signatures

#### Runtime boot

```rust
async fn start_embedded_runtime(
    tauri_emitter: Option<TauriEventEmitterHandle>,
) -> anyhow::Result<AppState>;
fn launch_tauri(app_state: AppState) -> anyhow::Result<()>;
```

#### Frontend transport

```text
Tauri IPC commands for settings, sessions, thread messages, approvals, and workspace operations
Tauri event stream for live assistant updates
```

Workbench-specific read surface:

```text
get_workbench_capabilities
index_workspace
get_workspace_index_job
get_workspace_tree
search_workspace
```

#### Storage

```rust
async fn open_libsql(path: &std::path::Path) -> anyhow::Result<libsql::Database>;
```

### 3. Contracts

#### Repository ownership

| Area | Responsibility |
|------|----------------|
| `src-tauri` | Tauri IPC bridge, notifications, tray, drag-and-drop, desktop lifecycle |
| `ui/` | Svelte app over Tauri IPC and Tauri-emitted runtime events |
| runtime crates | Agent loop, sessions, threads, runs, approvals, tools, workspace, storage |
| storage layer | libSQL-only persistence |

#### Product-center contract

- `session` is the primary user-facing container in the desktop UI.
- `thread` is the primary conversational and execution unit inside a session.
- `run` or `task` is a secondary execution snapshot attached to a thread when the runtime needs approval, progress tracking, or auditability.
- Desktop-facing contracts must use Tauri IPC commands plus Tauri runtime events.
- Optional external ingress is limited to installable WASM channels; built-in legacy chat/channel surfaces must not reappear as first-class product paths.
- Internal automation concepts such as `routine` and `heartbeat` remain valid, but they layer on top of the same session/thread runtime.
- workspace ingestion must populate the same persisted workspace document/chunk corpus used by retrieval; it must not be a disconnected sidecar index.
- The product must not require predefined workflows as a first-class concept.
- Future routines or presets may exist, but they must layer on top of the session/thread model rather than replace it.

#### Workspace indexing contract

- `index_workspace` starts a background ingestion job for a selected filesystem directory.
- `get_workspace_index_job` returns authoritative progress and final counts for that ingestion job.
- re-indexing the same source root must replace stale imported documents under the corresponding workspace import prefix.
- workspace search results should expose enough metadata for supervision, including workspace document path and source file path when the result originates from filesystem ingestion.

#### Forbidden boundary crossings

- `ui/` must use Tauri IPC as the primary desktop transport.
- `src-tauri/` must not own session state, thread state, or approval decisions.
- Business operations must not bypass the runtime/storage layer to talk directly to storage.
- Risky actions must not bypass the tool/safety layer.
- New code must not reintroduce channel-first, gateway-first, PostgreSQL, or predefined-workflow-first assumptions.

#### Binding and exposure

- The desktop app is local-only and uses embedded runtime + Tauri IPC by default.
- Optional webhook-facing ingress exists only through installable WASM channels.
- The app does not ship built-in public chat/channel exposure features beyond the desktop shell.

### 4. Validation And Error Matrix

| Condition | Expected Behavior | Error Shape |
|-----------|-------------------|-------------|
| UI expects anything other than Tauri IPC/runtime events as primary desktop transport | reject in review | design rejection |
| Desktop shell lacks a native capability | degrade gracefully | capability unavailable signal or no-op |
| New code reintroduces predefined workflow CRUD as a core API | reject in review | design rejection |
| Thread-driven runtime logic tries to mutate files outside the tool layer | block the implementation | review/test failure |

### 5. Good / Base / Bad

#### Good

- Tauri starts the embedded runtime and passes a shared `AppState` into IPC commands.
- Svelte writes settings via Tauri IPC, receives runtime updates from Tauri events, and renders approvals from structured payloads.
- Folder drops become IPC calls to workspace commands.

#### Base

- The desktop shell is the authoritative user-facing runtime.
- Native capabilities add convenience without changing runtime authority.

#### Bad

- Tauri command writes thread/run status directly into SQLite.
- UI flow bypasses Tauri IPC/runtime events and talks to runtime state through an ad hoc transport.
- A new feature requires predefined workflow CRUD before a user can talk to the agent.

### 6. Tests Required

- startup test proving Tauri boot still constructs shared runtime state
- integration test proving desktop shell callbacks still flow through IPC commands and Tauri events
- regression test proving session/thread/run flows work without predefined workflow endpoints
- regression test proving optional WASM channels can inject messages without bypassing session/thread routing

### 7. Wrong vs Correct

#### Wrong

```rust
#[tauri::command]
async fn approve_task(run_id: String, db: tauri::State<'_, Db>) -> Result<(), String> {
    let _ = run_id;
    let _ = db;
    Err("business state must flow through runtime services, not direct DB writes".to_string())
}
```

#### Correct

```rust
#[tauri::command]
async fn send_session_message(
    session_id: String,
    payload: SendSessionMessageRequest,
    app_state: tauri::State<'_, AppState>,
) -> Result<SendSessionMessageResponse, String> {
    let _ = session_id;
    let _ = payload;
    let _ = app_state;
    unimplemented!("IPC command delegates into runtime-owned session/thread services")
}
```

Native affordances may help the user, but the authoritative action still goes through the shared runtime service layer reached from Tauri IPC.
