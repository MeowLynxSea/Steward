# IronCowork Post-Phase-1 Execution Roadmap

**Date:** 2026-03-31  
**Status:** Active planning baseline  
**Scope:** Work after the initial Axum + Svelte + Tauri shell exists

---

## Current State Snapshot

As of 2026-03-31, the repo already has the rough shell of the target system:

- libSQL-first local runtime
- health/settings/session/task/workspace API skeletons
- first-pass Ask/Yolo interception
- Svelte shell for settings, sessions, tasks, and workspace
- Tauri shell with notifications, tray, and folder-drop forwarding

The remaining work is not "invent a shell". It is "correct the product model and make the shell behave like a desktop autonomous agent".

---

## Delivery Principles

- Keep sessions as the main user-facing object.
- Keep tasks/runs as durable execution records and background units.
- Delete predefined-workflow-centric product and API assumptions instead of layering over them.
- Optimize for general-purpose agent behavior before vertical workflows.
- Preserve the HTTP/SSE contract as the only business interface.
- Treat Ask/Yolo as core runtime policy.

---

## Phase Structure

### Phase 1 Closeout

Goal: make the current shell usable and align contracts with the corrected product direction.

### Phase 2 Autonomous Agent Core

Goal: ship a general desktop agent that can work inside persistent sessions and produce inspectable runs.

### Phase 3 Stability And Background Operation

Goal: make the agent reliable across restart, indexing, long-running work, and future routine execution.

### Phase 4 Packaging, Rebrand, And Docs

Goal: finish the product identity and make the system understandable to developers and users.

---

## Active Trellis Task Registry

- `04-01-phase2-session-first-agent-runtime`
- `04-01-phase2-run-history-and-approval-center`
- `04-01-phase2-general-agent-workbench`
- `03-31-phase3-workspace-indexing-and-retrieval`
- `03-31-phase3-settings-and-recovery`
- `03-31-phase3-observability-and-safety`
- `03-31-phase4-rebrand-and-bootstrap`
- `03-31-phase4-packaging-and-user-docs`

Deleted from the active plan:

- predefined-workflow persistence direction
- specialized recurring automation as a primary phase target

---

## Phase 1 Closeout

### Exit Criteria

- Browser mode can save settings, create/select sessions, send messages, and observe run state changes.
- Desktop mode can do the same flows and additionally show notifications and accept folder drops.
- API naming and SSE payloads reflect a session/run mental model rather than predefined workflow execution.
- The UI survives refresh without losing authoritative session/run state.

### Issue 1.6: Manual Runtime Validation And Gap Fixes

Purpose: close the gap between "shell exists" and "developers can actually use it".

Work items:

- validate browser mode against `127.0.0.1`
- validate Tauri mode against the same backend
- fix stream resubscribe and refresh-state rebuild issues
- define capability fallback behavior for browser-only mode
- remove UI wording that implies the product is predefined-workflow-driven

### Issue 1.7: API Contract Cleanup Around Sessions And Runs

Purpose: normalize the current implementation before Phase 2 builds on it.

Work items:

- standardize `task` vs `run` terminology in API and UI
- normalize Ask/Yolo route shapes and error payloads
- remove predefined workflow CRUD from the intended v0 contract
- ensure run detail and run stream payloads are stable enough for typed frontend handling

### Issue 1.8: Frontend State Hygiene

Purpose: keep the session-first UI from collapsing into ad hoc state.

Work items:

- isolate stores for settings, sessions, runs, and workspace
- centralize SSE envelope parsing
- add loading, empty, and error states
- support direct links into a session or run detail view

---

## Phase 2 Autonomous Agent Core

### Exit Criteria

- Users can stay inside a persistent session and drive the system through natural language goals.
- The agent can inspect workspace context, use tools, and emit visible step/run history.
- Ask mode surfaces actionable approval payloads for risky operations.
- Yolo mode can continue autonomously under the same safety constraints.

### Dependency Order

