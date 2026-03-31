#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri_plugin_notification::NotificationExt;

struct ApiBase(String);

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
        .post(format!("{}/api/v0/workspace/index", api_base.0.trim_end_matches('/')))
        .json(&payload)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(ApiBase("http://127.0.0.1:8765".to_string()))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![notify, index_dropped_path])
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let app_handle = app.handle().clone();

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
