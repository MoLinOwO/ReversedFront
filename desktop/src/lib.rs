pub mod account_manager;
pub mod commands;
pub mod config_manager;
#[cfg(feature = "desktop")]
mod gpui_host;
pub mod host;
pub mod ipc;
pub mod mod_updater;
pub mod resource_manager;
pub mod updater;

use fs2::FileExt;
use host::DesktopHost;
use resource_manager::ResourceManager;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use warp::{http::Response, http::StatusCode, Filter};

pub struct DesktopRuntime {
    pub host: Arc<DesktopHost>,
    pub server_url: String,
    pub session_dir: PathBuf,
    // Hold the slot and runtime for the entire native window lifetime.
    _process_slot_lock: fs::File,
    _runtime: tokio::runtime::Runtime,
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

// The loopback server must never serve the account DB or private Mod source.
fn public_resource_path(path: &str) -> bool {
    if path.is_empty()
        || matches!(
            path,
            "index.html" | "desktop_bridge.js" | "favicon.ico" | "transporter.html"
        )
    {
        return true;
    }
    if path.starts_with("static/") || path.starts_with("tiles/") {
        return true;
    }
    if let Some(name) = path.strip_prefix("mod/js/") {
        return !name.contains('/')
            && (name.ends_with(".bundle.js") || name.ends_with(".LICENSE.txt"));
    }
    if let Some(name) = path.strip_prefix("mod/data/") {
        return !name.contains('/')
            && !matches!(
                name.to_ascii_lowercase().as_str(),
                "config.json" | "accounts.json"
            )
            && matches!(
                Path::new(name).extension().and_then(|ext| ext.to_str()),
                Some("json" | "yaml" | "yml" | "svg")
            );
    }
    // Preserve the official SPA's history routes, without exposing arbitrary files.
    Path::new(path).extension().is_none() && !path.starts_with("mod/")
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
    if relative_path.split('/').any(|part| part == "..")
        || relative_path.contains('\\')
        || !public_resource_path(relative_path)
    {
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Vec::new())
            .unwrap());
    }

    let (file_path, allowed_root) =
        if relative_path.is_empty() || Path::new(relative_path).extension().is_none() {
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

/// Keep resource, profile and database paths compatible with the existing installers.
fn resource_root() -> io::Result<PathBuf> {
    if cfg!(debug_assertions) {
        return Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("web"));
    }
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| io::Error::other("Missing executable directory"))?;
    #[cfg(target_os = "macos")]
    return Ok(directory.join("../Resources"));
    #[cfg(target_os = "linux")]
    {
        if let Some(app_dir) = std::env::var_os("APPDIR") {
            return Ok(PathBuf::from(app_dir).join("usr/lib/rf-desktop"));
        }
        if directory.join("index.html").is_file() {
            return Ok(directory.to_path_buf());
        }
        return Ok(PathBuf::from("/usr/lib/rf-desktop"));
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Ok(directory.to_path_buf())
}

pub fn prepare(ui_tx: std::sync::mpsc::Sender<host::UiRequest>) -> anyhow::Result<DesktopRuntime> {
    updater::cleanup_old_installers();
    let web_root = resource_root()?;
    anyhow::ensure!(
        web_root.join("index.html").is_file(),
        "找不到遊戲資源：{}",
        web_root.display()
    );
    let app_data_dir = directories::BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("無法取得應用程式資料目錄"))?
        .data_dir()
        .join("com.reversedfront.desktop");
    anyhow::ensure!(
        !update_is_in_progress(&app_data_dir)?,
        "ReversedFront 正在更新，請稍後再啟動"
    );
    let (process_slot, process_slot_lock) = acquire_process_slot(&app_data_dir)?;
    config_manager::set_resource_base_path(runtime_storage_root(&web_root, &app_data_dir));
    config_manager::set_user_data_base_path(app_data_dir.clone());
    fs::create_dir_all(config_manager::get_hidden_config_dir("data"))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _entered = runtime.enter();
    let resource_manager = ResourceManager::new();
    let host = Arc::new(DesktopHost::new(
        web_root.clone(),
        app_data_dir.clone(),
        process_slot,
        resource_manager.clone(),
        ui_tx,
    ));
    let mod_update_root = mod_updater::update_root(&host)?;
    fs::create_dir_all(mod_update_root.join("js"))?;
    fs::create_dir_all(mod_update_root.join("data"))?;
    if let Err(error) = mod_updater::migrate_legacy_update_root(&host) {
        eprintln!("Unable to migrate legacy Mod data: {error}");
    }
    let rm_filter = warp::any().map(move || resource_manager.clone());
    let status_route = warp::path("status")
        .and(warp::path::end())
        .and(rm_filter.clone())
        .map(|rm: Arc<ResourceManager>| warp::reply::json(&rm.get_status()));
    let passionfruit_route = warp::path("passionfruit")
        .and(warp::path::tail())
        .and(rm_filter.clone())
        .and_then(handle_passionfruit_request);
    let assets_route = warp::path("assets")
        .and(warp::path("passionfruit"))
        .and(warp::path::tail())
        .and(rm_filter)
        .and_then(handle_passionfruit_request);
    let static_route = warp::path::tail().and_then(move |path| {
        let web_root = web_root.clone();
        let mod_update_root = mod_update_root.clone();
        async move { handle_static_file(path, web_root, mod_update_root).await }
    });
    let routes = warp::get().and(
        status_route
            .or(passionfruit_route)
            .or(assets_route)
            .or(static_route),
    );
    let (address, server) = warp::serve(routes).try_bind_ephemeral(([127, 0, 0, 1], 0))?;
    runtime.spawn(server);
    let session_dir = app_data_dir
        .join("account-slots")
        .join(process_slot.to_string());
    fs::create_dir_all(&session_dir)?;
    let server_url = format!("http://{address}");
    println!("ReversedFront GPUI: {server_url}/?rf_account_slot={process_slot}");
    drop(_entered);
    Ok(DesktopRuntime {
        host,
        server_url,
        session_dir,
        _process_slot_lock: process_slot_lock,
        _runtime: runtime,
    })
}

#[cfg(feature = "desktop")]
pub fn run() {
    if let Err(error) = gpui_host::run() {
        eprintln!("ReversedFront 啟動失敗：{error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::{acquire_process_slot, has_other_processes, public_resource_path};

    #[test]
    fn static_server_only_exposes_game_assets() {
        for path in [
            "accounts.db",
            "config.json",
            "mod/data/accounts.json",
            "mod/data/config.json",
            "mod/js/core/api.js",
        ] {
            assert!(!public_resource_path(path), "{path}");
        }
        for path in [
            "",
            "users/log_in",
            "static/js/main.js",
            "mod/js/main.bundle.js",
            "mod/js/911.main.bundle.js",
            "mod/data/transportRoutes.json",
        ] {
            assert!(public_resource_path(path), "{path}");
        }
    }

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
