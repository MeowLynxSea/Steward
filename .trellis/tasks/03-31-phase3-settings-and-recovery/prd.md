# Phase 3 Settings And Recovery

## Goal

Make provider configuration and runtime recovery dependable for repeated local use.

## Scope

- Provider-specific validation
- Secret storage policy refinement
- Settings schema migration handling
- Task/session recovery after restart
- SSE re-subscribe and state-rebuild behavior

## Requirements

- Validate provider configuration before the user starts work.
- Define restart behavior for waiting approvals and scheduled tasks.
- Ensure the UI can rebuild state after refresh or process restart.
- Preserve safety guarantees while storing local configuration and credentials.

## Acceptance Criteria

- [ ] Provider settings are validated with actionable failures.
- [ ] Tasks and sessions recover predictably after restart.
- [ ] Waiting approvals do not disappear across process restarts.
- [ ] Recovery behavior is covered by tests.

## Out Of Scope

- New LLM provider integrations for their own sake
