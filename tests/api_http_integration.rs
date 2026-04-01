use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use ironclaw::{
    agent::SessionManager,
    api::{ApiState, local_api_addr, router, run_api},
    channels::IncomingMessage,
    db::{ConversationStore, Database, SettingsStore, libsql::LibSqlBackend},
    runtime_events::SseManager,
    secrets::{InMemorySecretsStore, SecretsCrypto, SecretsStore},
    task_runtime::{TaskDetail, TaskRuntime},
    workspace::Workspace,
};
use serde_json::json;
use tokio::time::sleep;
use tower::util::ServiceExt;

async fn test_router() -> axum::Router {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-http-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let crypto = Arc::new(
        SecretsCrypto::new(secrecy::SecretString::from(
            ironclaw::secrets::keychain::generate_master_key_hex(),
        ))
        .expect("crypto"),
    );
    let secrets: Arc<dyn SecretsStore + Send + Sync> = Arc::new(InMemorySecretsStore::new(crypto));

    let state = ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::with_store(
            "http-test-user".to_string(),
            db.clone(),
        ))),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    )
    .with_secrets_store(secrets);

    router(state)
}

async fn wait_for_task_status(
    app: &axum::Router,
    task_id: uuid::Uuid,
    expected_status: &str,
) -> serde_json::Value {
    for _ in 0..20 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v0/tasks/{task_id}"))
                    .body(Body::empty())
                    .expect("task detail request"),
            )
            .await
            .expect("task detail response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("task detail body")
            .to_bytes();
        let detail: serde_json::Value = serde_json::from_slice(&body).expect("task detail json");
        if detail["task"]["status"] == expected_status {
            return detail;
        }
        sleep(Duration::from_millis(25)).await;
    }

    panic!("task {task_id} did not reach status {expected_status}");
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
async fn patch_settings_rejects_invalid_remote_http_provider_url() {
    let app = test_router().await;

    let patch_payload = json!({
        "llm_backend": "openai_compatible",
        "openai_compatible_base_url": "http://example.com/v1",
        "selected_model": "gpt-test"
    });

    let response = app
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
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("patch body")
        .to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&body).expect("error json");
    assert!(
        error["error"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid LLM settings")
    );
}

#[tokio::test]
async fn settings_api_keys_move_to_secrets_store_and_reload_after_restart() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-settings-secrets-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");
    let crypto = Arc::new(
        SecretsCrypto::new(secrecy::SecretString::from(
            ironclaw::secrets::keychain::generate_master_key_hex(),
        ))
        .expect("crypto"),
    );
    let secrets: Arc<dyn SecretsStore + Send + Sync> = Arc::new(InMemorySecretsStore::new(crypto));

    let app = router(
        ApiState::new(
            "http-test-user".to_string(),
            local_api_addr(8765),
            db.clone(),
            Arc::new(SseManager::new()),
            Some(Arc::new(TaskRuntime::with_store(
                "http-test-user".to_string(),
                db.clone(),
            ))),
            None,
            Some(Arc::new(SessionManager::new())),
            Some(Arc::new(Workspace::new_with_db(
                "http-test-user",
                db.clone(),
            ))),
        )
        .with_secrets_store(Arc::clone(&secrets)),
    );

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

    let response = app
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
    assert_eq!(response.status(), StatusCode::OK);

    let stored = db
        .get_all_settings("http-test-user")
        .await
        .expect("stored settings");
    assert_eq!(
        stored["llm_builtin_overrides.openai.model"],
        json!("gpt-4.1")
    );
    assert!(
        !stored.contains_key("llm_builtin_overrides.openai.api_key"),
        "builtin api_key leaked to settings table"
    );
    let custom = stored["llm_custom_providers"][0].clone();
    assert_eq!(custom["id"], "local-openai-compatible");
    assert!(
        custom.get("api_key").is_none(),
        "custom api_key leaked to settings table"
    );

    let openai_secret = secrets
        .get_decrypted("http-test-user", "llm_builtin_openai_api_key")
        .await
        .expect("builtin secret");
    assert_eq!(openai_secret.expose(), "sk-test-123");
    let custom_secret = secrets
        .get_decrypted(
            "http-test-user",
            "llm_custom_local-openai-compatible_api_key",
        )
        .await
        .expect("custom secret");
    assert_eq!(custom_secret.expose(), "local-secret");

    let restarted = router(
        ApiState::new(
            "http-test-user".to_string(),
            local_api_addr(8765),
            db.clone(),
            Arc::new(SseManager::new()),
            Some(Arc::new(TaskRuntime::with_store(
                "http-test-user".to_string(),
                db.clone(),
            ))),
            None,
            Some(Arc::new(SessionManager::new())),
            Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
        )
        .with_secrets_store(secrets),
    );

    let get_response = restarted
        .oneshot(
            Request::builder()
                .uri("/api/v0/settings")
                .body(Body::empty())
                .expect("get request"),
        )
        .await
        .expect("get response");
    assert_eq!(get_response.status(), StatusCode::OK);
    let body = get_response
        .into_body()
        .collect()
        .await
        .expect("get body")
        .to_bytes();
    let restored: serde_json::Value = serde_json::from_slice(&body).expect("settings json");
    assert_eq!(
        restored["llm_builtin_overrides"]["openai"]["api_key"],
        "sk-test-123"
    );
    assert_eq!(
        restored["llm_custom_providers"][0]["api_key"],
        "local-secret"
    );
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
async fn task_endpoints_patch_mode_and_list_runtime_state() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-task-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));
    let task_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "sort downloads")
        .with_thread(task_id.to_string());
    runtime.ensure_task(&message, task_id).await;

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

    let patch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v0/tasks/{task_id}/mode"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"mode":"yolo"}"#))
                .expect("patch mode request"),
        )
        .await
        .expect("patch mode response");
    assert_eq!(patch_response.status(), StatusCode::OK);

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
    assert_eq!(tasks["tasks"][0]["template_id"], "legacy:session-thread");
    assert_eq!(tasks["tasks"][0]["mode"], "yolo");
    assert_eq!(tasks["tasks"][0]["status"], "queued");
    assert_eq!(tasks["tasks"][0]["current_step"]["kind"], "log");
}

