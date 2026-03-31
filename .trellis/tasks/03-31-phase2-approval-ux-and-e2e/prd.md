# Phase 2 Approval UX And MVP Coverage

## Goal

Finish the user-facing Ask/Yolo loop and freeze the MVP contract with end-to-end coverage.

## Scope

- Approval modal or side-panel UX
- Task-level mode switching UX
- Rejection handling and rendering
- Notification hooks for pending approval
- E2E coverage for template and task flows

## Requirements

- UI must render approval operations from typed API/SSE payloads.
- Mode switches must persist and appear in task history.
- Rejected tasks must have explicit reason and status rendering.
- Add E2E or integration coverage for template CRUD, task creation, approval flow, and detail rendering.

## Acceptance Criteria

- [ ] Waiting tasks can be approved or rejected from the UI deterministically.
- [ ] Mode changes are visible in real time and in persisted history.
- [ ] Approval UI is not driven by log scraping.
- [ ] MVP-critical cross-layer flows have automated coverage.

## Out Of Scope

- New business workflows beyond archive and briefing
