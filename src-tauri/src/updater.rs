use fs2::FileExt;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const REPOSITORY: &str = "MoLinOwO/ReversedFront";
const RELEASES_API_URL: &str =
    "https://api.github.com/repos/MoLinOwO/ReversedFront/releases/latest";
const STATIC_UPDATE_MANIFEST_URL: &str =
    "https://github.com/MoLinOwO/ReversedFront/releases/latest/download/update.json";
const GITHUB_RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/MoLinOwO/ReversedFront/releases/download/";
const STANDALONE_UPDATE_FLAG: &str = "--rf-apply-update";
const UPDATE_WAIT_LOCK_ARG: &str = "--wait-lock";
const UPDATE_GLOBAL_LOCK_ARG: &str = "--update-lock";
const UPDATE_PROCESS_LOCK_DIR_ARG: &str = "--process-lock-dir";
const UPDATE_INSTALLER_ARG: &str = "--installer";
const UPDATE_TARGET_DIR_ARG: &str = "--target-dir";
const UPDATE_RESTART_ARG: &str = "--restart";
const UPDATE_WAIT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const UPDATE_WAIT_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct StaticUpdateManifest {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Serialize, Clone)]
pub struct UpdateInfo {
    pub version: String,
    pub download_url: String,
    pub filename: String,
    pub has_update: bool,
}

/// 從 tag 或檔名中解析版本號，支援語意化版本（例如 v3.1.1）。
fn extract_version(value: &str) -> Option<String> {
    let re_version = Regex::new(r"v(\d+(?:\.\d+)+)").ok()?;
    re_version
        .captures(value)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().to_string()))
}

fn release_asset_priority(filename: &str) -> Option<u8> {
    let lower = filename.to_ascii_lowercase();
    if !lower.starts_with("reversedfront_") {
        return None;
    }

    if cfg!(target_os = "windows") {
        if lower.ends_with(".exe") {
            return Some(0);
        }
        if lower.ends_with(".msi") {
            return Some(1);
        }
    } else if cfg!(target_os = "macos") {
        if lower.ends_with(".dmg") {
            return Some(0);
        }
        if lower.ends_with(".app.tar.gz") {
            return Some(1);
        }
    } else if cfg!(target_os = "linux") {
        if lower.ends_with(".appimage") {
            return Some(0);
        }
        if lower.ends_with(".deb") {
            return Some(1);
        }
    }

    None
}

fn select_release_asset(release: &GithubRelease) -> Option<&GithubReleaseAsset> {
    release
        .assets
        .iter()
        .filter_map(|asset| release_asset_priority(&asset.name).map(|priority| (priority, asset)))
        .min_by_key(|(priority, _)| *priority)
        .map(|(_, asset)| asset)
}

/// 查詢 GitHub Releases 的最新正式版本。
///
/// 桌面程式版本由 Git tag 決定；推送 `v*` tag 觸發 GitHub Actions 編譯後，
/// 這裡會從同一個 repository 的 latest release 取得對應平台的安裝檔。
pub async fn check_update(current_version: &str) -> Result<UpdateInfo, String> {
    let client = update_client()?;

    match check_update_from_api(&client, current_version).await {
        Ok(info) => Ok(info),
        Err(api_error) => check_update_from_manifest(&client, current_version)
            .await
            .map_err(|manifest_error| {
                format!(
                    "GitHub API 更新檢查失敗：{}；靜態描述檔 fallback 也失敗：{}",
                    api_error, manifest_error
                )
            }),
    }
}

fn update_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("ReversedFront-Updater")
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

fn download_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("ReversedFront-Updater")
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(10 * 60))
        .build()
        .map_err(|e| e.to_string())
}

fn download_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .download_dir()
        .map_err(|e| format!("無法取得系統下載目錄：{}", e))?;
    fs::create_dir_all(&directory)
        .map_err(|e| format!("無法建立系統下載目錄 {}：{}", directory.display(), e))?;
    Ok(directory)
}

