# IronCowork Post-Phase-1 Execution Roadmap

**Date:** 2026-03-31
**Status:** Active planning baseline
**Scope:** Detailed execution plan for work after Phase 1.5

---

## Current State Snapshot

As of 2026-03-31, the repository has completed the minimum structural work for Phase 0 and the first pass of Phase 1:

- libSQL-only local runtime is established as the storage baseline.
- `/api/v0/health`, settings, session, task, workspace, and SSE skeleton routes exist.
- Ask/Yolo runtime interception exists in first-pass form.
- `ui/` exists as a Svelte shell with settings, sessions, tasks, and workspace views.
- `src-tauri/` exists as a Tauri shell with notifications, tray, and folder-drop forwarding.

This means the project has moved past "architecture declaration" and into "product completion". The remaining work is no longer about shell creation. It is about turning the shell into a usable desktop workflow product.

---

## Delivery Principles For Remaining Work

- Do not reopen deleted product surfaces such as channels, gateway, or NEAR login flows.
- Keep HTTP/SSE as the only business contract between frontend and backend.
- Prefer shipping complete vertical slices over expanding shallow surface area.
- Treat task templates, approvals, and execution history as authoritative persisted objects.
- Every phase must end with one or more user-visible workflows that can be manually exercised end to end.

---

## Remaining Phase Structure

### Phase 1 Closeout

**Goal:** Convert the current shell from "compile-time complete" to "manually usable for local development".

### Phase 2 MVP Delivery

**Goal:** Ship two complete workflows:

- file organization / archive
- periodic briefing / report generation

### Phase 3 Stabilization

**Goal:** Make the MVP dependable enough for repeated local use.

### Phase 4 Packaging And Release Readiness

**Goal:** Make IronCowork distributable as a local desktop product with clear setup and upgrade paths.

---

## Phase 1 Closeout

### Exit Criteria

- Local browser mode can complete settings save, session send/receive, task list refresh, and workspace indexing manually.
- Desktop mode can complete the same flows, plus notifications and folder-drop indexing.
- SSE reconnect behavior is stable enough that page refresh does not leave orphaned UI state.
- The current API/UI naming drift around task mode switching is resolved.

### Issue 1.6: Manual Runtime Validation And Gap Fixes

**Purpose:** Close the gap between "builds" and "usable for developers".

**Work items**

- Boot the Axum service and verify browser access against `127.0.0.1`.
- Boot the Tauri shell against the same backend and verify the desktop path.
- Fix any broken assumptions in the current Svelte pages, especially optimistic session updates and stream re-subscription.
- Add an explicit capability flag or graceful fallback contract for browser mode when Tauri-only APIs are missing.
- Replace the temporary workspace-index placeholder with a clearer "stub" status if full ingestion is not yet present.

**Acceptance**

- Manual smoke script exists in docs.
- One clean run in browser mode and one clean run in Tauri mode are both documented.

### Issue 1.7: API Contract Cleanup

**Purpose:** Align the current implementation with the intended v0 contract before Phase 2 builds on it.

**Work items**

- Normalize task mode switching to one route shape.
- Ensure task stream events use stable event names and envelope fields.
- Add missing session detail route and task detail route shape checks.
- Add explicit error payloads for `404`, `409`, and `422` responses used by UI workflows.
- Decide whether approval endpoints accept `request_id`, `approval_id`, or both; remove ambiguity.

**Acceptance**

- A single API contract document reflects the actual route shapes.
- API integration tests cover the normalized behavior.

### Issue 1.8: Frontend State Hygiene

**Purpose:** Prevent Phase 2 work from collapsing under duplicated or ad hoc state handling.

**Work items**

- Extract Svelte view state into focused modules for settings, sessions, tasks, and workspace.
- Add a shared SSE event adapter that converts raw event envelopes into typed frontend events.
- Add loading, empty, and error states for all current views.
- Add a basic route model so direct links to sessions or tasks do not require full-app manual navigation.

**Acceptance**

- `App.svelte` is no longer a large all-in-one controller.
- Frontend build remains static-output compatible for Tauri and backend static serving.

### Phase 1 Test Gate

- `cargo test --test api_http_integration`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `npm run build`
- One manual smoke checklist for browser mode
- One manual smoke checklist for desktop mode

---

## Phase 2 MVP Delivery

### Exit Criteria

- Users can create, edit, inspect, run, and review task templates.
- File archive task works in both Ask and Yolo modes with explicit operation previews.
- Periodic briefing task can run on schedule and write a Markdown file to disk.
- Task detail UI shows step logs, approvals, mode, and final outputs.

