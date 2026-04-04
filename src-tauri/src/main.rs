#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::Arc;

mod commands;

use ironclaw::desktop_runtime::TauriEventEmitterHandle;
use crate::commands::{
    approve_task, create_session, create_workspace_checkpoint, create_workspace_mount,
    delete_session, delete_task, get_session, get_settings, get_task, get_workbench_capabilities,
    get_workspace_index_job, get_workspace_mount, get_workspace_mount_diff, get_workspace_tree,
    index_workspace, keep_workspace_mount, list_sessions, list_tasks, list_workspace_mounts,
    patch_settings, patch_task_mode, reject_task, resolve_workspace_mount_conflict,
    revert_workspace_mount, search_workspace, send_session_message,
};
use ironclaw::ipc::AppState;
use ironclaw::llm::{OpenAiCodexConfig, OpenAiCodexDeviceCode, OpenAiCodexSessionManager};
use ironclaw::runtime_events::RuntimeEventEmitter;
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
}

impl TauriEventEmitter {
    fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl RuntimeEventEmitter for TauriEventEmitter {
    fn emit_for_user(&self, user_id: &str, event: ironclaw_common::AppEvent) {
        // Map AppEvent type to Tauri event name
        let tauri_event_name = format!("session:{}", event.event_type());
        let payload = serde_json::json!({
            "user_id": user_id,
            "event": event,
        });
        if let Err(e) = self.app.emit(&tauri_event_name, payload) {
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
    let config = OpenAiCodexConfig::default();
    let manager = OpenAiCodexSessionManager::new(config).map_err(|error| error.to_string())?;
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
    manager: OpenAiCodexSessionManager,
    device_code: &OpenAiCodexDeviceCode,
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
            "POSIX path of (choose folder with prompt \"Select a folder to mount\")",
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
        .args(["-c", "zenity --file-selection --directory 2>/dev/null || true"])
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
async fn pick_mount_directory() -> Result<Option<String>, String> {
    pick_directory_with_system_dialog()
}

fn main() {
    tauri::Builder::default()
        .manage(CodexLoginJobs(Arc::new(RwLock::new(HashMap::new()))))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            notify,
            start_openai_codex_login,
            get_openai_codex_login_status,
            pick_mount_directory,
            get_settings,
            patch_settings,
            list_sessions,
            create_session,
            get_session,
            delete_session,
            send_session_message,
            list_tasks,
            get_task,
            delete_task,
            approve_task,
            reject_task,
            patch_task_mode,
            index_workspace,
            get_workspace_index_job,
            get_workspace_tree,
            search_workspace,
            list_workspace_mounts,
            create_workspace_mount,
            get_workspace_mount,
            get_workspace_mount_diff,
            create_workspace_checkpoint,
            keep_workspace_mount,
            revert_workspace_mount,
            resolve_workspace_mount_conflict,
            get_workbench_capabilities
        ])
        .setup(|app| {
            let tauri_emitter: Option<TauriEventEmitterHandle> = {
                let emitter = TauriEventEmitter::new(app.handle().clone());
                Some(Arc::new(emitter) as TauriEventEmitterHandle)
            };
            let app_state: AppState = tauri::async_runtime::block_on(async {
                ironclaw::desktop_runtime::start_embedded_runtime(tauri_emitter)
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
        .expect("failed to run IronCowork desktop shell");
}
