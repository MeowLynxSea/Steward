//! Tauri IPC command wrappers.
//!
//! These commands expose the IPC layer via Tauri's IPC mechanism.

use tauri::State;
use uuid::Uuid;

use steward_core::channels::IncomingMessage;
use steward_core::desktop_runtime::AppState;
use steward_core::ipc::{
    ApproveTaskRequest, CreateSessionRequest, CreateWorkspaceCheckpointRequest,
    CreateWorkspaceMountRequest, PatchSettingsRequest, PatchTaskModeRequest,
    RejectTaskRequest, ResolveWorkspaceConflictRequest, SendSessionMessageRequest,
    WorkspaceActionRequest, WorkspaceIndexRequest, WorkspaceSearchRequest,
};
use steward_core::settings::Settings;

// =============================================================================
// Settings (2 commands)
// =============================================================================

fn build_settings_response(settings: &Settings) -> steward_core::ipc::SettingsResponse {
    steward_core::ipc::SettingsResponse {
        llm_backend: settings.llm_backend.clone(),
        selected_model: settings.selected_model.clone(),
        ollama_base_url: settings.ollama_base_url.clone(),
        openai_compatible_base_url: settings.openai_compatible_base_url.clone(),
        llm_custom_providers: settings.llm_custom_providers.clone(),
        llm_builtin_overrides: settings.llm_builtin_overrides.clone(),
        llm_ready: settings.llm_backend.is_some(),
        llm_onboarding_required: !settings.onboard_completed,
        llm_readiness_error: None,
    }
}

#[tauri::command]
pub async fn get_settings(_state: State<'_, AppState>) -> Result<steward_core::ipc::SettingsResponse, String> {
    let settings = Settings::load_toml(&Settings::default_toml_path())
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    Ok(build_settings_response(&settings))
}

#[tauri::command]
pub async fn patch_settings(
    _state: State<'_, AppState>,
    payload: PatchSettingsRequest,
) -> Result<steward_core::ipc::SettingsResponse, String> {
    let mut settings = Settings::load_toml(&Settings::default_toml_path())
        .map_err(|e| e.to_string())?
        .unwrap_or_default();

    if let Some(llm_backend) = payload.llm_backend {
        settings.llm_backend = Some(llm_backend);
        settings.onboard_completed = true;
    }
    if let Some(selected_model) = payload.selected_model {
        settings.selected_model = Some(selected_model);
    }
    if let Some(ollama_base_url) = payload.ollama_base_url {
        settings.ollama_base_url = Some(ollama_base_url);
    }
    if let Some(openai_compatible_base_url) = payload.openai_compatible_base_url {
        settings.openai_compatible_base_url = Some(openai_compatible_base_url);
    }
    if let Some(llm_custom_providers) = payload.llm_custom_providers {
        settings.llm_custom_providers = llm_custom_providers;
    }
    if let Some(llm_builtin_overrides) = payload.llm_builtin_overrides {
        settings.llm_builtin_overrides = llm_builtin_overrides;
    }

    settings
        .save_toml(&Settings::default_toml_path())
        .map_err(|e| e.to_string())?;

    Ok(build_settings_response(&settings))
}

// =============================================================================
// Sessions (5 commands)
// =============================================================================

#[tauri::command]
pub async fn list_sessions(
    state: State<'_, AppState>,
) -> Result<steward_core::ipc::SessionListResponse, String> {
    let session_manager = &state.agent_session_manager;
    let sessions = session_manager.list_sessions(&state.owner_id).await;

    let mut summaries = Vec::new();
    for (_, session) in sessions {
        let sess = session.lock().await;
        let message_count: i64 = sess.threads.values()
            .map(|t| t.turns.len() as i64)
            .sum();
        summaries.push(steward_core::ipc::SessionSummaryResponse {
            id: sess.id,
            title: "Untitled Session".to_string(),
            message_count,
            started_at: sess.created_at,
            last_activity: sess.last_active_at,
            thread_type: None,
            channel: "desktop".to_string(),
        });
    }

    Ok(steward_core::ipc::SessionListResponse { sessions: summaries })
}