#[tokio::test]
async fn run_alias_endpoints_list_and_load_task_detail() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-runs-alias-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));
    let task_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "prepare workspace report")
        .with_thread(task_id.to_string());
    runtime.ensure_task(&message, task_id).await;
    runtime.mark_running(&message, task_id).await;

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(runtime),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    ));

    let list_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v0/runs")
                .body(Body::empty())
                .expect("runs list request"),
        )
        .await
        .expect("runs list response");
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body = list_response
        .into_body()
        .collect()
        .await
        .expect("runs list body")
        .to_bytes();
    let runs: serde_json::Value = serde_json::from_slice(&list_body).expect("runs list json");
    assert_eq!(runs["tasks"][0]["id"], task_id.to_string());

    let detail_response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/runs/{task_id}"))
                .body(Body::empty())
                .expect("run detail request"),
        )
        .await
        .expect("run detail response");
    assert_eq!(detail_response.status(), StatusCode::OK);
    let detail_body = detail_response
        .into_body()
        .collect()
        .await
        .expect("run detail body")
        .to_bytes();
    let detail: serde_json::Value = serde_json::from_slice(&detail_body).expect("run detail json");
    assert_eq!(detail["task"]["id"], task_id.to_string());
    assert_eq!(detail["timeline"][1]["event"], "task.step.started");
}

#[tokio::test]
async fn task_detail_persists_timeline_across_runtime_restart() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-task-detail-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let task_id = uuid::Uuid::new_v4();
    let runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));
    let message = IncomingMessage::new("test", "http-test-user", "sort downloads")
        .with_thread(task_id.to_string())
        .with_metadata(serde_json::json!({"source":"api-test"}));
    runtime.ensure_task(&message, task_id).await;
    runtime.mark_running(&message, task_id).await;
    runtime.mark_failed(task_id, "disk unavailable").await;

    let restarted_runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));
    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(restarted_runtime),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/tasks/{task_id}"))
                .body(Body::empty())
                .expect("task detail request"),
        )
        .await
        .expect("task detail response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("detail body")
        .to_bytes();
    let detail: TaskDetail = serde_json::from_slice(&body).expect("task detail json");
    assert_eq!(
        detail.task.status,
        ironclaw::task_runtime::TaskStatus::Failed
    );
    assert_eq!(detail.task.last_error.as_deref(), Some("disk unavailable"));
    assert_eq!(detail.timeline.len(), 3);
    assert_eq!(detail.timeline[0].event, "task.created");
    assert_eq!(detail.timeline[1].event, "task.step.started");
    assert_eq!(detail.timeline[2].event, "task.failed");
}

