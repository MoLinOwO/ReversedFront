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

pub fn update_config_fields(fields: Value) {
    let mut config = load_config();

    // Extract target account if present
    let target_account = fields.get("target_account").cloned();

    if let Some(target) = target_account {
        if let Some(accounts) = config.get_mut("accounts").and_then(|v| v.as_array_mut()) {
            let target_email = target.get("account").and_then(|v| v.as_str()).unwrap_or("");

            for account in accounts {
                if account.get("account").and_then(|v| v.as_str()) == Some(target_email) {
                    // Ensure settings object exists
                    if !account.get("settings").is_some() {
                        account["settings"] = serde_json::json!({});
                    }

                    if let Some(settings) =
                        account.get_mut("settings").and_then(|v| v.as_object_mut())
                    {
                        // Update fields if they are not null
                        if let Some(bgm) = fields.get("bgm") {
                            if !bgm.is_null() {
                                settings.insert("bgm_volume".to_string(), bgm.clone());
                            }
                        }
                        if let Some(se) = fields.get("se") {
                            if !se.is_null() {
                                settings.insert("se_volume".to_string(), se.clone());
                            }
                        }
                        if let Some(muted) = fields.get("se147Muted") {
                            if !muted.is_null() {
                                settings.insert("se147_muted".to_string(), muted.clone());
                            }
                        }
                        if let Some(filter) = fields.get("report_faction_filter") {
                            if !filter.is_null() {
                                settings
                                    .insert("report_faction_filter".to_string(), filter.clone());
                            }
                        }
                    }
                    break;
                }
            }
        }
    } else {
        // Fallback to global update if no target account (legacy behavior, but safer)
        if let Value::Object(ref mut map) = config {
            if let Value::Object(fields_map) = fields {
                for (k, v) in fields_map {
                    if k != "target_account" && !v.is_null() {
                        map.insert(k, v);
                    }
                }
            }
        }
    }

    save_config(&config);
}

pub fn get_account_settings(target_account: Option<String>) -> Value {
    let config = load_config();

    // Default values
    let mut bgm = 1.0;
    let mut se = 1.0;
    let mut se147_muted = false;
    let mut report_faction_filter = "全部".to_string();

    if let Some(target_json) = target_account {
        if let Ok(target) = serde_json::from_str::<Value>(&target_json) {
            let target_email = target.get("account").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(accounts) = config.get("accounts").and_then(|v| v.as_array()) {
                for account in accounts {
                    if account.get("account").and_then(|v| v.as_str()) == Some(target_email) {
                        if let Some(settings) = account.get("settings") {
                            if let Some(v) = settings.get("bgm_volume").and_then(|v| v.as_f64()) {
                                bgm = v;
                            }
                            if let Some(v) = settings.get("se_volume").and_then(|v| v.as_f64()) {
                                se = v;
                            }
                            if let Some(v) = settings.get("se147_muted").and_then(|v| v.as_bool()) {
                                se147_muted = v;
                            }
                            if let Some(v) = settings
                                .get("report_faction_filter")
                                .and_then(|v| v.as_str())
                            {
                                report_faction_filter = v.to_string();
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    serde_json::json!({
        "bgm": bgm,
        "se": se,
        "se147Muted": se147_muted,
        "report_faction_filter": report_faction_filter
    })
}
