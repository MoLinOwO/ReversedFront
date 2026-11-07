use anyhow::{bail, Result};
use reqwest::Client;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tokio::time::{sleep, Duration};

const SERVER_BASE_URL: &str = "https://media.komisureiya.com/";
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub queue_length: usize,
    pub is_downloading: bool,
    pub active_downloads: usize,
    pub max_workers: usize,
    pub downloaded_count: usize,
}

pub struct ResourceManager {
    client: Client,
    base_dir: PathBuf,
    downloaded_resources: Mutex<HashSet<String>>,
    inflight: Mutex<HashSet<String>>,
    active_downloads: Mutex<usize>,
    downloaded_count: Mutex<usize>,
    semaphore: Arc<Semaphore>,
    max_workers: usize,
}

impl ResourceManager {
    pub fn new() -> Arc<Self> {
        // Keep downloaded game resources beside the installed executable.
        // Account data and Mod settings remain in the separate user-data DB.
        let base_dir = crate::config_manager::get_resource_base_dir();
        // Ensure passionfruit directory exists
        let passionfruit_dir = base_dir.join("passionfruit");
        fs::create_dir_all(&passionfruit_dir).unwrap_or_default();

        let client = Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap();

        let cpu_count = num_cpus::get();
        let max_workers = (cpu_count * 2).clamp(4, 32);

        Arc::new(ResourceManager {
            client,
            base_dir,
            downloaded_resources: Mutex::new(HashSet::new()),
            inflight: Mutex::new(HashSet::new()),
            active_downloads: Mutex::new(0),
            downloaded_count: Mutex::new(0),
            semaphore: Arc::new(Semaphore::new(max_workers)),
            max_workers,
        })
    }

