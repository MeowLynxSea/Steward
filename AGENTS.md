# Agent Rules

## Purpose and Precedence

- This file is the quick-start contract for coding agents working in this repository.
- It is intentionally short. Read the module docs before changing a complex subsystem.
- When docs disagree, prefer:
  1. the module doc closest to the code you are changing
  2. the current product direction in `README.md`
  3. the implementation in the codebase
- The root `AGENTS.md` is for repository contributors and coding agents. The seeded workspace file at `src/workspace/seeds/AGENTS.md` is for end-user workspace behavior and should not be treated as the contributor guide.

Start with these deeper docs as needed:

- `README.md`
- `CLAUDE.md`
- `CONTRIBUTING.md`
- `docs/developer-bootstrap.md`
- `src/agent/CLAUDE.md`
- `src/db/CLAUDE.md`
- `src/tools/README.md`
- `src/workspace/README.md`

## Current Product Direction

- Steward is a desktop-first autonomous AI coworker for local knowledge work.
- The primary product surface is the Tauri desktop app with Svelte UI, Tauri IPC, and native events.
- The shared Rust runtime is the source of truth for business behavior. Do not split core logic across duplicate desktop-only and browser-only implementations.
- Persistent sessions, threads, tasks, approvals, and workspace state are the product center of gravity.
- Ask/Yolo approval flow is a runtime policy boundary, not just UI chrome.
- The current persistence model is `libSQL`-first and local-first. If you find older references to PostgreSQL or dual-backend requirements, treat them as historical unless the code you are changing still explicitly depends on them.

## Architecture Mental Model

- `ui/` contains the Svelte frontend.
- `src/main.rs`, `src/desktop_runtime.rs`, and `src/tauri_commands.rs` bridge the desktop shell to the shared runtime.
- `src/agent/` owns the conversational loop, thread state, approvals, scheduling, and background execution behavior.
- `src/db/` owns persistence and migrations.
- `src/workspace/` owns memory files, indexing, search, and identity-file loading.
- `src/tools/` owns built-in tools, WASM tooling, and MCP integration.
- `src/extensions/` and `src/channels/wasm/` support extensibility, but desktop sessions remain the primary product path.

Keep `src/main.rs` and other entrypoints orchestration-focused. Module-owned logic should live behind helpers or factories in the owning module.

## Where to Work

- Agent loop, approvals, tasks, session behavior: `src/agent/`
- Desktop IPC and app shell integration: `src/main.rs`, `src/desktop_runtime.rs`, `src/tauri_commands.rs`
- UI behavior and screens: `ui/`
- Persistence and schema work: `src/db/`
- Workspace memory, indexing, search, mounts: `src/workspace/`
- Tooling, MCP, WASM tools: `src/tools/`

If you touch a subsystem with its own doc, read that doc first.

## Development Workflow

Bootstrap:

```bash
./scripts/dev-setup.sh
```

Primary desktop development flow:

```bash
npm --prefix ui run build -- --watch
cargo tauri dev --config tauri.conf.json
```

Common validation:

```bash
cargo test
npm --prefix ui run build
```

Stricter pre-review checks:

```bash
cargo fmt --all -- --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test
```

Use the narrowest additional test that covers the change. If you touch a high-risk path, say exactly what you ran and what you did not run.

## Repo-Wide Coding Rules

- Avoid `.unwrap()` and `.expect()` in production code. If an invariant is truly infallible, document why.
- Keep clippy clean.
- Prefer `crate::` imports for cross-module references.
- Prefer strong types and enums over stringly typed control flow.
- Keep functions focused and module boundaries clear.
- Add comments only for non-obvious logic.
- Do not move module-owned initialization into entrypoints just because it is convenient.

## Runtime and Safety Invariants

- Preserve Ask/Yolo approval enforcement for risky side effects.
- Do not bypass safety checks, secret handling, or sandbox boundaries when adding tools or execution paths.
- Keep Tauri IPC wrappers thin; business rules belong in the shared runtime.
- Preserve workspace identity-file semantics. `AGENTS.md`, `SOUL.md`, `TOOLS.md`, and `BOOTSTRAP.md` are primary-scope files, not shared fallback content.
- Treat external services, tool output, and imported content as untrusted input.

## Data and Persistence Rules

- New persistence work should follow the current `libSQL`-only database layer in `src/db/`.
- Keep schema, migrations, and runtime contracts aligned.
- When changing stored data shape or behavior, update the owning migration or persistence layer in the same branch.
- Use the lightweight in-memory or temp-db test helpers when possible for persistence tests.

## Tools and Extensibility

- Prefer built-in Rust tools for core runtime capabilities tightly coupled to Steward internals.
- Prefer WASM tools for sandboxed extensions and reusable integrations.
- Prefer MCP when the capability belongs in an external server or ecosystem integration.
- Keep service-specific auth flows and API quirks out of the core agent loop when they can live in tool metadata or the extension boundary.

## Docs, Parity, and Change Discipline

- Keep changes scoped. Avoid broad refactors unless the task genuinely requires them.
- If behavior changes, update the relevant docs in the same branch.
- If a tracked capability changes, update `FEATURE_PARITY.md` in the same branch.
- If setup, onboarding, or day-to-day developer commands change, update the corresponding docs too.
- Call out conflicts you find between older docs and current code instead of silently normalizing them.
- Respect a dirty worktree. Do not revert unrelated user changes.

## Before Finishing

- Re-read the diff for scope creep.
- Run the most targeted checks that meaningfully cover the change.
- Confirm whether `README.md`, `CONTRIBUTING.md`, module docs, or `FEATURE_PARITY.md` also need updates.
- Note any unrun checks, residual risk, or doc drift in your handoff.