#[tokio::test]
async fn task_detail_preserves_pending_approval_across_runtime_restart() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-task-approval-restart-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let task_id = uuid::Uuid::new_v4();
    let request_id = uuid::Uuid::new_v4();
    let runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));
    let message = IncomingMessage::new("test", "http-test-user", "archive downloads")
        .with_thread(task_id.to_string())
        .with_metadata(serde_json::json!({"source":"api-test"}));
    runtime.ensure_task(&message, task_id).await;
    let pending = ironclaw::agent::session::PendingApproval {
        request_id,
        tool_name: "write_file".to_string(),
        parameters: json!({"path":"/tmp/report.md"}),
        display_parameters: json!({"path":"/tmp/report.md"}),
        description: "write a file".to_string(),
        tool_call_id: "call_1".to_string(),
        context_messages: Vec::new(),
        deferred_tool_calls: Vec::new(),
        user_timezone: Some("UTC".to_string()),
        allow_always: true,
    };
    runtime
        .mark_waiting_approval(&message, task_id, &pending)
        .await;

    let restarted_runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));
    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(restarted_runtime),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/tasks/{task_id}"))
                .body(Body::empty())
                .expect("task detail request"),
        )
        .await
        .expect("task detail response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("detail body")
        .to_bytes();
    let detail: serde_json::Value = serde_json::from_slice(&body).expect("task detail json");
    assert_eq!(detail["task"]["status"], "waiting_approval");
    assert_eq!(
        detail["task"]["pending_approval"]["id"],
        request_id.to_string()
    );
    assert_eq!(detail["timeline"][1]["event"], "task.waiting_approval");
}

#[tokio::test]
async fn delete_task_cancels_run_and_persists_after_restart() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-task-cancel-restart-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));
    let task_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "organize these files")
        .with_thread(task_id.to_string());
    runtime.ensure_task(&message, task_id).await;
    runtime.mark_running(&message, task_id).await;

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(runtime),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db(
            "http-test-user",
            db.clone(),
        ))),
    ));

    let cancel_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v0/tasks/{task_id}"))
                .body(Body::empty())
                .expect("cancel request"),
        )
        .await
        .expect("cancel response");
    assert_eq!(cancel_response.status(), StatusCode::OK);
    let cancel_body = cancel_response
        .into_body()
        .collect()
        .await
        .expect("cancel body")
        .to_bytes();
    let cancelled: serde_json::Value =
        serde_json::from_slice(&cancel_body).expect("cancelled task json");
    assert_eq!(cancelled["status"], "cancelled");

    let restarted = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::with_store(
            "http-test-user".to_string(),
            db,
        ))),
        None,
        Some(Arc::new(SessionManager::new())),
        None,
    ));

    let detail_response = restarted
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/tasks/{task_id}"))
                .body(Body::empty())
                .expect("task detail request"),
        )
        .await
        .expect("task detail response");
    assert_eq!(detail_response.status(), StatusCode::OK);
    let detail_body = detail_response
        .into_body()
        .collect()
        .await
        .expect("task detail body")
        .to_bytes();
    let detail: serde_json::Value = serde_json::from_slice(&detail_body).expect("task detail json");
    assert_eq!(detail["task"]["status"], "cancelled");
    assert_eq!(detail["timeline"][2]["event"], "task.cancelled");
}

