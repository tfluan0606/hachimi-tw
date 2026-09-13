use std::{fs, path::{Path, PathBuf}, sync::{atomic::{self, AtomicBool}, Arc, Mutex}};

use arc_swap::ArcSwap;
use fnv::FnvHashMap;
use rust_i18n::t;
use serde::Deserialize;
use windows::Win32::{Foundation::MAX_PATH, System::LibraryLoader::GetModuleFileNameW};

use crate::core::{gui::{PersistentMessageWindow, SimpleYesNoDialog}, http, Error, Gui, Hachimi};

use super::main::DLL_HMODULE;

// 繁中 fork 的自己的 repo。自動更新只認這裡的 release。
const REPO_PATH: &str = "tfluan0606/hachimi-tw";
// release 要附這兩個 asset：實際的 DLL，以及一份 blake3 雜湊清單。
const DLL_ASSET_NAME: &str = "version.dll";
const HASH_ASSET_NAME: &str = "blake3.json";
const CHUNK_SIZE: usize = 8192; // 8KiB

#[derive(Clone)]
struct DllUpdate {
    dll_url: String,
    hash_url: String
}

#[derive(Default)]
pub struct Updater {
    update_check_mutex: Mutex<()>,
    new_update: ArcSwap<Option<DllUpdate>>
}

impl Updater {
    pub fn check_for_updates(self: Arc<Self>, callback: fn(bool)) {
        std::thread::spawn(move || {
            match self.check_for_updates_internal() {
                Ok(v) => callback(v),
                Err(e) => error!("{}", e)
            }
        });
    }

    fn check_for_updates_internal(&self) -> Result<bool, Error> {
        // Prevent multiple update checks running at the same time
        let Ok(_guard) = self.update_check_mutex.try_lock() else {
            return Ok(false);
        };

        if let Some(mutex) = Gui::instance() {
            mutex.lock().unwrap().show_notification(&t!("notification.checking_for_updates"));
        }

        let latest: Release = http::get_json(&format!("https://api.github.com/repos/{}/releases/latest", REPO_PATH))?;
        if latest.is_different_version() {
            let mut dll_url = None;
            let mut hash_url = None;
            for asset in latest.assets {
                if asset.name == DLL_ASSET_NAME {
                    dll_url = Some(asset.browser_download_url);
                }
                else if asset.name == HASH_ASSET_NAME {
                    hash_url = Some(asset.browser_download_url);
                }
            }

            if let (Some(dll_url), Some(hash_url)) = (dll_url, hash_url) {
                self.new_update.store(Arc::new(Some(DllUpdate { dll_url, hash_url })));
                if let Some(mutex) = Gui::instance() {
                    mutex.lock().unwrap().show_window(Box::new(SimpleYesNoDialog::new(
                        &t!("update_prompt_dialog.title"),
                        &t!("update_prompt_dialog.content", version = latest.tag_name),
                        |ok| {
                            if !ok { return; }
                            Hachimi::instance().updater.clone().run();
                        }
                    )));
                }
                return Ok(true);
            }
            else {
                // 有新版但 release 少了 DLL 或 blake3.json，沒法安全更新，只記 log 不打擾使用者。
                warn!("Release '{}' is missing '{}' or '{}' asset; skipping update", latest.tag_name, DLL_ASSET_NAME, HASH_ASSET_NAME);
            }
        }
        else if let Some(mutex) = Gui::instance() {
            mutex.lock().unwrap().show_notification(&t!("notification.no_updates"));
        }

        Ok(false)
    }

    pub fn run(self: Arc<Self>) {
        std::thread::spawn(move || {
            let dialog_show = Arc::new(AtomicBool::new(true));
            if let Some(mutex) = Gui::instance() {
                mutex.lock().unwrap().show_window(Box::new(PersistentMessageWindow::new(
                    &t!("updating_dialog.title"),
                    &t!("updating_dialog.content"),
                    dialog_show.clone()
                )));
            }

            let res = self.clone().run_internal();

            dialog_show.store(false, atomic::Ordering::Relaxed);

            if let Some(mutex) = Gui::instance() {
                let mut gui = mutex.lock().unwrap();
                match res {
                    Ok(()) => gui.show_notification(&t!("notification.update_ready_restart")),
                    Err(e) => {
                        error!("{}", e);
                        gui.show_notification(&t!("notification.update_failed", reason = e.to_string()));
                    }
                }
            }
            else if let Err(e) = res {
                error!("{}", e);
            }
        });
    }

