//! 賽馬娘「種馬卡片」原生繪圖：把遊戲 API 回傳的 `trained_chara` 資料畫成一張 PNG。
//!
//! 這裡刻意不碰網路與檔案系統（除了可選的 `assets` 內建靜態表），
//! 讓 Hachimi 端與離線工具共用同一份排版／繪圖碼：
//!   1. [`Maps`]：因子／卡片／賽事名稱表
//!   2. [`CardData::from_trained_chara`]：把一筆 trained_chara JSON 轉成排版用的資料
//!   3. [`render`]：畫成 RGBA pixmap（呼叫端自行編碼成 PNG／貼到 UI）

pub mod data;
pub mod maps;
pub mod render;
pub mod text;

pub use data::{CardData, Extras};
pub use maps::Maps;
pub use render::{render, Portrait, Theme};
pub use text::Fonts;
