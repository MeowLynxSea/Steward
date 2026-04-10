#![cfg(feature = "libsql")]

use std::sync::Arc;

use uuid::Uuid;

use crate::agent::routine_engine::RoutineEngine;
use crate::agent::routine::{NotifyConfig, RoutineGuardrails, next_cron_fire};
use crate::agent::{Routine, RoutineAction, Trigger};
use crate::config::RoutineConfig;
use crate::context::JobContext;
use crate::memory::{MemoryManager, MemoryNodeKind, MemoryVersionStatus, MemoryVisibility};
use crate::tenant::AdminScope;
use crate::testing::TestHarnessBuilder;
use crate::tools::Tool;
use crate::tools::builtin::{
    AddAliasTool, DeleteMemoryTool, ManageTriggersTool, ReadMemoryTool, UpdateMemoryTool,
};
use crate::workspace::Workspace;

#[tokio::test]
async fn update_memory_patch_requires_unique_match() {
    let harness = TestHarnessBuilder::new().build().await;
    let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));

    // Create a node with repeated substring.
    let (detail, _cs) = memory
        .create(
            "user1",
            None,
            None,
            "Patch Test",
            MemoryNodeKind::Curated,
            "hello world\nhello world\n",
            "core",
            "tests/patch_unique",
            50,
            None,
            MemoryVisibility::Private,
            Vec::new(),
            serde_json::json!({"source": "test"}),
        )
        .await
        .expect("create memory");

    let tool = UpdateMemoryTool::new(Arc::clone(&memory));
    let ctx = JobContext::with_user("user1", "t", "d");

    // Non-unique patch should fail.
    let err = tool
        .execute(
            serde_json::json!({
                "uri": "core://tests/patch_unique",
                "expected_version_id": detail.active_version.id.to_string(),
                "old_string": "hello world",
                "new_string": "hi",
            }),
            &ctx,
        )
        .await
        .expect_err("expected patch to fail when match is non-unique");
    let msg = format!("{err}");
    assert!(
        msg.contains("match exactly once"),
        "error should mention uniqueness, got: {msg}"
    );
}

#[tokio::test]
async fn delete_memory_is_route_only_and_preserves_alias() {
    let harness = TestHarnessBuilder::new().build().await;
    let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));

    // Create a node.
    let (detail, _cs) = memory
        .create(
            "user1",
            None,
            None,
            "Alias Target",
            MemoryNodeKind::Curated,
            "content",
            "core",
            "tests/alias_target",
            50,
            None,
            MemoryVisibility::Private,
            Vec::new(),
            serde_json::json!({"source": "test"}),
        )
        .await
        .expect("create memory");

    // Add an alias route.
    let alias_tool = AddAliasTool::new(Arc::clone(&memory));
    let ctx = JobContext::with_user("user1", "t", "d");
    alias_tool
        .execute(
            serde_json::json!({
                "new_uri": "project://tests/alias",
                "target_uri": "core://tests/alias_target",
                "priority": 50,
                "visibility": "private"
            }),
            &ctx,
        )
        .await
        .expect("add alias");

    // Route-only delete should reject node ids.
    let delete_tool = DeleteMemoryTool::new(Arc::clone(&memory));
    let err = delete_tool
        .execute(
            serde_json::json!({
                "uri": detail.node.id.to_string(),
            }),
            &ctx,
        )
        .await
        .expect_err("expected delete to reject node id");
    assert!(format!("{err}").contains("domain://path"));

    // Delete the original route; alias should still resolve.
    delete_tool
        .execute(
            serde_json::json!({ "uri": "core://tests/alias_target" }),
            &ctx,
        )
        .await
        .expect("delete original route");
    let still = memory
        .open("user1", None, "project://tests/alias")
        .await
        .expect("open alias route");
    assert!(still.is_some(), "alias route should still resolve");

    // Delete the alias route too; node should become orphaned but still openable by node id.
    delete_tool
        .execute(
            serde_json::json!({ "uri": "project://tests/alias" }),
            &ctx,
        )
        .await
        .expect("delete alias route");
    let by_id = memory
        .open("user1", None, &detail.node.id.to_string())
        .await
        .expect("open by node id");
    let by_id = by_id.expect("node should still exist");
    assert_eq!(by_id.active_version.status, MemoryVersionStatus::Orphaned);
}

