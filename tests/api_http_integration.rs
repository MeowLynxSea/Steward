use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use ironclaw::{
    agent::SessionManager,
    api::{ApiState, local_api_addr, router, run_api},
    channels::IncomingMessage,
    db::{ConversationStore, Database, libsql::LibSqlBackend},
    runtime_events::SseManager,
    task_runtime::TaskRuntime,
    workspace::Workspace,
};
use serde_json::json;
use tower::util::ServiceExt;

async fn test_router() -> axum::Router {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-http-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let state = ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::new())),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    );

    router(state)
}

#[tokio::test]
async fn health_endpoint_returns_local_bind() {
    let app = test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v0/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body).expect("health json");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["bind"], "127.0.0.1:8765");
}

#[tokio::test]
async fn settings_round_trip_over_http_and_emits_sse_event() {
    let app = test_router().await;

    let sse_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v0/events")
                .body(Body::empty())
                .expect("sse request"),
        )
        .await
        .expect("sse response");
    assert_eq!(sse_response.status(), StatusCode::OK);
    let mut sse_body = sse_response.into_body();

    let patch_payload = json!({
        "llm_backend": "openai",
        "selected_model": "gpt-4.1",
        "llm_builtin_overrides": {
            "openai": {
                "api_key": "sk-test-123",
                "model": "gpt-4.1"
            }
        },
        "llm_custom_providers": [
            {
                "id": "local-openai-compatible",
                "name": "Local OpenAI Compatible",
                "adapter": "open_ai_completions",
                "base_url": "http://127.0.0.1:11434/v1",
                "default_model": "qwen2.5-coder",
                "api_key": "local-secret",
                "builtin": false
            }
        ]
    });

    let patch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v0/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(patch_payload.to_string()))
                .expect("patch request"),
        )
        .await
        .expect("patch response");
    assert_eq!(patch_response.status(), StatusCode::OK);
    let patched = patch_response
        .into_body()
        .collect()
        .await
        .expect("patch body")
        .to_bytes();
    let patched: serde_json::Value = serde_json::from_slice(&patched).expect("patched json");
    assert_eq!(patched["llm_backend"], "openai");
    assert_eq!(
        patched["llm_builtin_overrides"]["openai"]["api_key"],
        "sk-test-123"
    );

    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v0/settings")
                .body(Body::empty())
                .expect("get request"),
        )
        .await
        .expect("get response");
    assert_eq!(get_response.status(), StatusCode::OK);
    let stored = get_response
        .into_body()
        .collect()
        .await
        .expect("get body")
        .to_bytes();
    let stored: serde_json::Value = serde_json::from_slice(&stored).expect("stored json");
    assert_eq!(stored["selected_model"], "gpt-4.1");
    assert_eq!(
        stored["llm_custom_providers"][0]["base_url"],
        "http://127.0.0.1:11434/v1"
    );

    let first_frame = sse_body
        .frame()
        .await
        .expect("sse frame present")
        .expect("sse frame ok");
    let event_bytes = first_frame.into_data().expect("data frame");
    let event_text = std::str::from_utf8(&event_bytes).expect("utf8 sse");
    assert!(event_text.contains("event: status"));
    assert!(event_text.contains("\"message\":\"settings.updated\""));
}

#[tokio::test]
async fn run_api_rejects_non_localhost_bind() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-bind-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let bind_addr = "0.0.0.0:8765".parse().expect("bind addr");
    let state = ApiState::new(
        "bind-test-user".to_string(),
        bind_addr,
        db,
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::new())),
        None,
        None,
        None,
    );

    let err = run_api(bind_addr, state)
        .await
        .expect_err("bind should fail");
    assert!(
        err.to_string().contains("Phase 1 only allows 127.0.0.1"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn task_endpoints_toggle_yolo_and_list_runtime_state() {
    let runtime = Arc::new(TaskRuntime::new());
    let task_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "sort downloads")
        .with_thread(task_id.to_string());
    runtime.ensure_task(&message, task_id).await;

    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-task-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(runtime.clone()),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    ));

    let toggle_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v0/tasks/{task_id}/yolo-toggle"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .expect("toggle request"),
        )
        .await
        .expect("toggle response");
    assert_eq!(toggle_response.status(), StatusCode::OK);

    let list_response = app
        .oneshot(
            Request::builder()
                .uri("/api/v0/tasks")
                .body(Body::empty())
                .expect("list request"),
        )
        .await
        .expect("list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let tasks = list_response
        .into_body()
        .collect()
        .await
        .expect("list body")
        .to_bytes();
    let tasks: serde_json::Value = serde_json::from_slice(&tasks).expect("tasks json");
    assert_eq!(tasks["tasks"][0]["id"], task_id.to_string());
    assert_eq!(tasks["tasks"][0]["mode"], "yolo");
}

#[tokio::test]
async fn sessions_endpoints_create_and_load_history() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-session-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");
    let inject_tx = tokio::sync::mpsc::channel(8).0;

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::new())),
        Some(inject_tx),
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db(
            "http-test-user",
            db.clone(),
        ))),
    ));

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v0/sessions")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"Inbox triage"}"#))
                .expect("create request"),
        )
        .await
        .expect("create response");
    assert_eq!(create_response.status(), StatusCode::OK);
    let body = create_response
        .into_body()
        .collect()
        .await
        .expect("create body")
        .to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&body).expect("create json");
    let session_id = created["id"].as_str().expect("session id").to_string();

    db.add_conversation_message(
        session_id.parse::<uuid::Uuid>().expect("uuid"),
        "assistant",
        "Archive plan ready",
    )
    .await
    .expect("add message");

    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/sessions/{session_id}"))
                .body(Body::empty())
                .expect("get session request"),
        )
        .await
        .expect("get session response");
    assert_eq!(get_response.status(), StatusCode::OK);
    let body = get_response
        .into_body()
        .collect()
        .await
        .expect("get session body")
        .to_bytes();
    let session: serde_json::Value = serde_json::from_slice(&body).expect("session json");
    assert_eq!(session["session"]["id"], session_id);
    assert_eq!(session["messages"][0]["content"], "Archive plan ready");
}

#[tokio::test]
async fn workspace_endpoints_index_and_list_tree() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-workspace-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");
    let source_dir = tempfile::tempdir().expect("tempdir");

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::new())),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    ));

    let index_payload = json!({ "path": source_dir.path().display().to_string() });
    let index_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v0/workspace/index")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(index_payload.to_string()))
                .expect("index request"),
        )
        .await
        .expect("index response");
    assert_eq!(index_response.status(), StatusCode::OK);

    let tree_response = app
        .oneshot(
            Request::builder()
                .uri("/api/v0/workspace/tree")
                .body(Body::empty())
                .expect("tree request"),
        )
        .await
        .expect("tree response");
    assert_eq!(tree_response.status(), StatusCode::OK);
    let body = tree_response
        .into_body()
        .collect()
        .await
        .expect("tree body")
        .to_bytes();
    let tree: serde_json::Value = serde_json::from_slice(&body).expect("tree json");
    assert!(
        tree["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|entry| !entry["path"].as_str().unwrap_or_default().is_empty())
    );
}
