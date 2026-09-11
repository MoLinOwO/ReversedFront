use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

// 全局資源基礎路徑
static RESOURCE_BASE_PATH: OnceLock<PathBuf> = OnceLock::new();
// 可寫入的使用者資料根目錄。Release 版不能直接寫入安裝目錄，
// Mod 更新與使用者編輯的 YAML 都放在這裡。
static USER_DATA_BASE_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn set_resource_base_path(path: PathBuf) {
    RESOURCE_BASE_PATH.set(path).ok();
}

pub fn set_user_data_base_path(path: PathBuf) {
    USER_DATA_BASE_PATH.set(path).ok();
}

pub fn get_hidden_config_dir(target: &str) -> PathBuf {
    // Dev mode: use local web folder
    #[cfg(debug_assertions)]
    {
        let mut path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if path.ends_with("src-tauri") {
            path.pop();
        }
        path.push("web");
        if target == "passionfruit" {
            path.push("passionfruit");
        } else if target == "root" {
            // Return web root
        } else {
            path.push("mod");
            path.push("data");
        }
        fs::create_dir_all(&path).unwrap_or_default();
        return path;
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

        if target == "passionfruit" {
            path.push("passionfruit");
        } else if target == "root" {
            // Return root
        } else {
            path.push("mod");
            path.push("data");
        }
        fs::create_dir_all(&path).unwrap_or_default();
        path
    }
}

pub fn get_config_file() -> PathBuf {
    get_hidden_config_dir("data").join("config.json")
}

/// 帳號資料永遠放在使用者資料目錄，不放進專案的 web 資源或 config.json。
/// 這個資料庫只會在程式執行後由使用者端建立，因此不會被 Tauri 打包帶走。
pub fn get_account_store_file() -> PathBuf {
    let base = USER_DATA_BASE_PATH
        .get()
        .cloned()
        .or_else(|| {
            directories::ProjectDirs::from("com", "MoLinOwO", "ReversedFront")
                .map(|dirs| dirs.data_local_dir().to_path_buf())
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    base.join("accounts.db")
}

/// 舊版帳號檔案的位置，只供第一次啟動時匯入，之後不再作為資料來源。
pub fn get_legacy_account_store_file() -> PathBuf {
    get_account_store_file()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("accounts.json")
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
