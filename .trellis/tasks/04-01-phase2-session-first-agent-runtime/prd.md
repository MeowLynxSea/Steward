# Phase 2 Session-First Agent Runtime

## Goal

Make persistent chat sessions the primary way users interact with IronCowork, with autonomous execution happening behind the conversation.

## Scope

- Session lifecycle and durable state
- Session message to run/task creation semantics
- Session replay and restart behavior
- UI/API language cleanup around session-first interaction

## Requirements

- A user can create a session and give the agent a broad goal in natural language.
- Session messages may create or continue execution runs without forcing the user through predefined forms.
- The runtime persists enough session state for refresh and restart recovery.
- Session detail can show the relationship between messages, active work, and completed runs.

## Acceptance Criteria

- [ ] Creating a session and sending a goal works end to end.
- [ ] Session state survives refresh and process restart.
- [ ] Message-to-run linkage is explicit in the backend contract.
- [ ] No core flow depends on predefined workflow CRUD.

## Out Of Scope

- Specialized workflow builders
- Rich routine authoring UI