#[tauri::command]
pub async fn create_session(
    state: State<'_, AppState>,
    payload: Option<CreateSessionRequest>,
) -> Result<steward_core::ipc::CreateSessionResponse, String> {
    let session_manager = &state.agent_session_manager;
    let user_id = &state.owner_id;

    let session = session_manager.create_new_session(user_id).await;

    if let Some(req) = payload {
        if let Some(title) = req.title {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.values_mut().next() {
                if let Ok(metadata) = serde_json::from_value::<serde_json::Value>(thread.metadata.clone()) {
                    if let Some(mut obj) = metadata.as_object().cloned() {
                        obj.insert("title".to_string(), serde_json::json!(title));
                        thread.metadata = serde_json::to_value(&obj).unwrap_or(thread.metadata.clone());
                    }
                }
            }
        }
    }

    let id = session.lock().await.id;
    Ok(steward_core::ipc::CreateSessionResponse { id })
}

#[tauri::command]
pub async fn get_session(
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<steward_core::ipc::SessionDetailResponse, String> {
    let session_manager = &state.agent_session_manager;

    let session = session_manager.get_session_by_id(&state.owner_id, id).await
        .ok_or_else(|| "Session not found".to_string())?;

    // Get or create a thread if none exists
    let thread_id = {
        let mut sess = session.lock().await;
        let thread_id = sess
            .active_thread
            .or_else(|| sess.threads.keys().copied().next());

        match thread_id {
            Some(tid) => tid,
            None => {
                // Create a new thread if none exists
                let new_thread = sess.create_thread();
                tracing::debug!("Created new thread {} for session", new_thread.id);
                new_thread.id
            }
        }
    };

    let sess = session.lock().await;
    let thread = sess
        .threads
        .get(&thread_id)
        .ok_or_else(|| "Thread not found".to_string())?;

    let messages: Vec<steward_core::ipc::SessionMessageResponse> = thread
        .turns
        .iter()
        .flat_map(|turn| {
            let mut msgs = Vec::new();
            msgs.push(steward_core::ipc::SessionMessageResponse {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: turn.user_input.clone(),
                created_at: turn.started_at,
                turn_number: turn.turn_number,
            });
            if let Some(response) = &turn.response {
                msgs.push(steward_core::ipc::SessionMessageResponse {
                    id: Uuid::new_v4(),
                    role: "assistant".to_string(),
                    content: response.clone(),
                    created_at: turn.completed_at.unwrap_or(turn.started_at),
                    turn_number: turn.turn_number,
                });
            }
            msgs
        })
        .collect();

    let summary = steward_core::ipc::SessionSummaryResponse {
        id: sess.id,
        title: "Untitled Session".to_string(),
        message_count: thread.turns.len() as i64,
        started_at: sess.created_at,
        last_activity: sess.last_active_at,
        thread_type: None,
        channel: "desktop".to_string(),
    };

    let current_task = state
        .task_runtime
        .get_task(thread_id)
        .await;

    Ok(steward_core::ipc::SessionDetailResponse {
        session: summary,
        messages,
        current_task,
    })
}

#[tauri::command]
pub async fn delete_session(
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<serde_json::Value, String> {
    let session_manager = &state.agent_session_manager;
    let deleted = session_manager.delete_session_by_id(&state.owner_id, id).await;
    Ok(serde_json::json!({ "deleted": deleted }))
}

#[tauri::command]
pub async fn send_session_message(
    state: State<'_, AppState>,
    id: Uuid,
    payload: SendSessionMessageRequest,
) -> Result<steward_core::ipc::SendSessionMessageResponse, String> {
    tracing::info!(session_id = %id, content_len = payload.content.len(), "==> send_session_message CALLED");
    let session_manager = &state.agent_session_manager;

    let session = session_manager
        .get_session_by_id(&state.owner_id, id)
        .await
        .ok_or_else(|| "Session not found".to_string())?;
    tracing::info!(session_id = %id, "Got session, acquiring lock...");

    // Get or create a thread if none exists
    let thread_id = {
        let lock_result = tokio::time::timeout(std::time::Duration::from_secs(5), session.lock()).await;
        if lock_result.is_err() {
            tracing::error!("FIRST session.lock() TIMEOUT - session_id={}", id);
            return Err("Session lock timeout".to_string());
        }
        let mut sess = lock_result.unwrap();
        tracing::info!("FIRST session lock acquired");
        let tid = sess
            .active_thread
            .or_else(|| sess.threads.keys().copied().next());

        match tid {
            Some(id) => id,
            None => {
                let new_thread = sess.create_thread();
                tracing::debug!("Created new thread {} for session", new_thread.id);
                new_thread.id
            }
        }
    };

    let sess_result = tokio::time::timeout(std::time::Duration::from_secs(5), session.lock()).await;
    if sess_result.is_err() {
        tracing::error!("SECOND session.lock() TIMEOUT - thread_id={}", thread_id);
        return Err("Session lock timeout".to_string());
    }
    let sess = sess_result.unwrap();
    let thread = sess.threads.get(&thread_id).ok_or_else(|| "Thread not found".to_string())?;
    tracing::info!(thread_id = %thread_id, state = ?thread.state, "Thread state checked, matching...");

    tracing::info!(thread_id = %thread_id, "About to match thread state");
    match thread.state {
        steward_core::agent::session::ThreadState::Processing => {
            drop(sess);
            let session = session_manager
                .get_session_by_id(&state.owner_id, id)
                .await
                .ok_or_else(|| "Session not found".to_string())?;
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                if thread.queue_message(payload.content.clone()) {
                    Ok(steward_core::ipc::SendSessionMessageResponse {
                        accepted: true,
                        session_id: id,
                        task_id: None,
                        task: None,
                    })
                } else {
                    Err("Message queue full".to_string())
                }
            } else {
                Err("Thread not found".to_string())
            }
        }
        steward_core::agent::session::ThreadState::Idle
        | steward_core::agent::session::ThreadState::Interrupted => {
            tracing::info!(thread_id = %thread_id, "ENTERED Idle/Interrupted branch, dropping outer sess");
            drop(sess); // Drop the guard from line 273 before re-acquiring
            tracing::info!(thread_id = %thread_id, "ENTERED Idle/Interrupted branch, about to acquire lock");
            // Queue the message and inject into the agent stream to trigger processing
            let lock_result = tokio::time::timeout(std::time::Duration::from_secs(5), session.lock()).await;
            if lock_result.is_err() {
                tracing::error!(thread_id = %thread_id, "SESSION LOCK TIMEOUT after 5s - possible deadlock");
                return Err("Session lock timeout - possible deadlock".to_string());
            }
            let mut sess = lock_result.unwrap();
            tracing::info!(thread_id = %thread_id, "Lock acquired, queuing message");
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                if !thread.queue_message(payload.content.clone()) {
                    return Err("Message queue full".to_string());
                }
            } else {
                return Err("Thread not found".to_string());
            }
            drop(sess); // Release lock before injecting

            // Inject a message into the agent's stream to trigger processing
            let msg = IncomingMessage::new("api", state.owner_id.clone(), payload.content)
                .with_thread(thread_id.to_string());
            tracing::info!(message_id = %msg.id, thread_id = %thread_id, "Injecting message into agent stream");
            state
                .message_inject_tx
                .send(msg)
                .await
                .map_err(|e| format!("Failed to inject message: {}", e))?;
            tracing::info!(thread_id = %thread_id, "Message injected successfully");

            tracing::info!(thread_id = %thread_id, "==> send_session_message RETURNING OK");
            Ok(steward_core::ipc::SendSessionMessageResponse {
                accepted: true,
                session_id: id,
                task_id: None,
                task: None,
            })
        }
        steward_core::agent::session::ThreadState::AwaitingApproval => {
            Err("Thread is awaiting approval. Use /interrupt to cancel.".to_string())
        }
        steward_core::agent::session::ThreadState::Completed => {
            Err("Thread completed. Use /thread new to start a new conversation.".to_string())
        }
    }
}