#[tokio::test]
async fn template_endpoints_support_builtin_and_user_crud() {
    let app = test_router().await;

    let builtin_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v0/templates")
                .body(Body::empty())
                .expect("list templates request"),
        )
        .await
        .expect("list templates response");
    assert_eq!(builtin_response.status(), StatusCode::OK);
    let builtin_body = builtin_response
        .into_body()
        .collect()
        .await
        .expect("builtin body")
        .to_bytes();
    let builtin_json: serde_json::Value =
        serde_json::from_slice(&builtin_body).expect("builtin templates json");
    assert_eq!(builtin_json["templates"][0]["builtin"], true);

    let create_payload = json!({
        "name": "Custom Archive",
        "description": "User-defined archive variant",
        "parameter_schema": {
            "type": "object",
            "properties": {
                "source_path": { "type": "string" }
            },
            "required": ["source_path"]
        },
        "default_mode": "ask",
        "output_expectations": {
            "kind": "file_operation_plan"
        }
    });

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v0/templates")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(create_payload.to_string()))
                .expect("create template request"),
        )
        .await
        .expect("create template response");
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let created_body = create_response
        .into_body()
        .collect()
        .await
        .expect("created body")
        .to_bytes();
    let created_json: serde_json::Value =
        serde_json::from_slice(&created_body).expect("created template json");
    let template_id = created_json["id"].as_str().expect("template id");
    assert_eq!(created_json["builtin"], false);
    assert_eq!(created_json["name"], "Custom Archive");

    let get_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/templates/{template_id}"))
                .body(Body::empty())
                .expect("get template request"),
        )
        .await
        .expect("get template response");
    assert_eq!(get_response.status(), StatusCode::OK);

    let update_payload = json!({
        "name": "Custom Archive Updated",
        "description": "Updated description",
        "parameter_schema": {
            "type": "object",
            "properties": {
                "source_path": { "type": "string" },
                "target_root": { "type": "string" }
            },
            "required": ["source_path", "target_root"]
        },
        "default_mode": "yolo",
        "output_expectations": {
            "kind": "file_operation_plan",
            "artifacts": [{"type": "result_summary"}]
        }
    });

    let update_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v0/templates/{template_id}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(update_payload.to_string()))
                .expect("update template request"),
        )
        .await
        .expect("update template response");
    assert_eq!(update_response.status(), StatusCode::OK);
    let update_body = update_response
        .into_body()
        .collect()
        .await
        .expect("update body")
        .to_bytes();
    let update_json: serde_json::Value =
        serde_json::from_slice(&update_body).expect("updated template json");
    assert_eq!(update_json["name"], "Custom Archive Updated");
    assert_eq!(update_json["default_mode"], "yolo");

    let delete_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v0/templates/{template_id}"))
                .body(Body::empty())
                .expect("delete template request"),
        )
        .await
        .expect("delete template response");
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let missing_response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/templates/{template_id}"))
                .body(Body::empty())
                .expect("missing template request"),
        )
        .await
        .expect("missing template response");
    assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn template_endpoints_reject_invalid_payloads_and_builtin_mutations() {
    let app = test_router().await;

    let invalid_payload = json!({
        "name": "",
        "description": "",
        "parameter_schema": {
            "type": "array"
        },
        "default_mode": "maybe",
        "output_expectations": []
    });

    let invalid_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v0/templates")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(invalid_payload.to_string()))
                .expect("invalid template request"),
        )
        .await
        .expect("invalid template response");
    assert_eq!(invalid_response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let invalid_body = invalid_response
        .into_body()
        .collect()
        .await
        .expect("invalid body")
        .to_bytes();
    let invalid_json: serde_json::Value =
        serde_json::from_slice(&invalid_body).expect("invalid template json");
    assert_eq!(invalid_json["error"], "invalid template definition");
    assert_eq!(invalid_json["field_errors"]["name"], "name is required");
    assert_eq!(
        invalid_json["field_errors"]["default_mode"],
        "default_mode must be \"ask\" or \"yolo\""
    );

    let builtin_update = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v0/templates/builtin:file-archive")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": "Illegal",
                        "description": "",
                        "parameter_schema": {
                            "type": "object",
                            "properties": {}
                        },
                        "default_mode": "ask",
                        "output_expectations": {}
                    })
                    .to_string(),
                ))
                .expect("builtin update request"),
        )
        .await
        .expect("builtin update response");
    assert_eq!(builtin_update.status(), StatusCode::CONFLICT);

    let builtin_delete = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v0/templates/builtin:file-archive")
                .body(Body::empty())
                .expect("builtin delete request"),
        )
        .await
        .expect("builtin delete response");
    assert_eq!(builtin_delete.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn task_stream_emits_normalized_envelope() {
    let runtime = Arc::new(TaskRuntime::new());
    let task_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "archive downloads")
        .with_thread(task_id.to_string())
        .with_owner_id("owner-1")
        .with_sender_id("sender-1")
        .with_metadata(serde_json::json!({"source":"api-test"}))
        .with_timezone("UTC");
    runtime.ensure_task(&message, task_id).await;
    runtime
        .toggle_mode(task_id, ironclaw::task_runtime::TaskMode::Yolo)
        .await;

    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-task-stream-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let sse_manager = Arc::new(SseManager::new());
    sse_manager.broadcast_for_user(
        "http-test-user",
        ironclaw_common::AppEvent::Status {
            message: "task.mode_changed:yolo".to_string(),
            thread_id: Some(task_id.to_string()),
        },
    );

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        sse_manager,
        Some(runtime.clone()),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/tasks/{task_id}/stream"))
                .body(Body::empty())
                .expect("task stream request"),
        )
        .await
        .expect("task stream response");
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();

    let first_frame = body
        .frame()
        .await
        .expect("first frame present")
        .expect("first frame ok");
    let first_bytes = first_frame.into_data().expect("first data");
    let first_text = std::str::from_utf8(&first_bytes).expect("first utf8");
    assert!(first_text.contains("event: task.created"));
    assert!(first_text.contains("\"event\":\"task.created\""));
    assert!(first_text.contains("\"thread_id\":\""));
}

