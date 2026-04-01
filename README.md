<p align="center">
  <img src="ironcowork.png?v=2" alt="IronCowork" width="200"/>
</p>

<h1 align="center">IronCowork</h1>

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
  <a href="#developer-bootstrap">Developer Bootstrap</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a>
</p>

---

## Positioning

IronCowork is not a GUI wrapper around a coding CLI, and it is not being built as a predefined-workflow product.

The target experience is closer to a desktop-native autonomous agent for knowledge work:

- you give the agent a goal in a persistent desktop session
- the agent explores files, notes, and MCP-connected sources
- it plans and executes multi-step work
- Ask/Yolo controls decide whether risky side effects require approval
- the same backend can run inside a local desktop shell or in a browser against `127.0.0.1`

Saved routines may exist later as accelerators, but they are not the product center. The primary interaction model is an ongoing agent session with durable context, tool use, and background execution.

## Principles

- **Desktop-first**: the product should feel native on macOS, Windows, and Linux through Tauri, without coupling business logic to Tauri IPC.
- **Local-first**: libSQL is the embedded storage baseline; no PostgreSQL, no required cloud account, no mandatory external services.
- **Autonomous but reviewable**: the agent should act independently, but Ask/Yolo and event logs keep risky actions inspectable.
- **Workspace-centric**: local files, indexed notes, reports, and external MCP tools are agent context, not an afterthought.
- **Fork, not skin**: this project keeps useful Rust runtime/safety pieces from IronCowork but deliberately diverges from its old channel-first product model.

## Current Direction

This repository is mid-migration from IronCowork to IronCowork.

What stays:

- Rust agent loop and orchestration
- WASM sandbox and prompt-safety controls
- MCP/tool registry support
- workspace indexing and hybrid retrieval
- multi-provider LLM support

What is being removed or demoted:

- channel-first interaction surfaces
- NEAR-account-oriented onboarding
- PostgreSQL assumptions
- docs that describe IronCowork as a predefined-workflow runner

What replaces the old center of gravity:

- persistent agent sessions
- delegated runs/tasks as execution records
- Ask/Yolo approval checkpoints
- optional saved routines for recurring background work
- Svelte UI over HTTP/SSE, optionally hosted inside Tauri

## Features

### Runtime

- **Autonomous agent sessions** for multi-step desktop work
- **Ask/Yolo execution control** for risky file or network side effects
- **Background routines** for recurring jobs once the core agent loop is stable
- **libSQL storage** for settings, sessions, runs, approvals, and workspace state
- **Workspace retrieval** with full-text and vector search

### Safety

- **WASM sandbox** for untrusted tools
- **Credential boundary protection** with injection and leak scanning
- **Prompt-injection defenses** for external content
- **Endpoint allowlisting** for networked tool access
- **Audit-friendly event streams** across session, run, and approval lifecycles

### Extensibility

- **MCP protocol support** for external capability providers
- **Plugin/tool architecture** for new local capabilities
- **Multiple LLM backends** through direct provider adapters or OpenAI-compatible APIs

## Developer Bootstrap

Fresh-clone developer setup is documented in [docs/developer-bootstrap.md](docs/developer-bootstrap.md).

Shortest path:

```bash
./scripts/dev-setup.sh
```

That bootstrap prepares Rust + WASM prerequisites, installs UI dependencies, builds the static frontend bundle, and installs git hooks.

Daily development flows:

- Browser mode: `cargo run -- api serve --port 8765`, then open `http://127.0.0.1:8765`
- Desktop mode: run `npm --prefix ui run build -- --watch`, `cargo run -- api serve --port 8765`, then `cargo tauri dev --config src-tauri/tauri.conf.json`

## Configuration

The local bootstrap path is config-file and env-var driven:

```env
DATABASE_BACKEND=libsql
LIBSQL_PATH=~/.ironcowork/ironcowork.db
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

No NEAR login or PostgreSQL bootstrap should be required for the target product.

See [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md) for provider details.

## Security

IronCowork keeps the original defense-in-depth posture and applies it to desktop automation:

- risky side effects must go through the retained tool/safety boundary
- Ask mode can suspend execution for approval before mutation
- Yolo mode still runs inside the same policy and sandbox constraints
- secrets remain outside tool-visible execution environments
- local-first storage does not imply unrestricted shell access

## Architecture

```
+------------------------+      HTTP/SSE      +------------------------+
|  Svelte UI             | <----------------> |  Axum API              |
|  - sessions            |                    |  127.0.0.1 by default  |
|  - runs                |                    |  settings/sessions     |
|  - approvals           |                    |  tasks/workspace       |
+-----------+------------+                    +-----------+------------+
            |                                             |
            | optional Tauri shell                        |
            v                                             v
+------------------------+                    +------------------------+
|  Native bridge         |                    |  Rust runtime          |
|  notifications         |                    |  agent loop            |
|  tray                  |                    |  tools + MCP           |
|  drag-and-drop         |                    |  safety + storage      |
+------------------------+                    +------------------------+
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
- sessions/runs first
- routines second
- no predefined workflow system at the center of the product

## License

Licensed under either of

- Apache License, Version 2.0
- MIT license
