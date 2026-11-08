//! 外掛（`load_libraries` 載入的 DLL）：啟用／停用清單、外掛更新、開機時套用下載好的新檔。
//!
//! 外掛更新只在「有啟用的外掛」而且設定了 `plugin_index_url` 時才會做：讀那份 index.json，
//! 找出包含已啟用 DLL 的模組，逐檔比對 blake3，不同就下載成 `<檔名>.new`（下載時驗證雜湊），
//! 下次開遊戲、載入外掛之前換上（此時外掛和它啟動的程式都還沒跑，不會有檔案被鎖住）。
//! Hachimi 只認得這個通用格式，不知道任何模組的內容：
//!
//! ```json
//! {"modules": {"<模組>": {"version": "1.0.0", "plugin_api": 1, "notes": "…",
//!   "files": {"<檔名>": {"url": "https://…", "blake3": "<hex>"}}}}}
//! ```

use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, sync::{atomic::{self, AtomicBool}, Arc, Mutex}, time::Duration};

use serde::{Deserialize, Serialize};

use crate::core::{http, Error, Gui, Hachimi};

#[derive(Deserialize, Clone)]
pub struct IndexFile {
    pub url: String,
    pub blake3: String,
}

#[derive(Deserialize, Clone)]
pub struct IndexModule {
    pub version: String,
    #[serde(default = "default_api")]
    pub plugin_api: i32,
    #[serde(default)]
    #[allow(dead_code)]
    pub notes: Option<String>,
    pub files: BTreeMap<String, IndexFile>,
}

fn default_api() -> i32 {
    1
}

#[derive(Deserialize)]
struct Index {
    modules: BTreeMap<String, IndexModule>,
}

#[derive(Clone, PartialEq)]
pub enum PluginStatus {
    Idle,
    Checking,
    /// (模組名稱, 版本)
    Available(Vec<(String, String)>),
    Downloading,
    /// 已下載，重開遊戲後生效
    Installed,
}

pub struct PluginUpdater {
    status: Mutex<PluginStatus>,
    pending: Mutex<Vec<(String, IndexModule)>>,
    busy: AtomicBool,
}

impl Default for PluginUpdater {
    fn default() -> Self {
        PluginUpdater { status: Mutex::new(PluginStatus::Idle), pending: Mutex::new(vec![]), busy: AtomicBool::new(false) }
    }
}

fn notify(msg: &str) {
    if let Some(mutex) = Gui::instance() {
        mutex.lock().unwrap().show_notification(msg);
    }
}

/// 遊戲資料夾（外掛 DLL 和它的檔案都放這裡，跟 version.dll 同一層）
pub fn game_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf())
}

fn pending_list_path() -> PathBuf {
    Hachimi::instance().get_data_path("plugin_pending.json")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.to_path_buf().into_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

fn file_blake3(p: &Path) -> Option<String> {
    let mut f = fs::File::open(p).ok()?;
    let mut h = blake3::Hasher::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(h.finalize().to_hex().to_string())
}

/// 開機時、載入外掛之前：把上次下載好的 `.new` 換上
pub fn apply_pending() {
    let list = pending_list_path();
    let Ok(txt) = fs::read_to_string(&list) else { return };
    let names: Vec<String> = serde_json::from_str(&txt).unwrap_or_default();
    let Some(dir) = game_dir() else { return };
    apply_in(&dir, &names);
    let _ = fs::remove_file(&list);
}

/// 只接受單純檔名（不能跳出遊戲資料夾）
fn safe_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && !name.contains("..")
}

/// dir 裡的 `<name>.new` 換成 `<name>`（換失敗就把原檔搬回去）
fn apply_in(dir: &Path, names: &[String]) {
    for name in names {
        if !safe_name(name) {
            continue;
        }
        let target = dir.join(name);
        let new = with_suffix(&target, ".new");
        if !new.is_file() {
            continue;
        }
        let old = with_suffix(&target, ".old");
        let _ = fs::remove_file(&old);
        let had_old = target.exists() && fs::rename(&target, &old).is_ok();
        match fs::rename(&new, &target) {
            Ok(()) => {
                info!("Plugin file updated: {}", name);
                let _ = fs::remove_file(&old);
            }
            Err(e) => {
                error!("Failed to apply plugin update {}: {}", name, e);
                if had_old {
                    let _ = fs::rename(&old, &target);
                }
            }
        }
    }
}