    fn run_internal(self: Arc<Self>) -> Result<(), Error> {
        let Some(update) = (**self.new_update.load()).clone() else {
            return Ok(());
        };
        self.new_update.store(Arc::new(None));

        // 現役 version.dll 的完整路徑（可能被改名成別的檔名，用實際載入路徑最準）。
        let dll_path = current_dll_path()?;
        let new_path = with_suffix(&dll_path, ".new");
        let old_path = with_suffix(&dll_path, ".old");

        // 先抓 blake3.json，拿到預期的雜湊。
        let hashes: FnvHashMap<String, String> = http::get_json(&update.hash_url)?;
        let Some(expected_hash) = hashes.get(DLL_ASSET_NAME) else {
            return Err(Error::RuntimeError(format!("'{}' has no entry for '{}'", HASH_ASSET_NAME, DLL_ASSET_NAME)));
        };

        // 清掉上次殘留的 .new，再下載新 DLL 並邊算 blake3。
        if new_path.exists() {
            fs::remove_file(&new_path)?;
        }
        {
            let mut file = fs::File::create(&new_path)?;
            let res = ureq::get(&update.dll_url).call()?;
            let mut hasher = blake3::Hasher::new();
            let mut buffer = [0u8; CHUNK_SIZE];
            http::download_file_buffered(res, &mut file, &mut buffer, |bytes| {
                hasher.update(bytes);
            })?;
            file.sync_data()?;

            let hash = hasher.finalize().to_hex().to_string();
            if &hash != expected_hash {
                let _ = fs::remove_file(&new_path);
                return Err(Error::FileHashMismatch(new_path.to_string_lossy().into_owned()));
            }
        }

        // 清掉上次殘留的 .old（可能上次啟動來不及清）。
        if old_path.exists() {
            fs::remove_file(&old_path)?;
        }

        // Windows 允許「改名」載入中的 DLL（只是不允許刪除），所以不需要 helper exe 或管理員權限：
        //   version.dll -> version.dll.old ，接著 .new -> version.dll。
        // 都在同一個資料夾（同磁碟機）內，是純 metadata rename，不會複製檔案。
        fs::rename(&dll_path, &old_path)?;
        if let Err(e) = fs::rename(&new_path, &dll_path) {
            // 換檔失敗，把舊 DLL 搬回去回滾，避免遊戲下次找不到 version.dll。
            error!("Failed to move new DLL into place: {}", e);
            if let Err(re) = fs::rename(&old_path, &dll_path) {
                error!("Rollback failed too: {}", re);
            }
            return Err(e.into());
        }

        info!("Updated DLL in place; old version kept at '{}' until next launch", old_path.display());
        Ok(())
    }
}

/// 開機時清掉上次更新留下的 `version.dll.old`（此時它已不再被載入，可以安全刪除）。
pub fn cleanup_old_dll() {
    let Ok(dll_path) = current_dll_path() else {
        return;
    };
    let old_path = with_suffix(&dll_path, ".old");
    if old_path.is_file() {
        match fs::remove_file(&old_path) {
            Ok(()) => info!("Cleaned up old DLL: {}", old_path.display()),
            // 刪不掉不是致命錯誤（例如檔案還被別的程序抓著），下次啟動再試。
            Err(e) => warn!("Failed to remove old DLL '{}': {}", old_path.display(), e)
        }
    }
}

fn current_dll_path() -> Result<PathBuf, Error> {
    let mut buf = [0u16; MAX_PATH as usize];
    let len = unsafe { GetModuleFileNameW(DLL_HMODULE, &mut buf) } as usize;
    if len == 0 || len >= buf.len() {
        return Err(Error::RuntimeError("Failed to get current DLL path".to_owned()));
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.to_path_buf().into_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

#[derive(Deserialize)]
pub struct Release {
    // STUB
    tag_name: String,
    assets: Vec<ReleaseAsset>
}

impl Release {
    pub fn is_different_version(&self) -> bool {
        self.tag_name != format!("v{}", env!("CARGO_PKG_VERSION"))
    }
}

#[derive(Deserialize)]
pub struct ReleaseAsset {
    // STUB
    name: String,
    browser_download_url: String
}
