use crate::account_manager;
use crate::config_manager;
use crate::mod_updater;
use crate::AppState;
use serde_json::Value;
use std::fs;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn get_accounts() -> Vec<Value> {
    account_manager::get_accounts()
}

#[tauri::command]
pub fn add_account(data: String) -> bool {
    if let Ok(json) = serde_json::from_str(&data) {
        account_manager::add_account(json)
    } else {
        false
    }
}

#[tauri::command]
pub fn delete_account(idx: usize) -> bool {
    account_manager::delete_account(idx)
}

#[tauri::command]
pub fn set_active_account(idx: usize) -> bool {
    account_manager::set_active_account(idx)
}

#[tauri::command]
pub fn get_active_account() -> Option<Value> {
    account_manager::get_active_account()
}

#[tauri::command]
pub fn save_yaml(filename: String, content: String) -> bool {
    // Simplified: just write to file in mod/data or similar
    // Python implementation used yaml_utils.save_yaml
    // We need to check where it saves.
    // Assuming it saves to mod/data/filename
    let path = config_manager::get_hidden_config_dir("data").join(filename);
    fs::write(path, content).is_ok()
}

#[tauri::command]
pub fn load_yaml(filename: String) -> Option<String> {
    let path = config_manager::get_hidden_config_dir("data").join(filename);
    fs::read_to_string(path).ok()
}

#[tauri::command]
pub fn check_resource_exists(state: State<AppState>, resource_path: String) -> Value {
    let (exists, abs_path) = state.resource_manager.exists_local(resource_path.clone());

    let downloading = !exists
        && (resource_path.starts_with("passionfruit/")
            || resource_path.starts_with("assets/passionfruit/"));

    serde_json::json!({
        "exists": exists,
        "downloading": downloading,
        "path": resource_path,
        "absPath": abs_path
    })
}

#[tauri::command]
pub fn get_resource_download_status(state: State<AppState>) -> Value {
    let status = state.resource_manager.get_status();
    serde_json::to_value(status).unwrap()
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    // 關閉所有視窗並退出應用
    app.exit(0);
}

#[tauri::command]
pub fn save_config_volume(data: String) -> bool {
    if let Ok(json) = serde_json::from_str::<Value>(&data) {
        config_manager::update_config_fields(json);
        true
    } else {
        false
    }
}

#[tauri::command]
pub fn get_config_volume(target_account: Option<Value>) -> Value {
    // Convert Value to String if it's an object, or handle it directly
    let target_str = if let Some(val) = target_account {
        serde_json::to_string(&val).ok()
    } else {
        None
    };
    config_manager::get_account_settings(target_str)
}

#[tauri::command]
pub fn save_report_faction_filter(faction: String, _target_account: Option<Value>) -> bool {
    // Simplified
    config_manager::update_config_fields(serde_json::json!({
        "report_faction_filter": faction
    }));
    true
}

#[tauri::command]
pub fn get_report_faction_filter(_target_account: Option<Value>) -> String {
    let config = config_manager::load_config();
    config
        .get("report_faction_filter")
        .and_then(|v| v.as_str())
        .unwrap_or("全部")
        .to_string()
}

use crate::updater;

#[tauri::command]
pub fn log_message(message: String) {
    println!("[Frontend Log] {}", message);
}

#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Value {
    let version = app.package_info().version.to_string();
    match updater::check_update(&version).await {
        Ok(info) => serde_json::to_value(info).unwrap(),
        Err(e) => serde_json::json!({ "error": e }),
    }
}

#[tauri::command]
pub async fn perform_update(app: AppHandle, url: String, filename: String) -> Result<(), String> {
    updater::download_and_install(app, &url, &filename).await
}

#[tauri::command]
pub async fn check_mod_update(app: AppHandle) -> Result<Value, String> {
    mod_updater::check_for_update(&app).await
}

#[tauri::command]
pub async fn download_mod_update(app: AppHandle, remote_ref: String) -> Result<Value, String> {
    mod_updater::download_and_install(&app, &remote_ref).await
}

#[tauri::command]
pub fn toggle_fullscreen(app: AppHandle) -> Result<bool, String> {
    if let Some(window) = app.get_webview_window("main") {
        let is_fullscreen = window
            .is_fullscreen()
            .map_err(|e| format!("Failed to get fullscreen state: {}", e))?;

        window
            .set_fullscreen(!is_fullscreen)
            .map_err(|e| format!("Failed to set fullscreen: {}", e))?;

        Ok(!is_fullscreen)
    } else {
        Err("Main window not found".to_string())
    }
}