### Dependency Order

1. Template data model and CRUD
2. Task execution persistence and detail view
3. File archive vertical slice
4. Briefing vertical slice
5. Scheduling and recurring execution
6. UI refinement for approval and history

### Issue 2.1: Template Persistence Model

**Purpose:** Introduce the core object that makes IronCowork task-driven instead of message-driven.

**Work items**

- Define template storage schema in libSQL.
- Distinguish built-in templates from user-authored templates.
- Persist parameter schema, display metadata, default mode, and output expectations.
- Add template CRUD routes and validation rules.
- Add frontend template list and template detail editor scaffolding.

**Acceptance**

- Built-in templates are read-only unless explicitly cloned.
- User templates can be created, edited, and deleted.
- Invalid template schemas return field-level validation errors.

### Issue 2.2: Task Instance Model And History

**Purpose:** Make task execution durable and inspectable.

**Work items**

- Persist task instances, execution steps, checkpoints, and final outputs.
- Add task detail API that returns timeline-ready data.
- Persist mode changes and approval decisions as part of task history.
- Expose task result metadata such as output paths, summary text, and failure reason.

**Acceptance**

- Refreshing the UI retains task history and current execution state.
- Completed and failed tasks are fully inspectable after process restart.

### Issue 2.3: File Archive Template Runtime

**Purpose:** Deliver the first real knowledge-work automation loop.

**Work items**

- Add directory scanning and classification pipeline.
- Generate proposed rename/move operations with confidence and category metadata.
- Persist preview operations before execution.
- Route all file mutations through the safe tool layer.
- Distinguish low-risk preview generation from high-risk file mutation checkpoints.

**Acceptance**

- Ask mode pauses before mutating files and displays a structured preview.
- Yolo mode runs the same operations without extra UI approval, subject to policy.
- Result state includes moved files, skipped files, and failure reasons.

### Issue 2.4: File Archive UX

**Purpose:** Make the first template understandable and reversible from the UI perspective.

**Work items**

- Add template parameter form for source directory, target root, naming strategy, and exclusions.
- Add operation preview table with old path, new path, action, and risk.
- Add task detail panel sections for progress, approval, and final summary.
- Add "run again with same parameters" action.

**Acceptance**

- A user can launch the archive workflow without typing raw JSON.
- Approval UI is driven by typed operations, not log text parsing.

### Issue 2.5: Periodic Briefing Template Runtime

**Purpose:** Deliver the second workflow that proves scheduled synthesis, not just file operations.

**Work items**

- Define a built-in template for recurring reports.
- Support source configuration for MCP-backed sources and local workspace notes.
- Add prompt assembly for summary generation with deterministic output sections.
- Write Markdown output to a target path through the safe file-writing tool path.
- Record generated report metadata and output path in task history.

**Acceptance**

- A configured report task can run once manually and produce a Markdown file.
- Ask mode pauses before network or external side effects when policy marks them risky.

### Issue 2.6: Scheduler And Recurring Runs

**Purpose:** Turn the briefing workflow into a real periodic automation feature.

**Work items**

- Define a schedule record model.
- Support cron validation and next-run calculation.
- Add a scheduler loop that instantiates tasks from templates on schedule.
- Ensure recurring runs produce separate task instances linked to the schedule.
- Add UI for enabling, disabling, and inspecting recurring schedules.

**Acceptance**

- A scheduled briefing creates independent historical task runs.
- Invalid cron expressions fail early with actionable errors.

### Issue 2.7: Approval UX And Mode Control

**Purpose:** Finish the Ask/Yolo user-facing loop.

**Work items**

- Add task-level mode switch with persistence and immediate UI feedback.
- Add approval modal or side panel with approve/reject flow.
- Add rejection reason capture and rejected-task rendering.
- Surface notification events when attention is required.

**Acceptance**

- A waiting task can be approved or rejected from the UI and resume or terminate deterministically.
- Mode changes are visible in history and stream updates.

### Issue 2.8: Template And Task E2E Coverage

**Purpose:** Freeze the MVP contract before stabilization work.

**Work items**

- Add end-to-end API tests for template CRUD and task creation.
- Add backend runtime tests for Ask vs Yolo behavior in file archive and briefing paths.
- Add UI integration tests for approval rendering and task timeline rendering.
- Add one reproducible fixture set for file-archive dry runs and report generation.

**Acceptance**

- MVP workflows can be exercised in automated tests without manual setup drift.

