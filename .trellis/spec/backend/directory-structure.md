# Directory Structure

> How backend code is organized in this project during the Steward -> Steward migration.

---

## Overview

The repository is in a transition state:

- the main backend still lives in the root `src/` tree
- desktop and web UI code now live in `src-tauri/` and `ui/`
- Trellis specs must describe both the current monolith and the target split

When adding backend code, prefer extending the current module boundaries cleanly instead of scattering logic across CLI, API, runtime, and storage call sites.

---

## Directory Layout

```text
src/
├── api.rs                 # Local Axum HTTP/SSE API surface
├── main.rs                # Runtime boot and CLI dispatch
├── bootstrap.rs           # Environment/bootstrap loading and migration helpers
├── agent/                 # Agent loop, sessions, routing, routines, approval flow
├── db/                    # libSQL-backed persistence
├── workspace/             # Indexing, tree access, search, retrieval helpers
├── settings/              # Runtime settings model and persistence helpers
├── runtime_events/        # SSE/event fanout primitives
└── tools/                 # Built-in tool implementations

tests/
└── api_http_integration.rs # Cross-layer API regression coverage

src-tauri/
└── src/main.rs            # Native shell only

ui/
└── src/                   # Svelte client consuming HTTP/SSE
```

Real examples in the current codebase:

- API routing and DTOs: [src/api.rs](/Users/MeowLynxSea/Development/Steward/src/api.rs)
- Runtime boot wiring: [src/main.rs](/Users/MeowLynxSea/Development/Steward/src/main.rs)
- Session lifecycle logic: [src/agent/session_manager.rs](/Users/MeowLynxSea/Development/Steward/src/agent/session_manager.rs)
- Task approval emission: [src/agent/thread_ops.rs](/Users/MeowLynxSea/Development/Steward/src/agent/thread_ops.rs)
- Desktop bridge: [src-tauri/src/main.rs](/Users/MeowLynxSea/Development/Steward/src-tauri/src/main.rs)

---

## Module Organization

### Current rule

- HTTP request parsing, response DTOs, and SSE endpoints belong in `src/api.rs`.
- Runtime behavior belongs in domain modules under `src/agent/`, `src/workspace/`, `src/settings/`, or `src/db/`.
- `src/main.rs` is for bootstrapping and dependency wiring only. Do not move feature logic into it.
- `src-tauri/` must remain a thin bridge to the HTTP backend.
- `ui/` must not become a second source of business truth.

### Target rule

As the migration continues, logic should move toward:

- `crates/steward-api` for Axum routes
- retained core/runtime crates for agent, storage, workspace, and tools
- `src-tauri/` only for native capabilities

Until that split exists, write code so that extraction is straightforward: keep DTOs, runtime state, and persistence concerns separated.

---

## Naming Conventions

- Rust modules use snake_case file names such as `session_manager.rs`, `thread_ops.rs`, and `api.rs`.
- Types use descriptive product-facing names such as `ApiState`, `SettingsResponse`, `TaskRecord`, and `WorkspaceTreeResponse`.
- Route handler helpers should be named by resource/action, not UI concepts.
- Avoid reusing old "channel" or "gateway" terminology for new desktop-first code.

---

## Examples

### Good

- [src/api.rs](/Users/MeowLynxSea/Development/Steward/src/api.rs) keeps request/response types near the route handlers that use them.
- [src/main.rs](/Users/MeowLynxSea/Development/Steward/src/main.rs) wires `ApiState`, `TaskRuntime`, `SseManager`, and session manager together without owning their business rules.
- [tests/api_http_integration.rs](/Users/MeowLynxSea/Development/Steward/tests/api_http_integration.rs) validates the HTTP layer from the outside instead of unit-testing handlers in isolation only.

### Bad

- Adding new task approval logic directly in `src-tauri/src/main.rs`.
- Hiding storage mutations inside CLI command handlers when the same behavior should be reachable through API/runtime modules.
- Spreading one feature across `main.rs`, `api.rs`, and UI code without a clear owning backend module.