/// 清單裡包含已啟用 DLL、外掛 API 版本支援、而且檔案和清單不一樣（或缺檔）的模組
fn stale_modules(index: Index, enabled: &[String], dir: &Path) -> Vec<(String, IndexModule)> {
    let enabled: Vec<String> = enabled.iter().map(|s| s.to_lowercase()).collect();
    let mut out = vec![];
    for (name, m) in index.modules {
        if !m.files.keys().any(|f| enabled.contains(&f.to_lowercase())) {
            continue;
        }
        if m.plugin_api > crate::core::plugin_api::VERSION {
            warn!("Plugin module {} {} needs plugin API {}, skipping", name, m.version, m.plugin_api);
            continue;
        }
        if m.files.keys().any(|f| !safe_name(f)) {
            warn!("Plugin module {} has an invalid file name, skipping", name);
            continue;
        }
        let stale = m.files.iter().any(|(f, x)| file_blake3(&dir.join(f)).map_or(true, |h| !h.eq_ignore_ascii_case(&x.blake3)));
        if stale {
            out.push((name, m));
        }
    }
    out
}

impl PluginUpdater {
    pub fn status(&self) -> PluginStatus {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, s: PluginStatus) {
        *self.status.lock().unwrap() = s;
    }

    /// 有沒有需要檢查的外掛（有啟用的外掛、也設定了清單網址）
    pub fn enabled() -> bool {
        let config = Hachimi::instance().config.load();
        !config.windows.load_libraries.is_empty() && config.windows.plugin_index_url.as_deref().map_or(false, |u| !u.is_empty())
    }

    pub fn check(self: Arc<Self>, manual: bool) {
        // 已下載好、等重開：再查一定又是「不一樣」（新檔還是 .new），不能再下載一次
        if !Self::enabled() || self.busy.load(atomic::Ordering::Acquire) || self.status() == PluginStatus::Installed {
            return;
        }
        std::thread::spawn(move || {
            let before = self.status();
            self.set_status(PluginStatus::Checking);
            match self.check_internal() {
                Ok(found) => {
                    if found.is_empty() {
                        self.set_status(PluginStatus::Idle);
                        if manual {
                            notify("外掛都是最新版本。");
                        }
                        return;
                    }
                    let list: Vec<(String, String)> = found.iter().map(|(n, m)| (n.clone(), m.version.clone())).collect();
                    *self.pending.lock().unwrap() = found;
                    self.set_status(PluginStatus::Available(list.clone()));
                    let names = list.iter().map(|(n, v)| format!("{n} {v}")).collect::<Vec<_>>().join("、");
                    if Hachimi::instance().config.load().auto_update && !manual {
                        self.clone().run();
                    } else {
                        notify(&format!("外掛有新版：{names}，到設定的「外掛」頁更新"));
                    }
                }
                Err(e) => {
                    error!("Plugin update check failed: {}", e);
                    self.set_status(before);
                    if manual {
                        notify(&format!("檢查外掛更新失敗：{e}"));
                    }
                }
            }
        });
    }

    /// 已啟用的外掛裡，檔案和清單不一樣（或缺檔）的模組
    fn check_internal(&self) -> Result<Vec<(String, IndexModule)>, Error> {
        let config = Hachimi::instance().config.load();
        let Some(url) = config.windows.plugin_index_url.clone() else { return Ok(vec![]) };
        let index: Index = http::get_json(&url)?;
        let dir = game_dir().ok_or_else(|| Error::RuntimeError("找不到遊戲資料夾".into()))?;
        Ok(stale_modules(index, &config.windows.load_libraries, &dir))
    }

    /// 下載待更新的模組（存成 .new，下次開遊戲換上）
    pub fn run(self: Arc<Self>) {
        if self.busy.swap(true, atomic::Ordering::AcqRel) {
            return;
        }
        let before = self.status();
        self.set_status(PluginStatus::Downloading);
        std::thread::spawn(move || {
            match self.run_internal() {
                Ok(true) => {
                    self.set_status(PluginStatus::Installed);
                    notify("外掛更新完成，重新啟動遊戲後生效。");
                }
                Ok(false) => self.set_status(before),
                Err(e) => {
                    error!("Plugin update failed: {}", e);
                    self.set_status(before);
                    notify(&format!("外掛更新失敗：{e}"));
                }
            }
            self.busy.store(false, atomic::Ordering::Release);
        });
    }

