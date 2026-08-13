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
        atomic::{self, AtomicBool, AtomicI32, AtomicU32},
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

use crate::il2cpp::{
    hook::umamusume::{HomeCharacterCreator, SceneDefine::{self, Scene}},
    symbols::Thread
};

/// 本 fork 自己的 Discord Application ID（Edge 用的是它自己的，會顯示成 Edge 的名字）。
const APPLICATION_ID: &str = "1535642752665260093";

/// Discord 的 SET_ACTIVITY 節流間隔。
const MIN_UPDATE_INTERVAL: Duration = Duration::from_secs(15);

/// 連不上（Discord 沒開）時的重試間隔。
const RECONNECT_INTERVAL: Duration = Duration::from_secs(30);

/// 角色頭像來源。**已實測 `large_image` 可以直接吃 https 網址**（2026-08-09），
/// 所以不必把上百張圖逐一上傳到 Discord 後台，RPC 平常只能引用預先上傳的資產名稱。
///
/// 檔名是 card id（6 碼），和 `factor-card` 內建的 `card_map.json` 同一套索引，
/// 所以圖片和名字共用同一個查表結果。
const CHARA_IMAGE_BASE: &str = "https://img.kurue.uk/chara";

/// 首頁代表馬娘的 card id；0 代表還不知道。
static HOME_CARD_ID: AtomicU32 = AtomicU32::new(0);

/// 還要嘗試幾次去讀首頁角色。進首頁時重設；角色是在 ChangeView 之後才由 coroutine 擺出來的，
/// 所以第一次一定讀不到，得重試。讀到就停，讀不到也會自己用完，不會無限排程。
static HOME_PROBES_LEFT: AtomicI32 = AtomicI32::new(0);
const HOME_PROBE_ATTEMPTS: i32 = 30;

/// 卡片／服裝對照。來源是 `uma-pc-datamine` 的 `out/card_dress_map.json`，原檔照搬，
/// 更新時直接覆蓋即可，不需要另外的轉檔步驟。
///
/// **不能用 `dress_id_by_rarity` 來反轉**：那裡面 2★ 那格是共用制服（`101`），22 張卡
/// 都指向它，反轉會變成一對多。只有頂層的 `dress_id` 是該卡專屬的。
#[derive(serde::Deserialize)]
struct CardEntry {
    card_id: u32,
    #[serde(default)]
    dress_id: Option<u32>,
    name: String
}

struct CharaMap {
    /// card_id → 卡名（例：`[藍寶石假期]目白多伯`）
    names: fnv::FnvHashMap<u32, String>,
    /// dress_id → card_id
    by_dress: fnv::FnvHashMap<u32, u32>
}

static CHARA_MAP: Lazy<CharaMap> = Lazy::new(|| {
    let raw: fnv::FnvHashMap<String, CardEntry> =
        serde_json::from_str(include_str!("../../assets/card_dress_map.json"))
            .expect("card_dress_map.json 損毀");

    let mut map = CharaMap {
        names: fnv::FnvHashMap::default(),
        by_dress: fnv::FnvHashMap::default()
    };
    for entry in raw.into_values() {
        if let Some(dress_id) = entry.dress_id {
            map.by_dress.insert(dress_id, entry.card_id);
        }
        map.names.insert(entry.card_id, entry.name);
    }
    map
});

/// 由 `HomeCharacterCreator` 讀到首頁角色後呼叫（遊戲主執行緒）。
pub fn on_home_chara(chara_id: i32, dress_id: i32) {
    let card_id = resolve_card_id(chara_id, dress_id);
    if card_id == 0 {
        debug!("Home chara: unmapped charaId={} dressId={}", chara_id, dress_id);
    }
    HOME_CARD_ID.store(card_id, atomic::Ordering::Relaxed);
}

