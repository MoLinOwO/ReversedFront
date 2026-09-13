pub mod account_manager;
pub mod commands;
pub mod config_manager;
pub mod mod_updater;
pub mod resource_manager;
pub mod updater;

use fs2::FileExt;
use resource_manager::ResourceManager;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use warp::{http::Response, http::StatusCode, Filter};

pub struct AppState {
    pub resource_manager: Arc<ResourceManager>,
    pub(crate) app_data_dir: PathBuf,
    pub(crate) process_slot: usize,
    // 持有期間即代表這個跨程序槽位仍被使用；File drop 時 OS 會釋放鎖。
    _process_slot_lock: fs::File,
}

pub(crate) const MAX_PROCESS_SLOTS: usize = 64;

pub(crate) fn update_lock_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(".reversedfront-update.lock")
}

fn lock_is_busy(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || cfg!(target_os = "windows") && matches!(error.raw_os_error(), Some(32 | 33))
}

/// 啟動時確認是否有另一個程序正在進行更新。
/// 可取得鎖時立即釋放，讓正常啟動不會永久佔用更新鎖。
pub(crate) fn update_is_in_progress(app_data_dir: &Path) -> io::Result<bool> {
    fs::create_dir_all(app_data_dir)?;
    let lock_path = update_lock_path(app_data_dir);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;

    match file.try_lock_exclusive() {
        Ok(()) => Ok(false),
        Err(error) if lock_is_busy(&error) => Ok(true),
        Err(error) => Err(error),
    }
}

/// 檢查除了目前程序以外，是否還有 ReversedFront 正在使用程序槽位。
pub(crate) fn has_other_processes(app_data_dir: &Path, current_slot: usize) -> io::Result<bool> {
    let lock_dir = app_data_dir.join("process-locks");
    if !lock_dir.exists() {
        return Ok(false);
    }

    for slot in 0..MAX_PROCESS_SLOTS {
        if slot == current_slot {
            continue;
        }

        let path = lock_dir.join(format!("slot-{slot}.lock"));
        if !path.exists() {
            continue;
        }

        let file = OpenOptions::new().read(true).write(true).open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(error) if lock_is_busy(&error) => return Ok(true),
            Err(error) => return Err(error),
        }
    }

    Ok(false)
}

fn acquire_process_slot(app_data_dir: &Path) -> io::Result<(usize, fs::File)> {
    let lock_dir = app_data_dir.join("process-locks");
    fs::create_dir_all(&lock_dir)?;

    for slot in 0..MAX_PROCESS_SLOTS {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_dir.join(format!("slot-{slot}.lock")))?;
        match file.try_lock_exclusive() {
            Ok(()) => return Ok((slot, file)),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || cfg!(target_os = "windows") && error.raw_os_error() == Some(33) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "ReversedFront process slots are exhausted",
    ))
}

fn directory_is_writable(path: &Path) -> bool {
    if fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(format!(".rf-write-probe-{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn runtime_storage_root(web_root: &Path, app_data_dir: &Path) -> PathBuf {
    if cfg!(debug_assertions) {
        return web_root.to_path_buf();
    }

    // Windows 可攜版優先沿用「執行檔旁」的資料配置；標準安裝若目錄
    // 不可寫則退回 AppData。macOS app bundle 與 Linux AppImage/deb 不應
    // 寫入程式本體，因此一律使用使用者資料目錄。
    if cfg!(target_os = "windows") {
        if let Some(executable_dir) = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
        {
            if directory_is_writable(&executable_dir) {
                return executable_dir;
            }
        }
    }

    app_data_dir.join("runtime")
}

// 提供靜態前端檔案
async fn handle_static_file(
    path: warp::path::Tail,
    resource_base: PathBuf,
    mod_update_root: PathBuf,
) -> Result<impl warp::Reply, warp::Rejection> {
    let path_str = path.as_str();

    // 處理根路徑
    let relative_path = path_str.trim_start_matches('/');
    if relative_path.split('/').any(|part| part == "..") || relative_path.contains('\\') {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Vec::new())
            .unwrap());
    }

    let (file_path, allowed_root) = if relative_path.is_empty() {
        (resource_base.join("index.html"), resource_base.clone())
    } else if let Some(mod_relative) = relative_path.strip_prefix("mod/") {
        let update_path = mod_update_root.join(mod_relative);
        if update_path.exists() {
            (update_path, mod_update_root.clone())
        } else {
            (resource_base.join(relative_path), resource_base.clone())
        }
    } else {
        (resource_base.join(relative_path), resource_base.clone())
    };

    // 安全檢查：防止路徑遍歷
    if !file_path.starts_with(&allowed_root) {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Vec::new())
            .unwrap());
    }

    match fs::read(&file_path) {
        Ok(data) => {
            let mime_type = match file_path.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "application/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("json") => "application/json",
                Some("png") => "image/png",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                Some("gif") => "image/gif",
                Some("svg") => "image/svg+xml",
                Some("ico") => "image/x-icon",
                Some("mp3") => "audio/mpeg",
                Some("mp4") => "video/mp4",
                Some("webm") => "video/webm",
                Some("yaml") | Some("yml") => "text/yaml",
                _ => "application/octet-stream",
            };

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime_type)
                .header("Cache-Control", "no-cache")
                .body(data)
                .unwrap())
        }
        // React Router uses browser history for pages such as
        // /users/log_in. Return the SPA entry for extension-less routes so
        // navigating from the splash screen does not become a false 404.
        Err(_) if file_path.extension().is_none() => {
            match fs::read(resource_base.join("index.html")) {
                Ok(data) => Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/html; charset=utf-8")
                    .header("Cache-Control", "no-cache")
                    .body(data)
                    .unwrap()),
                Err(_) => Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Vec::new())
                    .unwrap()),
            }
        }
        Err(_) => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Vec::new())
            .unwrap()),
    }
}

