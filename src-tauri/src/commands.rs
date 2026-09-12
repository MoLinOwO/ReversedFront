use crate::account_manager;
use crate::config_manager;
use crate::mod_updater;
use crate::AppState;
use serde_json::Value;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;
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
pub fn get_active_account() -> Option<Value> {
    account_manager::get_active_account()
}

#[cfg(target_os = "macos")]
fn stable_account_hash(account: &str, salt: &str) -> u64 {
    // 使用固定 FNV-1a，確保不同進程／重新啟動後仍會得到同一個 session 目錄。
    let mut hash = 0xcbf29ce484222325u64;
    for byte in format!("{salt}:{account}").as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(target_os = "macos")]
pub(crate) fn process_data_store_identifier(seed: &str) -> [u8; 16] {
    let first = stable_account_hash(seed, "ReversedFront-macos-1").to_le_bytes();
    let second = stable_account_hash(seed, "ReversedFront-macos-2").to_le_bytes();
    let mut identifier = [0u8; 16];
    identifier[..8].copy_from_slice(&first);
    identifier[8..].copy_from_slice(&second);
    identifier
}

#[tauri::command]
pub fn save_yaml(filename: String, content: String) -> bool {
    let Some(filename) = safe_data_filename(&filename) else {
        return false;
    };
    let path = config_manager::get_hidden_config_dir("data").join(filename);
    fs::write(path, content).is_ok()
}

fn safe_data_filename(filename: &str) -> Option<&str> {
    let requested = Path::new(filename);
    let valid_extension = requested
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "yaml" | "yml" | "json"
            )
        });
    let is_single_relative_file = !requested.is_absolute()
        && requested
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && requested
            .parent()
            .is_none_or(|parent| parent.as_os_str().is_empty());

    (valid_extension && is_single_relative_file).then_some(filename)
}

#[tauri::command]
pub fn load_yaml(app: AppHandle, filename: String) -> Option<String> {
    // 儲存與載入共用相同白名單，避免兩個命令的路徑規則漂移。
    let filename = safe_data_filename(&filename)?;

    // 使用者資料優先，讓使用者可以自訂退出提示詞；沒有自訂檔時再讀取
    // 安裝包內的 mod/data 預設檔。這修正 Release 版只會拿到 fallback 的問題。
    let user_path = config_manager::get_hidden_config_dir("data").join(filename);
    if let Ok(content) = fs::read_to_string(user_path) {
        return Some(content);
    }

    let resource_path = app
        .path()
        .resource_dir()
        .ok()
        .map(|root| root.join("mod").join("data").join(filename))?;
    fs::read_to_string(resource_path).ok()
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
pub async fn download_resource(
    state: State<'_, AppState>,
    resource_path: String,
) -> Result<Value, String> {
    let is_passionfruit = resource_path.starts_with("passionfruit/")
        || resource_path.starts_with("assets/passionfruit/");
    if !is_passionfruit {
        return Err("只允許下載 passionfruit 資源".to_string());
    }

    match state.resource_manager.get_or_fetch(&resource_path).await {
        Ok(Some(response)) => Ok(serde_json::json!({
            "exists": true,
            "downloaded": true,
            "path": response.local_path,
            "absPath": response.abs_path
        })),
        Ok(None) => Err("找不到資源".to_string()),
        Err(error) => Err(format!("資源下載失敗：{}", error)),
    }
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
pub fn open_external_url(url: String) -> Result<(), String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "無效的網址".to_string())?;
    if parsed.scheme() != "https" {
        return Err("只允許透過 HTTPS 開啟外部網址".to_string());
    }

    let normalized_url = parsed.as_str();
    let result = if cfg!(target_os = "windows") {
        Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", normalized_url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(normalized_url).spawn()
    } else if cfg!(target_os = "linux") {
        Command::new("xdg-open").arg(normalized_url).spawn()
    } else {
        return Err("目前平台不支援開啟外部網址".to_string());
    };

    result
        .map(|_| ())
        .map_err(|error| format!("無法啟動預設瀏覽器：{error}"))
}

#[tauri::command]
pub fn save_config_volume(data: String) -> bool {
    if let Ok(json) = serde_json::from_str::<Value>(&data) {
        account_manager::update_account_settings(json)
    } else {
        false
    }
}

#[tauri::command]
pub fn get_config_volume(target_account: Option<Value>) -> Value {
    account_manager::get_account_settings(target_account)
}

#[tauri::command]
pub fn save_locale(locale: String) -> bool {
    account_manager::save_locale(locale)
}

#[tauri::command]
pub fn get_locale() -> Option<String> {
    account_manager::get_locale()
}

#[tauri::command]
pub fn save_report_faction_filter(faction: String, target_account: Option<Value>) -> bool {
    account_manager::update_account_settings(serde_json::json!({
        "report_faction_filter": faction,
        "target_account": target_account
    }))
}

#[tauri::command]
pub fn get_report_faction_filter(target_account: Option<Value>) -> String {
    account_manager::get_account_settings(target_account)
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

/// 一次查詢桌面版與 Mod 版本，交給前端顯示同一個更新通知。
#[tauri::command]
pub async fn check_all_updates(app: AppHandle) -> Value {
    let version = app.package_info().version.to_string();
    let app_result = updater::check_update(&version).await;
    let mod_result = mod_updater::check_for_update(&app).await;

    let app_update = match app_result {
        Ok(info) => serde_json::json!({
            "ok": true,
            "info": info
        }),
        Err(error) => serde_json::json!({
            "ok": false,
            "error": error
        }),
    };
    let mod_update = match mod_result {
        Ok(info) => serde_json::json!({
            "ok": true,
            "info": info
        }),
        Err(error) => serde_json::json!({
            "ok": false,
            "error": error
        }),
    };

    serde_json::json!({
        "app": app_update,
        "mod": mod_update
    })
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
pub async fn download_mod_update(
    app: AppHandle,
    remote_ref: String,
    download_url: Option<String>,
    filename: Option<String>,
) -> Result<Value, String> {
    mod_updater::download_and_install(
        &app,
        &remote_ref,
        download_url.as_deref(),
        filename.as_deref(),
    )
    .await
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

#[cfg(test)]
mod tests {
    use super::safe_data_filename;

    #[test]
    fn accepts_supported_data_files() {
        assert_eq!(
            safe_data_filename("exit_prompts.yaml"),
            Some("exit_prompts.yaml")
        );
        assert_eq!(safe_data_filename("routes.JSON"), Some("routes.JSON"));
    }

    #[test]
    fn rejects_paths_and_unsupported_extensions() {
        assert_eq!(safe_data_filename("../accounts.db"), None);
        assert_eq!(safe_data_filename("folder/data.yaml"), None);
        assert_eq!(safe_data_filename("script.js"), None);
        assert_eq!(safe_data_filename(""), None);
    }
}
