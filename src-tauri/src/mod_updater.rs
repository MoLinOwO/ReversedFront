use anyhow::{bail, Context, Result};
use fs2::FileExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager};

const REPOSITORY: &str = "MoLinOwO/ReversedFront_Public";
const API_BASE: &str = "https://api.github.com/repos/MoLinOwO/ReversedFront_Public";
const RAW_BASE: &str = "https://raw.githubusercontent.com/MoLinOwO/ReversedFront_Public";

// 目前隨桌面版內附的公開倉庫版本。沒有本機 update.json 時，
// 以這個版本作為基準，避免第一次啟動就把目前的 Mod 覆蓋掉。
const BUNDLED_REF: &str = env!("RF_BUNDLED_REF");
const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;
static UPDATE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
struct CommitResponse {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct TreeResponse {
    tree: Vec<TreeEntry>,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    size: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateState {
    ref_name: String,
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent("RF-Desktop-Mod-Updater")
        .build()?)
}

fn update_root(app: &AppHandle) -> Result<PathBuf> {
    Ok(app
        .path()
        .app_data_dir()
        .context("無法取得應用程式資料目錄")?
        .join("mod"))
}

fn state_path(app: &AppHandle) -> Result<PathBuf> {
    Ok(update_root(app)?.join("update.json"))
}

fn local_ref(app: &AppHandle) -> Result<String> {
    let path = state_path(app)?;
    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(state) = serde_json::from_str::<UpdateState>(&content) {
            if is_sha(&state.ref_name) {
                return Ok(state.ref_name);
            }
        }
    }
    Ok(BUNDLED_REF.to_string())
}

fn is_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn allowed_path(path: &str) -> bool {
    if path.ends_with(".pfx") || path.ends_with("config.json") || path.ends_with("RFcity.yaml") {
        return false;
    }

    (path.starts_with("web/mod/js/")
        && (path.ends_with(".bundle.js") || path.ends_with(".LICENSE.txt")))
        || (path.starts_with("web/mod/data/")
            && (path.ends_with(".yaml")
                || path.ends_with(".yml")
                || path.ends_with(".svg")
                || path.ends_with(".json")))
}

fn safe_relative_path(path: &str) -> Option<&str> {
    let relative = path.strip_prefix("web/mod/")?;
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains("..")
        || relative.contains('\\')
    {
        return None;
    }
    Some(relative)
}

