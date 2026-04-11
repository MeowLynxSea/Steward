# Native Memory Graph

`src/memory/` is Steward's long-term memory subsystem.

## Design

- `memory_spaces` isolate memory for an owner or agent persona.
- `memory_nodes` provide stable concept identities.
- `memory_versions` store immutable content history per node.
- `memory_edges` carry relation, priority, visibility, and trigger metadata.
- `memory_routes` expose human-friendly `domain://path` entry points and aliases.
- `memory_keywords` support recall enrichment and linking.
- `memory_search_docs` are derived search projections, not the memory source of truth.
- `memory_changesets` and `memory_changeset_rows` track AI-authored graph edits for review and rollback workflows.

## Runtime Role

- Agent prompt assembly should prefer `MemoryManager::build_prompt_context(...)`.
- Compaction and heartbeat should write episodic/procedural findings here instead of appending to workspace markdown files.
- Built-in graph-native tools (`search_memory`, `read_memory`, `create_memory`, `update_memory`, `add_alias`, `delete_memory`) should target this subsystem.

## Boundary With Workspace

`src/workspace/` still owns mounted files, workspace search, and file-context indexing.

Legacy files such as `MEMORY.md`, `HEARTBEAT.md`, `IDENTITY.md`, `USER.md`, and `daily/*.md` are migration inputs only. After import, the graph is the runtime truth, and imported routes are treated as ordinary memory nodes rather than fixed runtime entry points.
