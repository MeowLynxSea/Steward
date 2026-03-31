# Phase 3 Workspace Indexing And Retrieval

## Goal

Replace the current placeholder indexing path with a real ingestion and hybrid-retrieval baseline for repeated local use.

## Scope

- Recursive filesystem ingestion
- Text extraction/chunk persistence
- Hybrid search tuning
- Search result metadata and snippets
- Index freshness and progress reporting

## Requirements

- Walk selected directories recursively and persist file metadata plus extracted text.
- Rebuild retrieval around real stored corpus data.
- Provide progress feedback for large indexing jobs.
- Support re-index and stale-index handling explicitly.

## Acceptance Criteria

- [ ] Indexed workspace content is persisted, searchable, and inspectable.
- [ ] Search results include path/source metadata and snippets.
- [ ] Long-running index jobs surface progress.
- [ ] Retrieval changes are covered by regression tests.

## Out Of Scope

- Visual search redesign beyond functional clarity
