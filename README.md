<p align="center">
  <img src="steward.png?v=2" alt="Steward" width="200"/>
</p>

<h1 align="center">Steward</h1>

<p align="center">
  <strong>Desktop-first autonomous AI coworker for local knowledge work</strong>
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
</p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a> |
  <a href="README.ru.md">Русский</a> |
  <a href="README.ja.md">日本語</a>
</p>

<p align="center">
  <a href="#positioning">Positioning</a> •
  <a href="#principles">Principles</a> •
  <a href="#current-direction">Current Direction</a> •
  <a href="#user-docs">User Docs</a> •
  <a href="#developer-bootstrap">Developer Bootstrap</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a>
</p>

---

## Positioning

Steward is not a GUI wrapper around a coding CLI, and it is not being built as a predefined-workflow product.

The target experience is closer to a desktop-native autonomous agent for knowledge work:

- you give the agent a goal in a persistent desktop session
- the agent explores files, notes, and MCP-connected sources
- it plans and executes multi-step work
- Ask/Yolo controls decide whether risky side effects require approval
- the shared runtime can power the local desktop shell and optional extension ingress

Saved routines may exist later as accelerators, but they are not the product center. The primary interaction model is an ongoing agent session with durable context, tool use, and background execution.

## Principles

- **Desktop-first**: the product should feel native on macOS, Windows, and Linux through Tauri, with Tauri IPC and native events as the primary desktop transport.
- **Local-first**: libSQL is the embedded storage baseline; no PostgreSQL, no required cloud account, no mandatory external services.
- **Autonomous but reviewable**: the agent should act independently, but Ask/Yolo and event logs keep risky actions inspectable.
- **Workspace-centric**: local files, indexed notes, reports, and external MCP tools are agent context, not an afterthought.
- **Fork, not skin**: this project keeps useful Rust runtime and safety pieces from the original codebase but deliberately diverges from its old channel-first product model.

## Current Direction

This repository is now aligned around the Steward product direction and is finishing packaging and end-user polish.

What stays:

- Rust agent loop and orchestration
- WASM sandbox and prompt-safety controls
- MCP/tool registry support
- workspace indexing and hybrid retrieval
- multi-provider LLM support

What is being removed or demoted:

- channel-first product assumptions and named chat gateways
- NEAR-account-oriented onboarding
- PostgreSQL assumptions
- docs that describe Steward as a predefined-workflow runner

What replaces the old center of gravity:

- persistent agent sessions
- session-owned threads as the core conversational unit
- delegated runs/tasks as secondary execution records
- Ask/Yolo approval checkpoints
- optional saved routines for recurring background work
- Svelte UI over Tauri IPC and native event delivery

## Features

### Runtime

- **Autonomous agent sessions** for multi-step desktop work
- **Thread-driven chat execution** inside each session
- **Ask/Yolo execution control** for risky file or network side effects
- **Background routines** for recurring jobs once the core agent loop is stable
- **libSQL storage** for settings, sessions, runs, approvals, and workspace state
- **Workspace retrieval** with full-text and vector search
- **Filesystem-backed SKILL.md skills** from `~/.steward/skills`, auto-mounted into Workspace as `workspace://skills` for browsing and preview

### Safety

- **WASM sandbox** for untrusted tools
- **Credential boundary protection** with injection and leak scanning
- **Prompt-injection defenses** for external content
- **Endpoint allowlisting** for networked tool access
- **Audit-friendly event streams** across session, run, and approval lifecycles

### Extensibility

- **Desktop MCP panel and runtime** for server management, auth, custom headers/env/OAuth config, tools, resources, prompts, roots, activity, sampling, and elicitation
- **MCP protocol support** for external capability providers, including bidirectional sessions and server-originated requests
- **Plugin/tool architecture** for new local capabilities
- **Multiple LLM backends** through direct provider adapters or OpenAI-compatible APIs

## User Docs

End-user setup and usage docs live here:

- [docs/user-guide.md](docs/user-guide.md) for installation, desktop usage, provider setup, storage paths, sessions, approvals, workspace usage, and optional WASM channel ingress
- [docs/release-readiness.md](docs/release-readiness.md) for supported packaging targets, build commands, and release verification

## Developer Bootstrap

Fresh-clone developer setup is documented in [docs/developer-bootstrap.md](docs/developer-bootstrap.md).

Shortest path:

```bash
./scripts/dev-setup.sh
```

That bootstrap prepares Rust + WASM prerequisites, installs UI dependencies, builds the static frontend bundle, and installs git hooks.

Daily development flow:

- Desktop mode: run `cargo desktop`

## Configuration

The local bootstrap path is desktop-first. Database bootstrap still uses `.env`
for infrastructure concerns, but model-provider configuration now happens only
through the app's onboarding flow and Settings page.

```env
DATABASE_BACKEND=libsql
LIBSQL_PATH=~/.steward/steward.db
```

No NEAR login or PostgreSQL bootstrap should be required for the target product.

## Security

Steward keeps the original defense-in-depth posture and applies it to desktop automation:

- risky side effects must go through the retained tool/safety boundary
- Ask mode can suspend execution for approval before mutation
- Yolo mode still runs inside the same policy and sandbox constraints
- secrets remain outside tool-visible execution environments
- local-first storage does not imply unrestricted shell access

## Architecture

```
+------------------------+      Tauri IPC     +------------------------+
|  Svelte UI             | <----------------> |  Tauri commands        |
|  - sessions            |                    |  settings/sessions     |
|  - threads             |                    |  tasks/workspace       |
|  - approvals           |                    |  desktop transport     |
+-----------+------------+                    +-----------+------------+
            |                                             |
            | Tauri events                                |
            v                                             v
+------------------------+                    +------------------------+
|  Native bridge         |                    |  Rust runtime          |
|  notifications         |                    |  agent loop            |
|  tray                  |                    |  threads + tasks       |
|  drag-and-drop         |                    |  tools + MCP           |
+------------------------+                    |  safety + storage      |
                                              +-----------+------------+
                                                          |
                                                          v
                                               +------------------------+
                                               |  libSQL                |
                                               |  local embedded DB     |
                                               +------------------------+
```

## Status

The documentation is being updated to match the corrected product direction:

- desktop-native autonomous agent first
- sessions/threads first
- routines second
- no predefined workflow system at the center of the product

## License

Licensed under either of

- Apache License, Version 2.0
- MIT license
