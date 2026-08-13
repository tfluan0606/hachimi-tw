//! 因子卡片：把攔到的 `trained_chara` 資料畫成一張 PNG（版面同種馬資料庫網站）。
//!
//! - 資料來源：[`super::api_packet::capture_response`] 每解出一包 response 就丟過來，
//!   我們把裡面的 `trained_chara_array` / `trained_chara` 收下來（以 trained_chara_id 累積）。
//! - 繪圖：`factor-card` crate（`crates/factor-card`，離線工具與這裡共用同一份排版碼）。
//! - 立繪：跟網站同一個圖源，抓過就快取在 `<data>/factor_card/portraits/`。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;

use factor_card::{data::CardData, Extras, render::Portrait, Fonts, Maps, Theme};
use once_cell::sync::Lazy;
use serde_json::Value;

use super::{Error, Hachimi};

/// 已攔到的練成角色，key = trained_chara_id
static TRAINED: Lazy<Mutex<HashMap<i64, Value>>> = Lazy::new(|| Mutex::new(HashMap::new()));

static MAPS: Lazy<Maps> = Lazy::new(Maps::bundled);
/// 卡片正中央的 ASKR 浮水印
static WATERMARK: Lazy<Option<Portrait>> =
    Lazy::new(|| decode_png(include_bytes!("../../assets/ASKRNB_ver2.png")).ok());
static FONTS: Lazy<Result<Fonts, String>> = Lazy::new(Fonts::system);

fn err(msg: impl Into<String>) -> Error {
    Error::RuntimeError(msg.into())
}

/// 卡片主題：預設暗色，可在選單切換（初值取自 config 的 `factor_card_light_theme`）
static LIGHT_THEME: Lazy<AtomicBool> =
    Lazy::new(|| AtomicBool::new(Hachimi::instance().config.load().factor_card_light_theme));

pub fn light_theme() -> bool {
    LIGHT_THEME.load(Ordering::Relaxed)
}

pub fn set_light_theme(light: bool) {
    LIGHT_THEME.store(light, Ordering::Relaxed);
    update_config(|c| c.factor_card_light_theme = light);
}

/// 卡片輸出資料夾：config 有設就用它，否則 `<data>/factor_card`
pub fn output_dir() -> PathBuf {
    let configured = Hachimi::instance().config.load().factor_card_output_dir.clone();
    match configured {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => Hachimi::instance().get_data_path("factor_card"),
    }
}

pub fn set_output_dir(dir: &str) {
    let value = if dir.trim().is_empty() { None } else { Some(dir.trim().to_owned()) };
    update_config(|c| c.factor_card_output_dir = value.clone());
}

/// 改一項設定並寫回 config.json
fn update_config(f: impl FnOnce(&mut super::hachimi::Config)) {
    let hachimi = Hachimi::instance();
    let mut config = (**hachimi.config.load()).clone();
    f(&mut config);
    if let Err(e) = hachimi.save_config(&config) {
        warn!("[factor_card] 設定寫入失敗：{e}");
    }
}

/// 玩家的追蹤者數（好友頁 API 才會帶）
static FOLLOWER_NUM: AtomicI64 = AtomicI64::new(-1);

/// 從一包已解碼的 response 收集練成角色資料（順便撿玩家的追蹤者數）。
pub fn store_response(json: &Value) {
    let Some(data) = json.get("data") else { return };
    if let Some(n) = data.get("follower_num").and_then(|v| v.as_i64()) {
        FOLLOWER_NUM.store(n, Ordering::Relaxed);
    }
    let mut found: Vec<&Value> = Vec::new();
    if let Some(arr) = data.get("trained_chara_array").and_then(|v| v.as_array()) {
        found.extend(arr.iter());
    }
    if let Some(one) = data.get("trained_chara").filter(|v| v.is_object()) {
        found.push(one);
    }
    if found.is_empty() {
        return;
    }

    let mut store = TRAINED.lock().unwrap();
    let before = store.len();
    for e in found {
        // 只收有因子資料的（有些 endpoint 只回精簡欄位）
        if e.get("factor_info_array").is_none() {
            continue;
        }
        if let Some(id) = e.get("trained_chara_id").and_then(|v| v.as_i64()) {
            store.insert(id, e.clone());
        }
    }
    if store.len() != before {
        info!("[factor_card] 已收集 {} 隻練成角色", store.len());
    }
}

/// 目前收集到的練成角色數量（給選單顯示用）
pub fn stored_count() -> usize {
    TRAINED.lock().unwrap().len()
}

/// 使用者最後點開詳細視窗的那隻（0 = 還沒點過）。
/// 由 `DialogTrainedCharacterDetail` hook 寫入。
static LAST_VIEWED: AtomicI64 = AtomicI64::new(0);

