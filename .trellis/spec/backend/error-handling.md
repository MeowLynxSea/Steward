# Error Handling

> How errors are handled across CLI boot, HTTP API, and runtime paths.

---

## Overview

The backend currently uses:

- `anyhow::Result` for top-level boot and command execution
- typed HTTP status mapping in the API layer
- `tracing` for error logging inside long-lived runtime paths

Examples:

- top-level formatting and recovery hints: [src/main.rs](/Users/MeowLynxSea/Development/Steward/src/main.rs)
- API error bodies and status mapping: [src/api.rs](/Users/MeowLynxSea/Development/Steward/src/api.rs)
- runtime warnings around session/task behavior: [src/agent/session_manager.rs](/Users/MeowLynxSea/Development/Steward/src/agent/session_manager.rs)

---

## Error Types

- Use `anyhow::Result` at process boot and broad orchestration boundaries where multiple failure sources must be combined.
- Use typed API response shapes at HTTP boundaries instead of returning raw internal errors.
- Use domain-specific status codes for request validation and runtime-state conflicts:
  - `404` for missing resource
  - `409` for invalid state transition such as stale approval or missing pending action
  - `422` for invalid input

---

## Error Handling Patterns

- Log the internal failure with context.
- Return a user-safe message at the HTTP boundary.
- Keep recovery hints at the top-level boot path where the app can guide the operator.
- For long-running runtime loops, warn on recoverable issues and error on operation failure that changes task/session outcome.

### Good

- Boot path prints one formatted top-level error plus a concrete hint.
- API handler returns `StatusCode` plus `{ "error": "..." }`.
- Runtime emits SSE error/status events when the UI must react.

### Bad

- Returning raw debug strings or stack traces directly to API clients.
- Swallowing errors in runtime loops without logging.
- Using one generic `500` path for validation and conflict failures that the UI needs to distinguish.

---

## API Error Responses

The current HTTP pattern is a small JSON body:

```json
{
  "error": "task not found"
}
```

Rules:

- error payloads must be stable enough for UI display
- status code must carry the primary class of failure
- request validation failures should be distinct from resource lookup failures

---

## Common Mistakes

- Letting API/UI contracts depend on exact free-form error text instead of status code plus stable message shape.
- Logging after the point where context has already been lost.
- Converting recoverable runtime situations into fatal process exits.
