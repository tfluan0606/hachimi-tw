//! 遊戲 API 封包解碼（UM:PD 繁中 / Komoe client）。
//!
//! 管線（逆向自 `uma-pc-datamine/decrypt_capture.py`，已對真實封包驗證）：
//! `response → Coneshell 解密(native) → HttpHelper.DecompressResponse 解壓(LZ4,managed) →
//!  msgpack → Task.Deserialize(body)`。
//!
//! 我們在遊戲內 hook `Gallop.HttpHelper::DecompressResponse(byte[]) -> byte[]` 的**回傳值**，
//! 那正是「已解密＋已解壓」的 msgpack 明文 → 交給 [`decode_plaintext`] 轉 JSON。單一 static
//! 方法涵蓋所有 API response，不必逐一 hook 幾百個 Task。
//!
//! 離線 wire pipeline（b64→AES→LZ4→msgpack）與其常數僅供測試用（`out/pc_cap` 封包端到端驗證），
//! 以 `#[cfg(test)]` 隔開，不進執行期。

use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Error, Hachimi};

const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4d, 0x18];

fn err(msg: impl Into<String>) -> Error {
    Error::RuntimeError(msg.into())
}

/// 若是 LZ4 frame 就解壓，否則原樣回傳（DecompressResponse 通常已解壓，這裡是保險）。
fn maybe_lz4(data: &[u8]) -> Result<Vec<u8>, Error> {
    if data.len() >= 4 && data[0..4] == LZ4_FRAME_MAGIC {
        let mut dec = lz4_flex::frame::FrameDecoder::new(data);
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut dec, &mut out).map_err(|e| err(format!("lz4: {e}")))?;
        Ok(out)
    } else {
        Ok(data.to_vec())
    }
}

fn rmpv_to_json(v: rmpv::Value) -> serde_json::Value {
    use rmpv::Value as V;
    use serde_json::Value as J;
    match v {
        V::Nil => J::Null,
        V::Boolean(b) => J::Bool(b),
        V::Integer(i) => {
            if let Some(u) = i.as_u64() {
                J::from(u)
            } else if let Some(s) = i.as_i64() {
                J::from(s)
            } else {
                i.as_f64().and_then(serde_json::Number::from_f64).map(J::Number).unwrap_or(J::Null)
            }
        }
        V::F32(f) => serde_json::Number::from_f64(f as f64).map(J::Number).unwrap_or(J::Null),
        V::F64(f) => serde_json::Number::from_f64(f).map(J::Number).unwrap_or(J::Null),
        V::String(s) => J::String(s.into_str().unwrap_or_default()),
        V::Binary(b) => J::Array(b.into_iter().map(J::from).collect()),
        V::Array(a) => J::Array(a.into_iter().map(rmpv_to_json).collect()),
        V::Map(m) => {
            let mut obj = serde_json::Map::with_capacity(m.len());
            for (k, val) in m {
                let key = match k {
                    V::String(s) => s.into_str().unwrap_or_default(),
                    V::Integer(i) => i.to_string(),
                    other => other.to_string(),
                };
                obj.insert(key, rmpv_to_json(val));
            }
            J::Object(obj)
        }
        V::Ext(_, data) => J::Array(data.into_iter().map(J::from).collect()),
    }
}

/// 解碼「已解密＋已解壓」的 response body（msgpack，必要時先 LZ4）→ JSON。
pub fn decode_plaintext(plaintext: &[u8]) -> Result<serde_json::Value, Error> {
    let unpacked = maybe_lz4(plaintext)?;
    let value = rmpv::decode::read_value(&mut &unpacked[..])
        .map_err(|e| err(format!("msgpack decode: {e}")))?;
    Ok(rmpv_to_json(value))
}

static CAPTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// 從 top-level `data` 物件的 key 組出檔名標籤（辨識是哪個 endpoint）。
fn label_from_json(json: &serde_json::Value) -> String {
    let keys: Vec<&str> = json
        .get("data")
        .and_then(|d| d.as_object())
        .map(|o| o.keys().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    if keys.is_empty() {
        "unknown".to_string()
    } else {
        let joined = keys.join("_");
        joined.chars().filter(|c| c.is_alphanumeric() || *c == '_').take(60).collect()
    }
}

/// 攔到一個 response 的 msgpack 明文：解碼後把整包 JSON 落檔到 `<data>/api_capture/`，
/// 並 log 一行摘要（供辨識因子 response）。解不出來的（非遊戲 API msgpack）安靜跳過。
///
/// 這是「先全部抓下來、之後再挑因子 endpoint」的測試階段行為。
pub fn capture_response(bytes: &[u8]) {
    let json = match decode_plaintext(bytes) {
        Ok(j) => j,
        Err(_) => return, // 非 msgpack（BUMA 等）→ 跳過
    };
    // 只保留看起來是遊戲 API envelope 的（有 response_code）。
    if json.get("response_code").is_none() {
        return;
    }

    // 因子卡片要用的練成角色資料
    #[cfg(target_os = "windows")]
    super::factor_card::store_response(&json);

    // 全量落檔只在「使用者自己建了 api_capture 資料夾」時啟用（調查 endpoint 用）
    let dir = Hachimi::instance().get_data_path("api_capture");
    if !dir.is_dir() {
        return;
    }

    let n = CAPTURE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = label_from_json(&json);
    info!("[api_capture] #{n:04} data=[{label}] ({} bytes msgpack)", bytes.len());

    let path = dir.join(format!("{n:04}_{label}.json"));
    match serde_json::to_string_pretty(&json) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                warn!("[api_capture] write failed: {e}");
            }
        }
        Err(e) => warn!("[api_capture] serialize failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use cbc::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
    use md5::{Digest, Md5};
    use std::path::PathBuf;

    const SALT: [u8; 20] = [
        0x7a, 0x5a, 0xde, 0xf0, 0x5e, 0x2b, 0x49, 0x93, 0x31, 0x47,
        0x4c, 0xa7, 0x34, 0xf6, 0x27, 0xf5, 0x3c, 0x90, 0x30, 0xca,
    ];
    const XOR_KEY: [u8; 32] = [
        0xdc, 0x3c, 0x45, 0x6b, 0x59, 0x7f, 0xdf, 0xb4, 0xe5, 0x9a, 0x5f, 0xae, 0x0e, 0xa3, 0x80, 0x45,
        0x38, 0x26, 0x69, 0x9d, 0xee, 0xc5, 0x96, 0x96, 0x29, 0x33, 0x94, 0xcf, 0xdd, 0x77, 0xde, 0x46,
    ];
    type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

    fn b64_decode(s: &str) -> Vec<u8> {
        let cleaned: String = s.chars().filter(|c| !c.is_whitespace() && *c != '=').collect();
        base64::engine::general_purpose::STANDARD_NO_PAD.decode(cleaned.as_bytes()).unwrap()
    }
    fn hex_decode(s: &str) -> Vec<u8> {
        let s = s.trim();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
    fn md5_of(parts: &[&[u8]]) -> [u8; 16] {
        let mut h = Md5::new();
        for p in parts { h.update(p); }
        let mut out = [0u8; 16];
        out.copy_from_slice(&h.finalize());
        out
    }

    /// 完整 wire pipeline（測試用）：request(SID+body) + response body → JSON。
    fn decode_wire_response(sid_hex: &str, req_body_b64: &str, resp_body_b64: &str) -> serde_json::Value {
        let raw = b64_decode(req_body_b64);
        let b1 = &raw[4..68];
        let rk = &b1[32..64];
        let mut udid16 = [0u8; 16];
        for i in 0..16 {
            udid16[i] = b1[16 + i] ^ rk[16 + i] ^ XOR_KEY[16 + i];
        }
        let key = md5_of(&[&hex_decode(sid_hex), &SALT]);
        let iv = md5_of(&[&udid16, &SALT]);

        let raw_resp = b64_decode(resp_body_b64);
        let mut buf = raw_resp[36..].to_vec();
        let pt = Aes128CbcDec::new_from_slices(&key, &iv).unwrap()
            .decrypt_padded_mut::<NoPadding>(&mut buf).unwrap();
        decode_plaintext(pt).unwrap()
    }

    fn cap_dir() -> Option<PathBuf> {
        if let Ok(d) = std::env::var("HACHIMI_PCCAP_DIR") {
            let p = PathBuf::from(d);
            if p.is_dir() { return Some(p); }
        }
        // hachimi manifest 上一層是 C:\Users\tflua，再進 uma-pc-datamine/out/pc_cap
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent()
            .map(|p| p.join("uma-pc-datamine").join("out").join("pc_cap"))
            .filter(|p| p.is_dir())
    }

    fn header<'a>(headers: &'a serde_json::Value, name: &str) -> Option<&'a str> {
        headers.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| v.as_str())
    }

    fn decode_capture_file(path: &std::path::Path) -> Option<serde_json::Value> {
        let rec: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let req = &rec["request"];
        let resp = &rec["response"];
        let sid = header(&req["headers"], "SID")?;
        if req["body_kind"] != "text" || resp["body_kind"] != "text" { return None; }
        Some(decode_wire_response(sid, req["body"].as_str()?, resp["body"].as_str()?))
    }

    #[test]
    fn decodes_factor_select_fixture() {
        let Some(dir) = cap_dir() else { eprintln!("skip: no pc_cap dir"); return; };
        let path = dir.join("0011_single_mode_legend_factor_select.json");
        if !path.is_file() { eprintln!("skip: fixture missing"); return; }
        let out = decode_capture_file(&path).expect("decode failed");
        assert_eq!(out["response_code"], serde_json::json!(1));
        assert!(out["data"]["single_mode_factor_select_common"].is_object());
    }

    #[test]
    fn bulk_decodes_captured_flows() {
        let Some(dir) = cap_dir() else { eprintln!("skip: no pc_cap dir"); return; };
        let (mut game_api, mut ok) = (0usize, 0usize);
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") { continue; }
            if let Some(v) = decode_capture_file(&path) {
                game_api += 1;
                if v.get("response_code").is_some() { ok += 1; }
            }
        }
        eprintln!("decoded {ok}/{game_api} game-API flows");
        assert!(game_api > 0 && ok == game_api, "decoded {ok}/{game_api}");
    }
}
