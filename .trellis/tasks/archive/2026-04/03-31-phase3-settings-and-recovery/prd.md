# Phase 3 Settings, Recovery, And Background Operation

## Goal

Make provider configuration, session/run recovery, and future background execution dependable for repeated local use.

## Scope

- Provider-specific validation
- Secret storage policy refinement
- Settings schema migration handling
- Session/run recovery after restart
- Recovery rules for pending approvals and background work

## Requirements

- Validate provider configuration before the user starts serious work.
- Define restart behavior for sessions, runs, pending approvals, and future routines.
- Ensure the UI can rebuild state after refresh or process restart.
- Preserve safety guarantees while storing local configuration and credentials.

## Acceptance Criteria

- [ ] Provider settings are validated with actionable failures.
- [ ] Sessions and runs recover predictably after restart.
- [ ] Pending approvals do not disappear across process restarts.
- [ ] Recovery behavior is covered by tests.

## Out Of Scope

- New LLM provider integrations for their own sake