    fn run_internal(&self) -> Result<bool, Error> {
        let mods = std::mem::take(&mut *self.pending.lock().unwrap());
        if mods.is_empty() {
            return Ok(false);
        }
        let dir = game_dir().ok_or_else(|| Error::RuntimeError("找不到遊戲資料夾".into()))?;
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(30)).build();
        let mut done: Vec<String> = fs::read_to_string(pending_list_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        for (_, m) in &mods {
            for (name, f) in &m.files {
                if !safe_name(name) {
                    return Err(Error::RuntimeError(format!("清單裡的檔名不合法：{name}")));
                }
                let target = dir.join(name);
                if file_blake3(&target).map_or(false, |h| h.eq_ignore_ascii_case(&f.blake3)) {
                    continue;
                }
                let new = with_suffix(&target, ".new");
                let _ = fs::remove_file(&new);
                let res = agent.get(&f.url).call()?;
                let mut file = fs::File::create(&new)?;
                let mut hasher = blake3::Hasher::new();
                let mut buffer = [0u8; 8192];
                http::download_file_buffered(res, &mut file, &mut buffer, |b| {
                    hasher.update(b);
                })?;
                file.sync_data()?;
                drop(file);
                if !hasher.finalize().to_hex().to_string().eq_ignore_ascii_case(&f.blake3) {
                    let _ = fs::remove_file(&new);
                    return Err(Error::FileHashMismatch(new.to_string_lossy().into_owned()));
                }
                if !done.contains(name) {
                    done.push(name.clone());
                }
                fs::write(pending_list_path(), serde_json::to_string(&done).unwrap_or_default())?;
            }
        }
        Ok(true)
    }
}

/// 設定頁用：已知的外掛（啟用的＋停用的），依名稱排序、去重
#[derive(Serialize)]
pub struct PluginEntry {
    pub name: String,
    pub enabled: bool,
}

pub fn known_plugins(config: &crate::windows::hachimi_impl::Config) -> Vec<PluginEntry> {
    let mut v: Vec<PluginEntry> = config.load_libraries.iter().map(|n| PluginEntry { name: n.clone(), enabled: true }).collect();
    for n in &config.disabled_libraries {
        if !v.iter().any(|e| e.name.eq_ignore_ascii_case(n)) {
            v.push(PluginEntry { name: n.clone(), enabled: false });
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hachimi_plugins_test_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn hash(b: &[u8]) -> String {
        blake3::hash(b).to_hex().to_string()
    }

    fn idx(dll: &[u8], exe: &[u8], api: i32) -> Index {
        serde_json::from_value(serde_json::json!({"modules": {
            "a": {"version": "1.1", "plugin_api": api, "files": {"a.dll": {"url": "u", "blake3": hash(dll)}, "a_helper.exe": {"url": "u", "blake3": hash(exe)}}},
            "b": {"version": "2.0", "files": {"b.dll": {"url": "u", "blake3": "00"}}}
        }}))
        .unwrap()
    }

    #[test]
    fn stale_detection() {
        let d = tmp("stale");
        fs::write(d.join("a.dll"), b"v1").unwrap();
        fs::write(d.join("a_helper.exe"), b"h1").unwrap();
        let en = vec!["A.dll".to_string()];
        // 檔案都一樣 → 不用更新；沒啟用的 b 不檢查
        assert!(stale_modules(idx(b"v1", b"h1", 1), &en, &d).is_empty());
        // helper 不一樣 → 整個模組要更新
        assert_eq!(stale_modules(idx(b"v1", b"h2", 1), &en, &d).len(), 1);
        // 需要較新的外掛 API → 不更新
        assert!(stale_modules(idx(b"v2", b"h2", 99), &en, &d).is_empty());
        // 缺檔 → 要更新
        fs::remove_file(d.join("a_helper.exe")).unwrap();
        assert_eq!(stale_modules(idx(b"v1", b"h1", 1), &en, &d).len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn apply_new_files() {
        let d = tmp("apply");
        fs::write(d.join("a.dll"), b"old").unwrap();
        fs::write(d.join("a.dll.new"), b"new").unwrap();
        fs::write(d.join("b.exe.new"), b"fresh").unwrap();
        apply_in(&d, &["a.dll".into(), "b.exe".into(), "missing.dll".into(), "../evil.dll".into()]);
        assert_eq!(fs::read(d.join("a.dll")).unwrap(), b"new");
        assert_eq!(fs::read(d.join("b.exe")).unwrap(), b"fresh");
        assert!(!d.join("a.dll.new").exists() && !d.join("a.dll.old").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn names_must_stay_in_folder() {
        assert!(safe_name("island_live.dll"));
        for bad in ["", "../x.dll", "a/b.dll", "a\\b.dll", "C:x.dll"] {
            assert!(!safe_name(bad), "{bad}");
        }
    }
}
