//! Discord Rich Presence。
//!
//! 場景資訊來自 `SceneManager::ChangeView` hook，經 [`on_view_changed`] 丟給背景執行緒。
//! **hook 跑在遊戲主執行緒上，這裡絕對不能做任何阻塞 IO** —— 所有 named pipe 通訊都在
//! worker 裡完成，`on_view_changed` 只寫一個 atomic。
//!
//! Discord 對 `SET_ACTIVITY` 有節流（約每 15 秒一次），worker 會把期間內的變化合併成
//! 最後一筆再送，所以在選單裡快速跳來跳去不會被丟包或懲罰。

use std::{
    sync::{
        atomic::{self, AtomicBool, AtomicI32},
        mpsc::{self, Sender, RecvTimeoutError}
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH}
};

use discord_rich_presence::{
    activity::{Activity, ActivityType, Assets, Timestamps},
    DiscordIpc, DiscordIpcClient
};
use egui::mutex::Mutex;
use once_cell::sync::Lazy;

use crate::il2cpp::hook::umamusume::SceneDefine::{self, Scene};

/// 本 fork 自己的 Discord Application ID（Edge 用的是它自己的，會顯示成 Edge 的名字）。
const APPLICATION_ID: &str = "1535642752665260093";

/// Discord 的 SET_ACTIVITY 節流間隔。
const MIN_UPDATE_INTERVAL: Duration = Duration::from_secs(15);

/// 連不上（Discord 沒開）時的重試間隔。
const RECONNECT_INTERVAL: Duration = Duration::from_secs(30);

/// 目前畫面。`-1` 代表還沒收到任何 ChangeView。
static CURRENT_VIEW: AtomicI32 = AtomicI32::new(-1);

static RUNNING: AtomicBool = AtomicBool::new(false);
static STOP_TX: Lazy<Mutex<Option<Sender<()>>>> = Lazy::new(|| Mutex::new(None));

/// 由 `SceneManager::ChangeView` 呼叫（遊戲主執行緒）。只寫 atomic，不做別的。
pub fn on_view_changed(view_id: i32) {
    CURRENT_VIEW.store(view_id, atomic::Ordering::Relaxed);

    // ViewId 數值來自日服，繁中服未逐項實測（見 SceneDefine.rs）。開 debug_mode 後
    // 每次切畫面都會留下原始 id 和目前對到的名稱，照 log 校正 describe() 的分段即可。
    if log::log_enabled!(log::Level::Debug) {
        let scene = SceneDefine::describe(view_id);
        debug!("ChangeView: id={} -> {}{}", view_id, scene.group,
            scene.detail.map(|d| format!(" / {}", d)).unwrap_or_default());
    }
}

pub fn start() {
    if RUNNING.swap(true, atomic::Ordering::AcqRel) {
        return;
    }

    let (tx, rx) = mpsc::channel();
    *STOP_TX.lock() = Some(tx);

    std::thread::spawn(move || {
        info!("Discord RPC: starting");
        worker(rx);
        RUNNING.store(false, atomic::Ordering::Release);
        info!("Discord RPC: stopped");
    });
}

pub fn stop() {
    let tx = STOP_TX.lock().take();
    if let Some(tx) = tx {
        _ = tx.send(());
    }
}

fn worker(rx: mpsc::Receiver<()>) {
    // 整段遊玩時間，不隨場景切換重置。
    let session_start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64;

    let mut client: Option<DiscordIpcClient> = None;
    let mut next_connect_at = Instant::now();
    let mut last_sent_at: Option<Instant> = None;
    let mut sent_scene: Option<Scene> = None;

    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => ()
        }

        let now = Instant::now();

        if client.is_none() {
            if now < next_connect_at {
                continue;
            }
            next_connect_at = now + RECONNECT_INTERVAL;

            let mut new_client = DiscordIpcClient::new(APPLICATION_ID);
            match new_client.connect() {
                Ok(()) => {
                    info!("Discord RPC: connected");
                    client = Some(new_client);
                    // 重連後必須重送一次，否則 Discord 那邊是空的。
                    sent_scene = None;
                    last_sent_at = None;
                },
                Err(e) => {
                    // Discord 沒開是常態，不當錯誤處理。
                    debug!("Discord RPC: not connected ({}), retrying in {}s",
                        e, RECONNECT_INTERVAL.as_secs());
                    continue;
                }
            }
        }

        let scene = SceneDefine::describe(CURRENT_VIEW.load(atomic::Ordering::Relaxed));
        if sent_scene == Some(scene) {
            continue;
        }
        if let Some(last) = last_sent_at {
            if now.duration_since(last) < MIN_UPDATE_INTERVAL {
                continue; // 節流：留到下一輪，屆時送的是當下最新狀態
            }
        }

        let Some(c) = client.as_mut() else { continue };

        let mut activity = Activity::new()
            .activity_type(ActivityType::Playing)
            .details(scene.group)
            .assets(Assets::new().large_image(scene.asset_key).large_text(scene.group))
            .timestamps(Timestamps::new().start(session_start));
        if let Some(detail) = scene.detail {
            activity = activity.state(detail);
        }

        match c.set_activity(activity) {
            Ok(()) => {
                sent_scene = Some(scene);
                last_sent_at = Some(now);
            },
            Err(e) => {
                // 通常是 Discord 被關掉了，丟掉 client 讓下一輪重連。
                warn!("Discord RPC: failed to set activity ({}), reconnecting", e);
                client = None;
                next_connect_at = Instant::now() + RECONNECT_INTERVAL;
            }
        }
    }

    if let Some(mut c) = client {
        _ = c.close();
    }
}