fn save_download_file(app: &AppHandle, filename: &str, content: &[u8]) -> Result<PathBuf, String> {
    let directory = download_dir(app)?;
    let destination = directory.join(filename);
    let partial = directory.join(format!(".{filename}.part-{}", std::process::id()));

    fs::write(&partial, content)
        .map_err(|e| format!("無法儲存更新檔 {}：{}", partial.display(), e))?;
    if destination.exists() {
        fs::remove_file(&destination).map_err(|e| {
            let _ = fs::remove_file(&partial);
            format!("無法覆蓋下載檔 {}：{}", destination.display(), e)
        })?;
    }
    fs::rename(&partial, &destination).map_err(|e| {
        let _ = fs::remove_file(&partial);
        format!("無法完成更新檔下載 {}：{}", destination.display(), e)
    })?;

    Ok(destination)
}

#[derive(Debug)]
struct StandaloneUpdateArgs {
    wait_lock: PathBuf,
    update_lock: PathBuf,
    process_lock_dir: PathBuf,
    installer: PathBuf,
    target_dir: PathBuf,
    restart: PathBuf,
}

/// 如果目前程序是由自己啟動的獨立更新模式，執行更新後直接結束，
/// 不要建立 Tauri 視窗。回傳 true 代表 main 不應再啟動遊戲。
pub fn run_standalone_update_if_requested() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(STANDALONE_UPDATE_FLAG)) {
        return false;
    }

    let result = parse_standalone_update_args(args).and_then(apply_standalone_update);
    if let Err(error) = result {
        eprintln!("ReversedFront standalone update failed: {error}");
    }
    true
}

fn parse_standalone_update_args(
    mut args: impl Iterator<Item = std::ffi::OsString>,
) -> Result<StandaloneUpdateArgs, String> {
    let mut wait_lock = None;
    let mut update_lock = None;
    let mut process_lock_dir = None;
    let mut installer = None;
    let mut target_dir = None;
    let mut restart = None;

    while let Some(argument) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("更新參數缺少值：{}", argument.to_string_lossy()))?;
        match argument.to_string_lossy().as_ref() {
            UPDATE_WAIT_LOCK_ARG => wait_lock = Some(PathBuf::from(value)),
            UPDATE_GLOBAL_LOCK_ARG => update_lock = Some(PathBuf::from(value)),
            UPDATE_PROCESS_LOCK_DIR_ARG => process_lock_dir = Some(PathBuf::from(value)),
            UPDATE_INSTALLER_ARG => installer = Some(PathBuf::from(value)),
            UPDATE_TARGET_DIR_ARG => target_dir = Some(PathBuf::from(value)),
            UPDATE_RESTART_ARG => restart = Some(PathBuf::from(value)),
            unknown => return Err(format!("未知的更新參數：{}", unknown)),
        }
    }

    Ok(StandaloneUpdateArgs {
        wait_lock: wait_lock.ok_or_else(|| "缺少更新等待鎖檔".to_string())?,
        update_lock: update_lock.ok_or_else(|| "缺少全域更新鎖檔".to_string())?,
        process_lock_dir: process_lock_dir.ok_or_else(|| "缺少程序槽位鎖目錄".to_string())?,
        installer: installer.ok_or_else(|| "缺少更新安裝檔".to_string())?,
        target_dir: target_dir.ok_or_else(|| "缺少更新目標目錄".to_string())?,
        restart: restart.ok_or_else(|| "缺少更新重啟路徑".to_string())?,
    })
}

fn apply_standalone_update(args: StandaloneUpdateArgs) -> Result<(), String> {
    wait_for_parent_exit(&args.wait_lock)?;

    let helper_path = std::env::current_exe().ok();
    let update_lock = match wait_for_exclusive_lock(&args.update_lock) {
        Ok(lock) => lock,
        Err(error) => {
            let _ = restart_application(&args.restart);
            cleanup_helper(helper_path.as_deref(), &args.restart);
            return Err(error);
        }
    };

    // 父程序釋放等待鎖後，再確認全部程序槽位都已釋放。
    // 這能涵蓋觸發更新以外的多開視窗，避免 Windows 還有其他程序
    // 鎖住同一個 rf-desktop.exe 時就開始安裝。
    let update_result = wait_for_process_slots_exit(&args.process_lock_dir)
        .and_then(|_| install_downloaded_update(&args));
    drop(update_lock);

    let restart_after_update = match update_result {
        Ok(restart_after_update) => restart_after_update,
        Err(error) => {
            // 安裝失敗時仍嘗試把舊版帶回來，避免使用者只剩下關閉的視窗。
            let _ = restart_application(&args.restart);
            cleanup_helper(helper_path.as_deref(), &args.restart);
            return Err(error);
        }
    };

    cleanup_helper(helper_path.as_deref(), &args.restart);
    if restart_after_update {
        restart_application(&args.restart)
    } else {
        Ok(())
    }
}