#[tokio::test]
async fn manage_triggers_updates_system_glossary() {
    let harness = TestHarnessBuilder::new().build().await;
    let memory = Arc::new(MemoryManager::new(Arc::clone(&harness.db)));

    memory
        .create(
            "user1",
            None,
            None,
            "Glossary Node",
            MemoryNodeKind::Curated,
            "content",
            "core",
            "tests/glossary_node",
            50,
            None,
            MemoryVisibility::Private,
            Vec::new(),
            serde_json::json!({"source": "test"}),
        )
        .await
        .expect("create memory");

    let ctx = JobContext::with_user("user1", "t", "d");
    let triggers_tool = ManageTriggersTool::new(Arc::clone(&memory));
    triggers_tool
        .execute(
            serde_json::json!({
                "uri": "core://tests/glossary_node",
                "add": ["database migration strategy"]
            }),
            &ctx,
        )
        .await
        .expect("manage_triggers add");

    // system://glossary is supported in read_memory and memory_open.
    let read_tool = ReadMemoryTool::new(Arc::clone(&memory));
    let out = read_tool
        .execute(serde_json::json!({ "uri": "system://glossary" }), &ctx)
        .await
        .expect("read glossary");
    let text = out.result.to_string();
    assert!(
        text.contains("database migration strategy") && text.contains("core://tests/glossary_node"),
        "glossary should map keyword to uri, got: {text}"
    );
}

#[tokio::test]
async fn system_event_turn_completed_fires_routine_run() {
    let harness = TestHarnessBuilder::new().build().await;
    let db = Arc::clone(&harness.db);

    let workspace = Arc::new(Workspace::new_with_db("user1", Arc::clone(&db)));
    let memory = Arc::new(MemoryManager::new(Arc::clone(&db)));

    let (notify_tx, _notify_rx) = tokio::sync::mpsc::channel(8);
    let engine = Arc::new(RoutineEngine::new(
        RoutineConfig::default(),
        AdminScope::new(Arc::clone(&db)),
        Arc::clone(&harness.deps.llm),
        workspace,
        Some(memory),
        notify_tx,
        None,
        None,
        Arc::clone(&harness.deps.tools),
        Arc::clone(&harness.deps.safety),
    ));

    let routine = Routine {
        id: Uuid::new_v4(),
        name: "test_memory_reflection".to_string(),
        description: "test".to_string(),
        user_id: "user1".to_string(),
        enabled: true,
        trigger: Trigger::SystemEvent {
            source: "agent".to_string(),
            event_type: "turn_completed".to_string(),
            filters: std::collections::HashMap::new(),
        },
        action: RoutineAction::Lightweight {
            prompt: "Return ok.".to_string(),
            context_paths: Vec::new(),
            max_tokens: 256,
            use_tools: false,
            max_tool_rounds: 1,
        },
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(0),
            max_concurrent: 1,
            dedup_window: None,
        },
        notify: NotifyConfig {
            channel: None,
            user: None,
            on_attention: false,
            on_failure: false,
            on_success: false,
        },
        last_run_at: None,
        next_fire_at: None,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    db.create_routine(&routine).await.expect("create routine");
    engine.refresh_event_cache().await;

    let payload = serde_json::json!({
        "thread_id": "t",
        "user_input": "u",
        "assistant_output": "a",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let fired = engine
        .emit_system_event("agent", "turn_completed", &payload, Some("user1"))
        .await;
    assert!(fired >= 1, "expected at least one routine to be fired");

    // The run is recorded asynchronously; poll briefly.
    let mut found = false;
    for _ in 0..20 {
        let runs = db
            .list_routine_runs(routine.id, 10)
            .await
            .expect("list runs");
        if !runs.is_empty() {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(found, "expected a routine_run record to be created");

    // Also sanity-check cron helper still works (used by maintenance seeding).
    let _ = next_cron_fire("0 3 * * *", Some("UTC")).expect("cron parse");
}
