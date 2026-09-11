use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tauri::{AppHandle, Manager};

const REPOSITORY: &str = "MoLinOwO/ReversedFront_Public";
const RELEASES_API_URL: &str =
    "https://api.github.com/repos/MoLinOwO/ReversedFront_Public/releases/latest";
const GITHUB_RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/MoLinOwO/ReversedFront_Public/releases/download/";

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
    let client = Client::builder()
        .user_agent("ReversedFront-Updater")
        .build()
        .map_err(|e| e.to_string())?;

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

    let client = Client::builder()
        .user_agent("ReversedFront-Updater")
        .build()
        .map_err(|e| e.to_string())?;
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

    // 安裝檔先放到使用者暫存目錄，避免安裝在受保護的程式目錄時無法寫入。
    let update_dir = std::env::temp_dir().join("reversedfront-updates");
    fs::create_dir_all(&update_dir).map_err(|e| {
        format!(
            "Failed to create update directory {}: {}",
            update_dir.display(),
            e
        )
    })?;

    let installer_path = update_dir.join(filename);
    let mut file = fs::File::create(&installer_path).map_err(|e| {
        format!(
            "Failed to create installer {}: {}",
            installer_path.display(),
            e
        )
    })?;
    file.write_all(&content)
        .map_err(|e| format!("Failed to save installer: {}", e))?;
    file.flush()
        .map_err(|e| format!("Failed to flush installer: {}", e))?;

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

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }

    if cfg!(target_os = "windows") && filename.to_ascii_lowercase().ends_with(".msi") {
        Command::new("msiexec")
            .arg("/i")
            .arg(&installer_path)
            .spawn()
            .map_err(|e| e.to_string())?;
    } else if cfg!(target_os = "windows") {
        Command::new(&installer_path)
            .spawn()
            .map_err(|e| e.to_string())?;
    } else if cfg!(target_os = "macos") {
        Command::new("open")
            .arg(&installer_path)
            .spawn()
            .map_err(|e| e.to_string())?;
    } else {
        #[cfg(target_os = "linux")]
        Command::new("xdg-open")
            .arg(&installer_path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    app.exit(0);
    Ok(())
}
