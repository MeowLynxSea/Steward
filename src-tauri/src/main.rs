#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::Arc;

use ironclaw::api::DEFAULT_API_PORT;
use ironclaw::llm::{OpenAiCodexConfig, OpenAiCodexDeviceCode, OpenAiCodexSessionManager};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;
use tokio::sync::RwLock;

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSColor, NSWindow};
#[cfg(target_os = "macos")]
use objc2_quartz_core::CALayer;

struct ApiBase(String);
struct CodexLoginJobs(Arc<RwLock<HashMap<String, CodexLoginJob>>>);

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
async fn index_dropped_path(
    path: String,
    api_base: tauri::State<'_, ApiBase>,
) -> Result<(), String> {
    let payload = serde_json::json!({ "path": path });
    reqwest::Client::new()
        .post(format!(
            "{}/api/v0/workspace/index",
            api_base.0.trim_end_matches('/')
        ))
        .json(&payload)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    Ok(())
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
        .manage(ApiBase(format!("http://127.0.0.1:{DEFAULT_API_PORT}")))
        .manage(CodexLoginJobs(Arc::new(RwLock::new(HashMap::new()))))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            notify,
            index_dropped_path,
            start_openai_codex_login,
            get_openai_codex_login_status,
            pick_mount_directory
        ])
        .setup(|app| {
            tauri::async_runtime::block_on(async {
                ironclaw::desktop_runtime::start_embedded_runtime(DEFAULT_API_PORT)
                    .await
                    .map_err(|error| {
                        std::io::Error::other(format!(
                            "failed to start embedded desktop runtime: {error}"
                        ))
                    })
            })?;

            if let Some(window) = app.get_webview_window("main") {
                let api_base = format!("http://127.0.0.1:{DEFAULT_API_PORT}/api/v0");
                let escaped = serde_json::to_string(&api_base)?;
                window.eval(&format!("window.__IRONCOWORK_API_BASE__ = {escaped};"))?;
            }

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