pub fn set_last_viewed(trained_chara_id: i64) {
    if LAST_VIEWED.swap(trained_chara_id, Ordering::Relaxed) != trained_chara_id {
        info!("[factor_card] 目前檢視的練成角色 trained_chara_id={trained_chara_id}");
    }
}

/// 目前鎖定的截圖目標（給選單顯示用）：最後點開的那隻的名字
pub fn last_viewed_label() -> Option<String> {
    let id = LAST_VIEWED.load(Ordering::Relaxed);
    if id == 0 {
        return None;
    }
    let store = TRAINED.lock().unwrap();
    let name = store
        .get(&id)
        .and_then(|e| e.get("card_id").and_then(|v| v.as_i64()))
        .map(|cid| MAPS.card_name(cid))
        .unwrap_or_else(|| "（資料尚未攔到）".to_owned());
    Some(format!("{name} #{id}"))
}

/// 畫出一張因子卡片，回傳存檔路徑。
///
/// `trained_chara_id` 為 `None` 時取「使用者最後點開詳細視窗的那隻」。
pub fn capture(trained_chara_id: Option<i64>) -> Result<PathBuf, Error> {
    let target = trained_chara_id.or_else(|| match LAST_VIEWED.load(Ordering::Relaxed) {
        0 => None,
        id => Some(id),
    });
    let entry = {
        let store = TRAINED.lock().unwrap();
        let id = target.ok_or_else(|| err("還沒點開任何一隻馬，請先在遊戲裡點開想截圖的那隻的詳細視窗"))?;
        store.get(&id).cloned().ok_or_else(|| {
            err(format!("有點開 #{id}，但還沒攔到牠的因子資料（已收集 {} 隻）", store.len()))
        })?
    };

    let fonts = FONTS.as_ref().map_err(|e| err(format!("字型載入失敗：{e}")))?;
    let extras = Extras {
        follower_num: match FOLLOWER_NUM.load(Ordering::Relaxed) {
            -1 => None,
            n => Some(n),
        },
        // 遊戲內這條路攔到的是完整的 trained_chara（含 race_result_list），
        // 交給 factor-card 自己算即可。網站那條資料流沒有勝鞍資訊才需要外面算好餵進來。
        g1_wins: None,
    };
    let data = CardData::from_trained_chara_with(&entry, &MAPS, extras);

    let dir = output_dir();
    std::fs::create_dir_all(&dir).map_err(|e| err(format!("建立資料夾失敗：{e}")))?;

    let mut portraits = HashMap::new();
    for b in &data.blocks {
        if let Some(cid) = b.card_id {
            match portrait(&dir, cid) {
                Ok(p) => {
                    portraits.insert(cid, p);
                }
                Err(e) => warn!("[factor_card] 立繪 {cid} 取得失敗：{e}"),
            }
        }
    }

    let theme = if light_theme() { Theme::Light } else { Theme::Dark };
    let pixmap = factor_card::render(&data, &portraits, fonts, 2.0, theme, WATERMARK.as_ref());
    let path = dir.join(format!("{}_{}.png", data.trained_chara_id, sanitize(&data.name)));
    pixmap.save_png(&path).map_err(|e| err(format!("存檔失敗：{e}")))?;
    info!("[factor_card] 已輸出 {}", path.display());
    Ok(path)
}

/// 檔名安全化（Windows 不接受 `[]` 以外的一些字元，順手把空白也換掉）
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if r#"\/:*?"<>| "#.contains(c) { '_' } else { c })
        .collect()
}

const PORTRAIT_URL: &str = "https://img.kurue.uk/chara";

/// 取立繪：先看快取，沒有才下載。
fn portrait(dir: &std::path::Path, card_id: i64) -> Result<Portrait, Error> {
    let cache_dir = dir.join("portraits");
    std::fs::create_dir_all(&cache_dir).map_err(|e| err(e.to_string()))?;
    let path = cache_dir.join(format!("{card_id}.png"));

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            let resp = ureq::get(&format!("{PORTRAIT_URL}/{card_id}.png"))
                .call()
                .map_err(|e| err(format!("下載失敗：{e}")))?;
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf)
                .map_err(|e| err(e.to_string()))?;
            _ = std::fs::write(&path, &buf);
            buf
        }
    };
    decode_png(&bytes)
}

fn decode_png(bytes: &[u8]) -> Result<Portrait, Error> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| err(e.to_string()))?;
    let mut raw = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw).map_err(|e| err(e.to_string()))?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => raw[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => raw[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => return Err(err(format!("未支援的立繪色彩格式 {other:?}"))),
    };
    Ok(Portrait { width: info.width, height: info.height, rgba })
}