#[tokio::test]
async fn task_stream_emits_waiting_approval_then_mode_changed() {
    let runtime = Arc::new(TaskRuntime::new());
    let task_id = uuid::Uuid::new_v4();
    let request_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "archive downloads")
        .with_thread(task_id.to_string())
        .with_owner_id("owner-1")
        .with_sender_id("sender-1")
        .with_metadata(serde_json::json!({"source":"api-test"}))
        .with_timezone("UTC");
    runtime.ensure_task(&message, task_id).await;

    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-task-stream-live-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");

    let sse_manager = Arc::new(SseManager::new());
    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        sse_manager.clone(),
        Some(runtime.clone()),
        None,
        Some(Arc::new(SessionManager::new())),
        Some(Arc::new(Workspace::new_with_db("http-test-user", db))),
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/tasks/{task_id}/stream"))
                .body(Body::empty())
                .expect("task stream request"),
        )
        .await
        .expect("task stream response");
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();

    let pending = ironclaw::agent::session::PendingApproval {
        request_id,
        tool_name: "write_file".to_string(),
        parameters: serde_json::json!({"path":"/tmp/report.md"}),
        display_parameters: serde_json::json!({"path":"/tmp/report.md"}),
        description: "write a file".to_string(),
        tool_call_id: "call_1".to_string(),
        context_messages: Vec::new(),
        deferred_tool_calls: Vec::new(),
        user_timezone: Some("UTC".to_string()),
        allow_always: false,
    };
    runtime
        .mark_waiting_approval(&message, task_id, &pending)
        .await;
    sse_manager.broadcast_for_user(
        "http-test-user",
        ironclaw_common::AppEvent::ApprovalNeeded {
            request_id: request_id.to_string(),
            tool_name: "write_file".to_string(),
            description: "write a file".to_string(),
            parameters: "{\"path\":\"/tmp/report.md\"}".to_string(),
            thread_id: Some(task_id.to_string()),
            allow_always: false,
        },
    );
    runtime
        .toggle_mode(task_id, ironclaw::task_runtime::TaskMode::Yolo)
        .await;
    sse_manager.broadcast_for_user(
        "http-test-user",
        ironclaw_common::AppEvent::Status {
            message: "task.mode_changed:yolo".to_string(),
            thread_id: Some(task_id.to_string()),
        },
    );

    let _initial = body
        .frame()
        .await
        .expect("initial frame present")
        .expect("initial frame ok");
    let waiting_frame = body
        .frame()
        .await
        .expect("waiting frame present")
        .expect("waiting frame ok");
    let waiting_bytes = waiting_frame.into_data().expect("waiting data");
    let waiting_text = std::str::from_utf8(&waiting_bytes).expect("utf8");
    assert!(waiting_text.contains("event: task.waiting_approval"));
    assert!(waiting_text.contains(&format!("\"correlation_id\":\"{task_id}\"")));
    assert!(waiting_text.contains("\"status\":\"waiting_approval\""));

    let mode_frame = body
        .frame()
        .await
        .expect("mode frame present")
        .expect("mode frame ok");
    let mode_bytes = mode_frame.into_data().expect("mode data");
    let mode_text = std::str::from_utf8(&mode_bytes).expect("utf8");
    assert!(mode_text.contains("event: task.mode_changed"));
    assert!(mode_text.contains(&format!("\"correlation_id\":\"{task_id}\"")));
    assert!(mode_text.contains("\"mode\":\"yolo\""));
}

