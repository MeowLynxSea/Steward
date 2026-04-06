# Quality Guidelines

> Code quality standards for backend development in this repository.

---

## Overview

This project is in an architectural migration. Quality means more than "code compiles":

- changes must respect the desktop-first target architecture
- cross-layer contracts must be explicit
- tests must cover the behavior that moved, not only the function that changed

Current concrete examples:

- desktop IPC boundary code: [src/tauri_commands.rs](/Users/MeowLynxSea/Development/IronCowork/src/tauri_commands.rs)
- shared IPC DTOs: [src/ipc.rs](/Users/MeowLynxSea/Development/IronCowork/src/ipc.rs)
- runtime boot and wiring: [src/desktop_runtime.rs](/Users/MeowLynxSea/Development/IronCowork/src/desktop_runtime.rs)

---

## Forbidden Patterns

- Reintroducing PostgreSQL-first or dual-backend abstractions after the Phase 0 cleanup.
- Putting business logic into `src-tauri/` instead of the runtime/service layer.
- Driving task approval off untyped log strings instead of explicit task state and event payloads.
- Adding new product flows that depend on deleted channel/gateway assumptions.
- Hiding significant behavior changes behind undocumented Trellis spec drift.

---

## Required Patterns

- Add or update tests when changing IPC commands, task runtime behavior, settings persistence, or workspace behavior.
- Keep request validation, state transition validation, and error status codes explicit at the API boundary.
- Update `.trellis/spec/backend/` when you establish or change an infra or cross-layer contract.
- Prefer vertical-slice verification: boot/runtime/IPC/UI path should remain consistent.

---

## Testing Requirements

- IPC contract changes require integration coverage through the Tauri command layer where practical.
- Task runtime changes require state-transition coverage, especially around Ask/Yolo behavior.
- Storage changes require temp-database regression tests.
- Desktop shell changes require at least a compile check plus one backend-facing verification path.

Minimum examples already in tree:

- `cargo test`
- `cargo check --manifest-path src-tauri/Cargo.toml`

---

## Code Review Checklist

- Does the change preserve the desktop-first Tauri IPC boundary?
- Does it keep Tauri as a native bridge instead of a second backend?
- Are IPC errors and payloads usable by the frontend?
- Are task/session/workspace state changes persisted where required?
- Were Trellis specs updated if the contract or convention changed?
