use crate::config_manager;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL UNIQUE,
    password TEXT NOT NULL DEFAULT '',
    profile_json TEXT NOT NULL DEFAULT '{}',
    settings_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS app_state (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
"#;

fn database_path() -> PathBuf {
    config_manager::get_account_store_file()
}

fn open_database() -> rusqlite::Result<Connection> {
    let path = database_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }

    let mut connection = Connection::open(path)?;
    // SQLite 的 busy timeout、WAL 與交易是跨進程同步的核心；不再需要
    // accounts.json 那種「讀取後整份寫回」的競爭鎖。
    connection.busy_timeout(Duration::from_secs(10))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.execute_batch(SCHEMA)?;
    migrate_legacy_data(&mut connection)?;
    Ok(connection)
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

fn account_parts(data: &Value) -> Option<(String, String, String, String)> {
    let name = account_name(data)?.to_owned();
    let password = data
        .get("password")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let mut profile = data.as_object().cloned().unwrap_or_default();
    profile.remove("account");
    profile.remove("password");
    profile.remove("settings");

    let settings = data
        .get("settings")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    Some((
        name,
        password,
        serde_json::to_string(&profile).ok()?,
        serde_json::to_string(&settings).ok()?,
    ))
}

fn value_from_row(
    account: String,
    password: String,
    profile_json: String,
    settings_json: String,
) -> Value {
    let mut profile = serde_json::from_str::<Value>(&profile_json)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let settings = serde_json::from_str::<Value>(&settings_json)
        .ok()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| serde_json::json!({}));

    profile.insert("account".to_string(), Value::String(account));
    profile.insert("password".to_string(), Value::String(password));
    profile.insert("settings".to_string(), settings);
    Value::Object(profile)
}

fn query_accounts(tx: &Transaction<'_>) -> rusqlite::Result<Vec<Value>> {
    let mut statement = tx.prepare(
        "SELECT account, password, profile_json, settings_json
         FROM accounts ORDER BY id ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(value_from_row(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
        ))
    })?;

    rows.collect()
}