1. Session-first runtime semantics
2. Durable run history and approval center
3. General workspace/tool orchestration
4. Conversation UX and agent visibility

### Issue 2.1: Session-First Agent Runtime

Purpose: make sessions authoritative and make run creation an implementation detail of the agent loop.

Work items:

- define how session messages create or attach to runs
- persist session state needed for restart and replay
- make current agent action visible as part of the session experience
- ensure the API does not force users through predefined forms

Acceptance:

- a user can create a session, send a goal, and observe the agent progress
- session history survives refresh and restart
- new execution records can be traced back to the initiating session turn

### Issue 2.2: Run History And Approval Center

Purpose: make autonomy inspectable instead of opaque.

Work items:

- persist run timeline, steps, approvals, mode changes, and outputs
- provide run detail API and stream payloads
- show pending approvals as structured proposed side effects
- allow approval, rejection, cancellation, and mode switching without losing auditability

Acceptance:

- runs remain inspectable after completion or restart
- Ask/Yolo decisions are preserved in history
- the UI can reconstruct run state without scraping raw logs

### Issue 2.3: General Agent Workbench

Purpose: support broad desktop knowledge-work goals instead of narrow workflow-specific paths.

Work items:

- improve workspace browsing and retrieval integration inside sessions
- expose tool and MCP capability visibility to the agent UI
- support agent planning/execution around files, notes, summaries, and research-style tasks
- avoid hard-coding one or two specialized workflows as the primary path

Acceptance:

- a session can use workspace context and tools to complete a broad goal
- the UI shows enough context for the user to understand what the agent is doing next
- product copy and API contracts no longer assume specialization

### Issue 2.4: Conversation UX And Agent Visibility

Purpose: make autonomy legible to users.

Work items:

- display agent thinking/step state at a product-safe level
- show current plan, current action, pending approval, and final outputs
- keep the chat surface central while exposing deeper run detail on demand

Acceptance:

- users can follow agent progress without switching mental models
- run detail feels like a drill-down from the conversation, not a separate product

---

## Phase 3 Stability And Background Operation

### Exit Criteria

- settings, sessions, approvals, and runs recover cleanly after restart
- workspace indexing is real and dependable, not a stub
- background runs and future routines have clear lifecycle rules
- logs and audit records are enough to debug failures without guesswork

### Issue 3.1: Workspace Indexing And Retrieval

Handled by the existing Trellis task `03-31-phase3-workspace-indexing-and-retrieval`.

Focus:

- recursive ingestion
- extracted text persistence
- search snippets and metadata
- progress and freshness handling

### Issue 3.2: Settings, Recovery, And Background Operation

Handled by `03-31-phase3-settings-and-recovery`.

Focus:

- provider validation
- restart behavior for sessions, approvals, and runs
- recovery of in-flight background work
- future-compatible routine support without making routines the primary product

### Issue 3.3: Observability And Safety

Handled by `03-31-phase3-observability-and-safety`.

Focus:

- structured logs across session/run/approval lifecycles
- correlation between REST, SSE, and runtime events
- regression tests proving no risky side effects bypass Ask/Yolo and safety boundaries

---

## Phase 4 Packaging, Rebrand, And Docs

### Exit Criteria

- repository identity consistently reflects IronCowork
- developer startup and architecture docs reflect the session-first agent direction
- user docs explain desktop mode, browser mode, workspace usage, approvals, and safety
- packaging targets and release metadata are coherent

### Issue 4.1: Rebrand And Bootstrap

Handled by `03-31-phase4-rebrand-and-bootstrap`.

### Issue 4.2: Packaging And User Docs

Handled by `03-31-phase4-packaging-and-user-docs`.

---

## Sequence

1. Finish Phase 1 cleanup around naming and API contracts.
2. Ship session-first runtime behavior.
3. Ship durable run history and approval center.
4. Turn the shell into a general agent workbench.
5. Harden indexing, recovery, and safety.
6. Finish packaging, rebrand, and docs.