### Phase 2 Test Gate

- Template CRUD API tests
- Task creation/detail/history tests
- Ask/Yolo approval state machine tests
- File archive fixture test
- Report generation fixture test
- UI integration tests for settings, task detail, and approval flow

---

## Phase 3 Stabilization

### Exit Criteria

- Repeated local use does not lose task state, corrupt workspace data, or leave broken UI streams.
- Startup, shutdown, and restart behavior are deterministic.
- Safety and observability are sufficient to diagnose failures without attaching a debugger.

### Issue 3.1: Workspace Indexing Upgrade

**Purpose:** Replace the current placeholder indexing path with a real ingestion pipeline.

**Work items**

- Recursively walk selected directories.
- Persist file metadata, extracted text, and chunk records into libSQL.
- Rebuild hybrid retrieval around the actual stored corpus.
- Add progress reporting for long-running index jobs.

### Issue 3.2: Retrieval Quality And Search UX

**Purpose:** Make workspace search meaningful for briefing and future template workflows.

**Work items**

- Add FTS weighting and vector ranking tuning.
- Add search result snippets and source metadata.
- Add explicit re-index and stale-index handling.

### Issue 3.3: Settings Hardening

**Purpose:** Make provider setup dependable for real users.

**Work items**

- Add provider-specific validation and connectivity checks.
- Add secret storage policy for desktop mode versus browser-only mode.
- Add migration path for settings schema changes.

### Issue 3.4: Reliability And Recovery

**Purpose:** Make long-running tasks survivable across refreshes and restarts.

**Work items**

- Add task recovery semantics on process restart.
- Add stream replay or snapshot-plus-resubscribe behavior.
- Define restart behavior for waiting approvals and scheduled tasks.

### Issue 3.5: Observability

**Purpose:** Make the runtime inspectable without reopening the old gateway/operator surface.

**Work items**

- Add structured logs around task lifecycle, approvals, scheduler events, and tool execution boundaries.
- Add developer-facing diagnostics page or log export entry point.
- Add correlation IDs that tie REST, SSE, and runtime events together.

### Issue 3.6: Security Regression Pass

**Purpose:** Ensure the new product shape did not bypass the retained safety model.

**Work items**

- Audit all file and network side effects for tool-layer enforcement.
- Add tests for approval bypass attempts.
- Revalidate secret handling for settings and MCP credentials.

### Phase 3 Test Gate

- Restart recovery tests
- Workspace indexing and search regression tests
- Provider validation tests
- Safety regression tests for file/network mutation paths

---

## Phase 4 Packaging And Release Readiness

### Exit Criteria

- A new contributor can boot the project locally without hidden knowledge.
- A local user can install, configure, and use the app with a documented setup path.
- The repository no longer presents itself as a half-migrated fork.

### Issue 4.1: Repository Rebrand Completion

**Work items**

- Finish project naming updates across README, package metadata, binary names, and release assets.
- Remove stale upstream references that imply channels, PostgreSQL, or NEAR account dependency.

### Issue 4.2: Developer Bootstrap

**Work items**

- Add one-command local dev startup for backend plus UI and, where practical, Tauri.
- Document required environment variables and optional local config file paths.

### Issue 4.3: Desktop Packaging

**Work items**

- Define Tauri packaging targets for macOS, Windows, and Linux where supported.
- Replace placeholder icons and bundle metadata.
- Add release checklist for signing, packaging, and artifact verification.

### Issue 4.4: User Documentation

**Work items**

- Write setup docs for API keys, local storage path, browser mode, and desktop mode.
- Write usage docs for file archive and periodic briefing templates.
- Document explicit non-goals such as built-in remote exposure.

### Phase 4 Test Gate

- Clean local bootstrap from a fresh clone
- Package build checks for supported desktop targets
- README and user-doc walkthrough validation

---

## Critical Path Summary

The minimum path to a real MVP is:

1. Finish Phase 1 closeout so the shell is manually usable.
2. Build template persistence and task history.
3. Ship file archive as the first complete vertical slice.
4. Ship periodic briefing with scheduling.
5. Stabilize indexing, recovery, and observability.

Any work outside that path should be treated as optional until those five steps are complete.

---

## Explicit Non-Goals For Remaining Phases

- No revival of Telegram, Slack, web gateway, or REPL-first product surfaces.
- No in-app public exposure, tunnel setup, or LAN discovery.
- No direct shell-command execution path outside the approved tool/safety boundary.
- No heavy visual redesign before the two MVP workflows are dependable.
