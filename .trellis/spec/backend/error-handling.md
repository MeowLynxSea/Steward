# Error Handling

> How errors are handled across CLI boot, Tauri IPC, and runtime paths.

---

## Overview

The backend currently uses:

- `anyhow::Result` for top-level boot and command execution
- typed IPC error mapping in the desktop command layer
- `tracing` for error logging inside long-lived runtime paths

Examples:

- top-level formatting and recovery hints: [src/main.rs](/Users/MeowLynxSea/Development/IronCowork/src/main.rs)
- IPC error payloads: [src/tauri_commands.rs](/Users/MeowLynxSea/Development/IronCowork/src/tauri_commands.rs)
- runtime warnings around session/task behavior: [src/agent/session_manager.rs](/Users/MeowLynxSea/Development/IronCowork/src/agent/session_manager.rs)

---

## Error Types

- Use `anyhow::Result` at process boot and broad orchestration boundaries where multiple failure sources must be combined.
- Use typed IPC response or error shapes at the desktop command boundary instead of returning raw internal errors.
- Use stable error categories for request validation and runtime-state conflicts:
  - not found for missing resource
  - conflict for invalid state transition such as stale approval or missing pending action
  - validation error for invalid input

---

## Error Handling Patterns

- Log the internal failure with context.
- Return a user-safe message at the IPC boundary.
- Keep recovery hints at the top-level boot path where the app can guide the operator.
- For long-running runtime loops, warn on recoverable issues and error on operation failure that changes task/session outcome.

### Good

- Boot path prints one formatted top-level error plus a concrete hint.
- Tauri command returns a stable string or structured payload that the desktop UI can display safely.
- Runtime emits Tauri error/status events when the UI must react.

### Bad

- Returning raw debug strings or stack traces directly to desktop clients.
- Swallowing errors in runtime loops without logging.
- Using one generic `500` path for validation and conflict failures that the UI needs to distinguish.

---

## IPC Error Responses

The current desktop pattern is a small user-safe error payload or command error string:

```json
{
  "error": "task not found"
}
```

Rules:

- error payloads must be stable enough for UI display
- the command contract must carry the primary class of failure
- request validation failures should be distinct from resource lookup failures

---

## Common Mistakes

- Letting IPC/UI contracts depend on exact free-form error text instead of stable message shape and error category.
- Logging after the point where context has already been lost.
- Converting recoverable runtime situations into fatal process exits.
