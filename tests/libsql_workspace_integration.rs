#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw::db::{Database, libsql::LibSqlBackend};
use ironclaw::workspace::{MockEmbeddings, SearchConfig, Workspace};

#[tokio::test]
async fn libsql_hybrid_search_uses_fts_and_vector_index() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let db_path = dir.path().join("workspace.db");

    let backend = LibSqlBackend::new_local(&db_path)
        .await
        .expect("failed to create libsql backend");
    backend
        .run_migrations()
        .await
        .expect("failed to run migrations");
    backend
        .ensure_vector_index(64)
        .await
        .expect("failed to create vector index");

    let db: Arc<dyn ironclaw::db::Database> = Arc::new(backend);
    let workspace = Workspace::new_with_db("test-libsql", Arc::clone(&db))
        .with_embeddings_uncached(Arc::new(MockEmbeddings::new(64)));

    workspace
        .write(
            "docs/auth.md",
            "JWT authentication uses signed access tokens and refresh flows.",
        )
        .await
        .expect("failed to write auth doc");
    workspace
        .write(
            "docs/storage.md",
            "libSQL with FTS5 and DiskANN powers local hybrid retrieval.",
        )
        .await
        .expect("failed to write storage doc");
    workspace
        .write(
            "docs/tasks.md",
            "Task templates support ask mode and yolo mode for local automation.",
        )
        .await
        .expect("failed to write task doc");

    let fts_results = workspace
        .search_with_config("JWT authentication", SearchConfig::default().fts_only())
        .await
        .expect("fts search failed");
    assert!(
        fts_results
            .iter()
            .any(|result| result.document_path == "docs/auth.md"),
        "expected auth doc in FTS results: {fts_results:?}"
    );

    let hybrid_results = workspace
        .search("JWT authentication uses signed access tokens", 5)
        .await
        .expect("hybrid search failed");
    assert!(
        !hybrid_results.is_empty(),
        "expected non-empty hybrid results"
    );
    assert!(
        hybrid_results
            .iter()
            .any(|result| result.document_path == "docs/auth.md"),
        "expected auth doc in hybrid results: {hybrid_results:?}"
    );
    assert!(
        hybrid_results
            .iter()
            .any(|result| result.fts_rank.is_some() && result.vector_rank.is_some()),
        "expected at least one fused result containing both FTS and vector ranks: {hybrid_results:?}"
    );
}
