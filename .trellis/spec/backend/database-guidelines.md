# Database Guidelines

> Database patterns and conventions for the local-first IronCowork runtime.

---

## Overview

The backend uses libSQL as the local embedded database baseline.

Current repository facts:

- `Cargo.toml` keeps `libsql` in the default feature set.
- storage-facing code already assumes local-first operation rather than mandatory external services.
- migration work must move the codebase further toward libSQL-only, not back toward mixed PostgreSQL support.

Examples:

- dependency baseline: [Cargo.toml](/Users/MeowLynxSea/Development/IronCowork/Cargo.toml)
- bootstrap env handling: [src/bootstrap.rs](/Users/MeowLynxSea/Development/IronCowork/src/bootstrap.rs)
- API integration over a temp libSQL database: [tests/api_http_integration.rs](/Users/MeowLynxSea/Development/IronCowork/tests/api_http_integration.rs)

---

## Query Patterns

- Keep persistence behind backend store/runtime modules. Route handlers should call domain services or store abstractions, not assemble ad hoc SQL everywhere.
- Prefer explicit typed request/response structs at the API boundary and convert to persistence shapes inside backend modules.
- Write tests against temporary local databases when changing persistence behavior.
- Treat task state, settings, sessions, and workspace data as persistent state, not ephemeral in-memory caches.

### Good

- Use a temp database per integration test and exercise the real API flow.
- Persist settings and then reload them through the same store contract the app uses in production.

### Bad

- Reintroducing branching logic that supports both PostgreSQL and libSQL "just in case".
- Reading configuration from environment on every request instead of persisting and loading through the settings/database path.

---

## Migrations

- Migration files belong under `migrations/`.
- New migration work must remain libSQL-compatible.
- Do not add PostgreSQL-only DDL, extension requirements, or test setup back into the repository.
- When a schema change affects task/runtime/API behavior, update the Trellis backend specs in the same change.

Required workflow:

1. Add or update migration files.
2. Verify app boot on a clean local database.
3. Add regression coverage for the changed behavior.

---

## Naming Conventions

- Use stable, descriptive names for persisted domain entities: settings, sessions, tasks, task_steps, template records, schedule records, workspace documents.
- Column names should stay snake_case and map cleanly to Rust struct fields.
- Avoid names that preserve old product assumptions such as `channel_*`, `gateway_*`, or provider-specific login tables unless they are still genuinely needed.

---

## Common Mistakes

- Treating libSQL as a drop-in compatibility layer for old PostgreSQL design instead of simplifying around the embedded-database model.
- Adding persistence through one-off files when the runtime already has a database-backed settings/task model.
- Changing API contracts without verifying persisted state survives restart and reload.