fn cleanup_helper(helper_path: Option<&Path>, restart_path: &Path) {
    if helper_path.is_some_and(|path| path != restart_path) {
        if let Some(path) = helper_path {
            let _ = fs::remove_file(path);
        }
    }
}

fn wait_for_parent_exit(lock_path: &Path) -> Result<(), String> {
    let deadline = std::time::Instant::now() + UPDATE_WAIT_TIMEOUT;

    loop {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(lock_path)
        {
            Ok(lock_file) => match lock_file.try_lock_exclusive() {
                Ok(()) => {
                    drop(lock_file);
                    let _ = fs::remove_file(lock_path);
                    return Ok(());
                }
                Err(error) if is_lock_busy(&error) => {}
                Err(error) => return Err(format!("無法取得更新等待鎖：{}", error)),
            },
            Err(error) if is_lock_busy(&error) => {}
            Err(error) => return Err(format!("無法開啟更新等待鎖：{}", error)),
        }

        if std::time::Instant::now() >= deadline {
            return Err("等待 ReversedFront 關閉逾時".to_string());
        }
        thread::sleep(UPDATE_WAIT_INTERVAL);
    }
}

fn wait_for_exclusive_lock(lock_path: &Path) -> Result<fs::File, String> {
    let deadline = std::time::Instant::now() + UPDATE_WAIT_TIMEOUT;

    loop {
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|error| format!("無法開啟全域更新鎖：{}", error))?;

        match lock_file.try_lock_exclusive() {
            Ok(()) => return Ok(lock_file),
            Err(error) if is_lock_busy(&error) => {}
            Err(error) => return Err(format!("無法取得全域更新鎖：{}", error)),
        }

        if std::time::Instant::now() >= deadline {
            return Err("等待全域更新鎖逾時".to_string());
        }
        thread::sleep(UPDATE_WAIT_INTERVAL);
    }
}

fn wait_for_process_slots_exit(lock_dir: &Path) -> Result<(), String> {
    let deadline = std::time::Instant::now() + UPDATE_WAIT_TIMEOUT;

    loop {
        let mut any_process_running = false;

        for slot in 0..crate::MAX_PROCESS_SLOTS {
            let lock_path = lock_dir.join(format!("slot-{slot}.lock"));
            if !lock_path.exists() {
                continue;
            }

            let lock_file = match OpenOptions::new().read(true).write(true).open(&lock_path) {
                Ok(lock_file) => lock_file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!("無法檢查程序槽位 {}：{}", slot, error));
                }
            };

            match lock_file.try_lock_exclusive() {
                Ok(()) => {}
                Err(error) if is_lock_busy(&error) => {
                    any_process_running = true;
                    break;
                }
                Err(error) => {
                    return Err(format!("無法檢查程序槽位 {}：{}", slot, error));
                }
            }
        }

        if !any_process_running {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("等待所有 ReversedFront 視窗關閉逾時".to_string());
        }
        thread::sleep(UPDATE_WAIT_INTERVAL);
    }
}

fn is_lock_busy(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || cfg!(target_os = "windows") && matches!(error.raw_os_error(), Some(32 | 33))
}

fn acquire_update_lock(lock_path: &Path) -> Result<fs::File, String> {
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|error| format!("無法開啟全域更新鎖：{}", error))?;

    match lock_file.try_lock_exclusive() {
        Ok(()) => Ok(lock_file),
        Err(error) if is_lock_busy(&error) => Err("另一個 ReversedFront 視窗正在更新".to_string()),
        Err(error) => Err(format!("無法取得全域更新鎖：{}", error)),
    }
}

