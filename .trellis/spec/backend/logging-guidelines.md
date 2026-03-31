# Logging Guidelines

> How logging is done in this project.

---

## Overview

The backend uses `tracing` and `tracing-subscriber`.

Examples in the current codebase:

- tracing setup imports and initialization: [src/main.rs](/Users/MeowLynxSea/Development/IronCowork/src/main.rs)
- bootstrap warnings and migration info: [src/bootstrap.rs](/Users/MeowLynxSea/Development/IronCowork/src/bootstrap.rs)
- runtime loop warnings/errors: [src/agent/agent_loop.rs](/Users/MeowLynxSea/Development/IronCowork/src/agent/agent_loop.rs)
- session/runtime warnings: [src/agent/session_manager.rs](/Users/MeowLynxSea/Development/IronCowork/src/agent/session_manager.rs)

---

## Log Levels

- `debug!` for lifecycle detail useful during development or local diagnosis.
- `info!` for major state transitions such as startup, migration completion, successful recovery, and completed background actions.
- `warn!` for recoverable failures, deprecated paths, partial persistence failure, or missing optional runtime pieces.
- `error!` for failed operations that change task/session outcome or indicate a broken runtime path.

---

## Structured Logging

- Prefer structured fields where identifiers matter, for example `job_id`, `routine`, `tool`, or `path`.
- Keep messages short and action-oriented.
- Do not rely on prose-only logs when a stable key field can be attached.

### Good

```rust
tracing::warn!(job_id = %uuid, "Failed to persist cancellation to DB: {}", e);
```

### Bad

```rust
tracing::warn!("Something went wrong: {}", e);
```

The second form loses the main correlation field.

---

## What to Log

- startup mode and major dependency wiring
- task lifecycle transitions
- approval checkpoints and rejection paths
- scheduler/routine transitions
- persistence failures that can cause drift between memory and database
- migration operations that move or rewrite local state

---

## What NOT to Log

- API keys, secrets, tokens, or raw credential material
- full prompt or tool payload bodies unless the path is explicitly designed for safe trace recording
- user file contents when a path or summary is enough
- noisy duplicate logs at multiple layers for the same error
