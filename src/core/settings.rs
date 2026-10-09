//! 改設定的唯一入口：改了立即套用到執行中的遊戲，停手一下再自動存檔。
//!
//! 以前同一個設定有兩條路而且行為相反：選單上的 FPS／VSync／置頂／游標只改記憶體裡的
//! atomic、不存檔（重開就沒了）；設定編輯器存檔、但 atomic 只在啟動時讀一次（要重開才生效）。
//! 現在不管從哪裡改，都是 `update` / `commit` 一個新的 `Config`：
//! 1. 換掉 `Hachimi::config`；
//! 2. `apply_runtime` 把跟舊值不同的項目推到遊戲（atomic、Unity API、視窗、Discord…）；
//!    需要遊戲主執行緒的工作走 `main_thread::post`，因為獨立設定視窗的執行緒沒有 attach il2cpp。
//! 3. 記下「有變更」，由 GUI 每幀呼叫的 `autosave_tick` 在停手 [`SAVE_DELAY`] 後寫檔。
//!    拖滑桿時每幀都會 commit，寫檔才不會每幀發生。

use std::{
    sync::{atomic, Arc, Mutex},
    time::{Duration, Instant},
};

use super::{hachimi::Config, Hachimi};

const SAVE_DELAY: Duration = Duration::from_millis(600);

/// 最後一次變更的時間；`None`＝已存檔
static DIRTY_SINCE: Mutex<Option<Instant>> = Mutex::new(None);

/// 拿目前設定改一改再 commit。
pub fn update(f: impl FnOnce(&mut Config)) {
    let mut config = (**Hachimi::instance().config.load()).clone();
    f(&mut config);
    commit(config);
}

/// 換上新設定、套用差異、排程存檔。
pub fn commit(new: Config) {
    let hachimi = Hachimi::instance();
    let old = hachimi.config.load_full();
    hachimi.config.store(Arc::new(new));
    apply_runtime(&old, &hachimi.config.load());
    *DIRTY_SINCE.lock().unwrap() = Some(Instant::now());
}

/// GUI 每幀呼叫：停手夠久就把設定寫回 config.json。
pub fn autosave_tick() {
    let due = matches!(*DIRTY_SINCE.lock().unwrap(), Some(t) if t.elapsed() >= SAVE_DELAY);
    if due {
        flush();
    }
}

/// 有未存的變更就立刻寫檔（關閉設定視窗時也會呼叫）。
pub fn flush() {
    if DIRTY_SINCE.lock().unwrap().take().is_none() {
        return;
    }
    let config = Hachimi::instance().config.load_full();
    // 寫檔丟到背景，避免卡 render thread
    std::thread::spawn(move || {
        if let Err(e) = Hachimi::instance().save_config(&config) {
            error!("Failed to save config: {}", e);
        }
    });
}

/// 只套用有變的項目。每次滑桿拖動都會走到這裡，所以不要做重的事。
fn apply_runtime(old: &Config, new: &Config) {
    let hachimi = Hachimi::instance();

    if old.language != new.language {
        new.language.set_locale();
    }

    if old.target_fps != new.target_fps {
        hachimi.target_fps.store(new.target_fps.unwrap_or(-1), atomic::Ordering::Relaxed);
        super::main_thread::post(|| {
            // 值不重要，hook 會換成 target_fps
            crate::il2cpp::hook::UnityEngine_CoreModule::Application::set_targetFrameRate(30);
        });
    }

    #[cfg(target_os = "windows")]
    {
        use super::main_thread;
        use crate::{
            il2cpp::hook::UnityEngine_CoreModule::{Cursor, QualitySettings},
            windows::{discord, utils::set_window_topmost, wnd_hook},
        };
        let (o, n) = (&old.windows, &new.windows);

        if o.vsync_count != n.vsync_count {
            hachimi.vsync_count.store(n.vsync_count, atomic::Ordering::Relaxed);
            // 值不重要，hook 會換成 vsync_count
            main_thread::post(|| QualitySettings::set_vSyncCount(1));
        }
        if o.window_always_on_top != n.window_always_on_top {
            hachimi.window_always_on_top.store(n.window_always_on_top, atomic::Ordering::Relaxed);
            main_thread::post(|| {
                let topmost = Hachimi::instance().window_always_on_top.load(atomic::Ordering::Relaxed);
                unsafe { _ = set_window_topmost(wnd_hook::get_target_hwnd(), topmost); }
            });
        }
        if o.disable_game_cursor != n.disable_game_cursor {
            hachimi.disable_game_cursor.store(n.disable_game_cursor, atomic::Ordering::Relaxed);
            // 開啟時立即還原成系統游標；關閉時遊戲會在下次重設游標時恢復自訂圖
            // （apply 內部會排程到主執行緒，所以也要從 attach 過的執行緒呼叫）
            main_thread::post(Cursor::apply);
        }
        if o.enable_discord_rpc != n.enable_discord_rpc {
            if n.enable_discord_rpc { discord::start(); } else { discord::stop(); }
        }
        if o.custom_title_name != n.custom_title_name {
            wnd_hook::apply_custom_title();
        }
    }
}