fn get_state(tx: &Transaction<'_>, key: &str) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT value FROM app_state WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

fn set_state(tx: &Transaction<'_>, key: &str, value: Option<&str>) -> rusqlite::Result<()> {
    match value {
        Some(value) => tx.execute(
            "INSERT INTO app_state(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?,
        None => tx.execute("DELETE FROM app_state WHERE key = ?1", params![key])?,
    };
    Ok(())
}

fn first_account_name(tx: &Transaction<'_>) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT account FROM accounts ORDER BY id ASC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
}

fn account_name_at(tx: &Transaction<'_>, index: usize) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT account FROM accounts ORDER BY id ASC LIMIT 1 OFFSET ?1",
        params![index as i64],
        |row| row.get(0),
    )
    .optional()
}

fn normalize_active_account(tx: &Transaction<'_>) -> rusqlite::Result<Option<String>> {
    let active = get_state(tx, "active_account")?;
    if let Some(active_name) = active {
        let exists: Option<String> = tx
            .query_row(
                "SELECT account FROM accounts WHERE account = ?1",
                params![active_name],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Ok(exists);
        }
    }

    let first = first_account_name(tx)?;
    set_state(tx, "active_account", first.as_deref())?;
    Ok(first)
}

fn upsert_account(tx: &Transaction<'_>, data: &Value) -> rusqlite::Result<bool> {
    let Some((name, password, profile_json, settings_json)) = account_parts(data) else {
        return Ok(false);
    };

    let existing: Option<String> = tx
        .query_row(
            "SELECT account FROM accounts WHERE account = ?1",
            params![name],
            |row| row.get(0),
        )
        .optional()?;

    if existing.is_some() {
        // 登入攔截器可能再次送出同一帳號；更新密碼及其他欄位，
        // 但保留這個帳號原本的模組設定。
        tx.execute(
            "UPDATE accounts SET password = ?1, profile_json = ?2 WHERE account = ?3",
            params![password, profile_json, name],
        )?;
    } else {
        tx.execute(
            "INSERT INTO accounts(account, password, profile_json, settings_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![name, password, profile_json, settings_json],
        )?;
    }
    Ok(true)
}

fn insert_legacy_account(tx: &Transaction<'_>, data: &Value) -> rusqlite::Result<bool> {
    let Some((name, password, profile_json, settings_json)) = account_parts(data) else {
        return Ok(false);
    };
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO accounts(account, password, profile_json, settings_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![name, password, profile_json, settings_json],
    )?;
    Ok(inserted > 0)
}

fn legacy_json_source() -> (Vec<Value>, Option<String>, bool) {
    let path = config_manager::get_legacy_account_store_file();
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(value) = serde_json::from_str::<Value>(&content) {
            let accounts = value
                .get("accounts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let active = value
                .get("active_account")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            if !accounts.is_empty() {
                return (accounts, active, true);
            }
        }
    }
    (Vec::new(), None, false)
}

fn legacy_config_source() -> (Vec<Value>, Option<String>) {
    let config = config_manager::load_config();
    let accounts = config
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let active = config
        .get("active_account_index")
        .and_then(Value::as_u64)
        .and_then(|index| accounts.get(index as usize))
        .and_then(account_name)
        .map(ToOwned::to_owned)
        .or_else(|| {
            accounts
                .first()
                .and_then(account_name)
                .map(ToOwned::to_owned)
        });
    (accounts, active)
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

fn import_legacy_database(
    connection: &mut Connection,
    legacy_path: &Path,
) -> rusqlite::Result<bool> {
    if !legacy_path.exists() || legacy_path == database_path().as_path() {
        return Ok(false);
    }

    let legacy = match Connection::open(legacy_path) {
        Ok(connection) => connection,
        Err(_) => return Ok(false),
    };

    let mut statement = match legacy.prepare(
        "SELECT account, password, profile_json, settings_json
         FROM accounts ORDER BY id ASC",
    ) {
        Ok(statement) => statement,
        Err(_) => return Ok(false),
    };
    let mut rows = statement.query([])?;
    let mut accounts = Vec::new();
    while let Some(row) = rows.next()? {
        accounts.push(value_from_row(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
        ));
    }
    drop(rows);
    drop(statement);

    if accounts.is_empty() {
        return Ok(false);
    }

    let active_name = legacy
        .query_row(
            "SELECT value FROM app_state WHERE key = 'active_account'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    let tx = connection.transaction()?;
    let mut first_imported_name = None;
    for account in &accounts {
        if insert_legacy_account(&tx, account)? && first_imported_name.is_none() {
            first_imported_name = account_name(account).map(ToOwned::to_owned);
        }
    }
    let selected = active_name
        .filter(|name| {
            tx.query_row(
                "SELECT 1 FROM accounts WHERE account = ?1",
                params![name],
                |_| Ok(()),
            )
            .is_ok()
        })
        .or(first_imported_name)
        .or_else(|| first_account_name(&tx).ok().flatten());
    set_state(&tx, "active_account", selected.as_deref())?;
    tx.commit()?;
    Ok(true)
}

fn migrate_legacy_data(connection: &mut Connection) -> rusqlite::Result<()> {
    let account_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))?;
    let legacy_path = config_manager::get_legacy_account_store_file();

    if account_count == 0
        && import_legacy_database(connection, &config_manager::get_legacy_account_db_file())?
    {
        // 保留舊的使用者資料庫作為備份；之後以安裝目錄內的新 DB 為唯一來源。
        purge_legacy_config_accounts();
        return Ok(());
    }

    let (accounts, active_name, imported_json) = if account_count == 0 {
        let (json_accounts, json_active, imported_json) = legacy_json_source();
        if imported_json {
            (json_accounts, json_active, true)
        } else {
            let (config_accounts, config_active) = legacy_config_source();
            (config_accounts, config_active, false)
        }
    } else {
        (Vec::new(), None, false)
    };

    if account_count == 0 && !accounts.is_empty() {
        let tx = connection.transaction()?;
        let mut first_imported_name = None;
        for account in &accounts {
            if insert_legacy_account(&tx, account)? && first_imported_name.is_none() {
                first_imported_name = account_name(account).map(ToOwned::to_owned);
            }
        }
        let selected = active_name
            .filter(|name| {
                tx.query_row(
                    "SELECT 1 FROM accounts WHERE account = ?1",
                    params![name],
                    |_| Ok(()),
                )
                .is_ok()
            })
            .or(first_imported_name)
            .or_else(|| first_account_name(&tx).ok().flatten());
        set_state(&tx, "active_account", selected.as_deref())?;
        tx.commit()?;

        // DB 已完成交易提交後才清掉舊的明文 JSON，避免遷移中斷造成帳號遺失。
        if imported_json {
            let _ = fs::remove_file(legacy_path);
        }
        purge_legacy_config_accounts();
    } else if account_count > 0 {
        // 舊檔案若因升級流程留下，DB 已是完整來源，直接移除避免之後誤用。
        let _ = fs::remove_file(legacy_path);
        purge_legacy_config_accounts();
    }

    Ok(())
}

fn with_transaction<T, F>(operation: F) -> Option<T>
where
    F: FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
{
    let mut connection = open_database().ok()?;
    let transaction = connection.transaction().ok()?;
    let result = operation(&transaction).ok()?;
    transaction.commit().ok()?;
    Some(result)
}

pub fn get_accounts() -> Vec<Value> {
    with_transaction(|tx| {
        normalize_active_account(tx)?;
        query_accounts(tx)
    })
    .unwrap_or_default()
}

pub fn add_account(data: Value) -> bool {
    with_transaction(|tx| {
        let Some(name) = account_name(&data).map(ToOwned::to_owned) else {
            return Ok(false);
        };
        let added = upsert_account(tx, &data)?;
        if get_state(tx, "active_account")?.is_none() {
            set_state(tx, "active_account", Some(&name))?;
        }
        Ok(added)
    })
    .unwrap_or(false)
}

pub fn delete_account(index: usize) -> bool {
    with_transaction(|tx| {
        let Some(deleted_name) = account_name_at(tx, index)? else {
            return Ok(false);
        };
        tx.execute(
            "DELETE FROM accounts WHERE account = ?1",
            params![deleted_name],
        )?;

        if get_state(tx, "active_account")?.as_deref() == Some(deleted_name.as_str()) {
            let next_index = index.saturating_sub(1);
            let next_name =
                account_name_at(tx, next_index)?.or_else(|| first_account_name(tx).ok().flatten());
            set_state(tx, "active_account", next_name.as_deref())?;
        } else {
            normalize_active_account(tx)?;
        }
        Ok(true)
    })
    .unwrap_or(false)
}

pub fn get_active_account() -> Option<Value> {
    with_transaction(|tx| {
        let name = normalize_active_account(tx)?;
        let Some(name) = name else {
            return Ok(None);
        };
        tx.query_row(
            "SELECT account, password, profile_json, settings_json
             FROM accounts WHERE account = ?1",
            params![name],
            |row| {
                Ok(value_from_row(
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                ))
            },
        )
        .optional()
    })
    .flatten()
}

fn default_settings() -> Value {
    serde_json::json!({
        "bgm": 1.0,
        "se": 1.0,
        "se147Muted": false,
        "report_faction_filter": "全部"
    })
}

fn settings_for_account(tx: &Transaction<'_>, name: &str) -> rusqlite::Result<Option<Value>> {
    tx.query_row(
        "SELECT settings_json FROM accounts WHERE account = ?1",
        params![name],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|value| {
        value.map(|json| {
            serde_json::from_str::<Value>(&json)
                .ok()
                .filter(|value| value.is_object())
                .unwrap_or_else(|| serde_json::json!({}))
        })
    })
}

pub fn get_account_settings(target_account: Option<Value>) -> Value {
    with_transaction(|tx| {
        let target_name = target_account_name(target_account.as_ref())
            .or(normalize_active_account(tx)?)
            .or(first_account_name(tx)?);
        let Some(target_name) = target_name else {
            return Ok(default_settings());
        };
        let Some(settings) = settings_for_account(tx, &target_name)? else {
            return Ok(default_settings());
        };

        Ok(serde_json::json!({
            "bgm": settings.get("bgm_volume").and_then(Value::as_f64).unwrap_or(1.0),
            "se": settings.get("se_volume").and_then(Value::as_f64).unwrap_or(1.0),
            "se147Muted": settings.get("se147_muted").and_then(Value::as_bool).unwrap_or(false),
            "report_faction_filter": settings
                .get("report_faction_filter")
                .and_then(Value::as_str)
                .unwrap_or("全部")
        }))
    })
    .unwrap_or_else(default_settings)
}

pub fn update_account_settings(fields: Value) -> bool {
    with_transaction(|tx| {
        let target_name = target_account_name(fields.get("target_account"))
            .or(normalize_active_account(tx)?)
            .or(first_account_name(tx)?);
        let Some(target_name) = target_name else {
            return Ok(false);
        };
        let Some(mut settings) = settings_for_account(tx, &target_name)? else {
            return Ok(false);
        };
        let Some(settings_object) = settings.as_object_mut() else {
            return Ok(false);
        };

        let mut changed = false;
        for (source, destination) in [
            ("bgm", "bgm_volume"),
            ("se", "se_volume"),
            ("se147Muted", "se147_muted"),
            ("report_faction_filter", "report_faction_filter"),
        ] {
            if let Some(value) = fields.get(source).filter(|value| !value.is_null()) {
                settings_object.insert(destination.to_string(), value.clone());
                changed = true;
            }
        }

        if changed {
            tx.execute(
                "UPDATE accounts SET settings_json = ?1 WHERE account = ?2",
                params![
                    serde_json::to_string(&settings).unwrap_or_else(|_| "{}".to_string()),
                    target_name
                ],
            )?;
        }
        Ok(changed)
    })
    .unwrap_or(false)
}
