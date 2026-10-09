use std::{fs, path::{Path, PathBuf}, sync::{atomic::{self, AtomicBool, AtomicU64}, Arc, Mutex}, time::Duration};

use arc_swap::ArcSwap;
use fnv::FnvHashMap;
use serde::Deserialize;
use windows::Win32::{Foundation::MAX_PATH, System::LibraryLoader::GetModuleFileNameW};

use crate::core::{http, Error, Gui, Hachimi};

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

/// 更新目前的狀態，遊戲內選單的「檢查更新」那一列照這個顯示。
#[derive(Clone, PartialEq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    /// 查到新版但還沒下載（自動下載關閉，或使用者在詢問視窗按了「稍後」）
    Available { version: String },
    Downloading,
    /// 已換好新 DLL，重開遊戲後生效
    Installed,
}

pub struct Updater {
    update_check_mutex: Mutex<()>,
    new_update: ArcSwap<Option<DllUpdate>>,
    status: Mutex<UpdateStatus>,
    /// 下載進度（位元組）；總長未知時 total 為 0
    downloaded: AtomicU64,
    total: AtomicU64,
    // 正在背景下載時為 true，擋掉同時觸發的第二次下載（會搶同一個 .new 檔）。
    downloading: AtomicBool,
    // 本次執行已經換好新 DLL、只等重開。此時再查到「有新版」其實就是剛裝的那個，
    // 不能再下載一次（現役 DLL 已改名成 .old 且仍被載入，刪不掉，會直接失敗）。
    installed: AtomicBool
}

impl Default for Updater {
    fn default() -> Self {
        Updater {
            update_check_mutex: Mutex::new(()),
            new_update: ArcSwap::new(Arc::new(None)),
            status: Mutex::new(UpdateStatus::Idle),
            downloaded: AtomicU64::new(0),
            total: AtomicU64::new(0),
            downloading: AtomicBool::new(false),
            installed: AtomicBool::new(false)
        }
    }
}

fn notify(msg: &str) {
    if let Some(mutex) = Gui::instance() {
        mutex.lock().unwrap().show_notification(msg);
    }
}

impl Updater {
    pub fn status(&self) -> UpdateStatus {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, status: UpdateStatus) {
        *self.status.lock().unwrap() = status;
    }

    /// 下載進度 0～1；總長未知時回 None
    pub fn progress(&self) -> Option<f32> {
        let total = self.total.load(atomic::Ordering::Relaxed);
        (total > 0).then(|| self.downloaded.load(atomic::Ordering::Relaxed) as f32 / total as f32)
    }

    /// `manual`：使用者在選單按「檢查更新」。查到新版就跳詢問視窗（附更新說明）讓使用者決定要不要下載；
    /// 沒新版、失敗也會跳通知。
    /// 背景（啟動）檢查傳 false：安靜進行。查到新版時，開了自動下載就直接下載，否則只通知、
    /// 選單上那一列會變成「有新版」。
    pub fn check_for_updates(self: Arc<Self>, manual: bool) {
        std::thread::spawn(move || {
            let before = self.status();
            if let Err(e) = self.check_for_updates_internal(manual) {
                error!("{}", e);
                // 查失敗就回到原本的狀態（例如本來已知有新版，就還是有新版）
                if self.status() == UpdateStatus::Checking {
                    self.set_status(before);
                }
                // 只有手動檢查才把錯誤跳給使用者；背景檢查（例如還沒發 release 會 404）不打擾。
                if manual {
                    notify(&format!("檢查更新失敗：{e}"));
                }
            }
        });
    }

    fn check_for_updates_internal(&self, manual: bool) -> Result<(), Error> {
        // Prevent multiple update checks running at the same time
        let Ok(_guard) = self.update_check_mutex.try_lock() else {
            return Ok(());
        };

        if self.installed.load(atomic::Ordering::Acquire) || self.downloading.load(atomic::Ordering::Acquire) {
            return Ok(());
        }
        self.set_status(UpdateStatus::Checking);

        let latest = fetch_latest_release()?;
        if !latest.is_newer_version() {
            self.set_status(UpdateStatus::Idle);
            if manual {
                notify("已經是最新版本。");
            }
            return Ok(());
        }

        let find = |name: &str| latest.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone());
        let (Some(dll_url), Some(hash_url)) = (find(DLL_ASSET_NAME), find(HASH_ASSET_NAME)) else {
            // 有新版但 release 少了 DLL 或 blake3.json，沒法安全更新，只記 log。
            warn!("Release '{}' is missing '{}' or '{}' asset; skipping update", latest.tag_name, DLL_ASSET_NAME, HASH_ASSET_NAME);
            self.set_status(UpdateStatus::Idle);
            if manual {
                notify("已經是最新版本。");
            }
            return Ok(());
        };
        self.new_update.store(Arc::new(Some(DllUpdate { dll_url, hash_url })));
        self.set_status(UpdateStatus::Available { version: latest.tag_name.clone() });