    fn normalize_path(&self, resource_path: &str) -> Option<(String, PathBuf)> {
        let resource_path = resource_path.replace('\\', "/");

        // Strip query params
        let clean = if let Some(idx) = resource_path.find('?') {
            &resource_path[..idx]
        } else {
            &resource_path
        };

        // Drop duplicated prefix
        let local_path = if let Some(stripped) = clean.strip_prefix("assets/") {
            stripped
        } else {
            clean
        };

        let trimmed = local_path.trim_start_matches('/');
        let relative = Path::new(trimmed);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            return None;
        }
        let normalized = trimmed.replace('/', std::path::MAIN_SEPARATOR_STR);
        let abs_path = self.base_dir.join(&normalized);
        Some((trimmed.to_string(), abs_path))
    }

    fn find_existing_case_insensitive(&self, abs_path: &Path) -> Option<PathBuf> {
        if abs_path.exists() {
            return Some(abs_path.to_path_buf());
        }

        if let Ok(canonical) = abs_path.canonicalize() {
            if canonical.exists() {
                return Some(canonical);
            }
        }

        if let Some(parent) = abs_path.parent() {
            if let Ok(entries) = fs::read_dir(parent) {
                let target_lower = abs_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                        if name.to_lowercase() == target_lower {
                            return Some(entry_path);
                        }
                    }
                }
            }
        }

        None
    }

    pub fn exists_local(&self, resource_path: String) -> (bool, Option<PathBuf>) {
        if let Some((local_path, abs_path)) = self.normalize_path(&resource_path) {
            if self
                .downloaded_resources
                .lock()
                .unwrap()
                .contains(&local_path)
            {
                return (true, Some(abs_path));
            }

            if let Some(found) = self.find_existing_case_insensitive(&abs_path) {
                self.downloaded_resources.lock().unwrap().insert(local_path);
                return (true, Some(found));
            }
        }

        (false, None)
    }

    async fn fetch_and_cache(&self, local_path: &str, abs_path: PathBuf) -> Result<Vec<u8>> {
        let _permit = self.semaphore.acquire().await?;
        let _active_guard = ActiveGuard::new(&self.active_downloads);

        // Inflight guard to avoid thrashing
        let inserted = self.inflight.lock().unwrap().insert(local_path.to_string());
        let inflight_guard = if inserted {
            Some(InflightGuard::new(&self.inflight, local_path.to_string()))
        } else {
            None
        };

        // 同程序內相同資源只允許一個下載者；其他請求等待該下載完成。
        if !inserted {
            for _ in 0..600 {
                if abs_path.exists() {
                    let bytes = fs::read(&abs_path)?;
                    self.downloaded_resources
                        .lock()
                        .unwrap()
                        .insert(local_path.to_string());
                    return Ok(bytes);
                }
                if !self.inflight.lock().unwrap().contains(local_path) {
                    break;
                }
                sleep(Duration::from_millis(50)).await;
            }
            if abs_path.exists() {
                return Ok(fs::read(&abs_path)?);
            }
            bail!("concurrent resource download failed: {}", local_path);
        }

        // Strip passionfruit/ prefix for remote URL if present, as the server likely serves from root
        let url_path = if let Some(stripped) = local_path.strip_prefix("passionfruit/") {
            stripped
        } else {
            local_path
        };

        let url = format!("{}{}", SERVER_BASE_URL, url_path);
        let resp = self.client.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            bail!("upstream {}", status);
        }

        let bytes = resp.bytes().await?.to_vec();

        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // 多個 ReversedFront 程序可能同時下載同一檔案，暫存檔名必須唯一。
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_path =
            abs_path.with_extension(format!("download-{}-{}", std::process::id(), sequence));
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(&bytes)?;
        file.flush()?;
        drop(file);
        if let Err(error) = fs::rename(&temp_path, &abs_path) {
            // 另一程序可能已經先完成相同檔案；Windows 不允許 rename
            // 覆蓋既有檔案，因此直接採用已完成的版本。
            if abs_path.exists() {
                let _ = fs::remove_file(&temp_path);
                return Ok(fs::read(&abs_path)?);
            }
            let _ = fs::remove_file(&temp_path);
            return Err(error.into());
        }

        self.downloaded_resources
            .lock()
            .unwrap()
            .insert(local_path.to_string());

        {
            let mut count = self.downloaded_count.lock().unwrap();
            *count += 1;
        }

        drop(inflight_guard);

        Ok(bytes)
    }

    pub async fn get_or_fetch(&self, resource_path: &str) -> Result<Option<ResourceResponse>> {
        let (local_path, abs_path) = match self.normalize_path(resource_path) {
            Some(v) => v,
            None => return Ok(None),
        };

        if self
            .downloaded_resources
            .lock()
            .unwrap()
            .contains(&local_path)
        {
            let bytes = fs::read(&abs_path)?;
            return Ok(Some(ResourceResponse {
                data: bytes,
                local_path,
                abs_path,
            }));
        }

        if let Some(found) = self.find_existing_case_insensitive(&abs_path) {
            let bytes = fs::read(&found)?;
            self.downloaded_resources
                .lock()
                .unwrap()
                .insert(local_path.clone());
            return Ok(Some(ResourceResponse {
                data: bytes,
                local_path,
                abs_path: found,
            }));
        }

        let bytes = self.fetch_and_cache(&local_path, abs_path.clone()).await?;
        Ok(Some(ResourceResponse {
            data: bytes,
            local_path,
            abs_path,
        }))
    }

    pub fn get_status(&self) -> DownloadStatus {
        let active = self.active_downloads.lock().unwrap();
        let inflight = self.inflight.lock().unwrap();
        let count = self.downloaded_count.lock().unwrap();

        DownloadStatus {
            queue_length: inflight.len(),
            is_downloading: !inflight.is_empty() || *active > 0,
            active_downloads: *active,
            max_workers: self.max_workers,
            downloaded_count: *count,
        }
    }
}

pub struct ResourceResponse {
    pub data: Vec<u8>,
    pub local_path: String,
    pub abs_path: PathBuf,
}

struct ActiveGuard<'a> {
    counter: &'a Mutex<usize>,
}

impl<'a> ActiveGuard<'a> {
    fn new(counter: &'a Mutex<usize>) -> Self {
        let mut c = counter.lock().unwrap();
        *c += 1;
        Self { counter }
    }
}

impl<'a> Drop for ActiveGuard<'a> {
    fn drop(&mut self) {
        let mut c = self.counter.lock().unwrap();
        *c = c.saturating_sub(1);
    }
}

struct InflightGuard<'a> {
    set: &'a Mutex<HashSet<String>>,
    key: String,
}

impl<'a> InflightGuard<'a> {
    fn new(set: &'a Mutex<HashSet<String>>, key: String) -> Self {
        Self { set, key }
    }
}

impl<'a> Drop for InflightGuard<'a> {
    fn drop(&mut self) {
        self.set.lock().unwrap().remove(&self.key);
    }
}

pub fn get_hidden_config_dir(target: &str) -> PathBuf {
    crate::config_manager::get_hidden_config_dir(target)
}
