// Tier 2 訓練助手：把支援卡技能資料庫（靈感/固定事件/隨機事件已分類）打包進 DLL，
// 依「當前養成裝的 6 張卡」列出可獲得技能。資料源＝uma-pc-datamine 拆包產出的 support_skills.json
// （489 卡，逐張含 hint/fixed/random 三類技能，來源見該專案 STATE「支援卡技能來源 pipeline」）。

use std::collections::HashMap;

use once_cell::sync::Lazy;
use serde::Deserialize;

use crate::il2cpp::hook::umamusume::SingleModeDeck::DeckInfo;

#[derive(Deserialize, Clone)]
pub struct Skill {
    pub id: i32,
    pub name: String,
}

#[derive(Deserialize)]
struct CardEntry {
    name: String,
    rarity: String,
    #[serde(rename = "type")]
    type_: String,
    hint_skills: Vec<Skill>,
    fixed_event_skills: Vec<Skill>,
    random_event_skills: Vec<Skill>,
}

static DB: Lazy<HashMap<i32, CardEntry>> = Lazy::new(|| {
    let raw = include_str!("../../assets/data/support_skills.json");
    let m: HashMap<String, CardEntry> = serde_json::from_str(raw).unwrap_or_default();
    m.into_iter()
        .filter_map(|(k, v)| k.parse::<i32>().ok().map(|id| (id, v)))
        .collect()
});

// 技能靈感 (group_id, rarity) → 技能名。SkillTips 的 rarity 直接對應 skill_data.rarity，
// 同 group 的 rarity1(普通/○) 與 rarity2(稀有/◎) 是不同技能名，必須 group+rarity 一起查。
// key 格式 "gid:rarity"。
static HINT_MAP: Lazy<HashMap<(i32, i32), String>> = Lazy::new(|| {
    let raw = include_str!("../../assets/data/skill_hint_map.json");
    let m: HashMap<String, String> = serde_json::from_str(raw).unwrap_or_default();
    m.into_iter()
        .filter_map(|(k, v)| {
            let (g, r) = k.split_once(':')?;
            Some(((g.parse().ok()?, r.parse().ok()?), v))
        })
        .collect()
});

/// 技能靈感 (group_id, rarity) → 技能名。
pub fn hint_name(group_id: i32, rarity: i32) -> Option<&'static str> {
    HINT_MAP.get(&(group_id, rarity)).map(|s| s.as_str())
}

// skill_id → (名, 稀有度, 是否繼承固有)。技能學習頁抓到的 SkillId 用此查。
#[derive(Deserialize)]
struct SkillMeta(String, i32, i32); // [name, rarity, is_unique]

static SKILL_MAP: Lazy<HashMap<i32, SkillMeta>> = Lazy::new(|| {
    let raw = include_str!("../../assets/data/skill_id_map.json");
    let m: HashMap<String, SkillMeta> = serde_json::from_str(raw).unwrap_or_default();
    m.into_iter()
        .filter_map(|(k, v)| k.parse::<i32>().ok().map(|id| (id, v)))
        .collect()
});

pub struct SkillDisplay {
    pub name: &'static str,
    pub rarity: i32,
    pub is_unique: bool,
}

/// skill_id → 顯示資訊（名/稀有/固有）。
pub fn skill_display(skill_id: i32) -> Option<SkillDisplay> {
    SKILL_MAP.get(&skill_id).map(|m| SkillDisplay {
        name: m.0.as_str(),
        rarity: m.1,
        is_unique: m.2 != 0,
    })
}

pub struct CardInfo {
    pub name: &'static str,
    pub rarity: &'static str,
    pub type_: &'static str,
}

/// 卡基本資訊（名/稀有度/類型）。DB 是 'static，回傳借用即可。
pub fn card_info(sid: i32) -> Option<CardInfo> {
    DB.get(&sid).map(|c| CardInfo {
        name: c.name.as_str(),
        rarity: c.rarity.as_str(),
        type_: c.type_.as_str(),
    })
}

/// 聚合後的可學技能：同技能多卡提供時合併，記來源卡 id。
pub struct LearnableSkill {
    pub id: i32,
    pub name: String,
    pub sources: Vec<i32>,
}

pub struct Grouped {
    pub hint: Vec<LearnableSkill>,
    pub fixed: Vec<LearnableSkill>,
    pub random: Vec<LearnableSkill>,
}

fn aggregate(deck: &DeckInfo, pick: impl Fn(&CardEntry) -> &Vec<Skill>) -> Vec<LearnableSkill> {
    let mut order: Vec<i32> = Vec::new();
    let mut map: HashMap<i32, LearnableSkill> = HashMap::new();
    for card in &deck.cards {
        let Some(entry) = DB.get(&card.support_card_id) else { continue };
        for s in pick(entry) {
            let e = map.entry(s.id).or_insert_with(|| {
                order.push(s.id);
                LearnableSkill { id: s.id, name: s.name.clone(), sources: Vec::new() }
            });
            if !e.sources.contains(&card.support_card_id) {
                e.sources.push(card.support_card_id);
            }
        }
    }
    order.into_iter().filter_map(|id| map.remove(&id)).collect()
}

/// 依當前牌組算出三類可獲得技能。
pub fn learnable_for_deck(deck: &DeckInfo) -> Grouped {
    Grouped {
        hint: aggregate(deck, |c| &c.hint_skills),
        fixed: aggregate(deck, |c| &c.fixed_event_skills),
        random: aggregate(deck, |c| &c.random_event_skills),
    }
}
