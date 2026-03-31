# Phase 2 File Archive Workflow

## Goal

Ship the first complete automation workflow: scan a directory, propose organization actions, and execute them safely in Ask or Yolo mode.

## Scope

- Directory scan and classification
- Proposed rename/move operation preview
- Ask/Yolo execution behavior for file mutations
- Workflow parameter form and preview UX
- Final result summary

## Requirements

- Classify files from a source directory into categories or target destinations.
- Persist proposed operations before execution.
- Route all filesystem mutations through the approved tool/safety path.
- Show structured previews in Ask mode and fully automated execution in Yolo mode.

## Acceptance Criteria

- [x] A user can run the archive workflow from the UI without raw JSON.
- [x] Ask mode pauses before file mutation with a typed preview payload.
- [x] Yolo mode executes the same plan automatically under policy.
- [x] Result state records moved, skipped, and failed operations.

## Out Of Scope

- Full undo/rollback engine
- Arbitrary shell-command file management
