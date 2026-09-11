use crate::config_manager;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct AccountStore {
    #[serde(default)]
    accounts: Vec<Value>,
    #[serde(default)]
    active_account: Option<String>,
}

fn account_name(value: &Value) -> Option<&str> {
    value
        .get("account")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn target_account_name(target: Option<&Value>) -> Option<String> {
    target
        .and_then(|value| {
            value
                .get("account")
                .or_else(|| value.get("accountName"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn save_store(store: &AccountStore) -> bool {
    let path = config_manager::get_account_store_file();
    let Some(parent) = path.parent() else {
        return false;
    };

    if fs::create_dir_all(parent).is_err() {
        return false;
    }

    match serde_json::to_string_pretty(store) {
        Ok(content) => fs::write(path, content).is_ok(),
        Err(_) => false,
    }
}

fn migrate_legacy_config() -> Option<AccountStore> {
    let mut config = config_manager::load_config();
    let accounts = config.get("accounts")?.as_array()?.clone();
    let active_index = config
        .get("active_account_index")
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    let active_account = active_index
        .and_then(|index| accounts.get(index))
        .and_then(account_name)
        .map(ToOwned::to_owned)
        .or_else(|| {
            accounts
                .first()
                .and_then(account_name)
                .map(ToOwned::to_owned)
        });

    let store = AccountStore {
        accounts,
        active_account,
    };

    if !save_store(&store) {
        return Some(store);
    }

    // 舊版本的帳號密碼已搬到 accounts.json，從 config.json 移除敏感資料。
    if let Some(object) = config.as_object_mut() {
        object.remove("accounts");
        object.remove("active_account_index");
        config_manager::save_config(&config);
    }

    Some(store)
}

fn purge_legacy_config_accounts() {
    let mut config = config_manager::load_config();
    if let Some(object) = config.as_object_mut() {
        let had_legacy_accounts = object.remove("accounts").is_some();
        let had_legacy_index = object.remove("active_account_index").is_some();
        if had_legacy_accounts || had_legacy_index {
            config_manager::save_config(&config);
        }
    }
}

fn load_store() -> AccountStore {
    let path = config_manager::get_account_store_file();
    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(store) = serde_json::from_str::<AccountStore>(&content) {
            purge_legacy_config_accounts();
            return store;
        }
    }

    migrate_legacy_config().unwrap_or_default()
}

fn normalize_active_account(store: &mut AccountStore) -> bool {
    let previous = store.active_account.clone();
    let is_valid = store.active_account.as_deref().is_some_and(|name| {
        store
            .accounts
            .iter()
            .any(|account| account_name(account) == Some(name))
    });

    if !is_valid {
        store.active_account = store
            .accounts
            .iter()
            .find_map(account_name)
            .map(ToOwned::to_owned);
    }

    previous != store.active_account
}

pub fn get_accounts() -> Vec<Value> {
    let mut store = load_store();
    if normalize_active_account(&mut store) {
        let _ = save_store(&store);
    }
    store.accounts
}

pub fn add_account(mut data: Value) -> bool {
    let Some(name) = account_name(&data).map(ToOwned::to_owned) else {
        return false;
    };

    let mut store = load_store();
    if let Some(existing) = store
        .accounts
        .iter_mut()
        .find(|account| account_name(account) == Some(name.as_str()))
    {
        // 登入攔截器可能再次送出同一帳號；更新密碼但保留該帳號的模組設定。
        if let (Some(existing_object), Some(new_object)) =
            (existing.as_object_mut(), data.as_object_mut())
        {
            for (key, value) in new_object.iter() {
                if key != "settings" && !value.is_null() {
                    existing_object.insert(key.clone(), value.clone());
                }
            }
        }
    } else {
        if !data.get("settings").is_some_and(Value::is_object) {
            data["settings"] = serde_json::json!({});
        }
        store.accounts.push(data);
    }

    if store.active_account.is_none() {
        store.active_account = Some(name);
    }

    save_store(&store)
}

pub fn delete_account(index: usize) -> bool {
    let mut store = load_store();
    if index >= store.accounts.len() {
        return false;
    }

    let deleted_name = account_name(&store.accounts[index]).map(ToOwned::to_owned);
    store.accounts.remove(index);
    if store.active_account == deleted_name {
        store.active_account = store
            .accounts
            .get(index.min(store.accounts.len().saturating_sub(1)))
            .and_then(account_name)
            .map(ToOwned::to_owned);
    }
    normalize_active_account(&mut store);
    save_store(&store)
}

pub fn set_active_account(index: usize) -> bool {
    let mut store = load_store();
    let Some(name) = store.accounts.get(index).and_then(account_name) else {
        return false;
    };

    store.active_account = Some(name.to_owned());
    save_store(&store)
}

pub fn get_active_account() -> Option<Value> {
    let mut store = load_store();
    if normalize_active_account(&mut store) {
        let _ = save_store(&store);
    }

    let name = store.active_account.clone()?;
    store
        .accounts
        .into_iter()
        .find(|account| account_name(account) == Some(name.as_str()))
}

pub fn get_account_settings(target_account: Option<Value>) -> Value {
    let store = load_store();
    let target_name = target_account_name(target_account.as_ref())
        .or_else(|| store.active_account.clone())
        .or_else(|| {
            store
                .accounts
                .first()
                .and_then(account_name)
                .map(ToOwned::to_owned)
        });

    let settings = target_name
        .as_deref()
        .and_then(|name| {
            store
                .accounts
                .iter()
                .find(|account| account_name(account) == Some(name))
        })
        .and_then(|account| account.get("settings"))
        .filter(|value| value.is_object());

    serde_json::json!({
        "bgm": settings.and_then(|value| value.get("bgm_volume")).and_then(Value::as_f64).unwrap_or(1.0),
        "se": settings.and_then(|value| value.get("se_volume")).and_then(Value::as_f64).unwrap_or(1.0),
        "se147Muted": settings.and_then(|value| value.get("se147_muted")).and_then(Value::as_bool).unwrap_or(false),
        "report_faction_filter": settings
            .and_then(|value| value.get("report_faction_filter"))
            .and_then(Value::as_str)
            .unwrap_or("全部")
    })
}

pub fn update_account_settings(fields: Value) -> bool {
    let mut store = load_store();
    let target_name = target_account_name(fields.get("target_account"))
        .or_else(|| store.active_account.clone())
        .or_else(|| {
            store
                .accounts
                .first()
                .and_then(account_name)
                .map(ToOwned::to_owned)
        });
    let Some(target_name) = target_name else {
        return false;
    };

    let Some(account) = store
        .accounts
        .iter_mut()
        .find(|account| account_name(account) == Some(target_name.as_str()))
    else {
        return false;
    };

    if !account.get("settings").is_some_and(Value::is_object) {
        account["settings"] = serde_json::json!({});
    }
    let Some(settings) = account.get_mut("settings").and_then(Value::as_object_mut) else {
        return false;
    };

    let mut changed = false;
    for (source, destination) in [
        ("bgm", "bgm_volume"),
        ("se", "se_volume"),
        ("se147Muted", "se147_muted"),
        ("report_faction_filter", "report_faction_filter"),
    ] {
        if let Some(value) = fields.get(source).filter(|value| !value.is_null()) {
            settings.insert(destination.to_string(), value.clone());
            changed = true;
        }
    }

    changed && save_store(&store)
}