        if manual {
            // 更新說明只有手動檢查才抓：查版號的回傳裡有就用，沒有再多問一次 GitHub API（抓不到就不顯示）
            let notes = latest.body.clone()
                .map(|b| b.replace("\r\n", "\n"))
                .filter(|b| !b.trim().is_empty())
                .or_else(|| fetch_release_notes(&latest.tag_name));
            if let Some(mutex) = Gui::instance() {
                mutex.lock().unwrap().show_window(Box::new(
                    crate::core::gui::UpdateDialog::new(&latest.tag_name, notes)
                ));
            }
        }
        else if Hachimi::instance().config.load().auto_update {
            Hachimi::instance().updater.clone().run();
        }
        else {
            notify(&format!("發現新版本 {}，打開選單按「更新」安裝", latest.tag_name));
        }
        Ok(())
    }

    /// 背景下載並換上新版（查到新版後才有東西可下載）。
    pub fn run(self: Arc<Self>) {
        // 已在背景下載中就不再開一個（會搶同一個 version.dll.new）。
        if self.downloading.swap(true, atomic::Ordering::AcqRel) {
            return;
        }
        let before = self.status();
        self.downloaded.store(0, atomic::Ordering::Relaxed);
        self.total.store(0, atomic::Ordering::Relaxed);
        self.set_status(UpdateStatus::Downloading);

        std::thread::spawn(move || {
            let res = self.clone().run_internal();
            match &res {
                Ok(true) => {
                    self.installed.store(true, atomic::Ordering::Release);
                    self.set_status(UpdateStatus::Installed);
                    notify("更新完成，重新啟動遊戲後生效。");
                }
                Ok(false) => self.set_status(before),
                Err(e) => {
                    error!("{}", e);
                    self.set_status(before);
                    notify(&format!("更新失敗：{e}"));
                }
            }
            self.downloading.store(false, atomic::Ordering::Release);
        });
    }

    /// 回傳是否真的換好了新 DLL（沒有待裝的更新時回傳 false）。
    fn run_internal(self: Arc<Self>) -> Result<bool, Error> {
        let Some(update) = (**self.new_update.load()).clone() else {
            return Ok(false);
        };

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
            // 網路斷掉時不要永遠卡住（以前沒設 timeout）
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(15))
                .timeout_read(Duration::from_secs(30))
                .build();
            let res = agent.get(&update.dll_url).call()?;
            if let Some(len) = res.header("content-length").and_then(|v| v.parse::<u64>().ok()) {
                self.total.store(len, atomic::Ordering::Relaxed);
            }
            let mut file = fs::File::create(&new_path)?;
            let mut hasher = blake3::Hasher::new();
            let mut buffer = [0u8; CHUNK_SIZE];
            http::download_file_buffered(res, &mut file, &mut buffer, |bytes| {
                hasher.update(bytes);
                self.downloaded.fetch_add(bytes.len() as u64, atomic::Ordering::Relaxed);
            })?;
            file.sync_data()?;

            let hash = hasher.finalize().to_hex().to_string();
            if &hash != expected_hash {
                let _ = fs::remove_file(&new_path);
                return Err(Error::FileHashMismatch(new_path.to_string_lossy().into_owned()));
            }
        }
        self.new_update.store(Arc::new(None));

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
        Ok(true)
    }
}

/// 這個 release 的說明文字（GitHub 上寫的更新內容）。抓不到就回 None，不影響更新本身。
fn fetch_release_notes(tag: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct ReleaseBody {
        body: Option<String>
    }
    let url = format!("https://api.github.com/repos/{}/releases/tags/{}", REPO_PATH, tag);
    match http::get_github_json::<ReleaseBody>(&url) {
        Ok(r) => r.body.map(|b| b.replace("\r\n", "\n")).filter(|b| !b.trim().is_empty()),
        Err(e) => {
            warn!("Failed to fetch release notes for {}: {}", tag, e);
            None
        }
    }
}

