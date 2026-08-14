//! 把一筆種馬資料畫成 PNG：stdin 收 JSON、stdout 吐 PNG。
//!
//! 給網站的 Discord bot（`/uma_show`）用。**刻意不碰網路**——立繪、字型、對照表
//! 全部由呼叫端給檔案路徑，這支程式只負責排版繪圖，所以可以編成一顆沒有 TLS
//! 相依的靜態 binary 丟進網站容器裡跑。
//!
//! ```text
//! echo '<json>' | render-card > card.png
//! ```
//!
//! 輸入 JSON：
//! ```json
//! {
//!   "card": { ...trained_chara 形狀... },
//!   "portraits": { "<card_id>": "/path/to/portrait.png" },
//!   "support_card": { "card_id": 30207, "label": "SSR | 持久力 | […]空中神宮", "limit_break": 4 },
//!   "watermark": "/path/to/logo.png",
//!   "fonts": { "regular": [["/path/font.ttc", 0]], "bold": [["/path/bold.ttc", 0]] },
//!   "maps":  { "factor": "/path/factor_map.json", "card": "...", "race": "..." },
//!   "scale": 2.0,
//!   "theme": "dark",
//!   "g1_wins": 12,
//!   "follower_num": 34
//! }
//! ```
//! `watermark` / `maps` / `g1_wins` / `follower_num` 可省略；`maps` 省略時用 crate
//! 內建的 `assets/`（會跟網站的 `static/*.json` 漂移，正式跑建議明確指定）。

use factor_card::{data::CardData, data::Extras, render::Portrait, Fonts, Maps, SupportCard, Theme};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};

type Err = Box<dyn std::error::Error>;

fn main() {
    if let Err(e) = run() {
        eprintln!("render-card: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Err> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let req: Value = serde_json::from_str(&input).map_err(|e| format!("stdin 不是合法 JSON: {e}"))?;

    let entry = req.get("card").ok_or("缺少 card 欄位")?;

    let maps = match req.get("maps") {
        Some(m) if !m.is_null() => {
            let path = |k: &str| -> Result<String, Err> {
                let p = m[k].as_str().ok_or_else(|| format!("maps.{k} 不是字串"))?;
                std::fs::read_to_string(p).map_err(|e| format!("讀不到 maps.{k} ({p}): {e}").into())
            };
            Maps::from_json(&path("factor")?, &path("card")?, &path("race")?)?
        }
        _ => Maps::bundled(),
    };

    let fonts = load_fonts(req.get("fonts"))?;

    // 立繪：呼叫端負責抓圖與快取，這裡只讀檔。讀不到就跳過（卡片會留空位）。
    let mut portraits: HashMap<i64, Portrait> = HashMap::new();
    if let Some(obj) = req["portraits"].as_object() {
        for (k, v) in obj {
            let Ok(card_id) = k.parse::<i64>() else { continue };
            let Some(path) = v.as_str() else { continue };
            match load_png(path) {
                Ok(p) => {
                    portraits.insert(card_id, p);
                }
                Err(e) => eprintln!("render-card: 立繪 {card_id} 讀取失敗（略過）：{e}"),
            }
        }
    }

    let watermark = match req["watermark"].as_str() {
        Some(p) => match load_png(p) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("render-card: 浮水印讀取失敗（略過）：{e}");
                None
            }
        },
        None => None,
    };

    // 支援卡那行。圖示走 portraits 表（支援卡 id 跟馬娘 card_id 範圍不重疊）。
    let support_card = req.get("support_card").filter(|v| !v.is_null()).and_then(|s| {
        Some(SupportCard {
            card_id: s["card_id"].as_i64()?,
            label: s["label"].as_str()?.to_owned(),
            limit_break: s["limit_break"].as_i64().unwrap_or(0),
        })
    });

    let extras = Extras {
        follower_num: req["follower_num"].as_i64(),
        g1_wins: req["g1_wins"].as_i64(),
        support_card,
    };
    let scale = req["scale"].as_f64().unwrap_or(2.0) as f32;
    let theme = match req["theme"].as_str() {
        Some("light") => Theme::Light,
        _ => Theme::Dark,
    };

    let data = CardData::from_trained_chara_with(entry, &maps, extras);
    let pixmap = factor_card::render(&data, &portraits, &fonts, scale, theme, watermark.as_ref());
    let png = pixmap.encode_png()?;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    out.write_all(&png)?;
    out.flush()?;
    Ok(())
}

/// `fonts` 省略時退回系統字型（開發機用）；給了就照 `[[路徑, ttc index], …]` 讀。
fn load_fonts(spec: Option<&Value>) -> Result<Fonts, Err> {
    let Some(spec) = spec.filter(|v| !v.is_null()) else {
        return Ok(Fonts::system()?);
    };
    let list = |key: &str| -> Vec<(String, u32)> {
        spec[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        let path = e[0].as_str()?.to_owned();
                        Some((path, e[1].as_u64().unwrap_or(0) as u32))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let (reg, bold) = (list("regular"), list("bold"));
    // from_paths 會默默略過讀不到的檔案，全部落空才會報錯——那通常是容器裡沒裝字型。
    Fonts::from_paths(&borrow(&reg), &borrow(&bold)).map_err(|e| {
        format!("字型載入失敗（{e}）；指定的路徑：{:?}", reg.iter().map(|(p, _)| p).collect::<Vec<_>>()).into()
    })
}

/// `Vec<(String, u32)>` → `from_paths` 要的 `&[(&str, u32)]`。
/// 寫成自由函式而不是 closure：closure 推不出「回傳值借用參數」的 lifetime。
fn borrow(v: &[(String, u32)]) -> Vec<(&str, u32)> {
    v.iter().map(|(p, i)| (p.as_str(), *i)).collect()
}

fn load_png(path: &str) -> Result<Portrait, Err> {
    let bytes = std::fs::read(path)?;
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info()?;
    let mut raw = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw)?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => raw[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => raw[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => return Err(format!("未支援的色彩格式 {other:?}").into()),
    };
    Ok(Portrait { width: info.width, height: info.height, rgba })
}
