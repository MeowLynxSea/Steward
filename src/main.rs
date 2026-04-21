#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tauri_commands;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use steward_core::desktop_runtime::{AppState, TauriEventEmitterHandle};
use steward_core::runtime_events::RuntimeEventEmitter;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::RwLock;

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSColor, NSWindow};
#[cfg(target_os = "macos")]
use objc2_quartz_core::CALayer;

struct CodexLoginJobs(Arc<RwLock<HashMap<String, CodexLoginJob>>>);

/// Tauri event emitter that implements RuntimeEventEmitter.
/// Emits events to the frontend via Tauri's event system.
struct TauriEventEmitter {
    app: tauri::AppHandle,
    sequence: AtomicU64,
}

impl TauriEventEmitter {
    fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            sequence: AtomicU64::new(1),
        }
    }
}

impl RuntimeEventEmitter for TauriEventEmitter {
    fn emit_for_user(&self, _user_id: &str, event: steward_common::AppEvent) {
        let tauri_event_name = format!("session:{}", event.event_type());

        // Extract thread_id and payload data from AppEvent to match frontend StreamEnvelope format
        let thread_id: Option<String> = match &event {
            steward_common::AppEvent::Response { thread_id, .. } => Some(thread_id.clone()),
            steward_common::AppEvent::Reflection { thread_id, .. } => Some(thread_id.clone()),
            steward_common::AppEvent::ReflectionStatus { thread_id, .. } => Some(thread_id.clone()),
            steward_common::AppEvent::Thinking { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::ToolStarted { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::ToolCompleted { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::ToolResult { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::StreamChunk { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::Status { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::ApprovalNeeded { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::Error { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::ImageGenerated { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::Suggestions { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::TurnCost { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::ReasoningUpdate { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::TitleUpdated { thread_id, .. } => thread_id.clone(),
            steward_common::AppEvent::ContextStats { thread_id, .. } => thread_id.clone(),
            _ => None,
        };

        let payload_data = match &event {
            steward_common::AppEvent::Response { content, .. } => {
                serde_json::json!({ "content": content })
            }
            steward_common::AppEvent::Reflection {
                content,
                source,
                routine_name,
                assistant_message_id,
                ..
            } => {
                serde_json::json!({
                    "content": content,
                    "source": source,
                    "routine_name": routine_name,
                    "assistant_message_id": assistant_message_id
                })
            }
            steward_common::AppEvent::ReflectionStatus {
                status,
                source,
                routine_name,
                assistant_message_id,
                ..
            } => {
                serde_json::json!({
                    "status": status,
                    "source": source,
                    "routine_name": routine_name,
                    "assistant_message_id": assistant_message_id
                })
            }
            steward_common::AppEvent::Thinking {
                message,
                message_id,
                ..
            } => {
                serde_json::json!({ "message": message, "message_id": message_id })
            }
            steward_common::AppEvent::ToolStarted {
                name,
                tool_call_id,
                parameters,
                source,
                routine_name,
                assistant_message_id,
                ..
            } => {
                serde_json::json!({
                    "name": name,
                    "tool_call_id": tool_call_id,
                    "parameters": parameters,
                    "source": source,
                    "routine_name": routine_name,
                    "assistant_message_id": assistant_message_id
                })
            }
            steward_common::AppEvent::ToolCompleted {
                name,
                tool_call_id,
                success,
                error,
                parameters,
                source,
                routine_name,
                assistant_message_id,
                ..
            } => {
                serde_json::json!({
                    "name": name,
                    "tool_call_id": tool_call_id,
                    "success": success,
                    "error": error,
                    "parameters": parameters,
                    "source": source,
                    "routine_name": routine_name,
                    "assistant_message_id": assistant_message_id
                })
            }
            steward_common::AppEvent::ToolResult {
                name,
                tool_call_id,
                preview,
                source,
                routine_name,
                assistant_message_id,
                ..
            } => {
                serde_json::json!({
                    "name": name,
                    "tool_call_id": tool_call_id,
                    "preview": preview,
                    "source": source,
                    "routine_name": routine_name,
                    "assistant_message_id": assistant_message_id
                })
            }
            steward_common::AppEvent::StreamChunk { content, .. } => {
                serde_json::json!({ "content": content })
            }
            steward_common::AppEvent::Status { message, .. } => {
                serde_json::json!({ "message": message })
            }
            steward_common::AppEvent::JobStarted {
                job_id,
                title,
                browse_url,
            } => {
                serde_json::json!({ "job_id": job_id, "title": title, "browse_url": browse_url })
            }
            steward_common::AppEvent::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
                allow_always,
                ..
            } => {
                serde_json::json!({
                    "request_id": request_id,
                    "tool_name": tool_name,
                    "description": description,
                    "parameters": parameters,
                    "allow_always": allow_always
                })
            }
            steward_common::AppEvent::AuthRequired {
                extension_name,
                instructions,
                auth_url,
                setup_url,
            } => {
                serde_json::json!({
                    "extension_name": extension_name,
                    "instructions": instructions,
                    "auth_url": auth_url,
                    "setup_url": setup_url
                })
            }
            steward_common::AppEvent::AuthCompleted {
                extension_name,
                success,
                message,
            } => {
                serde_json::json!({
                    "extension_name": extension_name,
                    "success": success,
                    "message": message
                })
            }
            steward_common::AppEvent::Error { message, .. } => {
                serde_json::json!({ "message": message })
            }
            steward_common::AppEvent::Heartbeat => {
                serde_json::json!({})
            }
            steward_common::AppEvent::JobMessage {
                job_id,
                role,
                content,
            } => {
                serde_json::json!({ "job_id": job_id, "role": role, "content": content })
            }
            steward_common::AppEvent::JobToolUse {
                job_id,
                tool_name,
                input,
            } => {
                serde_json::json!({ "job_id": job_id, "tool_name": tool_name, "input": input })
            }
            steward_common::AppEvent::JobToolResult {
                job_id,
                tool_name,
                output,
            } => {
                serde_json::json!({ "job_id": job_id, "tool_name": tool_name, "output": output })
            }
            steward_common::AppEvent::JobStatus { job_id, message } => {
                serde_json::json!({ "job_id": job_id, "message": message })
            }
            steward_common::AppEvent::JobResult {
                job_id,
                status,
                session_id,
                fallback_deliverable,
            } => {
                serde_json::json!({
                    "job_id": job_id,
                    "status": status,
                    "session_id": session_id,
                    "fallback_deliverable": fallback_deliverable
                })
            }
            steward_common::AppEvent::ImageGenerated { data_url, path, .. } => {
                serde_json::json!({ "data_url": data_url, "path": path })
            }
            steward_common::AppEvent::Suggestions { suggestions, .. } => {
                serde_json::json!({ "suggestions": suggestions })
            }
            steward_common::AppEvent::TurnCost {
                input_tokens,
                output_tokens,
                cost_usd,
                ..
            } => {
                serde_json::json!({
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "cost_usd": cost_usd
                })
            }
            steward_common::AppEvent::ExtensionStatus {
                extension_name,
                status,
                message,
            } => {
                serde_json::json!({
                    "extension_name": extension_name,
                    "status": status,
                    "message": message
                })
            }
            steward_common::AppEvent::ReasoningUpdate {
                narrative,
                decisions,
                ..
            } => {
                serde_json::json!({ "narrative": narrative, "decisions": decisions })
            }
            steward_common::AppEvent::TitleUpdated {
                session_id,
                title,
                emoji,
                pending,
                ..
            } => {
                serde_json::json!({
                    "session_id": session_id,
                    "title": title,
                    "emoji": emoji,
                    "pending": pending
                })
            }
            steward_common::AppEvent::JobReasoning {
                job_id,
                narrative,
                decisions,
            } => {
                serde_json::json!({ "job_id": job_id, "narrative": narrative, "decisions": decisions })
            }
            steward_common::AppEvent::ContextStats { stats, .. } => {
                serde_json::json!({
                    "model_context_length": stats.model_context_length,
                    "system_prompt_tokens": stats.system_prompt_tokens,
                    "mcp_prompts_tokens": stats.mcp_prompts_tokens,
                    "skills_tokens": stats.skills_tokens,
                    "messages_tokens": stats.messages_tokens,
                    "compact_buffer_tokens": stats.compact_buffer_tokens,
                    "free_tokens": stats.free_tokens,
                })
            }
        };

        // Format to match frontend StreamEnvelope: { event, thread_id, payload, sequence, timestamp }
        let stream_envelope = serde_json::json!({
            "event": format!("session.{}", event.event_type()),
            "thread_id": thread_id.unwrap_or_default(),
            "payload": payload_data,
            "sequence": self.sequence.fetch_add(1, Ordering::Relaxed),
            "timestamp": Utc::now().to_rfc3339()
        });

        if let Err(e) = self.app.emit(&tauri_event_name, stream_envelope) {
            tracing::warn!("Failed to emit Tauri event {}: {}", tauri_event_name, e);
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CodexLoginJob {
    Pending {
        verification_uri: String,
        user_code: String,
    },
    Success,
    Error {
        message: String,
    },
}

#[derive(serde::Serialize)]
struct CodexLoginStartResponse {
    login_id: String,
    verification_uri: String,
    user_code: String,
}

#[cfg(target_os = "macos")]
fn apply_window_corner_radius(window: &tauri::WebviewWindow, radius: f64) -> tauri::Result<()> {
    let ns_window = window.ns_window()?;
    let ns_window = unsafe { &*(ns_window as *mut NSWindow) };

    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));

    if let Some(content_view) = ns_window.contentView() {
        content_view.setWantsLayer(true);

        let layer = match content_view.layer() {
            Some(layer) => layer,
            None => {
                let layer = CALayer::layer();
                content_view.setLayer(Some(&layer));
                layer
            }
        };

        layer.setMasksToBounds(true);
        layer.setCornerRadius(radius);
        layer.setOpaque(false);
        layer.setBackgroundColor(None);
    }

    Ok(())
}

#[tauri::command]
async fn notify(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_openai_codex_login(
    jobs: tauri::State<'_, CodexLoginJobs>,
) -> Result<CodexLoginStartResponse, String> {
    let config = steward_core::llm::OpenAiCodexConfig::default();
    let manager = steward_core::llm::OpenAiCodexSessionManager::new(config)
        .map_err(|error| error.to_string())?;
    let device_code = manager
        .request_device_code()
        .await
        .map_err(|error| error.to_string())?;
    let login_id = uuid::Uuid::new_v4().to_string();
    let verification_uri = device_code.verification_uri.clone();
    let user_code = device_code.user_code.clone();

    {
        let mut guard = jobs.0.write().await;
        guard.insert(
            login_id.clone(),
            CodexLoginJob::Pending {
                verification_uri: verification_uri.clone(),
                user_code: user_code.clone(),
            },
        );
    }

    let jobs_handle = Arc::clone(&jobs.inner().0);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        let result = complete_openai_codex_login(manager, &device_code).await;
        let mut guard = jobs_handle.write().await;
        guard.insert(
            login_id_for_task,
            match result {
                Ok(()) => CodexLoginJob::Success,
                Err(message) => CodexLoginJob::Error { message },
            },
        );
    });

    Ok(CodexLoginStartResponse {
        login_id,
        verification_uri,
        user_code,
    })
}

async fn complete_openai_codex_login(
    manager: steward_core::llm::OpenAiCodexSessionManager,
    device_code: &steward_core::llm::OpenAiCodexDeviceCode,
) -> Result<(), String> {
    manager
        .finish_device_code_login(device_code)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_openai_codex_login_status(
    login_id: String,
    jobs: tauri::State<'_, CodexLoginJobs>,
) -> Result<CodexLoginJob, String> {
    let guard = jobs.0.read().await;
    guard
        .get(&login_id)
        .cloned()
        .ok_or_else(|| format!("unknown Codex login id '{login_id}'"))
}

#[cfg(target_os = "macos")]
fn pick_directory_with_system_dialog() -> Result<Option<String>, String> {
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "try",
            "-e",
            "POSIX path of (choose folder with prompt \"Select a folder to allowlist\")",
            "-e",
            "on error number -128",
            "-e",
            "return \"\"",
            "-e",
            "end try",
        ])
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Ok(None)
    } else {
        Ok(Some(path))
    }
}

#[cfg(target_os = "linux")]
fn pick_directory_with_system_dialog() -> Result<Option<String>, String> {
    let output = std::process::Command::new("sh")
        .args([
            "-c",
            "zenity --file-selection --directory 2>/dev/null || true",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Ok(None)
    } else {
        Ok(Some(path))
    }
}

#[cfg(target_os = "windows")]
fn pick_directory_with_system_dialog() -> Result<Option<String>, String> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "[void][System.Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms'); \
             $dialog = New-Object System.Windows.Forms.FolderBrowserDialog; \
             if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { \
               Write-Output $dialog.SelectedPath \
             }",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Ok(None)
    } else {
        Ok(Some(path))
    }
}

#[tauri::command]
async fn pick_allowlist_directory() -> Result<Option<String>, String> {
    pick_directory_with_system_dialog()
}

#[tauri::command]
async fn path_is_directory(path: String) -> Result<bool, String> {
    Ok(std::path::Path::new(&path).is_dir())
}

#[derive(serde::Serialize)]
struct DroppedAttachmentFileResponse {
    filename: String,
    mime_type: String,
    data_base64: String,
    size_bytes: u64,
}

#[tauri::command]
async fn read_dropped_attachment_file(
    path: String,
) -> Result<DroppedAttachmentFileResponse, String> {
    let file_path = Path::new(&path);
    if !file_path.is_file() {
        return Err("Dropped path is not a file".to_string());
    }

    let bytes = tokio::fs::read(file_path)
        .await
        .map_err(|error| format!("Failed to read dropped file: {error}"))?;
    let filename = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("attachment")
        .to_string();
    let mime_type = mime_guess::from_path(file_path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();

    use base64::Engine as _;

    Ok(DroppedAttachmentFileResponse {
        filename,
        mime_type,
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes.as_slice()),
        size_bytes: bytes.len() as u64,
    })
}

fn main() {
    tauri::Builder::default()
        .manage(CodexLoginJobs(Arc::new(RwLock::new(HashMap::new()))))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            notify,
            start_openai_codex_login,
            get_openai_codex_login_status,
            pick_allowlist_directory,
            path_is_directory,
            read_dropped_attachment_file,
            tauri_commands::get_settings,
            tauri_commands::patch_settings,
            tauri_commands::list_sessions,
            tauri_commands::create_session,
            tauri_commands::get_session,
            tauri_commands::get_session_runtime_status,
            tauri_commands::interrupt_session,
            tauri_commands::get_reflection_details,
            tauri_commands::delete_session,
            tauri_commands::send_session_message,
            tauri_commands::sheer_session_message,
            tauri_commands::queue_session_message,
            tauri_commands::list_tasks,
            tauri_commands::get_task,
            tauri_commands::delete_task,
            tauri_commands::cancel_task,
            tauri_commands::approve_task,
            tauri_commands::reject_task,
            tauri_commands::patch_task_mode,
            tauri_commands::get_workspace_tree,
            tauri_commands::get_workspace_document,
            tauri_commands::search_workspace,
            tauri_commands::list_memory_sidebar,
            tauri_commands::get_memory_node,
            tauri_commands::search_memory_graph,
            tauri_commands::list_memory_timeline,
            tauri_commands::list_memory_reviews,
            tauri_commands::apply_memory_review,
            tauri_commands::rollback_memory_changeset,
            tauri_commands::get_memory_versions,
            tauri_commands::list_workspace_allowlists,
            tauri_commands::create_workspace_allowlist,
            tauri_commands::get_workspace_allowlist,
            tauri_commands::get_workspace_allowlist_file,
            tauri_commands::get_workspace_allowlist_diff,
            tauri_commands::create_workspace_checkpoint,
            tauri_commands::delete_workspace_checkpoint,
            tauri_commands::delete_workspace_allowlist,
            tauri_commands::list_workspace_allowlist_checkpoints,
            tauri_commands::get_workspace_allowlist_history,
            tauri_commands::write_workspace_file,
            tauri_commands::delete_workspace_file,
            tauri_commands::keep_workspace_allowlist,
            tauri_commands::revert_workspace_allowlist,
            tauri_commands::restore_workspace_allowlist,
            tauri_commands::set_workspace_allowlist_baseline,
            tauri_commands::refresh_workspace_allowlist,
            tauri_commands::resolve_workspace_allowlist_conflict,
            tauri_commands::get_workbench_capabilities,
            tauri_commands::list_mcp_servers,
            tauri_commands::upsert_mcp_server,
            tauri_commands::delete_mcp_server,
            tauri_commands::test_mcp_server,
            tauri_commands::begin_mcp_auth,
            tauri_commands::finish_mcp_auth,
            tauri_commands::list_mcp_tools,
            tauri_commands::list_mcp_resources,
            tauri_commands::read_mcp_resource,
            tauri_commands::save_mcp_resource_snapshot,
            tauri_commands::add_mcp_resource_to_thread_context,
            tauri_commands::list_mcp_resource_templates,
            tauri_commands::subscribe_mcp_resource,
            tauri_commands::unsubscribe_mcp_resource,
            tauri_commands::list_mcp_prompts,
            tauri_commands::get_mcp_prompt,
            tauri_commands::complete_mcp_argument,
            tauri_commands::get_mcp_roots,
            tauri_commands::set_mcp_roots,
            tauri_commands::respond_mcp_sampling,
            tauri_commands::respond_mcp_elicitation,
            tauri_commands::list_mcp_activity
        ])
        .setup(|app| {
            let tauri_emitter: Option<TauriEventEmitterHandle> = {
                let emitter = TauriEventEmitter::new(app.handle().clone());
                Some(Arc::new(emitter) as TauriEventEmitterHandle)
            };
            let app_state: AppState = tauri::async_runtime::block_on(async {
                steward_core::desktop_runtime::start_embedded_runtime(tauri_emitter)
                    .await
                    .map_err(|error| {
                        std::io::Error::other(format!(
                            "failed to start embedded desktop runtime: {error}"
                        ))
                    })
            })?;
            app.manage(app_state);

            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let app_handle = app.handle().clone();

            #[cfg(target_os = "macos")]
            {
                let main_window = app.handle().get_webview_window("main");
                app.handle().run_on_main_thread(move || {
                    if let Some(window) = &main_window {
                        let _ = apply_window_corner_radius(window, 18.0);
                    }
                })?;
            }

            TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(move |_, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app_handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app_handle.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Steward desktop shell");
}
