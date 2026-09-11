use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// 全局資源基礎路徑
static RESOURCE_BASE_PATH: OnceLock<PathBuf> = OnceLock::new();
// 平台使用者資料根目錄，供標準安裝版的 Mod 更新、設定與舊資料遷移使用。
static USER_DATA_BASE_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn set_resource_base_path(path: PathBuf) {
    RESOURCE_BASE_PATH.set(path).ok();
}

pub fn set_user_data_base_path(path: PathBuf) {
    USER_DATA_BASE_PATH.set(path).ok();
}

/// 帳號資料與遊戲關卡素材共用的可寫入執行期根目錄。
///
/// 開發模式使用專案 web/；Windows 可攜版在執行檔旁。標準安裝目錄
/// 不可寫，以及 macOS/Linux 發佈版，會由啟動流程設定為使用者資料目錄。
pub fn get_resource_base_dir() -> PathBuf {
    RESOURCE_BASE_PATH.get().cloned().unwrap_or_else(|| {
        if cfg!(debug_assertions) {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("web")
        } else {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."))
        }
    })
}

pub fn get_hidden_config_dir(target: &str) -> PathBuf {
    if target == "passionfruit" {
        let path = get_resource_base_dir().join("passionfruit");
        fs::create_dir_all(&path).unwrap_or_default();
        return path;
    }

    // Dev mode: use local web folder
    #[cfg(debug_assertions)]
    {
        let mut path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if path.ends_with("src-tauri") {
            path.pop();
        }
        path.push("web");
        if target == "root" {
            // Return web root
        } else {
            path.push("mod");
            path.push("data");
        }
        fs::create_dir_all(&path).unwrap_or_default();
        path
    }

    // Release mode: use Tauri resource directory or exe parent
    #[cfg(not(debug_assertions))]
    {
        let mut path = if let Some(base) = USER_DATA_BASE_PATH.get() {
            base.clone()
        } else if let Some(base) = RESOURCE_BASE_PATH.get() {
            // Tauri resource_dir 已經是正確的根目錄
            base.clone()
        } else {
            // Fallback: use exe parent directory
            let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
            exe_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf()
        };

        if target == "root" {
            // Return root
        } else {
            path.push("mod");
            path.push("data");
        }
        fs::create_dir_all(&path).unwrap_or_default();
        path
    }
}

fn get_user_data_base_dir() -> PathBuf {
    USER_DATA_BASE_PATH
        .get()
        .cloned()
        .or_else(|| {
            directories::ProjectDirs::from("com", "MoLinOwO", "ReversedFront")
                .map(|dirs| dirs.data_local_dir().to_path_buf())
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn get_config_file() -> PathBuf {
    get_hidden_config_dir("data").join("config.json")
}

/// SQLite 帳號庫放在選定的執行期資料根目錄。可攜版可以隨程式目錄移動，
/// 標準安裝則落在使用者資料目錄，避免 macOS/Linux 或 Program Files 無法寫入。
pub fn get_account_store_file() -> PathBuf {
    get_resource_base_dir().join("accounts.db")
}

/// 舊版 JSON 帳號檔案的位置，只供第一次啟動時匯入。
pub fn get_legacy_account_store_file() -> PathBuf {
    get_user_data_base_dir().join("accounts.json")
}

/// 先前版本放在使用者資料目錄的 SQLite 帳號庫，只供一次性遷移使用。
pub fn get_legacy_account_db_file() -> PathBuf {
    get_user_data_base_dir().join("accounts.db")
}

pub fn load_config() -> Value {
    let config_file = get_config_file();
    if config_file.exists() {
        if let Ok(content) = fs::read_to_string(&config_file) {
            if let Ok(json) = serde_json::from_str(&content) {
                return json;
            }
        }
    } else {
        // Fallback: try to read config.json from project root
        let mut root_config = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        #[cfg(debug_assertions)]
        {
            if root_config.ends_with("src-tauri") {
                root_config.pop();
            }
        }
        root_config.push("config.json");

        if root_config.exists() {
            if let Ok(content) = fs::read_to_string(&root_config) {
                if let Ok(json) = serde_json::from_str(&content) {
                    // Save it to the correct location
                    let _ = fs::write(&config_file, &content);
                    return json;
                }
            }
        }
    }
    serde_json::json!({})
}

pub fn save_config(config: &Value) {
    let config_file = get_config_file();
    if let Ok(content) = serde_json::to_string_pretty(config) {
        let _ = fs::write(config_file, content);
    }
}
