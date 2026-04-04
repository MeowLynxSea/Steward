# Steward User Guide

## What Steward Is

Steward is a local-first desktop agent for knowledge work.

The primary model is:

- start or reopen a persistent session
- give the agent a goal in natural language
- watch the current run/plan/approval state
- approve risky actions when Ask mode pauses execution
- use indexed workspace material to ground the agent's work

Steward is not intended to be a hosted multi-user service and it does not expose a built-in remote control surface.

## Runtime Modes

### Browser Mode

Browser mode serves the Svelte UI from the local Axum backend and opens the product in a normal browser.

Start it with:

```bash
cargo run -- api serve --port 8765
```

Then open:

```text
http://127.0.0.1:8765
```

Use browser mode when you want the full session/run/workspace UI without packaging a native app.

### Desktop Mode

Desktop mode uses the same HTTP/SSE backend contract but wraps the UI inside the Tauri shell for native notifications, tray behavior, and folder-drop indexing.

Local development flow:

```bash
npm --prefix ui run build -- --watch
cargo run -- api serve --port 8765
cargo tauri dev --config src-tauri/tauri.conf.json
```

Packaged desktop builds are described in [release-readiness.md](./release-readiness.md).

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
- understand what run is currently attached to the conversation

### Runs And Approvals

Runs are durable execution records derived from session turns.

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
- built-in remote/public exposure of the local agent
- multi-tenant hosted administration

## Recommended First Run

1. Configure one provider and verify the backend starts.
2. Open browser mode at `127.0.0.1:8765`.
3. Create a session and send a small goal.
4. Switch between Ask and Yolo intentionally to understand the approval model.
5. Index one workspace folder and use search results to ground a follow-up session turn.