fn install_downloaded_update(args: &StandaloneUpdateArgs) -> Result<bool, String> {
    if !args.installer.is_file() {
        return Err(format!("找不到更新安裝檔：{}", args.installer.display()));
    }

    #[cfg(target_os = "windows")]
    {
        let is_msi = args
            .installer
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("msi"));
        let status = if is_msi {
            Command::new("msiexec")
                .arg("/i")
                .arg(&args.installer)
                .arg(format!("INSTALLDIR={}", args.target_dir.display()))
                .status()
                .map_err(|error| format!("無法啟動 MSI 更新程式：{}", error))?
        } else {
            // NSIS 的 /D 參數必須是最後一個參數，確保更新到目前的安裝目錄。
            Command::new(&args.installer)
                .arg(format!("/D={}", args.target_dir.display()))
                .status()
                .map_err(|error| format!("無法啟動 Windows 更新程式：{}", error))?
        };

        if status.success() {
            return Ok(true);
        }
        return Err(format!("Windows 更新程式結束碼：{}", status));
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&args.installer)
            .status()
            .map_err(|error| format!("無法開啟 macOS 更新程式：{}", error))?;
        // DMG／App archive 的安裝由 macOS Installer 或 Finder 完成。
        return Ok(false);
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&args.installer)
            .status()
            .map_err(|error| format!("無法開啟 Linux 更新程式：{}", error))?;
        return Ok(false);
    }
}

fn restart_application(path: &Path) -> Result<(), String> {
    Command::new(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("更新後無法重新啟動 ReversedFront：{}", error))
}

async fn check_update_from_api(
    client: &Client,
    current_version: &str,
) -> Result<UpdateInfo, String> {
    let response = client
        .get(RELEASES_API_URL)
        .send()
        .await
        .map_err(|e| format!("GitHub release check failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GitHub release API returned {}", response.status()));
    }

    let release: GithubRelease = response
        .json()
        .await
        .map_err(|e| format!("Invalid GitHub release response: {}", e))?;

    let remote_version = extract_version(&release.tag_name)
        .ok_or_else(|| format!("Invalid GitHub release tag: {}", release.tag_name))?;

    let Some(asset) = select_release_asset(&release) else {
        return Err(format!(
            "No supported {} release asset found in {}",
            std::env::consts::OS,
            REPOSITORY
        ));
    };

    Ok(UpdateInfo {
        has_update: is_newer(&remote_version, current_version),
        version: remote_version,
        download_url: asset.browser_download_url.clone(),
        filename: asset.name.clone(),
    })
}

async fn check_update_from_manifest(
    client: &Client,
    current_version: &str,
) -> Result<UpdateInfo, String> {
    let response = client
        .get(STATIC_UPDATE_MANIFEST_URL)
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|e| format!("靜態更新描述檔請求失敗：{}", e))?;

    if !response.status().is_success() {
        return Err(format!("靜態更新描述檔回傳 {}", response.status()));
    }

    let manifest: StaticUpdateManifest = response
        .json()
        .await
        .map_err(|e| format!("靜態更新描述檔格式錯誤：{}", e))?;

    let remote_version = extract_version(&manifest.tag_name)
        .ok_or_else(|| format!("靜態更新描述檔版本標籤錯誤：{}", manifest.tag_name))?;

    let manifest_release = GithubRelease {
        tag_name: manifest.tag_name,
        assets: manifest.assets,
    };
    let Some(asset) = select_release_asset(&manifest_release) else {
        return Err(format!(
            "靜態更新描述檔沒有支援 {} 的安裝檔",
            std::env::consts::OS
        ));
    };

    Ok(UpdateInfo {
        has_update: is_newer(&remote_version, current_version),
        version: remote_version,
        download_url: asset.browser_download_url.clone(),
        filename: asset.name.clone(),
    })
}

fn is_newer(remote: &str, current: &str) -> bool {
    let parse_version = |value: &str| -> Vec<u32> {
        value
            .trim()
            .trim_start_matches('v')
            .split('.')
            .map(|part| {
                part.chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>()
            })
            .filter_map(|part| part.parse::<u32>().ok())
            .collect()
    };

    let remote_parts = parse_version(remote);
    let current_parts = parse_version(current);

    if remote_parts.is_empty() || current_parts.is_empty() {
        return false;
    }

    let length = remote_parts.len().max(current_parts.len());
    (0..length)
        .map(|index| {
            (
                remote_parts.get(index).copied().unwrap_or(0),
                current_parts.get(index).copied().unwrap_or(0),
            )
        })
        .find_map(|(remote_part, current_part)| {
            (remote_part != current_part).then_some(remote_part > current_part)
        })
        .unwrap_or(false)
}

