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
- Automatic recall uses **progressive disclosure**: only boot nodes are loaded with full content; triggered, relevant, and recent memories are surfaced as a directory (URI + priority + disclosure). The agent must call `read_memory` to load content on demand, matching human-like recall.
- Compaction and heartbeat should write episodic/procedural findings here instead of appending to workspace markdown files.
- Built-in graph-native tools (`search_memory`, `read_memory`, `create_memory`, `update_memory`, `add_alias`, `delete_memory`) should target this subsystem.

## Boundary With Workspace

`src/workspace/` still owns allowlisted files, workspace search, and file-context indexing.

Legacy workspace files are no longer part of the runtime memory contract. The graph is the source of truth, and any imported content is treated as ordinary memory nodes rather than fixed runtime entry points.
