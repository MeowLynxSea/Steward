use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use ironclaw::{
    api::{ApiState, local_api_addr, router, run_api},
    db::{Database, SettingsStore, libsql::LibSqlBackend},
    runtime_events::SseManager,
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
        db,
        Arc::new(SseManager::new()),
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
        db as Arc<dyn SettingsStore>,
        Arc::new(SseManager::new()),
    );

    let err = run_api(bind_addr, state)
        .await
        .expect_err("bind should fail");
    assert!(
        err.to_string().contains("Phase 1 only allows 127.0.0.1"),
        "unexpected error: {err}"
    );
}