#[tokio::test]
async fn patch_task_mode_returns_422_on_invalid_mode() {
    let runtime = Arc::new(TaskRuntime::new());
    let task_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "sort downloads")
        .with_thread(task_id.to_string());
    runtime.ensure_task(&message, task_id).await;

    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-mode-validation-{}.db",
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

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/v0/tasks/{task_id}/mode"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"mode":"auto"}"#))
                .expect("invalid mode request"),
        )
        .await
        .expect("invalid mode response");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reject_task_happy_path_and_conflict() {
    let runtime = Arc::new(TaskRuntime::new());
    let task_id = uuid::Uuid::new_v4();
    let request_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "archive downloads")
        .with_thread(task_id.to_string())
        .with_owner_id("owner-1")
        .with_sender_id("sender-1")
        .with_metadata(serde_json::json!({"source":"api-test"}))
        .with_timezone("UTC");
    runtime.ensure_task(&message, task_id).await;
    let pending = ironclaw::agent::session::PendingApproval {
        request_id,
        tool_name: "write_file".to_string(),
        parameters: serde_json::json!({"path":"/tmp/report.md"}),
        display_parameters: serde_json::json!({"path":"/tmp/report.md"}),
        description: "write a file".to_string(),
        tool_call_id: "call_1".to_string(),
        context_messages: Vec::new(),
        deferred_tool_calls: Vec::new(),
        user_timezone: Some("UTC".to_string()),
        allow_always: false,
    };
    runtime
        .mark_waiting_approval(&message, task_id, &pending)
        .await;

    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-reject-test-{}.db",
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

    // Happy path: reject with correct approval_id
    let reject_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v0/tasks/{task_id}/reject"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"approval_id":"{request_id}","reason":"unsafe"}}"#
                )))
                .expect("reject request"),
        )
        .await
        .expect("reject response");
    assert_eq!(reject_response.status(), StatusCode::OK);
    let body = reject_response
        .into_body()
        .collect()
        .await
        .expect("reject body")
        .to_bytes();
    let task: serde_json::Value = serde_json::from_slice(&body).expect("reject json");
    assert_eq!(task["status"], "rejected");

    // 409: reject already-rejected task
    let conflict_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v0/tasks/{task_id}/reject"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"reason":"again"}"#))
                .expect("conflict reject request"),
        )
        .await
        .expect("conflict reject response");
    assert_eq!(conflict_response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn approve_task_returns_409_on_wrong_approval_id() {
    let runtime = Arc::new(TaskRuntime::new());
    let task_id = uuid::Uuid::new_v4();
    let request_id = uuid::Uuid::new_v4();
    let message = IncomingMessage::new("test", "http-test-user", "archive downloads")
        .with_thread(task_id.to_string())
        .with_owner_id("owner-1")
        .with_sender_id("sender-1")
        .with_metadata(serde_json::json!({"source":"api-test"}))
        .with_timezone("UTC");
    runtime.ensure_task(&message, task_id).await;
    let pending = ironclaw::agent::session::PendingApproval {
        request_id,
        tool_name: "write_file".to_string(),
        parameters: serde_json::json!({"path":"/tmp/report.md"}),
        display_parameters: serde_json::json!({"path":"/tmp/report.md"}),
        description: "write a file".to_string(),
        tool_call_id: "call_1".to_string(),
        context_messages: Vec::new(),
        deferred_tool_calls: Vec::new(),
        user_timezone: Some("UTC".to_string()),
        allow_always: false,
    };
    runtime
        .mark_waiting_approval(&message, task_id, &pending)
        .await;

    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-approve-stale-{}.db",
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

    let stale_id = uuid::Uuid::new_v4();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v0/tasks/{task_id}/approve"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(r#"{{"approval_id":"{stale_id}"}}"#)))
                .expect("stale approve request"),
        )
        .await
        .expect("stale approve response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
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
    assert_eq!(session["current_task"], serde_json::Value::Null);
}

#[tokio::test]
async fn session_message_returns_attached_task_and_requested_mode() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-session-message-task-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");
    let (inject_tx, mut inject_rx) = tokio::sync::mpsc::channel(8);
    let runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(runtime),
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
                .body(Body::from(r#"{"title":"Research sprint"}"#))
                .expect("create request"),
        )
        .await
        .expect("create response");
    assert_eq!(create_response.status(), StatusCode::OK);
    let create_body = create_response
        .into_body()
        .collect()
        .await
        .expect("create body")
        .to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&create_body).expect("create json");
    let session_id = created["id"].as_str().expect("session id").to_string();

    let send_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v0/sessions/{session_id}/messages"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"content":"Summarize this workspace and propose next steps","mode":"yolo"}"#,
                ))
                .expect("send request"),
        )
        .await
        .expect("send response");
    assert_eq!(send_response.status(), StatusCode::OK);
    let send_body = send_response
        .into_body()
        .collect()
        .await
        .expect("send body")
        .to_bytes();
    let sent: serde_json::Value = serde_json::from_slice(&send_body).expect("send json");
    assert_eq!(sent["accepted"], true);
    assert_eq!(sent["session_id"], session_id);
    assert_eq!(sent["task_id"], session_id);
    assert_eq!(sent["task"]["id"], session_id);
    assert_eq!(sent["task"]["mode"], "yolo");

    let injected = inject_rx.recv().await.expect("injected message");
    assert_eq!(injected.thread_id.as_deref(), Some(session_id.as_str()));
    assert_eq!(
        injected.content,
        "Summarize this workspace and propose next steps"
    );

    let get_response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/sessions/{session_id}"))
                .body(Body::empty())
                .expect("get session request"),
        )
        .await
        .expect("get session response");
    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body = get_response
        .into_body()
        .collect()
        .await
        .expect("get session body")
        .to_bytes();
    let session: serde_json::Value = serde_json::from_slice(&get_body).expect("session json");
    assert_eq!(session["current_task"]["id"], session_id);
    assert_eq!(session["current_task"]["mode"], "yolo");
}

