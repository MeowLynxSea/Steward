# Phase 1 Frontend State Hygiene

## Goal

Refactor the current Svelte shell so it can safely carry Phase 2 complexity without centralizing all state in one root component.

## Scope

- View-state extraction from `App.svelte`
- Shared HTTP/SSE adapter cleanup
- Loading, empty, and error-state coverage
- Basic route/deep-link baseline

## Requirements

- Move settings, sessions, tasks, and workspace state into focused modules or stores.
- Introduce a typed stream adapter for backend event envelopes.
- Ensure page refresh and session switching do not leave stale subscriptions behind.
- Support direct navigation to core views without manual in-app bootstrapping.

## Acceptance Criteria

- [ ] `App.svelte` is no longer the single owner of all application state.
- [ ] All current views have explicit loading and error states.
- [ ] Stream subscription lifecycle is deterministic.
- [ ] Frontend still builds to static assets for backend/Tauri use.

## Out Of Scope

- Final UI polish
- New task workflows
