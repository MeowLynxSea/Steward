# Phase 2 Task History And Detail

## Goal

Make task execution durable, restart-safe, and inspectable before shipping real workflows.

## Scope

- Task instance persistence
- Step timeline persistence
- Approval and mode-change history
- Task detail API
- Result metadata persistence

## Requirements

- Persist execution steps, checkpoint state, and final outputs or failures.
- Expose task detail data in a timeline-ready shape.
- Preserve task state across refresh and process restart.
- Distinguish queued, running, waiting approval, completed, failed, cancelled, and rejected histories.

## Acceptance Criteria

- [x] Completed and failed tasks remain inspectable after restart.
- [x] Approval decisions and mode changes are visible in task history.
- [x] Task detail API returns enough data to drive a dedicated detail view.
- [x] Task history behavior is covered by regression tests.

## Out Of Scope

- Concrete file archive or briefing logic
