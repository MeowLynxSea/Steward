# Phase 2 Run History And Approval Center

## Goal

Make agent autonomy inspectable through durable run history, approval checkpoints, and clear Ask/Yolo behavior.

## Scope

- Run/task persistence model
- Step and timeline persistence
- Approval payload persistence
- Run detail API and UI
- Ask/Yolo mode changes and auditability

## Requirements

- Every meaningful agent execution path is recorded as a durable run/task.
- Ask mode exposes structured pending approvals instead of raw logs.
- Approval, rejection, cancellation, and mode switching are stored as history.
- Completed or failed runs remain inspectable after restart.

## Acceptance Criteria

- [ ] Run detail exposes timeline, current step, approvals, and outputs.
- [ ] Ask/Yolo decisions remain visible after refresh and restart.
- [ ] Approval actions use structured payloads that the UI can render directly.
- [ ] No approval path bypasses the safety boundary.

## Out Of Scope

- Predefined-workflow UIs
- Cosmetic timeline redesign without backend durability
