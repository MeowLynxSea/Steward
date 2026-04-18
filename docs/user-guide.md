# Steward User Guide

## What Steward Is

Steward is a local-first desktop agent for knowledge work.

The primary model is:

- start or reopen a persistent session
- give the agent a goal in natural language
- chat inside the active thread for that session
- watch the current thread/run/approval state
- approve risky actions when Ask mode pauses execution
- use indexed workspace material to ground the agent's work

Steward is not intended to be a hosted multi-user service and it does not expose built-in legacy chat gateways or a built-in remote control surface.

## Runtime Modes

### Desktop Mode

Desktop mode is the primary product path. The UI talks to the runtime through Tauri IPC and receives live updates through Tauri events, while the shell adds native notifications, tray behavior, and folder-drop indexing.

Local development flow:

```bash
npm --prefix ui run build -- --watch
cargo run -- api serve --port 8765
cargo tauri dev --config tauri.conf.json
```

Packaged desktop builds are described in [release-readiness.md](./release-readiness.md).

### Optional External Ingress

Steward also supports installable WASM channels for non-desktop message ingress.

These are not the primary product surface. They exist so the same session/thread runtime can later accept messages from other environments without restoring the old built-in channel product model.

## Initial Setup

### 1. Choose an LLM provider

Set provider credentials in `.env`, `config.toml`, or the local runtime env file under `~/.steward/.env`.

Example:

```env
DATABASE_BACKEND=libsql
LIBSQL_PATH=~/.steward/steward.db
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

Provider-specific details live in [docs/LLM_PROVIDERS.md](./LLM_PROVIDERS.md).

### 2. Understand local storage

Steward stores its local runtime state under `~/.steward/` by default.

Common paths:

- `~/.steward/steward.db` for the local libSQL database
- `~/.steward/.env` for bootstrap configuration that must exist before DB startup
- `~/.steward/config.toml` and `~/.steward/settings.json` for local settings
- `~/.steward/session.json` or provider-specific auth files for OAuth-backed providers

If you need a different root, set `STEWARD_BASE_DIR` before startup.

## Core Product Model

### Sessions First

Sessions are the main user-facing object.

Use sessions to:

- keep a persistent conversation context
- revisit prior work after restart
- understand which thread is active and what run is currently attached to it

### Threads Drive Execution

Threads are the actual conversational and execution unit inside a session.

Use threads to:

- send and receive messages
- attach run/task snapshots when approvals or execution tracking are needed
- preserve the exact conversational context that drove the agent's work

### Tasks And Approvals

Tasks are durable execution records attached to session threads when the runtime needs approvals, mode tracking, or auditability.

Use the Runs view to inspect:

- current step or current action
- pending approvals
- final result or failure state
- preserved Ask/Yolo decisions

### Ask vs Yolo

`ask` mode pauses for approval before risky side effects commit.

Use it when the task may:

- modify or delete files
- invoke risky shell commands
- call tools with meaningful external side effects

`yolo` mode keeps going automatically under the same safety boundary, but without pausing for every risky step.

The desktop composer has a dedicated mode toggle. Switching into `yolo` opens a risk warning first, and if the current thread is already waiting at an approval checkpoint the runtime resumes it automatically after the switch.

### Workspace

Workspace indexing imports text-like files into the local retrieval store so the agent can search and cite relevant context.

Typical workflow:

1. Open the Workspace view.
2. Enter a folder path and start indexing.
3. Wait for the index job to finish.
4. Search indexed content or inspect imported entries.
5. Pull useful results into the active session.

Drag-and-drop indexing is available in desktop mode.

## Explicit Non-Goals

Steward does not treat these as primary product goals:

- predefined workflow forms as the main UX
- mandatory cloud-hosted runtime
- built-in remote/public exposure of the local agent beyond optional installable WASM channels
- multi-tenant hosted administration

## Recommended First Run

1. Configure one provider and verify the backend starts.
2. Launch the desktop app through Tauri.
3. Create a session and send a small goal.
4. Switch between Ask and Yolo intentionally to understand the approval model.
5. Index one workspace folder and use search results to ground a follow-up thread turn.