pub async fn check_for_update(app: &AppHandle) -> Result<Value, String> {
    let client = client().map_err(|e| e.to_string())?;
    let remote = client
        .get(format!("{API_BASE}/commits/main"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<CommitResponse>()
        .await
        .map_err(|e| e.to_string())?;

    if !is_sha(&remote.sha) {
        return Err("GitHub 回傳的版本識別碼格式錯誤".to_string());
    }

    let current = local_ref(app).map_err(|e| e.to_string())?;
    Ok(json!({
        "hasUpdate": current != remote.sha,
        "currentRef": current,
        "remoteRef": remote.sha,
        "repository": REPOSITORY,
        "source": "GitHub main"
    }))
}

pub async fn download_and_install(app: &AppHandle, remote_ref: &str) -> Result<Value, String> {
    if !is_sha(remote_ref) {
        return Err("更新版本識別碼格式錯誤".to_string());
    }

    let result = download_and_install_inner(app, remote_ref).await;
    result.map_err(|e| e.to_string())
}

async fn download_and_install_inner(app: &AppHandle, remote_ref: &str) -> Result<Value> {
    let client = client()?;
    let tree = client
        .get(format!("{API_BASE}/git/trees/{remote_ref}?recursive=1"))
        .send()
        .await?
        .error_for_status()?
        .json::<TreeResponse>()
        .await?;

    if tree.truncated {
        bail!("GitHub 檔案清單過大，取消更新以避免不完整安裝");
    }

    let files: Vec<&TreeEntry> = tree
        .tree
        .iter()
        .filter(|entry| entry.entry_type == "blob" && allowed_path(&entry.path))
        .collect();

    if !files
        .iter()
        .any(|entry| entry.path == "web/mod/js/main.bundle.js")
    {
        bail!("更新內容缺少 main.bundle.js");
    }
    for entry in &files {
        if let Some(size) = entry.size {
            if size > MAX_FILE_SIZE {
                bail!("更新檔案過大：{}", entry.path);
            }
        }
    }

    let root = update_root(app)?;
    let sequence = UPDATE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stage = root.join(format!(
        ".staging-{remote_ref}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&stage)?;

    // 所有檔案先下載到 staging，完整成功後才套用，避免網路中斷留下半套 Mod。
    for entry in &files {
        let relative = safe_relative_path(&entry.path)
            .with_context(|| format!("不安全的更新路徑：{}", entry.path))?;
        let destination = stage.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        let response = client
            .get(format!("{RAW_BASE}/{remote_ref}/{}", entry.path))
            .send()
            .await?
            .error_for_status()?;
        if response.content_length().unwrap_or(0) > MAX_FILE_SIZE {
            bail!("更新檔案過大：{}", entry.path);
        }
        let bytes = response.bytes().await?;
        if bytes.len() as u64 > MAX_FILE_SIZE {
            bail!("更新檔案過大：{}", entry.path);
        }
        fs::write(destination, bytes)?;
    }

    let expected: HashSet<String> = files
        .iter()
        .filter_map(|entry| safe_relative_path(&entry.path).map(ToOwned::to_owned))
        .collect();

    // 下載不持鎖；只有套用階段使用跨程序排他鎖。這樣多個桌面程序
    // 同時偵測到更新時，不會一邊刪除另一邊正在套用的 bundle。
    let lock_path = root.join(".update.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    lock_file.lock_exclusive()?;

    let js_root = root.join("js");
    if js_root.exists() {
        for entry in fs::read_dir(&js_root)?.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_file()
                && (name.ends_with(".bundle.js") || name.ends_with(".LICENSE.txt"))
                && !expected.contains(&format!("js/{name}"))
            {
                fs::remove_file(path)?;
            }
        }
    }

    copy_staged_files(&stage, &root)?;
    fs::remove_dir_all(&stage)?;

    if let Some(parent) = state_path(app)?.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        state_path(app)?,
        serde_json::to_vec_pretty(&UpdateState {
            ref_name: remote_ref.to_string(),
        })?,
    )?;
    FileExt::unlock(&lock_file)?;

    Ok(json!({
        "updated": true,
        "remoteRef": remote_ref,
        "fileCount": files.len(),
        "restartRequired": true
    }))
}

fn copy_staged_files(stage: &Path, root: &Path) -> Result<()> {
    for entry in fs::read_dir(stage)? {
        let entry = entry?;
        let source = entry.path();
        let relative = source.strip_prefix(stage)?;
        let destination = root.join(relative);
        if source.is_dir() {
            fs::create_dir_all(&destination)?;
            // 遞迴時目的地也必須進入同名子目錄。若仍傳入 root，
            // stage/js/main.bundle.js 會被錯誤攤平成 root/main.bundle.js。
            copy_staged_files(&source, &destination)?;
        } else {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source, destination)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "reversedfront-{name}-{}-{}",
            std::process::id(),
            UPDATE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn validates_update_paths() {
        assert!(allowed_path("web/mod/js/main.bundle.js"));
        assert!(allowed_path("web/mod/data/transportRoutes.json"));
        assert!(!allowed_path("web/mod/js/core/api.js"));
        assert!(!allowed_path("web/mod/data/RFcity.yaml"));
        assert_eq!(
            safe_relative_path("web/mod/js/main.bundle.js"),
            Some("js/main.bundle.js")
        );
        assert_eq!(safe_relative_path("web/mod/../secret.json"), None);
    }

    #[test]
    fn staged_copy_preserves_subdirectories() {
        let base = test_directory("staged-copy");
        let stage = base.join("stage");
        let destination = base.join("destination");
        fs::create_dir_all(stage.join("js")).unwrap();
        fs::create_dir_all(stage.join("data")).unwrap();
        fs::write(stage.join("js/main.bundle.js"), b"bundle").unwrap();
        fs::write(stage.join("data/routes.json"), b"{}").unwrap();

        copy_staged_files(&stage, &destination).unwrap();

        assert_eq!(
            fs::read(destination.join("js/main.bundle.js")).unwrap(),
            b"bundle"
        );
        assert_eq!(
            fs::read(destination.join("data/routes.json")).unwrap(),
            b"{}"
        );
        assert!(!destination.join("main.bundle.js").exists());
        fs::remove_dir_all(base).unwrap();
    }
}