#[tokio::test]
async fn session_message_rejects_invalid_mode() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-session-message-mode-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");
    let (inject_tx, _inject_rx) = tokio::sync::mpsc::channel(8);

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db,
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::new())),
        Some(inject_tx),
        Some(Arc::new(SessionManager::new())),
        None,
    ));

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v0/sessions")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"Mode validation"}"#))
                .expect("create request"),
        )
        .await
        .expect("create response");
    let create_body = create_response
        .into_body()
        .collect()
        .await
        .expect("create body")
        .to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&create_body).expect("create json");
    let session_id = created["id"].as_str().expect("session id");

    let send_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v0/sessions/{session_id}/messages"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"content":"hello","mode":"turbo"}"#))
                .expect("send request"),
        )
        .await
        .expect("send response");
    assert_eq!(send_response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn session_detail_restores_current_task_after_runtime_restart() {
    let db_path = std::env::temp_dir().join(format!(
        "ironcowork-api-session-current-task-restart-{}.db",
        uuid::Uuid::new_v4()
    ));
    let db = Arc::new(LibSqlBackend::new_local(&db_path).await.expect("db"));
    db.run_migrations().await.expect("migrations");
    let runtime = Arc::new(TaskRuntime::with_store(
        "http-test-user".to_string(),
        db.clone(),
    ));
    let session_id = uuid::Uuid::new_v4();

    let session_manager = Arc::new(SessionManager::new());
    session_manager
        .create_bound_thread("http-test-user", "api", &session_id.to_string(), session_id)
        .await;
    db.ensure_conversation(
        session_id,
        "api",
        "http-test-user",
        Some(&session_id.to_string()),
    )
    .await
    .expect("ensure conversation");

    let message = IncomingMessage::new("api", "http-test-user", "Review the inbox")
        .with_owner_id("http-test-user".to_string())
        .with_sender_id("http-test-user".to_string())
        .with_thread(session_id.to_string());
    runtime.ensure_task(&message, session_id).await;
    runtime
        .toggle_mode(session_id, ironclaw::task_runtime::TaskMode::Yolo)
        .await;

    let app = router(ApiState::new(
        "http-test-user".to_string(),
        local_api_addr(8765),
        db.clone(),
        Arc::new(SseManager::new()),
        Some(Arc::new(TaskRuntime::with_store(
            "http-test-user".to_string(),
            db,
        ))),
        None,
        Some(session_manager),
        None,
    ));

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v0/sessions/{session_id}"))
                .body(Body::empty())
                .expect("get session request"),
        )
        .await
        .expect("get session response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("get session body")
        .to_bytes();
    let session: serde_json::Value = serde_json::from_slice(&body).expect("session json");
    assert_eq!(session["current_task"]["id"], session_id.to_string());
    assert_eq!(session["current_task"]["mode"], "yolo");
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

#[tokio::test]
async fn create_archive_task_in_ask_mode_persists_preview() {
    let app = test_router().await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let target_dir = tempfile::tempdir().expect("target tempdir");
    let source_file = source_dir.path().join("report.txt");
    std::fs::write(&source_file, "quarterly report").expect("write source file");

    let create_payload = json!({
        "template_id": "builtin:file-archive",
        "mode": "ask",
        "parameters": {
            "source_path": source_dir.path().display().to_string(),
            "target_root": target_dir.path().display().to_string(),
            "naming_strategy": "preserve",
            "exclude_patterns": []
        }
    });

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v0/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(create_payload.to_string()))
                .expect("create task request"),
        )
        .await
        .expect("create task response");
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let body = create_response
        .into_body()
        .collect()
        .await
        .expect("create task body")
        .to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&body).expect("create task json");
    assert_eq!(created["status"], "queued");
    let task_id = created["task_id"]
        .as_str()
        .expect("task_id")
        .parse::<uuid::Uuid>()
        .expect("uuid");

    let detail = wait_for_task_status(&app, task_id, "waiting_approval").await;
    assert_eq!(detail["task"]["template_id"], "builtin:file-archive");
    assert_eq!(detail["task"]["pending_approval"]["risk"], "file_write");
    assert_eq!(
        detail["task"]["pending_approval"]["operations"][0]["tool_name"],
        "move_file"
    );
    assert_eq!(
        detail["task"]["pending_approval"]["operations"][0]["destination_path"],
        target_dir
            .path()
            .join("Documents/report.txt")
            .display()
            .to_string()
    );
    assert_eq!(detail["timeline"][0]["event"], "task.created");
    assert_eq!(detail["timeline"][1]["event"], "task.waiting_approval");
    assert!(source_file.exists());
}

