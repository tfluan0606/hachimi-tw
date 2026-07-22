//! 離線對圖用：讀一份已擷取的 API JSON → 產生卡片 PNG（不需要開遊戲）。
//!
//! ```text
//! cargo run -p factor-card --example render_card -- <capture.json> [out.png] [trained_chara_id]
//! ```
//! 立繪從 img.kurue.uk 抓，快取在 `target/portrait_cache/`。

use factor_card::{data::CardData, data::Extras, render::Portrait, Fonts, Maps};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let cap = args.next().ok_or("用法: render_card <capture.json> [out.png] [trained_chara_id]")?;
    let out = args.next().unwrap_or_else(|| "card.png".to_owned());
    let want_id: Option<i64> = args.next().and_then(|s| s.parse().ok());

    let json: Value = serde_json::from_str(&std::fs::read_to_string(&cap)?)?;
    let arr = json["data"]["trained_chara_array"]
        .as_array()
        .ok_or("找不到 data.trained_chara_array")?;

    // 沒指定就挑「有父有母且因子最多」的那隻當範例
    let entry = match want_id {
        Some(id) => arr
            .iter()
            .find(|e| e["trained_chara_id"].as_i64() == Some(id))
            .ok_or("找不到該 trained_chara_id")?,
        None => arr
            .iter()
            .max_by_key(|e| {
                let pos: Vec<i64> = e["succession_chara_array"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|s| s["position_id"].as_i64()).collect())
                    .unwrap_or_default();
                let both = pos.contains(&10) && pos.contains(&20);
                (both, e["factor_info_array"].as_array().map(|a| a.len()).unwrap_or(0))
            })
            .ok_or("trained_chara_array 是空的")?,
    };

    let maps = Maps::bundled();
    let extras = Extras { follower_num: find_follower_num(std::path::Path::new(&cap)) };
    let data = CardData::from_trained_chara_with(entry, &maps, extras);
    let fonts = Fonts::system()?;

    let mut portraits = HashMap::new();
    for b in &data.blocks {
        if let Some(cid) = b.card_id {
            match fetch_portrait(cid) {
                Ok(p) => {
                    portraits.insert(cid, p);
                }
                Err(e) => eprintln!("立繪 {cid} 取得失敗：{e}"),
            }
        }
    }

    let scale: f32 = std::env::var("CARD_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let theme = match std::env::var("CARD_THEME").as_deref() {
        Ok("light") => factor_card::Theme::Light,
        _ => factor_card::Theme::Dark,
    };
    let logo_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ASKRNB_ver2.png");
    let watermark = std::fs::read(&logo_path).ok().and_then(|b| decode_png(&b).ok());
    let pixmap = factor_card::render(&data, &portraits, &fonts, scale, theme, watermark.as_ref());
    pixmap.save_png(&out)?;
    println!(
        "wrote {out} ({}x{}) | {} tcid={} 因子={} 共鳴={}",
        pixmap.width(),
        pixmap.height(),
        data.name,
        data.trained_chara_id,
        data.blocks[0].badges.len(),
        data.resonance.len()
    );
    Ok(())
}

/// 追蹤者數在另一支 API（好友頁）裡，離線時就從同一個 capture 資料夾找一份
fn find_follower_num(cap: &std::path::Path) -> Option<i64> {
    let dir = cap.parent()?;
    for entry in std::fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        if !text.contains("follower_num") {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(n) = v["data"]["follower_num"].as_i64() {
                return Some(n);
            }
        }
    }
    None
}

fn fetch_portrait(card_id: i64) -> Result<Portrait, Box<dyn std::error::Error>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/portrait_cache");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{card_id}.png"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            let resp = ureq::get(&format!("https://img.kurue.uk/chara/{card_id}.png")).call()?;
            let mut buf = Vec::new();
            resp.into_reader().read_to_end(&mut buf)?;
            std::fs::write(&path, &buf)?;
            buf
        }
    };

    decode_png(&bytes)
}

fn decode_png(bytes: &[u8]) -> Result<Portrait, Box<dyn std::error::Error>> {
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
        other => return Err(format!("未支援的立繪色彩格式 {other:?}").into()),
    };
    Ok(Portrait { width: info.width, height: info.height, rgba })
}

use std::io::Read as _;
