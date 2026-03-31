# Phase 1 Runtime Validation

## Goal

Turn the current Phase 1 shell from "builds successfully" into a manually usable browser and desktop development baseline.

## Scope

- Local Axum startup verification
- Browser-mode manual smoke flow
- Tauri-mode manual smoke flow
- Capability fallback behavior for browser mode
- Gap fixes discovered during end-to-end boot and manual use

## Requirements

- Verify settings save/load, session send/receive, task list refresh, and workspace indexing in browser mode.
- Verify the same flows in Tauri mode, plus notifications and folder-drop forwarding.
- Document one repeatable smoke script for local verification.
- Fix any blocking issues found while exercising the shell.

## Acceptance Criteria

- [ ] Browser mode works against `127.0.0.1` without native APIs.
- [ ] Tauri mode works against the same backend and can complete one smoke run.
- [ ] Missing native capabilities degrade cleanly in browser mode.
- [ ] A local manual verification checklist is persisted.

## Out Of Scope

- Template CRUD
- Full workspace ingestion redesign
- New product workflows beyond shell validation
