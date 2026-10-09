//! 技能目錄：給一鍵學習清單搜尋技能名稱、標稀有度用。
//!
//! 資料從 master.mdb 讀（`sql::load_skill_catalog`），那要呼叫遊戲的 SQLite（il2cpp），所以第一次
//! 用到時排進 `main_thread` 佇列，在遊戲主執行緒載入；載入完成前 `get` 回 `None`。

use std::sync::atomic::{AtomicBool, Ordering};

use fnv::FnvHashMap;
use once_cell::sync::OnceCell;

pub struct SkillCatalog {
    /// (名稱, 稀有度)，依 id 順序、名稱不重複
    pub entries: Vec<(String, i32)>,
    by_name: FnvHashMap<String, i32>,
}

static CATALOG: OnceCell<SkillCatalog> = OnceCell::new();
static LOAD_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 取得目錄；還沒載入的話排程載入並回 `None`（之後幾幀就會好）。任何執行緒都能呼叫。
pub fn get() -> Option<&'static SkillCatalog> {
    if let Some(c) = CATALOG.get() {
        return Some(c);
    }
    if !LOAD_REQUESTED.swap(true, Ordering::AcqRel) {
        super::main_thread::post(|| {
            let entries = crate::il2cpp::sql::load_skill_catalog();
            let by_name = entries.iter().map(|(n, r)| (n.clone(), *r)).collect();
            _ = CATALOG.set(SkillCatalog { entries, by_name });
        });
    }
    None
}

impl SkillCatalog {
    pub fn rarity(&self, name: &str) -> Option<i32> {
        self.by_name.get(name.trim()).copied()
    }

    /// 名稱包含 `query` 的技能（不分大小寫），前綴相符的排前面，最多 `limit` 筆。
    pub fn search(&self, query: &str, limit: usize) -> Vec<&(String, i32)> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<&(String, i32)> = self.entries.iter()
            .filter(|(name, _)| name.to_lowercase().contains(&q))
            .collect();
        hits.sort_by_key(|(name, _)| !name.to_lowercase().starts_with(&q));
        hits.truncate(limit);
        hits
    }
}

/// 稀有度的顯示名稱和底色。目錄裡只有技能頁買得到的：白（1）、金（2）、繼承固有。
pub fn rarity_label(rarity: i32) -> (&'static str, egui::Color32) {
    match rarity {
        1 => ("白", egui::Color32::from_rgb(0x4a, 0x55, 0x60)),
        2 => ("金", egui::Color32::from_rgb(0x8a, 0x66, 0x1c)),
        crate::il2cpp::sql::RARITY_INHERITED_UNIQUE => ("繼承", egui::Color32::from_rgb(0x5b, 0x3d, 0x7a)),
        _ => ("？", egui::Color32::from_gray(0x44)),
    }
}
