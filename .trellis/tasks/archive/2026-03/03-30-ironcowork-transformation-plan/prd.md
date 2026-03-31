# IronCowork Desktop-First Transformation Plan

## Goal

Transform this repository from the current IronClaw multi-channel assistant into IronCowork: a desktop-first, local-first AI automation product for knowledge workers. The transformation must preserve the Rust agent engine, safety model, MCP/tool infrastructure, and workspace retrieval capabilities while removing product assumptions that tie the system to chat channels, NEAR AI account flows, PostgreSQL, and the existing web gateway.

## What I Already Know

- The current repository is still branded and structured as IronClaw.
- The current workspace still contains `channels-src/`, `deploy/`, web gateway code, channel-specific code under `src/channels/`, and PostgreSQL as part of the default feature set.
- `Cargo.toml` still exposes both `postgres` and `libsql`, with `default = ["postgres", "libsql", "html-to-markdown"]`.
- The current task system in `.trellis/` is bootstrapped but the backend spec files are still mostly templates.
- The user has already defined non-negotiable product, architecture, storage, networking, and UI decisions for the fork.

## Assumptions

- The requested output for this turn is a persisted implementation plan and code-spec, not production code changes.
- Documentation should live in English to match Trellis workspace rules.
- The plan should be grounded in the current codebase, but it may define target structures that do not yet exist.
- The fork is intentionally non-upstream-compatible at the product and API level.

## Requirements

- Capture the product repositioning from chat-centric assistant to task/template-centric desktop automation tool.
- Define the target architecture around Axum HTTP/SSE, Svelte UI, and Tauri as a native bridge only.
- Define the migration boundaries: what is retained, what is deleted, and what is newly introduced.
- Lock in libSQL as the only storage backend and remove PostgreSQL from the future architecture.
- Lock in `127.0.0.1` binding as the default network model.
- Document the Ask/Yolo runtime model and approval flow.
- Break the work into concrete phases and issues starting with storage purification.
- Persist the plan in project docs and persist execution-critical contracts in `.trellis/spec/`.

## Acceptance Criteria

- [ ] A project-level transformation plan exists under `docs/plans/`.
- [ ] The plan includes product context, target repo layout, migration phases, issue breakdown, risks, and acceptance checks.
- [ ] The plan explicitly records the removal of channels, gateway, Docker deployment, and NEAR AI login assumptions.
- [ ] A backend code-spec documents the new desktop-first runtime and HTTP/SSE/API boundaries.
- [ ] A backend code-spec documents Ask/Yolo task contracts with validation and testing expectations.
- [ ] `.trellis/spec/backend/index.md` is updated so future work can discover the new specs.

## Out of Scope

- Implementing the fork rename across source code, package metadata, and release assets.
- Replacing PostgreSQL code in this turn.
- Adding the Axum API crate, Svelte UI, or Tauri shell in this turn.
- Writing production migrations or integration tests in this turn.

## Research Notes

### Current Repository Constraints

- Top-level source still uses a monolithic `src/` tree with channel, webhook, setup, workspace, tool, and orchestrator modules.
- `src/channels/` contains relay, wasm, and web channel concerns that do not fit the target desktop-first product.
- Existing docs and code still reference the web gateway as a first-class product surface.
- Existing docs and code still reference NEAR AI onboarding and PostgreSQL bootstrap flow.

### Migration Implications

- The transformation is architectural, not cosmetic. A compatibility layer that preserves the old chat/channel worldview will create long-term drag.
- The first durable contract must be the target runtime boundary: backend via HTTP/SSE, frontend via Svelte, desktop bridge via Tauri, and task execution via Ask/Yolo.
- The code-spec must describe the future write shape even before code exists, otherwise follow-up implementation tasks will drift toward the old structure.

## Technical Notes

- Current active task: `.trellis/tasks/00-bootstrap-guidelines/` remains separate; this planning task should not overwrite bootstrap setup history.
- Primary files inspected for this PRD:
  - `Cargo.toml`
  - `README.md`
  - `src/`
  - `.trellis/workflow.md`
  - `.trellis/spec/backend/index.md`
  - `.trellis/spec/guides/cross-layer-thinking-guide.md`
