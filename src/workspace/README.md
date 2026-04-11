# Workspace & File Context System

Inspired by [OpenClaw](https://github.com/openclaw/openclaw), the workspace provides mounted-file context and indexing for agents.

## Current Scope

`src/workspace/` is now responsible for mounted files, workspace indexing, and file-context retrieval.

Steward's long-term agent memory no longer lives here as runtime truth. The native graph-based memory system now lives under `src/memory/`, backed by libSQL tables such as `memory_nodes`, `memory_versions`, and `memory_routes`.

Legacy files like `MEMORY.md`, `HEARTBEAT.md`, `IDENTITY.md`, `USER.md`, and `daily/*.md` are imported once into the graph during migration. After that, the graph is the source of truth and the workspace copy is only legacy content.

## Key Principles

1. **Workspace means mounted files** - `workspace://...` points at real mounted project content
2. **Agent memory is separate** - long-term memory lives in `src/memory/`, not here
3. **Hybrid search is discovery** - workspace search helps find mounted file context
4. **No workspace memory truth** - legacy markdown memory files are migration inputs only

## Workspace Shape

```
workspace/
├── workspace://mount-a/   <- Mounted project tree
│   ├── src/
│   ├── README.md
│   └── Cargo.toml
├── workspace://mount-b/   <- Another mounted tree
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

// Read/write mounted files via workspace:// URIs
let doc = workspace.read("workspace://mount-a/README.md").await?;
workspace.write("workspace://mount-a/src/lib.rs", "pub fn run() {}").await?;

// List directory contents
let entries = workspace.list("projects/").await?;

// Search (hybrid FTS + vector)
let results = workspace.search("dark mode preference", 5).await?;

```

## Workspace Tools

Current LLM-facing workspace tools are mount-oriented:

- **`workspace_search`** - Search indexed mounted workspace content
- **`workspace_read`** - Read a mounted file via `workspace://...`
- **`workspace_write`** - Write a mounted file via `workspace://...`
- **`workspace_tree`** - Browse mounted workspace trees

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

**Legacy identity-file import helpers are exempt from multi-scope reads.** When the workspace layer reads identity/config documents during migration or profile sync, it reads them from the **primary scope only** (`read_primary()`), never from secondary scopes:

| File | Read method | Rationale |
|------|------------|-----------|
| AGENTS.md | `read_primary()` | Agent instructions are per-user |
| SOUL.md | `read_primary()` | Core values are per-user |
| USER.md | `read_primary()` | Legacy user-context imports remain per-user |
| IDENTITY.md | `read_primary()` | Legacy identity imports remain per-user |
| TOOLS.md | `read_primary()` | Tool config is per-user |
| BOOTSTRAP.md | `read_primary()` | Onboarding is per-user |
| MEMORY.md | `read()` | Legacy import content, not runtime truth |
| daily/*.md | `read()` | Legacy episodic import content |

**Why:** Without this, a user with read access to another scope could silently inherit that scope's identity if their own copy is missing. The agent would present itself as the wrong user — a correctness and security issue.

**Design rule:** If you want shared identity across users, seed the same content into each user's scope at setup time. Don't rely on multi-scope fallback for identity files.

## Heartbeat Runtime

Proactive periodic execution is now routine/config driven. It no longer depends
on a fixed graph route or on reading a workspace `HEARTBEAT.md` file as runtime
truth. Legacy heartbeat files can still exist as workspace documents, but the
active heartbeat behavior comes from routine prompt/config stored in the runtime.

## Chunking Strategy

Documents are chunked for search indexing:
- Default: 800 words per chunk (roughly 800 tokens for English)
- 15% overlap between chunks for context preservation
- Minimum chunk size: 50 words (tiny trailing chunks merge with previous)