// =============================================================================
// Tasks (6 commands)
// =============================================================================

#[tauri::command]
pub async fn list_tasks(
    state: State<'_, AppState>,
) -> Result<steward_core::ipc::TaskListResponse, String> {
    let tasks = state.task_runtime.list_tasks().await;
    Ok(steward_core::ipc::TaskListResponse { tasks })
}

#[tauri::command]
pub async fn get_task(
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<steward_core::ipc::TaskDetailResponse, String> {
    let task = state
        .task_runtime
        .get_task(id)
        .await
        .ok_or_else(|| "Task not found".to_string())?;

    let detail = state
        .task_runtime
        .get_task_detail(id)
        .await
        .ok_or_else(|| "Task detail not found".to_string())?;

    Ok(steward_core::ipc::TaskDetailResponse {
        task,
        timeline: detail.timeline,
    })
}

#[tauri::command]
pub async fn delete_task(
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<steward_core::ipc::TaskRecord, String> {
    let task = state
        .task_runtime
        .get_task(id)
        .await
        .ok_or_else(|| "Task not found".to_string())?;

    state
        .task_runtime
        .mark_cancelled(id, "Deleted via IPC")
        .await;

    Ok(task)
}

#[tauri::command]
pub async fn approve_task(
    state: State<'_, AppState>,
    id: Uuid,
    payload: ApproveTaskRequest,
) -> Result<steward_core::ipc::TaskRecord, String> {
    let task = state
        .task_runtime
        .get_task(id)
        .await
        .ok_or_else(|| "Task not found".to_string())?;

    if task.status != steward_core::task_runtime::TaskStatus::WaitingApproval {
        return Err(format!(
            "Task is not awaiting approval (current status: {:?})",
            task.status
        ));
    }

    if let Some(approval_id) = payload.approval_id {
        if let Some(ref pending) = task.pending_approval {
            if pending.id != approval_id {
                return Err("Approval ID mismatch".to_string());
            }
        }
    }

    let message = steward_core::channels::IncomingMessage::new(
        &task.route.channel,
        &task.route.user_id,
        "approval",
    )
    .with_owner_id(task.route.owner_id.clone())
    .with_sender_id(task.route.sender_id.clone())
    .with_thread(task.route.thread_id.clone());

    state
        .task_runtime
        .mark_running(&message, id)
        .await;

    let task = state
        .task_runtime
        .get_task(id)
        .await
        .ok_or_else(|| "Task not found after approval".to_string())?;

    Ok(task)
}

#[tauri::command]
pub async fn reject_task(
    state: State<'_, AppState>,
    id: Uuid,
    payload: RejectTaskRequest,
) -> Result<steward_core::ipc::TaskRecord, String> {
    let reason = payload.reason.unwrap_or_else(|| "Rejected via IPC".to_string());
    state.task_runtime.mark_rejected(id, reason).await;

    let task = state
        .task_runtime
        .get_task(id)
        .await
        .ok_or_else(|| "Task not found".to_string())?;

    Ok(task)
}

#[tauri::command]
pub async fn patch_task_mode(
    state: State<'_, AppState>,
    id: Uuid,
    payload: PatchTaskModeRequest,
) -> Result<steward_core::ipc::TaskRecord, String> {
    use steward_core::task_runtime::TaskMode;

    let mode = match payload.mode.to_lowercase().as_str() {
        "yolo" => TaskMode::Yolo,
        "ask" => TaskMode::Ask,
        _ => return Err(format!("Invalid mode: {}. Valid modes: 'ask', 'yolo'", payload.mode)),
    };

    let task = state
        .task_runtime
        .toggle_mode(id, mode)
        .await
        .ok_or_else(|| "Task not found".to_string())?;

    Ok(task)
}

// =============================================================================
// Workspace (4 commands)
// =============================================================================

#[tauri::command]
pub async fn index_workspace(
    state: State<'_, AppState>,
    payload: WorkspaceIndexRequest,
) -> Result<steward_core::ipc::WorkspaceIndexResponse, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let job_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let result = workspace.index_all().await;

    let (status, indexed_files, error) = match result {
        Ok(count) => ("completed".to_string(), count, None),
        Err(e) => ("failed".to_string(), 0, Some(e.to_string())),
    };

    Ok(steward_core::ipc::WorkspaceIndexResponse {
        job: steward_core::ipc::WorkspaceIndexJobResponse {
            id: job_id,
            path: payload.path,
            import_root: String::new(),
            manifest_path: String::new(),
            status,
            phase: if error.is_some() { "error".to_string() } else { "completed".to_string() },
            total_files: indexed_files,
            processed_files: indexed_files,
            indexed_files,
            skipped_files: 0,
            error,
            started_at: now,
            updated_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
        },
    })
}

#[tauri::command]
pub async fn get_workspace_index_job(
    _state: State<'_, AppState>,
    id: Uuid,
) -> Result<steward_core::ipc::WorkspaceIndexJobResponse, String> {
    Err(format!(
        "Job tracking by ID not fully implemented. \
         Call index_workspace to trigger indexing and get immediate results. \
         Job ID: {}",
        id
    ))
}

#[tauri::command]
pub async fn get_workspace_tree(
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<steward_core::ipc::WorkspaceTreeResponse, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let uri = path.unwrap_or_else(|| "memory://".to_string());
    let entries = workspace.list_tree(&uri).await
        .map_err(|e| e.to_string())?;

    Ok(steward_core::ipc::WorkspaceTreeResponse {
        path: uri,
        entries,
    })
}

#[tauri::command]
pub async fn search_workspace(
    state: State<'_, AppState>,
    payload: WorkspaceSearchRequest,
) -> Result<steward_core::ipc::WorkspaceSearchResponse, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let results = workspace
        .search(&payload.query, 20)
        .await
        .map_err(|e| e.to_string())?;

    let responses: Vec<steward_core::ipc::WorkspaceSearchResultResponse> = results
        .into_iter()
        .map(|r| steward_core::ipc::WorkspaceSearchResultResponse {
            document_id: r.document_id,
            document_path: r.document_path,
            chunk_id: r.chunk_id,
            content: r.content,
            score: r.score,
            fts_rank: r.fts_rank,
            vector_rank: r.vector_rank,
        })
        .collect();

    Ok(steward_core::ipc::WorkspaceSearchResponse { results: responses })
}

