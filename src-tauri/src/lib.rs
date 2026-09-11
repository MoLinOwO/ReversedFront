pub mod account_manager;
pub mod commands;
pub mod config_manager;
pub mod mod_updater;
pub mod resource_manager;
pub mod updater;

use resource_manager::ResourceManager;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use warp::{http::Response, http::StatusCode, Filter};

pub struct AppState {
    pub resource_manager: Arc<ResourceManager>,
    pub server_base_url: String,
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
                std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .join("web")
            } else {
                // 生產模式：使用 Tauri 的資源目錄
                app.path()
                    .resource_dir()
                    .expect("Failed to get resource directory")
            };

            println!("Web root directory: {:?}\n", web_root);

            // 初始化配置和目錄
            // 設置資源基礎路徑（用於配置檔案等）
            config_manager::set_resource_base_path(web_root.clone());

            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get application data directory");
            config_manager::set_user_data_base_path(app_data_dir.clone());
            let mod_update_root = app_data_dir.join("mod");
            let _ = fs::create_dir_all(mod_update_root.join("js"));
            let _ = fs::create_dir_all(mod_update_root.join("data"));

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
            app.manage(AppState {
                resource_manager: resource_manager.clone(),
                server_base_url: "http://127.0.0.1:8765".to_string(),
            });

            // === 步驟 3: 啟動 HTTP 伺服器 ===
            let resource_manager_filter = warp::any().map(move || resource_manager.clone());
            let resource_manager_filter_status = resource_manager_filter.clone();

            tauri::async_runtime::spawn(async move {
                println!("=== Starting HTTP Server ===");
                println!("Server: http://127.0.0.1:8765/");
                println!("Frontend root: {:?}\n", web_root);

                let cors = warp::cors()
                    .allow_any_origin()
                    .allow_methods(vec!["GET", "POST", "OPTIONS"]);

                let status_route = warp::path("status")
                    .and(resource_manager_filter_status)
                    .map(|rm: Arc<ResourceManager>| {
                        let status = rm.get_status();
                        warp::reply::json(&status)
                    });

                let passionfruit_route = warp::path("passionfruit")
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
                    .or(static_route)
                    .with(cors);

                warp::serve(routes).run(([127, 0, 0, 1], 8765)).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_accounts,
            commands::add_account,
            commands::delete_account,
            commands::set_active_account,
            commands::get_active_account,
            commands::open_account_window,
            commands::save_yaml,
            commands::load_yaml,
            commands::check_resource_exists,
            commands::get_resource_download_status,
            commands::exit_app,
            commands::save_config_volume,
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
