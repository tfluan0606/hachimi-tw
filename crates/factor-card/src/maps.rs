//! 靜態對照表（沿用網站 `static/*.json`，內容由 uma-pc-datamine 的 extractor 產生）。

use std::collections::HashMap;

pub struct FactorEntry {
    pub name: String,
    /// master `succession_factor.factor_type`；5/7/9 為隱藏類（賽事／事件／稱號）
    pub factor_type: Option<i64>,
}

pub struct RaceEntry {
    pub name: String,
    pub grade: i64,
    pub race_group: i64,
}

#[derive(Default)]
pub struct Maps {
    /// key = factor_id / 100
    pub factor: HashMap<u32, FactorEntry>,
    /// key = card_id
    pub card: HashMap<u32, String>,
    /// key = program_id
    pub race: HashMap<u32, RaceEntry>,
}

impl Maps {
    /// 由三份 JSON 文字建表；格式同網站 `static/{factor,card,race}_map.json`。
    pub fn from_json(factor_map: &str, card_map: &str, race_map: &str) -> Result<Maps, String> {
        let parse = |s: &str| -> Result<serde_json::Value, String> {
            serde_json::from_str(s).map_err(|e| e.to_string())
        };
        let (fm, cm, rm) = (parse(factor_map)?, parse(card_map)?, parse(race_map)?);
        let mut maps = Maps::default();

        for (k, v) in fm.as_object().ok_or("factor_map 不是物件")? {
            let Ok(key) = k.parse::<u32>() else { continue };
            maps.factor.insert(key, FactorEntry {
                name: v["name"].as_str().unwrap_or(k).to_owned(),
                factor_type: v["type"].as_i64(),
            });
        }
        for (k, v) in cm.as_object().ok_or("card_map 不是物件")? {
            let Ok(key) = k.parse::<u32>() else { continue };
            maps.card.insert(key, v.as_str().unwrap_or(k).to_owned());
        }
        for (k, v) in rm.as_object().ok_or("race_map 不是物件")? {
            let Ok(key) = k.parse::<u32>() else { continue };
            maps.race.insert(key, RaceEntry {
                name: v["name"].as_str().unwrap_or(k).to_owned(),
                grade: v["grade"].as_i64().unwrap_or(0),
                race_group: v["race_group"].as_i64().unwrap_or(0),
            });
        }
        Ok(maps)
    }

    /// 使用 crate 內建的靜態表（`assets/`）。
    pub fn bundled() -> Maps {
        Maps::from_json(
            include_str!("../assets/factor_map.json"),
            include_str!("../assets/card_map.json"),
            include_str!("../assets/race_map.json"),
        )
        .expect("內建靜態表損毀")
    }

    pub fn factor_name(&self, factor_id: i64) -> String {
        let group = (factor_id / 100) as u32;
        match self.factor.get(&group) {
            Some(e) => e.name.clone(),
            None => factor_id.to_string(),
        }
    }

    pub fn card_name(&self, card_id: i64) -> String {
        match self.card.get(&(card_id as u32)) {
            Some(n) => n.clone(),
            None => format!("card:{card_id}"),
        }
    }
}
