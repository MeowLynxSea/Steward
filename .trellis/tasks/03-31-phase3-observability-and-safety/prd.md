# Phase 3 Observability And Safety

## Goal

Harden diagnostics and confirm that the desktop-first fork still routes risky side effects through the retained safety model.

## Scope

- Structured task lifecycle logging
- Correlation IDs across REST/SSE/runtime events
- Developer-facing diagnostics or export hooks
- File/network side-effect audit
- Approval bypass regression tests

## Requirements

- Log task, approval, scheduler, and tool execution transitions with stable identifiers.
- Confirm no new workflow bypasses the tool/safety boundary.
- Revalidate secret handling for settings and MCP credentials.
- Add tests that prove approval checks still gate risky actions.

## Acceptance Criteria

- [ ] Logs are sufficient to diagnose task failures without a debugger.
- [ ] REST, SSE, and runtime events can be correlated.
- [ ] Safety regressions around file and network mutations are covered by tests.
- [ ] No new direct shell-side mutation path exists outside the tool layer.

## Out Of Scope

- Operator dashboard revival
- Legacy gateway-style observability UI
