# Database Module

libSQL-only persistence layer for the current Steward runtime.

## Quick Reference

```bash
cargo build
cargo check
cargo test
```

## Files

| File | Role |
|------|------|
| `mod.rs` | `Database` trait surface and backend connection helpers |
| `libsql/mod.rs` | libSQL/Turso backend struct, connection helpers, row parsing utilities |
| `libsql/conversations.rs` | `ConversationStore` impl |
| `libsql/jobs.rs` | `JobStore` impl |
| `libsql/local_jobs.rs` | `LocalJobStore` impl |
| `libsql/routines.rs` | `RoutineStore` impl |
| `libsql/settings.rs` | `SettingsStore` impl |
| `libsql/tool_failures.rs` | `ToolFailureStore` impl |
| `libsql/workspace.rs` | `WorkspaceStore` impl |
| `libsql_migrations.rs` | Consolidated libSQL schema and seed data |

## Current Rules

1. New persistence work targets libSQL only.
2. Keep schema and runtime contracts aligned with desktop-first behavior.
3. Use `LibSqlBackend::new_memory()` for isolated tests when possible.
4. Update `libsql_migrations.rs` when schema changes are required.

## Notes

- Timestamps are stored as text in RFC 3339 form.
- Booleans are stored as `INTEGER` (`0` / `1`).
- Vector search uses `libsql_vector_idx`.
- `json_patch` follows RFC 7396 semantics for JSON merge behavior.
