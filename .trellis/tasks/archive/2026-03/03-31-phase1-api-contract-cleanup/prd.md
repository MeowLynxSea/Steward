# Phase 1 API Contract Cleanup

## Goal

Normalize the current v0 API surface so Phase 2 can build on stable task, session, workspace, and SSE contracts.

## Scope

- Task mode switch route normalization
- Approval request shape normalization
- SSE event envelope consistency
- Missing status-code and error-shape cleanup
- Session/task detail payload checks

## Requirements

- Resolve drift between route names already shipped and route names documented in specs.
- Standardize `404`, `409`, and `422` response semantics for UI-facing flows.
- Ensure task/session stream envelopes are stable and typed.
- Expand integration tests to lock these contracts before Phase 2 starts.

## Acceptance Criteria

- [ ] Route and payload naming is consistent across code, docs, and tests.
- [ ] Task approval and mode-switch endpoints have one authoritative request shape.
- [ ] SSE event payloads are stable enough for typed frontend handling.
- [ ] API integration tests cover the normalized contract.

## Out Of Scope

- New template/business endpoints
- Visual frontend refactors except where needed for contract alignment
