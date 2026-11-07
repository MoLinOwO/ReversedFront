use crate::account_manager;
use crate::config_manager;
use crate::host::DesktopHost;
use crate::mod_updater;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;

pub fn get_accounts() -> Vec<Value> {
    account_manager::get_accounts()
}

pub fn add_account(data: String) -> bool {
    if let Ok(json) = serde_json::from_str(&data) {
        account_manager::add_account(json)
    } else {
        false
    }
}

pub fn delete_account(idx: usize) -> bool {
    account_manager::delete_account(idx)
}

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

pub fn load_yaml(host: &DesktopHost, filename: String) -> Option<String> {
    // 儲存與載入共用相同白名單，避免兩個命令的路徑規則漂移。
    let filename = safe_data_filename(&filename)?;

    // 使用者資料優先，讓使用者可以自訂退出提示詞；沒有自訂檔時再讀取
    // 安裝包內的 mod/data 預設檔。這修正 Release 版只會拿到 fallback 的問題。
    let user_path = config_manager::get_hidden_config_dir("data").join(filename);
    if let Ok(content) = fs::read_to_string(user_path) {
        return Some(content);
    }

    let resource_path = host.resource_root.join("mod").join("data").join(filename);
    fs::read_to_string(resource_path).ok()
}

pub fn check_resource_exists(host: &DesktopHost, resource_path: String) -> Value {
    let (exists, abs_path) = host.resource_manager.exists_local(resource_path.clone());

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

pub async fn download_resource(host: &DesktopHost, resource_path: String) -> Result<Value, String> {
    let is_passionfruit = resource_path.starts_with("passionfruit/")
        || resource_path.starts_with("assets/passionfruit/");
    if !is_passionfruit {
        return Err("只允許下載 passionfruit 資源".to_string());
    }

    match host.resource_manager.get_or_fetch(&resource_path).await {
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

pub fn get_resource_download_status(host: &DesktopHost) -> Value {
    let status = host.resource_manager.get_status();
    serde_json::to_value(status).unwrap()
}

pub fn exit_app(host: &DesktopHost) -> Result<(), String> {
    host.request_exit()
}

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

pub fn save_config_volume(data: String) -> bool {
    if let Ok(json) = serde_json::from_str::<Value>(&data) {
        account_manager::update_account_settings(json)
    } else {
        false
    }
}

pub fn get_config_volume(target_account: Option<Value>) -> Value {
    account_manager::get_account_settings(target_account)
}

pub fn save_locale(locale: String) -> bool {
    account_manager::save_locale(locale)
}

pub fn get_locale() -> Option<String> {
    account_manager::get_locale()
}

pub fn save_report_faction_filter(faction: String, target_account: Option<Value>) -> bool {
    account_manager::update_account_settings(serde_json::json!({
        "report_faction_filter": faction,
        "target_account": target_account
    }))
}

pub fn get_report_faction_filter(target_account: Option<Value>) -> String {
    account_manager::get_account_settings(target_account)
        .get("report_faction_filter")
        .and_then(|v| v.as_str())
        .unwrap_or("全部")
        .to_string()
}

use crate::updater;

pub fn log_message(message: String) {
    println!("[Frontend Log] {}", message);
}

pub async fn check_for_updates() -> Value {
    match updater::check_update(env!("CARGO_PKG_VERSION")).await {
        Ok(info) => serde_json::to_value(info).unwrap(),
        Err(e) => serde_json::json!({ "error": e }),
    }
}

/// 一次查詢桌面版與 Mod 版本，交給前端顯示同一個更新通知。
pub async fn check_all_updates(host: &DesktopHost) -> Value {
    let app_result = updater::check_update(env!("CARGO_PKG_VERSION")).await;
    let mod_result = mod_updater::check_for_update(host).await;

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

pub async fn perform_update(
    host: &DesktopHost,
    url: String,
    filename: String,
) -> Result<(), String> {
    updater::download_and_install(host, &url, &filename).await
}

pub async fn check_mod_update(host: &DesktopHost) -> Result<Value, String> {
    mod_updater::check_for_update(host).await
}

pub async fn download_mod_update(
    host: &DesktopHost,
    remote_ref: String,
    download_url: Option<String>,
    filename: Option<String>,
) -> Result<Value, String> {
    mod_updater::download_and_install(
        host,
        &remote_ref,
        download_url.as_deref(),
        filename.as_deref(),
    )
    .await
}

pub async fn toggle_fullscreen(host: &DesktopHost) -> Result<bool, String> {
    host.toggle_fullscreen().await
}

#[derive(Deserialize)]
struct DataArgs {
    data: String,
}

#[derive(Deserialize)]
struct DeleteAccountArgs {
    idx: usize,
}

#[derive(Deserialize)]
struct SaveYamlArgs {
    filename: String,
    content: String,
}

#[derive(Deserialize)]
struct FilenameArgs {
    filename: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceArgs {
    resource_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetAccountArgs {
    target_account: Option<Value>,
}

#[derive(Deserialize)]
struct LocaleArgs {
    locale: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FactionArgs {
    faction: String,
    target_account: Option<Value>,
}

#[derive(Deserialize)]
struct MessageArgs {
    message: String,
}

#[derive(Deserialize)]
struct UpdateArgs {
    url: String,
    filename: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModUpdateArgs {
    remote_ref: String,
    download_url: Option<String>,
    filename: Option<String>,
}

#[derive(Deserialize)]
struct UrlArgs {
    url: String,
}

fn decode_args<T: DeserializeOwned>(command: &str, args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|error| format!("{command} 參數無效：{error}"))
}

/// The IPC boundary validates the caller; this validates command arguments.
pub async fn dispatch(host: &DesktopHost, command: &str, args: Value) -> Result<Value, String> {
    if !args.is_object() {
        return Err(format!("{command} 參數必須是物件"));
    }
    match command {
        "get_accounts" => Ok(serde_json::json!(get_accounts())),
        "add_account" => {
            let args: DataArgs = decode_args(command, args)?;
            Ok(serde_json::json!(add_account(args.data)))
        }
        "delete_account" => {
            let args: DeleteAccountArgs = decode_args(command, args)?;
            Ok(serde_json::json!(delete_account(args.idx)))
        }
        "get_active_account" => Ok(serde_json::json!(get_active_account())),
        "save_yaml" => {
            let args: SaveYamlArgs = decode_args(command, args)?;
            Ok(serde_json::json!(save_yaml(args.filename, args.content)))
        }
        "load_yaml" => {
            let args: FilenameArgs = decode_args(command, args)?;
            Ok(serde_json::json!(load_yaml(host, args.filename)))
        }
        "check_resource_exists" => {
            let args: ResourceArgs = decode_args(command, args)?;
            Ok(check_resource_exists(host, args.resource_path))
        }
        "download_resource" => {
            let args: ResourceArgs = decode_args(command, args)?;
            download_resource(host, args.resource_path).await
        }
        "get_resource_download_status" => Ok(get_resource_download_status(host)),
        "exit_app" => {
            exit_app(host)?;
            Ok(Value::Null)
        }
        "open_external_url" => {
            let args: UrlArgs = decode_args(command, args)?;
            open_external_url(args.url)?;
            Ok(Value::Null)
        }
        "save_config_volume" => {
            let args: DataArgs = decode_args(command, args)?;
            Ok(serde_json::json!(save_config_volume(args.data)))
        }
        "get_config_volume" => {
            let args: TargetAccountArgs = decode_args(command, args)?;
            Ok(get_config_volume(args.target_account))
        }
        "save_locale" => {
            let args: LocaleArgs = decode_args(command, args)?;
            Ok(serde_json::json!(save_locale(args.locale)))
        }
        "get_locale" => Ok(serde_json::json!(get_locale())),
        "save_report_faction_filter" => {
            let args: FactionArgs = decode_args(command, args)?;
            Ok(serde_json::json!(save_report_faction_filter(
                args.faction,
                args.target_account
            )))
        }
        "get_report_faction_filter" => {
            let args: TargetAccountArgs = decode_args(command, args)?;
            Ok(serde_json::json!(get_report_faction_filter(
                args.target_account
            )))
        }
        "log_message" => {
            let args: MessageArgs = decode_args(command, args)?;
            log_message(args.message);
            Ok(Value::Null)
        }
        "check_for_updates" => Ok(check_for_updates().await),
        "check_all_updates" => Ok(check_all_updates(host).await),
        "perform_update" => {
            let args: UpdateArgs = decode_args(command, args)?;
            perform_update(host, args.url, args.filename).await?;
            Ok(Value::Null)
        }
        "check_mod_update" => check_mod_update(host).await,
        "download_mod_update" => {
            let args: ModUpdateArgs = decode_args(command, args)?;
            download_mod_update(host, args.remote_ref, args.download_url, args.filename).await
        }
        "toggle_fullscreen" => toggle_fullscreen(host).await.map(Value::Bool),
        _ => Err(format!("未知的命令：{command}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn validates_command_argument_contracts() {
        let resource: ResourceArgs = decode_args(
            "download_resource",
            serde_json::json!({ "resourcePath": "passionfruit/test.png" }),
        )
        .unwrap();
        assert_eq!(resource.resource_path, "passionfruit/test.png");
        assert!(decode_args::<ResourceArgs>(
            "download_resource",
            serde_json::json!({ "resource_path": "passionfruit/test.png" })
        )
        .is_err());
        assert!(decode_args::<DataArgs>("add_account", serde_json::json!({ "data": {} })).is_err());
        assert!(decode_args::<DeleteAccountArgs>(
            "delete_account",
            serde_json::json!({ "idx": -1 })
        )
        .is_err());

        let target: TargetAccountArgs = decode_args(
            "get_config_volume",
            serde_json::json!({ "targetAccount": { "accountName": "test" } }),
        )
        .unwrap();
        assert_eq!(target.target_account.unwrap()["accountName"], "test");
    }

    #[test]
    fn accepts_api_and_archive_mod_update_arguments() {
        let remote_ref = "a".repeat(40);
        let api: ModUpdateArgs = decode_args(
            "download_mod_update",
            serde_json::json!({ "remoteRef": remote_ref }),
        )
        .unwrap();
        assert_eq!(api.remote_ref, remote_ref);
        assert!(api.download_url.is_none());
        assert!(api.filename.is_none());

        let archive: ModUpdateArgs = decode_args(
            "download_mod_update",
            serde_json::json!({
                "remoteRef": remote_ref,
                "downloadUrl": "https://example.test/mod.zip",
                "filename": "mod.zip"
            }),
        )
        .unwrap();
        assert_eq!(
            archive.download_url.as_deref(),
            Some("https://example.test/mod.zip")
        );
        assert_eq!(archive.filename.as_deref(), Some("mod.zip"));
        assert!(
            decode_args::<ModUpdateArgs>("download_mod_update", serde_json::json!({})).is_err()
        );
    }
}
