# Directory Structure

> How backend code is organized in this project during the Steward -> Steward migration.

---

## Overview

The repository is in a transition state:

- the main backend still lives in the root `src/` tree
- desktop shell and UI code now live in `src-tauri/` and `ui/`
- Trellis specs must describe both the current monolith and the target split

When adding backend code, prefer extending the current module boundaries cleanly instead of scattering logic across CLI, API, runtime, and storage call sites.

---

## Directory Layout

```text
src/
├── ipc.rs                 # Shared Tauri IPC DTOs
├── tauri_commands.rs      # Tauri IPC command surface
├── main.rs                # Runtime boot and CLI dispatch
├── bootstrap.rs           # Environment/bootstrap loading and migration helpers
├── agent/                 # Agent loop, sessions, threads, routines, approval flow
├── db/                    # libSQL-backed persistence
├── workspace/             # Indexing, tree access, search, retrieval helpers
├── settings/              # Runtime settings model and persistence helpers
├── runtime_events/        # Runtime event fanout primitives
└── tools/                 # Built-in tool implementations

src-tauri/
└── src/main.rs            # Native shell only

ui/
└── src/                   # Svelte client consuming Tauri IPC and Tauri events
```

Real examples in the current codebase:

- IPC DTOs: [src/ipc.rs](/Users/MeowLynxSea/Development/IronCowork/src/ipc.rs)
- Tauri commands: [src/tauri_commands.rs](/Users/MeowLynxSea/Development/IronCowork/src/tauri_commands.rs)
- Runtime boot wiring: [src/desktop_runtime.rs](/Users/MeowLynxSea/Development/IronCowork/src/desktop_runtime.rs)
- Session lifecycle logic: [src/agent/session_manager.rs](/Users/MeowLynxSea/Development/IronCowork/src/agent/session_manager.rs)
- Task approval emission: [src/agent/thread_ops.rs](/Users/MeowLynxSea/Development/IronCowork/src/agent/thread_ops.rs)

---

## Module Organization

### Current rule

- Tauri command handlers and shared IPC DTOs belong in `src/tauri_commands.rs` and `src/ipc.rs`.
- Runtime behavior belongs in domain modules under `src/agent/`, `src/workspace/`, `src/settings/`, or `src/db/`.
- `src/main.rs` is for bootstrapping and dependency wiring only. Do not move feature logic into it.
- `src-tauri/` must remain a thin native bridge to the shared runtime.
- `ui/` must not become a second source of business truth.

### Target rule

As the migration continues, logic should move toward:

- runtime crates for session/thread/task services and storage
- retained core/runtime crates for agent, storage, workspace, and tools
- `src-tauri/` only for native capabilities

Until that split exists, write code so that extraction is straightforward: keep DTOs, runtime state, and persistence concerns separated.

---

## Naming Conventions

- Rust modules use snake_case file names such as `session_manager.rs`, `thread_ops.rs`, and `api.rs`.
- Types use descriptive product-facing names such as `ApiState`, `SettingsResponse`, `TaskRecord`, and `WorkspaceTreeResponse`.
- Command handler helpers should be named by resource/action, not UI concepts.
- Avoid reusing old "channel" or "gateway" terminology for new desktop-first code.

---

## Examples

### Good

- [src/tauri_commands.rs](/Users/MeowLynxSea/Development/IronCowork/src/tauri_commands.rs) keeps command handlers thin and delegates to runtime services.
- [src/desktop_runtime.rs](/Users/MeowLynxSea/Development/IronCowork/src/desktop_runtime.rs) wires task runtime, event emitter, and session manager together without owning their business rules.
- Tauri IPC regression tests should validate the desktop command layer from the outside instead of unit-testing handlers in isolation only.

### Bad

- Adding new task approval logic directly in `src-tauri/src/main.rs`.
- Hiding storage mutations inside CLI command handlers when the same behavior should be reachable through runtime modules.
- Spreading one feature across `main.rs`, `tauri_commands.rs`, and UI code without a clear owning backend module.