// =============================================================================
// Workspace Mounts (8 commands)
// =============================================================================

#[tauri::command]
pub async fn list_workspace_mounts(
    state: State<'_, AppState>,
) -> Result<steward_core::ipc::WorkspaceMountListResponse, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let mounts = workspace.list_mounts().await
        .map_err(|e| e.to_string())?;

    Ok(steward_core::ipc::WorkspaceMountListResponse { mounts })
}

#[tauri::command]
pub async fn create_workspace_mount(
    state: State<'_, AppState>,
    payload: CreateWorkspaceMountRequest,
) -> Result<steward_core::workspace::WorkspaceMountSummary, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let display_name = payload.display_name
        .unwrap_or_else(|| std::path::Path::new(&payload.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Mount")
            .to_string());

    let summary = workspace
        .create_mount(display_name, &payload.path, payload.bypass_write)
        .await
        .map_err(|e| e.to_string())?;

    Ok(summary)
}

#[tauri::command]
pub async fn get_workspace_mount(
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<steward_core::workspace::WorkspaceMountDetail, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let detail = workspace.get_mount(id).await
        .map_err(|e| e.to_string())?;

    Ok(detail)
}

#[tauri::command]
pub async fn get_workspace_mount_diff(
    state: State<'_, AppState>,
    id: Uuid,
    scope_path: Option<String>,
) -> Result<steward_core::workspace::WorkspaceMountDiff, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let diff = workspace.diff_mount(id, scope_path.as_deref()).await
        .map_err(|e| e.to_string())?;

    Ok(diff)
}

