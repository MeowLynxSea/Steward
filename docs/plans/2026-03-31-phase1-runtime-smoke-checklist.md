# Phase 1 Runtime Smoke Checklist

**Date:** 2026-03-31
**Scope:** Browser mode and Tauri desktop mode validation for the Phase 1 shell

---

## Preconditions

1. Start the local API/server from the repository root.
2. Confirm the health endpoint returns `200 OK` from `http://127.0.0.1:8765/api/v0/health`.
3. For browser mode, open `http://127.0.0.1:8765`.
4. For desktop mode, launch the Tauri shell from `src-tauri/` against the same backend.

---

## Browser Mode

### Settings

1. Open the `Settings` view.
2. Set `LLM Backend` and `Selected Model` to non-empty values.
3. Save.

Expected:
- Save succeeds without page reload.
- Refreshing the page keeps the saved values.

### Sessions

1. Open the `Sessions` view.
2. Create a new session.
3. Send one message.

Expected:
- The new session appears in the list immediately.
- The message is queued without native/Tauri dependencies.
- Session SSE stays connected after selecting the session.

### Tasks

1. Open the `Tasks` view.
2. Confirm task records render with `queued`, `running`, `waiting_approval`, `completed`, `failed`, or `rejected` status values.
3. Toggle one task from `ask` to `yolo` and refresh.

Expected:
- Mode changes persist.
- Waiting-approval tasks expose a structured approval object, not raw log parsing.

### Workspace

1. Open the `Workspace` view.
2. Index a local folder path.
3. Refresh the tree and run one search query.

Expected:
- Index request succeeds over HTTP.
- The indexed record appears in the workspace tree.
- Search returns stored snippets when indexed content exists.

### Browser Capability Fallback

Expected:
- No crash when notifications are unavailable.
- No crash when drag-and-drop native integration is unavailable.
- Missing desktop capabilities degrade to no-op behavior.

---

## Desktop Mode

### Shared Flows

Repeat the browser checks for:
- settings save/load
- session create and send
- task mode toggle
- workspace indexing

Expected:
- Desktop mode uses the same HTTP/SSE backend contract as browser mode.

### Native Notifications

1. Trigger a task transition that reaches `waiting_approval` or `completed`.

Expected:
- The desktop shell can show a native notification.
- Failure to show a notification must not break the UI flow.

### Folder Drop

1. Drag one folder onto the desktop window.

Expected:
- The shell forwards the dropped path to `POST /api/v0/workspace/index`.
- The workspace view can refresh and show the new indexed record.

### Tray

1. Use the tray menu `Show`.
2. Use the tray menu `Quit`.

Expected:
- `Show` focuses the main window.
- `Quit` exits cleanly.

---

## Known Phase 1 Limits

- Workspace indexing is still a recorded stub, not full ingestion.
- Task/run creation is still backed by legacy thread-driven runtime state, not a fully durable session-first run model.
- Task SSE is normalized for typed frontend handling, but Phase 2 will expand the event taxonomy with persisted step history.
