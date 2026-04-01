# IronCowork Desktop-First Autonomous Agent Transformation

**Date:** 2026-03-30  
**Status:** Product-direction baseline  
**Goal:** Turn the IronCowork fork into IronCowork, a desktop-first autonomous agent for local knowledge work.

---

## Product Direction

### Why The Previous Direction Was Wrong

The fork should not become a predefined-workflow product.

That model pushes the system toward predefined flows, parameter forms, and narrow vertical workflows. It underuses the strongest part of the inherited runtime: a general agent loop that can reason, inspect context, decide next actions, and execute across multiple steps.

The corrected target is:

- persistent session-based interaction
- user states goals through conversation
- the agent autonomously plans and executes
- `task` or `run` records exist as execution artifacts, not as the product center
- Ask/Yolo governs risky side effects
- desktop is the default operating environment

### Product Repositioning

| Dimension | Legacy / Wrong Direction | Correct IronCowork Direction |
|-----------|--------------------------|------------------------------|
| Primary interaction | Template/task instantiation | Persistent chat session with autonomous execution |
| Core object | Template | Session |
| Background unit | Template-derived task | Session-created run/task record |
| Product focus | Special workflows | General-purpose desktop coworker |
| Frontend | Web UI for task forms | Web UI for session, run, approval, and workspace visibility |
| Risk control | Approval inside predefined workflow flow | Approval inside general agent runtime |
| Storage | Mixed assumptions | libSQL only |

### Non-Negotiable Decisions

- IronCowork is not optimized for upstream mergeability.
- The default bind address remains `127.0.0.1`.
- Tauri is a native shell only, not a second backend.
- libSQL is the only supported storage backend.
- No built-in tunnel, LAN exposure, or cloud account requirement.
- High-risk actions must pass through Ask/Yolo and the retained safety layer.

---

## Current Baseline

Useful assets already present in the repo:

- Rust agent loop and orchestration
- WASM sandbox and secret-safety primitives
- MCP and tool registry support
- Workspace indexing and retrieval code
- Svelte shell and Tauri shell skeletons
- libSQL-first migration work

Incorrect assumptions still reflected in docs and plans:

- IronCowork described as predefined-workflow-driven
- recurring automation and special workflows treated as first-class MVP center
- API contracts written around predefined workflow CRUD

This transformation corrects those assumptions without throwing away the core runtime work already done.

---

## Target Architecture

### Runtime Layout

```
+------------------------+      HTTP/SSE      +------------------------+
|  Svelte UI             | <----------------> |  ironcowork-api        |
|  - sessions            |                    |  Axum on 127.0.0.1     |
|  - runs                |                    |  settings/sessions     |
|  - approvals           |                    |  runs/workspace        |
|  - workspace           |                    +-----------+------------+
+-----------+------------+                                |
            |                                             |
            | optional desktop shell                      |
            v                                             v
+------------------------+                    +------------------------+
|  src-tauri             |                    |  runtime crates        |
|  notifications         |                    |  agent loop            |
|  tray                  |                    |  tools / MCP / safety  |
|  drag-and-drop         |                    |  workspace / storage   |
+------------------------+                    +------------------------+
                                                         |
                                                         v
                                              +------------------------+
                                              |  libSQL                |
                                              |  local embedded DB     |
                                              +------------------------+
```

### Architectural Boundaries

- `src-tauri/` only adds native desktop affordances.
- All business state flows through Axum over HTTP/SSE.
- `session` is the main user-facing object.
- `run` or `task` is a persisted execution record created by a session or by a background routine.
- Saved routines are optional later-stage automation helpers, not the center of the app.
- Risky actions must flow through the tool/safety layer, never through ad hoc shell shortcuts.

---

## Execution Model

### 1. Session-First, Not Template-First

The normal user path is:

1. Open or create a session
2. State a goal in natural language
3. Watch the agent inspect context, reason, and act
4. Approve or reject risky actions when running in Ask mode
5. Review resulting run history, files, and outputs

### 2. Runs As Durable Execution Artifacts

The system still needs durable background units, but they are secondary objects.

- a session message may spawn one or more runs
- each run records mode, status, steps, approvals, outputs, and errors
- the UI must make runs inspectable without forcing the user to think in predefined workflows

### 3. Ask/Yolo Is Runtime Policy, Not UI Sugar

Ask/Yolo must be enforced before risky tool effects commit.

The API and storage need first-class support for:

- current mode
- pending approval payload
- approval decision history
- mid-run mode switching
- resumable run state after restart

---

## MVP Validation

Phase 2 should prove a general desktop autonomous agent, not two specialized predefined workflows.

The MVP target is:

- a user can create a session and give a broad desktop knowledge-work goal
- the agent can inspect workspace files and indexed content
- the agent can use tools and MCP capabilities to progress the task
- the user can supervise through Ask/Yolo
- the result remains visible through session and run history

Good example goals:

- "整理我这个文件夹，并说明你准备怎么处理"
- "总结这个项目目录和相关笔记，给我一个 Markdown 结论"
- "检查这批本地资料，列出下一步建议"

Bad MVP framing:

- forcing every action through built-in workflow forms
- specializing early around one or two narrow automations
- requiring users to model their goal as a predefined workflow before the agent can help

---

## Phase Plan

### Phase 0: Core Purification

Goal: libSQL-only local runtime, no channel/product baggage.

### Phase 1: Shell And Contract Baseline

Goal: Axum + Svelte + Tauri shell with sessions, runs, workspace, and Ask/Yolo wiring.

### Phase 2: Autonomous Agent Core

Goal: a usable general-purpose desktop agent in persistent sessions.

Core work:

- session-first API and UI refinement
- run history and approval center
- general workspace/tool orchestration
- agent visibility: plan, steps, current action, pending approvals

### Phase 3: Stability And Background Operation

Goal: reliable restarts, indexing robustness, routine support, and observability.

Core work:

- runtime recovery
- durable background routines
- workspace indexing quality
- logs, audits, and safety regressions

### Phase 4: Packaging And User Readiness

Goal: real branding, packaging, setup docs, and a coherent user/developer story.

---

## API Direction

The contract center should move away from predefined workflow CRUD and toward:

- `/api/v0/sessions`
- `/api/v0/sessions/:id/messages`
- `/api/v0/sessions/:id/stream`
- `/api/v0/runs`
- `/api/v0/runs/:id`
- `/api/v0/runs/:id/stream`
- `/api/v0/runs/:id/approve`
- `/api/v0/runs/:id/reject`
- `/api/v0/runs/:id/mode`
- `/api/v0/workspace/*`

If presets or routines are added later, they should sit on top of the session/run model instead of replacing it.

---

## Red Lines

- Do not reintroduce predefined-workflow-first product thinking.
- Do not grow special-case UI flows for narrow workflows before the general agent loop feels right.
- Do not bypass the safety boundary for convenience.
- Do not let Tauri IPC become an alternate business API.
