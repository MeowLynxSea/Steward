# Phase 1 Runtime Smoke Checklist

**Date:** 2026-03-31
**Scope:** Tauri desktop mode validation for the Phase 1 shell

---

## Preconditions

1. Start the desktop development shell from the repository root.
2. Confirm the desktop app can reach the shared runtime through Tauri IPC.
3. Open the main desktop window and verify the runtime event stream is active.

---

## Desktop Mode

### Core Flows

Validate:

- settings save/load
- session create and send
- thread selection and continued messaging
- task/run mode toggle
- workspace indexing

Expected:

- Desktop mode uses Tauri IPC and Tauri runtime events as its primary contract.
- Session/thread state survives view switching and refresh.
- Waiting-approval runs expose structured approval objects, not raw log parsing.

### Native Notifications

1. Trigger a task transition that reaches `waiting_approval` or `completed`.

Expected:

- The desktop shell can show a native notification.
- Failure to show a notification must not break the UI flow.

### Folder Drop

1. Drag one folder onto the desktop window.

Expected:

- The shell forwards the dropped path into the shared runtime through Tauri IPC.
- The workspace view can refresh and show the new indexed record.

### Tray

1. Use the tray menu `Show`.
2. Use the tray menu `Quit`.

Expected:

- `Show` focuses the main window.
- `Quit` exits cleanly.

### Optional WASM Channel Ingress

1. Install or enable one WASM channel.
2. Send one message through that channel into the running desktop runtime.

Expected:

- The message is routed into the same session/thread runtime rather than a separate legacy channel stack.
- Desktop state remains authoritative even when the message originated outside the desktop shell.

---

## Known Phase 1 Limits

- Workspace indexing is still a recorded stub, not full ingestion.
- Task/run creation is still backed by legacy thread-driven runtime state, not a fully durable session-first run model.
- Runtime event handling is stable enough for typed frontend handling, but Phase 2 will expand the event taxonomy with persisted step history.