async fn handle_passionfruit_request(
    path: warp::path::Tail,
    resource_manager: Arc<ResourceManager>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let path_str = path.as_str();
    let resource_key = path_str.to_string();
    let decoded_key = urlencoding::decode(&resource_key)
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string();

    // 將 HTTP /passionfruit/... 的路徑統一映射成 ResourceManager 內部使用的
    // "passionfruit/..." key，這樣才能：
    // 1) 實際把檔案下載到 passionfruit 子目錄底下
    // 2) 與 check_resource_exists("passionfruit/...") 的路徑一致
    let full_key = format!("passionfruit/{}", decoded_key);

    let response = match resource_manager.get_or_fetch(&full_key).await {
        Ok(Some(res)) => {
            let mime_type = match res.abs_path.extension().and_then(|e| e.to_str()) {
                Some("png") => "image/png",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                Some("gif") => "image/gif",
                Some("mp4") => "video/mp4",
                Some("webm") => "video/webm",
                Some("mp3") => "audio/mpeg",
                Some("wav") => "audio/wav",
                _ => "application/octet-stream",
            };

            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime_type)
                .body(res.data)
        }
        Ok(None) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Vec::new()),
        Err(e) => {
            eprintln!(
                "[HTTP] Failed to serve passionfruit resource {}: {}",
                decoded_key, e
            );
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Vec::new())
        }
    };

    match response {
        Ok(resp) => Ok(resp),
        Err(e) => {
            eprintln!("[HTTP] Failed to build response {}: {}", decoded_key, e);
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Vec::new())
                .unwrap())
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // 啟動時嘗試清理舊的安裝程式檔案
            crate::updater::cleanup_old_installers();

            // 開發 / 生產共用：決定前端資源根目錄
            let web_root = if cfg!(debug_assertions) {
                // 開發模式：使用專案目錄下的 web/
                let mut project_root =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                if project_root.ends_with("src-tauri") {
                    project_root.pop();
                }
                project_root.join("web")
            } else {
                // 生產模式：使用 Tauri 的資源目錄
                app.path()
                    .resource_dir()
                    .expect("Failed to get resource directory")
            };

            println!("Web root directory: {:?}\n", web_root);

            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get application data directory");

            if update_is_in_progress(&app_data_dir)? {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "ReversedFront 正在更新，請稍後再啟動",
                )
                .into());
            }

            let (process_slot, process_slot_lock) = acquire_process_slot(&app_data_dir)?;
            let resource_storage_root = runtime_storage_root(&web_root, &app_data_dir);
            config_manager::set_resource_base_path(resource_storage_root);
            config_manager::set_user_data_base_path(app_data_dir.clone());
            // Mod 更新檔案與 update.json 都和安裝包內的前端資源放在一起。
            // 這樣載入來源、更新內容與版本標記不會分散在 AppData。
            let mod_update_root = mod_updater::update_root(app.handle())
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
            let _ = fs::create_dir_all(mod_update_root.join("js"));
            let _ = fs::create_dir_all(mod_update_root.join("data"));
            if let Err(error) = mod_updater::migrate_legacy_update_root(app.handle()) {
                eprintln!("Unable to migrate legacy Mod data: {error}");
            }

            // Ensure config directory exists
            let config_dir = config_manager::get_hidden_config_dir("data");
            if !config_dir.exists() {
                let _ = fs::create_dir_all(&config_dir);
            }

            // Dynamically allow access to the passionfruit directory
            let resource_dir = config_manager::get_hidden_config_dir("passionfruit");
            if !resource_dir.exists() {
                let _ = fs::create_dir_all(&resource_dir);
            }

            let resource_manager = ResourceManager::new();

            // 每個 ReversedFront 程序使用獨立的 loopback port，避免多開時
            // 第二個程序綁定失敗或誤載入第一個程序的前端資源。
            let server_resource_manager = resource_manager.clone();
            let resource_manager_filter = warp::any().map(move || server_resource_manager.clone());
            let resource_manager_filter_status = resource_manager_filter.clone();

            let status_route = warp::path("status")
                .and(resource_manager_filter_status)
                .map(|rm: Arc<ResourceManager>| {
                    let status = rm.get_status();
                    warp::reply::json(&status)
                });

            let passionfruit_route = warp::path("passionfruit")
                .and(warp::path::tail())
                .and(resource_manager_filter.clone())
                .and_then(handle_passionfruit_request);

            // 部分新版素材路徑帶有 assets/ 前綴，與 builtinAssets
            // 的 passionfruit/ 路徑使用同一個快取與下載器。
            let assets_passionfruit_route = warp::path("assets")
                .and(warp::path("passionfruit"))
                .and(warp::path::tail())
                .and(resource_manager_filter)
                .and_then(handle_passionfruit_request);

            let static_route = warp::path::tail().and_then(move |path: warp::path::Tail| {
                let web_root = web_root.clone();
                let mod_update_root = mod_update_root.clone();
                async move { handle_static_file(path, web_root, mod_update_root).await }
            });

            let routes = status_route
                .or(passionfruit_route)
                .or(assets_passionfruit_route)
                .or(static_route);
            // hyper 會在 bind 時取得目前的 Tokio reactor，因此即使回傳的
            // server 日後才 spawn，綁定本身也必須在 Tauri async runtime 內執行。
            let (server_address, server) = tauri::async_runtime::block_on(async move {
                warp::serve(routes).bind_ephemeral(([127, 0, 0, 1], 0))
            });
            let server_base_url = format!("http://{}", server_address);

            app.manage(AppState {
                resource_manager: resource_manager.clone(),
                app_data_dir: app_data_dir.clone(),
                process_slot,
                _process_slot_lock: process_slot_lock,
            });

            println!("=== Starting HTTP Server ===");
            println!("Server: {server_base_url}/");
            tauri::async_runtime::spawn(server);

            // 主視窗也使用每程序獨立的 WebView 儲存區，讓 cookie、
            // localStorage 與登入 session 不會在多個程式程序之間互相覆蓋。
            let session_dir = app_data_dir
                .join("account-slots")
                .join(process_slot.to_string());
            fs::create_dir_all(&session_dir)?;
            let webview_url = WebviewUrl::External(
                format!("{server_base_url}/?rf_account_slot={process_slot}").parse()?,
            );
            let builder = WebviewWindowBuilder::new(app, "main", webview_url)
                .title("ReversedFront")
                .inner_size(1280.0, 720.0)
                .min_inner_size(800.0, 600.0)
                .resizable(true)
                .fullscreen(false)
                .data_directory(session_dir);

            #[cfg(target_os = "macos")]
            let builder = builder.data_store_identifier(commands::process_data_store_identifier(
                &format!("main-slot:{process_slot}"),
            ));

            builder.build()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_accounts,
            commands::add_account,
            commands::delete_account,
            commands::get_active_account,
            commands::save_yaml,
            commands::load_yaml,
            commands::check_resource_exists,
            commands::download_resource,
            commands::get_resource_download_status,
            commands::exit_app,
            commands::open_external_url,
            commands::save_config_volume,
            commands::save_locale,
            commands::get_locale,
            commands::save_report_faction_filter,
            commands::get_report_faction_filter,
            commands::get_config_volume,
            commands::log_message,
            commands::check_for_updates,
            commands::check_all_updates,
            commands::perform_update,
            commands::check_mod_update,
            commands::download_mod_update,
            commands::toggle_fullscreen
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{acquire_process_slot, has_other_processes};

    #[test]
    fn simultaneous_instances_receive_distinct_slots() {
        let directory =
            std::env::temp_dir().join(format!("reversedfront-slot-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        let (first_slot, first_lock) = acquire_process_slot(&directory).unwrap();
        let (second_slot, second_lock) = acquire_process_slot(&directory).unwrap();

        assert_eq!(first_slot, 0);
        assert_eq!(second_slot, 1);
        assert!(has_other_processes(&directory, first_slot).unwrap());
        drop(second_lock);
        assert!(!has_other_processes(&directory, first_slot).unwrap());
        drop(first_lock);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
