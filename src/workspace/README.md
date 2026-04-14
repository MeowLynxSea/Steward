# Workspace & File Context System

Inspired by [OpenClaw](https://github.com/openclaw/openclaw), the workspace provides allowlisted-file context and indexing for agents.

## Current Scope

`src/workspace/` is now responsible for allowlisted files, workspace indexing, and file-context retrieval.

Steward's long-term agent memory no longer lives here as runtime truth. The native graph-based memory system now lives under `src/memory/`, backed by libSQL tables such as `memory_nodes`, `memory_versions`, and `memory_routes`.

Legacy file-based memory is no longer part of the active workspace architecture. The graph is the source of truth for long-term memory.

## Key Principles

1. **Workspace means allowlisted files** - `workspace://...` points at real allowlisted project content
   Public workspace ids are compact reversible short ids in URIs and tool/UI payloads; legacy UUID input is still accepted for compatibility
2. **Real filesystem is the working tree** - allowlisted reads, writes, moves, deletes, and shell commands operate on host files directly
3. **Git-backed trackers are the workspace truth** - every allowlist is backed by a Git tracker that owns dirty-path detection, anchors, diff, history, and restore
4. **Background allowlist watch is tracker-driven** - the watcher collects dirty paths and asks the tracker to advance product revisions without manifest rescans
5. **Agent memory is separate** - long-term memory lives in `src/memory/`, not here
6. **Hybrid search is discovery** - workspace search helps find allowlisted file context
7. **No workspace memory truth** - long-term memory does not live in workspace markdown files

## Workspace Shape

```
workspace/
├── workspace://allowlist-a/   <- Allowlisted project tree
│   ├── src/
│   ├── README.md
│   └── Cargo.toml
├── workspace://allowlist-b/   <- Another allowlisted tree
│   └── ...
└── ...
```

## Using the Workspace

```rust
use std::sync::Arc;
use crate::workspace::{Workspace, OpenAiEmbeddings, paths};

// Create workspace for a user (wraps embeddings in a default LRU cache)
let workspace = Workspace::new("user_123", pool)
    .with_embeddings(Arc::new(OpenAiEmbeddings::new(api_key)));

// For tests: skip the cache layer (avoids unnecessary overhead with mocks)
// let workspace = Workspace::new("user_123", pool)
//     .with_embeddings_uncached(Arc::new(MockEmbeddings::new(1536)));

// Read/write allowlisted files via workspace:// URIs
let doc = workspace.read("workspace://allowlist-a/README.md").await?;
workspace.write("workspace://allowlist-a/src/lib.rs", "pub fn run() {}").await?;

// List directory contents
let entries = workspace.list("projects/").await?;

// Search (hybrid FTS + vector)
let results = workspace.search("dark mode preference", 5).await?;

```

## Workspace Tools

Current LLM-facing workspace tools are allowlist-oriented:

- **`workspace_search`** - Search indexed allowlisted workspace content
- **`workspace_read`** - Read a allowlisted file via `workspace://...`
- **`workspace_write`** - Write a allowlisted file via `workspace://...`
- **`workspace_apply_patch`** - Patch a allowlisted file in place
- **`workspace_move`** - Rename or move a allowlisted file within a allowlist
- **`workspace_delete`** - Delete a allowlisted file
- **`workspace_delete_tree`** - Delete a allowlisted directory tree
- **`workspace_tree`** - Browse allowlisted workspace trees
- **`workspace_diff`** - Compare `baseline`, `head`, revisions, or checkpoints
- **`workspace_history`** - List automatic revisions and named checkpoints
- **`workspace_checkpoint_create` / `workspace_checkpoint_list`** - Create and inspect named restore points
- **`workspace_restore`** - Force real files back to a target revision/checkpoint/baseline
- **`workspace_baseline_set`** - Change the default diff reference without modifying disk
- **`workspace_refresh`** - Ask the tracker to re-sync the current allowlisted working tree

## Hybrid Search (RRF)

Combines full-text search and vector similarity using Reciprocal Rank Fusion:

```
score(d) = Σ 1/(k + rank(d)) for each method where d appears
```

Default k=60. Results from both methods are combined, with documents appearing in both getting boosted scores.

**Current backend:**
- **libSQL:** FTS5 for keyword search + vector search via `libsql_vector_idx` (dimension set dynamically by `ensure_vector_index()` during startup)

## Legacy Import Helpers

Some legacy workspace-document helpers still exist in the Rust module to support migration, seeding, profile sync, and hygiene routines. They are not the runtime truth for agent memory and should not be treated as the active memory architecture.

## Multi-Scope Reads & Identity Isolation

When a workspace has additional read scopes (via `with_additional_read_scopes`), read operations can span multiple user scopes — a user with scopes `["alice", "shared"]` can read documents from both.

Identity/config prompt files are exempt from multi-scope reads. When the workspace layer reads them for prompt construction or profile sync, it reads them from the **primary scope only** (`read_primary()`), never from secondary scopes:

| File | Read method | Rationale |
|------|------------|-----------|
| AGENTS.md | `read_primary()` | Agent instructions are per-user |
| SOUL.md | `read_primary()` | Core values are per-user |
| TOOLS.md | `read_primary()` | Tool config is per-user |
| BOOTSTRAP.md | `read_primary()` | Onboarding is per-user |

**Why:** Without this, a user with read access to another scope could silently inherit that scope's identity if their own copy is missing. The agent would present itself as the wrong user — a correctness and security issue.

**Design rule:** If you want shared identity across users, seed the same content into each user's scope at setup time. Don't rely on multi-scope fallback for identity files.

## Heartbeat Runtime

Proactive periodic execution is now routine/config driven. The active heartbeat
behavior comes from routine prompt/config stored in the runtime rather than file
appends or daily-log style workspace memory.

## Chunking Strategy

Documents are chunked for search indexing:
- Default: 800 words per chunk (roughly 800 tokens for English)
- 15% overlap between chunks for context preservation
- Minimum chunk size: 50 words (tiny trailing chunks merge with previous)