fn validate_download_request(url: &str, filename: &str) -> Result<(), String> {
    if !url.starts_with(GITHUB_RELEASE_DOWNLOAD_PREFIX) {
        return Err("拒絕非 GitHub Releases 的更新下載網址".to_string());
    }

    let path = Path::new(filename);
    if path.file_name().and_then(|name| name.to_str()) != Some(filename) {
        return Err("更新檔名無效".to_string());
    }

    if extract_version(filename).is_none() || release_asset_priority(filename).is_none() {
        return Err("GitHub Releases 沒有支援目前平台的更新檔".to_string());
    }

    Ok(())
}

/// 啟動應用程式時清理舊的安裝程式檔案。
pub fn cleanup_old_installers() {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();

                    if path == exe_path {
                        continue;
                    }

                    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                        if name.starts_with("ReversedFront_v")
                            && (name.ends_with(".exe")
                                || name.ends_with(".msi")
                                || name.ends_with(".dmg")
                                || name.ends_with(".AppImage")
                                || name.ends_with(".deb")
                                || name.ends_with(".tar.gz"))
                        {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }
    }
}

pub async fn download_and_install(app: AppHandle, url: &str, filename: &str) -> Result<(), String> {
    validate_download_request(url, filename)?;

    let app_state = app.state::<crate::AppState>();
    let app_data_dir = app_state.app_data_dir.clone();
    let current_slot = app_state.process_slot;
    drop(app_state);

    let update_lock_path = crate::update_lock_path(&app_data_dir);
    let update_guard = acquire_update_lock(&update_lock_path)?;
    if crate::has_other_processes(&app_data_dir, current_slot)
        .map_err(|error| format!("無法檢查其他 ReversedFront 程序：{}", error))?
    {
        return Err("偵測到其他 ReversedFront 視窗，請先關閉其他視窗後再更新".to_string());
    }

    let client = download_client()?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("GitHub update download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "GitHub update download failed: {}",
            response.status()
        ));
    }

    let content = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read update: {}", e))?;

    // 更新安裝檔放在系統下載目錄，讓使用者可以保留並檢查實際下載的檔案。
    let installer_path = save_download_file(&app, filename, &content)?;

    #[cfg(unix)]
    if installer_path.extension().and_then(|ext| ext.to_str()) == Some("AppImage") {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&installer_path)
            .map_err(|e| format!("Failed to inspect AppImage: {}", e))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&installer_path, permissions)
            .map_err(|e| format!("Failed to prepare AppImage: {}", e))?;
    }

    // 下載期間若有其他程序剛好完成啟動，第二次檢查仍會阻止安裝。
    if crate::has_other_processes(&app_data_dir, current_slot)
        .map_err(|error| format!("無法檢查其他 ReversedFront 程序：{}", error))?
    {
        return Err(
            "下載完成，但偵測到其他 ReversedFront 視窗，請關閉其他視窗後再重新更新".to_string(),
        );
    }

    let current_exe = std::env::current_exe()
        .map_err(|error| format!("無法取得目前 ReversedFront 路徑：{}", error))?;
    let target_dir = current_exe
        .parent()
        .ok_or_else(|| "無法取得 ReversedFront 安裝目錄".to_string())?
        .to_path_buf();
    let wait_lock_path = download_dir(&app)?.join(format!(
        ".reversedfront-update-parent-{}.lock",
        std::process::id()
    ));
    let process_lock_dir = app_data_dir.join("process-locks");
    let parent_lock = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&wait_lock_path)
        .map_err(|error| format!("無法建立更新等待鎖：{}", error))?;
    parent_lock
        .lock_exclusive()
        .map_err(|error| format!("無法鎖定更新等待鎖：{}", error))?;

    // 更新程序不能直接執行目前的 rf-desktop.exe，否則 Windows 會因為
    // 更新器本身仍鎖住該檔案而無法替換。複製到下載目錄後再以副本接手。
    let helper_path = download_dir(&app)?.join(format!(
        ".reversedfront-updater-{}{}",
        std::process::id(),
        current_exe
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default()
    ));
    fs::copy(&current_exe, &helper_path)
        .map_err(|error| format!("無法建立獨立更新程序：{}", error))?;

    // 由同一個執行檔以無介面的更新模式接手，避免正在執行的程序
    // 直接啟動安裝程式時還持續鎖住自己的檔案。
    Command::new(&helper_path)
        .arg(STANDALONE_UPDATE_FLAG)
        .arg(UPDATE_WAIT_LOCK_ARG)
        .arg(&wait_lock_path)
        .arg(UPDATE_GLOBAL_LOCK_ARG)
        .arg(&update_lock_path)
        .arg(UPDATE_PROCESS_LOCK_DIR_ARG)
        .arg(&process_lock_dir)
        .arg(UPDATE_INSTALLER_ARG)
        .arg(&installer_path)
        .arg(UPDATE_TARGET_DIR_ARG)
        .arg(&target_dir)
        .arg(UPDATE_RESTART_ARG)
        .arg(&current_exe)
        .spawn()
        .map_err(|error| {
            let _ = fs::remove_file(&helper_path);
            format!("無法啟動獨立更新程序：{}", error)
        })?;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }

    app.exit(0);
    drop(parent_lock);
    drop(update_guard);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{extract_version, is_newer, parse_standalone_update_args};
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn extracts_semantic_versions() {
        assert_eq!(extract_version("v3.1.2"), Some("3.1.2".to_string()));
        assert_eq!(
            extract_version("ReversedFront_v10.20.30.exe"),
            Some("10.20.30".to_string())
        );
        assert_eq!(extract_version("latest"), None);
    }

    #[test]
    fn compares_versions_by_component() {
        assert!(is_newer("3.2.0", "3.1.9"));
        assert!(is_newer("3.1.1", "3.1"));
        assert!(!is_newer("3.1.0", "3.1"));
        assert!(!is_newer("2.9.9", "3.0.0"));
    }

    #[test]
    fn parses_standalone_update_arguments() {
        let args = vec![
            OsString::from("--wait-lock"),
            OsString::from("C:/Downloads/update.lock"),
            OsString::from("--update-lock"),
            OsString::from(
                "C:/Users/test/AppData/Roaming/ReversedFront/.reversedfront-update.lock",
            ),
            OsString::from("--process-lock-dir"),
            OsString::from("C:/Users/test/AppData/Roaming/ReversedFront/process-locks"),
            OsString::from("--installer"),
            OsString::from("C:/Downloads/ReversedFront_v3.2.6.exe"),
            OsString::from("--target-dir"),
            OsString::from("D:/ReversedFront"),
            OsString::from("--restart"),
            OsString::from("D:/ReversedFront/rf-desktop.exe"),
        ];

        let parsed = parse_standalone_update_args(args.into_iter()).unwrap();
        assert_eq!(parsed.wait_lock, PathBuf::from("C:/Downloads/update.lock"));
        assert_eq!(
            parsed.update_lock,
            PathBuf::from("C:/Users/test/AppData/Roaming/ReversedFront/.reversedfront-update.lock")
        );
        assert_eq!(
            parsed.process_lock_dir,
            PathBuf::from("C:/Users/test/AppData/Roaming/ReversedFront/process-locks")
        );
        assert_eq!(
            parsed.installer,
            PathBuf::from("C:/Downloads/ReversedFront_v3.2.6.exe")
        );
        assert_eq!(parsed.target_dir, PathBuf::from("D:/ReversedFront"));
        assert_eq!(
            parsed.restart,
            PathBuf::from("D:/ReversedFront/rf-desktop.exe")
        );
    }

    #[test]
    fn rejects_incomplete_standalone_update_arguments() {
        let args = vec![OsString::from("--installer")];
        let error = parse_standalone_update_args(args.into_iter()).unwrap_err();
        assert!(error.contains("更新參數缺少值"));
    }
}