/// 遊戲給的是 chara id（4 碼）＋ dress id，站上的圖以 card id 命名，是不同的表。
///
/// 服裝 id 和卡片 id 沒有可推導的關係——目白多伯正常版是 `105901 → 105901`（相同），
/// 換裝版是 `105923 → 105902`（差 21）。只能查表。
fn resolve_card_id(chara_id: i32, dress_id: i32) -> u32 {
    if dress_id > 0 {
        if let Some(&card_id) = CHARA_MAP.by_dress.get(&(dress_id as u32)) {
            return card_id;
        }
    }
    // 私服、共用服裝（體操服、制服…）沒有對應卡片，退回該角色的初始卡＝原版頭像。
    if chara_id > 0 {
        let base = chara_id as u32 * 100 + 1;
        if CHARA_MAP.names.contains_key(&base) {
            return base;
        }
    }
    0
}

/// 目前畫面。`-1` 代表還沒收到任何 ChangeView。
static CURRENT_VIEW: AtomicI32 = AtomicI32::new(-1);

static RUNNING: AtomicBool = AtomicBool::new(false);
static STOP_TX: Lazy<Mutex<Option<Sender<()>>>> = Lazy::new(|| Mutex::new(None));

/// 由 `SceneManager::ChangeView` 呼叫（遊戲主執行緒）。只寫 atomic，不做別的。
pub fn on_view_changed(view_id: i32) {
    CURRENT_VIEW.store(view_id, atomic::Ordering::Relaxed);

    // 每次進首頁都重新讀一次——玩家可能剛換過代表馬娘。
    if SceneDefine::describe(view_id).is_home {
        HOME_CARD_ID.store(0, atomic::Ordering::Relaxed);
        HOME_PROBES_LEFT.store(HOME_PROBE_ATTEMPTS, atomic::Ordering::Relaxed);
    }

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

/// 讀首頁角色要碰 il2cpp，只能在遊戲主執行緒上做，所以從 worker 這裡排程過去。
fn probe_home_chara() {
    if HOME_PROBES_LEFT.fetch_sub(1, atomic::Ordering::Relaxed) <= 0 {
        return;
    }
    Thread::main_thread().schedule(HomeCharacterCreator::read_footer_chara);
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
    // 場景和代表馬娘任一改變都要重送，所以去重的鍵是兩者的組合。
    let mut sent_state: Option<(Scene, u32)> = None;

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
                    sent_state = None;
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
        // 只有站在首頁時才拿代表馬娘當主圖；進了育成／賽事再掛著首頁的角色會對不上。
        let card_id = if scene.is_home { HOME_CARD_ID.load(atomic::Ordering::Relaxed) } else { 0 };
        if scene.is_home && card_id == 0 {
            probe_home_chara();
        }

        if sent_state == Some((scene, card_id)) {
            continue;
        }
        if let Some(last) = last_sent_at {
            if now.duration_since(last) < MIN_UPDATE_INTERVAL {
                continue; // 節流：留到下一輪，屆時送的是當下最新狀態
            }
        }

        let Some(c) = client.as_mut() else { continue };

        // Activity 借用這些字串，必須活到 set_activity 之後。
        let chara_name = (card_id != 0).then(|| CHARA_MAP.names.get(&card_id)).flatten();
        let image = match card_id {
            0 => scene.asset_key.to_owned(),
            id => format!("{}/{}.png", CHARA_IMAGE_BASE, id)
        };
        let image_text = chara_name.map(|s| s.as_str()).unwrap_or(scene.group);

        let mut activity = Activity::new()
            .activity_type(ActivityType::Playing)
            .details(scene.group)
            .assets(Assets::new().large_image(&image).large_text(image_text))
            .timestamps(Timestamps::new().start(session_start));
        // 首頁顯示代表馬娘的名字，其餘畫面沿用場景細節。
        if let Some(state) = chara_name.map(|s| s.as_str()).or(scene.detail) {
            activity = activity.state(state);
        }

        match c.set_activity(activity) {
            Ok(()) => {
                sent_state = Some((scene, card_id));
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