#[tokio::test]
async fn create_archive_task_in_yolo_mode_executes_plan() {
    let app = test_router().await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let target_dir = tempfile::tempdir().expect("target tempdir");
    let source_file = source_dir.path().join("photo.jpg");
    let skipped_dir = source_dir.path().join("nested");
    std::fs::create_dir_all(&skipped_dir).expect("create nested dir");
    std::fs::write(&source_file, "image-bytes").expect("write source file");

    let create_payload = json!({
        "template_id": "builtin:file-archive",
        "mode": "yolo",
        "parameters": {
            "source_path": source_dir.path().display().to_string(),
            "target_root": target_dir.path().display().to_string(),
            "naming_strategy": "preserve",
            "exclude_patterns": []
        }
    });

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v0/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(create_payload.to_string()))
                .expect("create task request"),
        )
        .await
        .expect("create task response");
    assert_eq!(create_response.status(), StatusCode::CREATED);
    let body = create_response
        .into_body()
        .collect()
        .await
        .expect("create task body")
        .to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&body).expect("create task json");
    let task_id = created["task_id"]
        .as_str()
        .expect("task_id")
        .parse::<uuid::Uuid>()
        .expect("uuid");

    let detail = wait_for_task_status(&app, task_id, "completed").await;
    let destination = target_dir.path().join("Images/photo.jpg");
    assert!(destination.exists());
    assert!(!source_file.exists());
    assert_eq!(
        detail["task"]["result_metadata"]["execution"]["moved"][0]["destination_path"],
        destination.display().to_string()
    );
    assert_eq!(
        detail["task"]["result_metadata"]["execution"]["skipped"][0]["reason"],
        "directories are skipped"
    );
}

#[tokio::test]
async fn approve_archive_task_executes_waiting_plan() {
    let app = test_router().await;
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let target_dir = tempfile::tempdir().expect("target tempdir");
    let source_file = source_dir.path().join("notes.md");
    std::fs::write(&source_file, "# Notes").expect("write source file");

    let create_payload = json!({
        "template_id": "builtin:file-archive",
        "mode": "ask",
        "parameters": {
            "source_path": source_dir.path().display().to_string(),
            "target_root": target_dir.path().display().to_string(),
            "naming_strategy": "preserve",
            "exclude_patterns": []
        }
    });

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v0/tasks")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(create_payload.to_string()))
                .expect("create task request"),
        )
        .await
        .expect("create task response");
    let body = create_response
        .into_body()
        .collect()
        .await
        .expect("create task body")
        .to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&body).expect("create task json");
    let task_id = created["task_id"]
        .as_str()
        .expect("task_id")
        .parse::<uuid::Uuid>()
        .expect("uuid");

    let waiting = wait_for_task_status(&app, task_id, "waiting_approval").await;
    let approval_id = waiting["task"]["pending_approval"]["id"]
        .as_str()
        .expect("approval id");

    let approve_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v0/tasks/{task_id}/approve"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "approval_id": approval_id,
                        "always": false
                    })
                    .to_string(),
                ))
                .expect("approve request"),
        )
        .await
        .expect("approve response");
    assert_eq!(approve_response.status(), StatusCode::OK);
    let body = approve_response
        .into_body()
        .collect()
        .await
        .expect("approve body")
        .to_bytes();
    let approved: serde_json::Value = serde_json::from_slice(&body).expect("approve json");
    assert_eq!(approved["status"], "completed");
    assert_eq!(approved["pending_approval"], serde_json::Value::Null);

    let destination = target_dir.path().join("Documents/notes.md");
    assert!(destination.exists());
    assert!(!source_file.exists());
}