#[tauri::command]
pub async fn create_workspace_checkpoint(
    state: State<'_, AppState>,
    id: Uuid,
    payload: CreateWorkspaceCheckpointRequest,
) -> Result<steward_core::workspace::WorkspaceMountCheckpoint, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let created_by = payload.created_by.unwrap_or_else(|| "desktop".to_string());

    let checkpoint = workspace
        .create_checkpoint(
            id,
            payload.label,
            payload.summary,
            created_by,
            payload.is_auto,
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(checkpoint)
}

#[tauri::command]
pub async fn keep_workspace_mount(
    state: State<'_, AppState>,
    id: Uuid,
    payload: WorkspaceActionRequest,
) -> Result<steward_core::workspace::WorkspaceMountDetail, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let detail = workspace
        .keep_mount(id, payload.scope_path, payload.checkpoint_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(detail)
}

#[tauri::command]
pub async fn revert_workspace_mount(
    state: State<'_, AppState>,
    id: Uuid,
    payload: WorkspaceActionRequest,
) -> Result<steward_core::workspace::WorkspaceMountDetail, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let detail = workspace
        .revert_mount(id, payload.scope_path, payload.checkpoint_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(detail)
}

#[tauri::command]
pub async fn resolve_workspace_mount_conflict(
    state: State<'_, AppState>,
    id: Uuid,
    payload: ResolveWorkspaceConflictRequest,
) -> Result<steward_core::workspace::WorkspaceMountDetail, String> {
    let workspace = state
        .workspace
        .as_ref()
        .ok_or_else(|| "Workspace not available".to_string())?;

    let detail = workspace
        .resolve_mount_conflict(
            id,
            payload.path,
            payload.resolution,
            payload.renamed_copy_path,
            payload.merged_content,
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(detail)
}

// =============================================================================
// Workbench (1 command)
// =============================================================================

#[tauri::command]
pub async fn get_workbench_capabilities(
    state: State<'_, AppState>,
) -> Result<steward_core::ipc::WorkbenchCapabilitiesResponse, String> {
    let tool_count = state.tools.count();
    let active_tool_names = state.tools.list().await;

    Ok(steward_core::ipc::WorkbenchCapabilitiesResponse {
        workspace_available: state.workspace.is_some(),
        tool_count,
        dev_loaded_tools: active_tool_names,
        mcp_servers: Vec::new(),
    })
}
