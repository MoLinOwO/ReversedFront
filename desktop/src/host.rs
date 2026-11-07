use crate::resource_manager::ResourceManager;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{mpsc::Sender, Arc, Mutex};
use tokio::sync::oneshot;

pub enum UiRequest {
    Exit,
    ToggleFullscreen(oneshot::Sender<Result<bool, String>>),
}

/// Plain backend context, shared with workers through `Arc<DesktopHost>`.
pub struct DesktopHost {
    pub resource_root: PathBuf,
    pub app_data_dir: PathBuf,
    pub process_slot: usize,
    pub resource_manager: Arc<ResourceManager>,
    pub ui_tx: Sender<UiRequest>,
    pub(crate) hold_update_guards: Mutex<Vec<File>>,
}

impl DesktopHost {
    pub fn new(
        resource_root: PathBuf,
        app_data_dir: PathBuf,
        process_slot: usize,
        resource_manager: Arc<ResourceManager>,
        ui_tx: Sender<UiRequest>,
    ) -> Self {
        Self {
            resource_root,
            app_data_dir,
            process_slot,
            resource_manager,
            ui_tx,
            hold_update_guards: Mutex::new(Vec::new()),
        }
    }

    pub fn request_exit(&self) -> Result<(), String> {
        self.ui_tx
            .send(UiRequest::Exit)
            .map_err(|_| "桌面介面已關閉，無法要求退出".to_string())
    }

    pub async fn toggle_fullscreen(&self) -> Result<bool, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.ui_tx
            .send(UiRequest::ToggleFullscreen(reply_tx))
            .map_err(|_| "桌面介面已關閉，無法切換全螢幕".to_string())?;
        reply_rx
            .await
            .map_err(|_| "桌面介面未回覆全螢幕狀態".to_string())?
    }
}

impl Drop for DesktopHost {
    fn drop(&mut self) {
        // ponytail: at most two handles after an update handoff. Let the OS
        // release them at process exit, even if UI/runtime teardown drops this
        // context first. A future dedicated updater can own this lifetime.
        let guards = self
            .hold_update_guards
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for guard in guards.drain(..) {
            std::mem::forget(guard);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;
    use std::fs::{self, OpenOptions};
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn ui_replies_and_update_guards_survive_host_teardown() {
        const CHILD_ROOT: &str = "RF_HOST_GUARDS_TEST_ROOT";
        const LOCKS: [&str; 2] = ["parent.lock", "update.lock"];
        if let Some(root) = std::env::var_os(CHILD_ROOT) {
            let root = PathBuf::from(root);
            // Isolate ResourceManager's global storage path in this child.
            crate::config_manager::set_resource_base_path(root.clone());
            let (ui_tx, ui_rx) = mpsc::channel();
            let host =
                DesktopHost::new(root.clone(), root.clone(), 0, ResourceManager::new(), ui_tx);
            host.request_exit().unwrap();
            assert!(matches!(ui_rx.recv().unwrap(), UiRequest::Exit));

            let ui = std::thread::spawn(move || {
                for response in [Ok(true), Err("fullscreen failed".to_string())] {
                    let UiRequest::ToggleFullscreen(reply) =
                        ui_rx.recv_timeout(Duration::from_secs(5)).unwrap()
                    else {
                        panic!("expected fullscreen request");
                    };
                    reply.send(response).unwrap();
                }
                // Dropping the reply must surface an error to the caller.
                let request = ui_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                assert!(matches!(request, UiRequest::ToggleFullscreen(_)));
                drop(request);
            });
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            runtime.block_on(async {
                assert_eq!(host.toggle_fullscreen().await, Ok(true));
                assert_eq!(
                    host.toggle_fullscreen().await,
                    Err("fullscreen failed".to_string())
                );
                assert!(host.toggle_fullscreen().await.is_err());
            });
            ui.join().unwrap();
            assert!(host.request_exit().is_err());
            assert!(runtime.block_on(host.toggle_fullscreen()).is_err());

            for name in LOCKS {
                let file = OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(root.join(name))
                    .unwrap();
                FileExt::lock_exclusive(&file).unwrap();
                host.hold_update_guards.lock().unwrap().push(file);
            }
            drop(host);
            for name in LOCKS {
                let probe = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(root.join(name))
                    .unwrap();
                assert!(FileExt::try_lock_exclusive(&probe).is_err());
            }
            return;
        }

        let root = std::env::temp_dir().join(format!(
            "rf-host-guards-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "host::tests::ui_replies_and_update_guards_survive_host_teardown",
                "--nocapture",
            ])
            .env(CHILD_ROOT, &root)
            .output()
            .unwrap();
        if !output.status.success() {
            let _ = fs::remove_dir_all(&root);
            panic!(
                "child failed: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        for name in LOCKS {
            let probe = OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.join(name))
                .unwrap();
            FileExt::try_lock_exclusive(&probe).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }
}