/// 查最新 release。
/// - 設了 `update_check_url`（測試用）：照舊讀 GitHub API 格式的 JSON
/// - 否則先走網頁網址 `github.com/<repo>/releases/latest`：它會 302 轉到 `/releases/tag/<tag>`，從轉址讀出
///   版號，asset 網址也是固定格式 `/releases/download/<tag>/<name>`。**不經過 GitHub API**——API 未登入時
///   每個 IP 每小時只有 60 次，共用對外 IP（家用網路常見）時很容易被別人用光而回 403。
/// - 網頁網址失敗才退回 API
fn fetch_latest_release() -> Result<Release, Error> {
    if let Some(url) = Hachimi::instance().config.load().update_check_url.clone() {
        return http::get_json(&url);
    }
    match latest_release_via_web(REPO_PATH) {
        Ok(release) => Ok(release),
        Err(e) => {
            warn!("Update check via release page failed ({}), falling back to GitHub API", e);
            http::get_json(&format!("https://api.github.com/repos/{}/releases/latest", REPO_PATH))
        }
    }
}

fn latest_release_via_web(repo: &str) -> Result<Release, Error> {
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let res = agent.get(&format!("https://github.com/{}/releases/latest", repo)).call()?;
    let location = res.header("location").unwrap_or_default();
    let Some(tag) = location.split("/releases/tag/").nth(1).map(|t| t.trim_end_matches('/')).filter(|t| !t.is_empty()) else {
        return Err(Error::RuntimeError(format!("unexpected releases/latest response (status {}, location '{}')", res.status(), location)));
    };
    let asset = |name: &str| ReleaseAsset {
        name: name.to_owned(),
        browser_download_url: format!("https://github.com/{}/releases/download/{}/{}", repo, tag, name)
    };
    Ok(Release { tag_name: tag.to_owned(), assets: vec![asset(DLL_ASSET_NAME), asset(HASH_ASSET_NAME)], body: None })
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
    let len = unsafe { GetModuleFileNameW(Some(DLL_HMODULE), &mut buf) } as usize;
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
    assets: Vec<ReleaseAsset>,
    /// 更新說明。GitHub API（和測試用的 update_check_url）回傳裡就有；走網頁轉址查版號時沒有
    #[serde(default)]
    body: Option<String>
}

impl Release {
    /// release tag 比目前 DLL 內建版號**新**才算有更新（只比 major.minor.patch，`-test` 之類後綴忽略）。
    /// 不能用 `!=`：發出去的 DLL 若版號跟 tag 對不上（例如 tag `v0.14.1-test` 但 DLL 內建 0.14.0），
    /// 裝完每次啟動都會再判定有新版，無限重抓；也會把比較新的本機 build「更新」回舊版。
    pub fn is_newer_version(&self) -> bool {
        match (parse_version(&self.tag_name), parse_version(env!("CARGO_PKG_VERSION"))) {
            (Some(latest), Some(current)) => latest > current,
            _ => {
                warn!("Can't parse release tag '{}' as a version; skipping update", self.tag_name);
                false
            }
        }
    }
}

/// `v0.14.1`、`0.14.1-test` → `(0, 14, 1)`。
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    /// 實際連 GitHub（網頁網址，不耗 API 額度）確認能讀出最新 tag。需要網路，預設不跑：
    /// `cargo test --lib latest_release_via_web -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn latest_release_via_web() {
        let r = super::latest_release_via_web(super::REPO_PATH).unwrap();
        eprintln!("latest tag = {}, assets = {:?}", r.tag_name, r.assets.iter().map(|a| &a.browser_download_url).collect::<Vec<_>>());
        assert!(r.tag_name.starts_with('v'));
    }

    #[test]
    fn parses_release_tags() {
        assert_eq!(parse_version("v0.14.1"), Some((0, 14, 1)));
        assert_eq!(parse_version("v0.14.1-test"), Some((0, 14, 1)));
        assert_eq!(parse_version("0.14.0"), Some((0, 14, 0)));
        assert_eq!(parse_version("v0.14"), None);
        assert_eq!(parse_version("latest"), None);
        assert!(parse_version("v0.14.10") > parse_version("v0.14.9"));
    }
}

#[derive(Deserialize)]
pub struct ReleaseAsset {
    // STUB
    name: String,
    browser_download_url: String
}
