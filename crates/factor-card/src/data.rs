//! 把一筆 `trained_chara` JSON 轉成排版用的資料。
//!
//! 因子邏輯完全沿用網站（`static/stallion*.jsx` / `game_client.py`）：
//! `apply_extends` → `classify` → 排序 → 統計 / 三方共鳴。

use crate::maps::Maps;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

// ── 網站的常數（stallion-data.jsx）────────────────────────────
const T0_GROUPS: &[u32] = &[21009, 21026, 21005];
const T1_GROUPS: &[u32] = &[20001, 20002, 20017, 20018, 20019, 20020];
const HIDDEN_FACTOR_TYPES: &[i64] = &[5, 7, 9];
const SHOWN_TITLE_GROUPS: &[u32] = &[50016, 50017, 50018, 50019, 50020, 50021];

fn is_race_factor_group(g: u32) -> bool {
    (10001..=10035).contains(&g)
}

/// 遺傳子（「○○的基因」，場地／距離／腳質）：性質上等同適性，所以跟紅因子同色
fn is_gene_group(g: u32) -> bool {
    (50001..=50010).contains(&g)
}

/// 覺醒因子（「順時針的覺醒」「冬之覺醒」等）：跟 T1 白因同色
fn is_awaken_group(g: u32) -> bool {
    SHOWN_TITLE_GROUPS.contains(&g)
}

/// 藍因子（屬性）縮寫：速度=速、持久力=耐、力量=力、意志力=根、智力=智
fn blue_abbr(g: u32) -> &'static str {
    match g {
        1 => "速",
        2 => "耐",
        3 => "力",
        4 => "根",
        5 => "智",
        _ => "?",
    }
}

/// 紅因子（適性）縮寫：場地／腳質／距離
fn red_abbr(g: u32) -> &'static str {
    match g {
        11 => "草",
        12 => "沙",
        21 => "領",
        22 => "前",
        23 => "居",
        24 => "後",
        31 => "短",
        32 => "哩",
        33 => "中",
        34 => "長",
        _ => "?",
    }
}

/// 遺傳子縮寫：「○○的基因」的 ○○ 跟紅因子同一套名稱，所以共用縮寫
fn gene_abbr(g: u32) -> &'static str {
    match g {
        50001 => "草",
        50002 => "沙",
        50003 => "短",
        50004 => "哩",
        50005 => "中",
        50006 => "長",
        50007 => "領",
        50008 => "前",
        50009 => "居",
        50010 => "後",
        _ => "?",
    }
}

/// 顯示排序：藍 → 紅 → 遺傳子 → 綠 → 覺醒 → 白
fn sort_rank(f: &Factor) -> u8 {
    let g = f.group();
    match f.ty {
        FactorType::Blue => 0,
        FactorType::Red => 1,
        FactorType::Green => 3,
        FactorType::White if is_gene_group(g) => 2,
        FactorType::White if is_awaken_group(g) => 4,
        FactorType::White => 5,
    }
}

/// badge 的視覺分類（對應 stallion.css 的 `.fb-*`）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FbClass {
    Blue,
    Red,
    Green,
    White,
    WhiteOut,
    T0,
    T1,
}

/// 因子本身的顏色分類（純由 factor_id 推導，不需 api 的 type）
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum FactorType {
    Blue = 0,
    Red = 1,
    Green = 2,
    White = 3,
}

fn classify(factor_id: i64) -> FactorType {
    if factor_id < 1000 {
        FactorType::Blue
    } else if factor_id < 10000 {
        FactorType::Red
    } else if factor_id >= 10_000_000 {
        FactorType::Green
    } else {
        FactorType::White
    }
}

#[derive(Clone, Copy)]
struct Factor {
    id: i64,
    ty: FactorType,
}

impl Factor {
    fn group(&self) -> u32 {
        (self.id / 100) as u32
    }
    fn stars(&self) -> u32 {
        (self.id % 10) as u32
    }
}

/// 一顆因子 badge
pub struct Badge {
    pub name: String,
    pub stars: u32,
    pub class: FbClass,
}

/// 本體 / 父 / 母 三個區塊
pub struct Block {
    pub label: &'static str,
    pub card_id: Option<i64>,
    pub name: Option<String>,
    pub badges: Vec<Badge>,
}

/// 三方共鳴白因
pub struct Resonance {
    pub name: String,
    pub class: FbClass,
    pub self_stars: u32,
    pub father_stars: u32,
    pub mother_stars: u32,
}

/// 文字的語意顏色（實際色碼由主題決定）
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Default,
    Blue,
    Red,
    Green,
}

/// 一行裡的一段文字（可各自上色）
pub struct Seg {
    pub text: String,
    pub tone: Tone,
}

impl Seg {
    fn new(text: String, tone: Tone) -> Seg {
        Seg { text, tone }
    }
}

/// 統計格的值：單一數字，或本體／父／母三方（每方可有多段）
pub enum StatValue {
    Single(String),
    PerSide([Vec<Seg>; 3]),
}

/// 上排的一格統計
pub struct Stat {
    pub key: &'static str,
    pub value: StatValue,
    pub tone: Tone,
}

/// trained_chara 本身沒有、但要顯示在卡片上的額外資訊（來自其他 API）
#[derive(Default, Clone, Copy)]
pub struct Extras {
    /// 玩家的追蹤者數（`follower_num`，來自好友頁 API）
    pub follower_num: Option<i64>,
    /// 由呼叫端算好的 G1 勝場。給了就直接用，不跑本檔的 [`g1_wins`]。
    ///
    /// 網站端已改成「以勝鞍 `group_id` 去重」推算——friend/search API 沒有
    /// `race_result_list`，祖輩更是完全沒這個欄位，所以舊演算法在那條資料流上
    /// 一定得 0。從網站餵資料進來時務必給這個值。
    pub g1_wins: Option<i64>,
}

pub struct CardData {
    pub name: String,
    pub trained_chara_id: i64,
    /// 持有者的 UID（顯示在名字旁邊）
    pub viewer_id: i64,
    /// 玩家追蹤者數（顯示在標題列 UID 旁邊）
    pub follower_num: Option<i64>,
    pub card_id: i64,
    pub stats: Vec<Stat>,
    pub resonance: Vec<Resonance>,
    pub blocks: Vec<Block>,
}

// ── 因子處理 ────────────────────────────────────────────────

/// `factor_extend_array` 會把某些因子升級成別的 factor_id（例如稱號覺醒），
/// 顯示前一定要套用，否則星數/名稱會錯。
fn apply_extends(factors: &[i64], extends: &[(i64, i64, i64)], positions: &BTreeSet<i64>) -> Vec<i64> {
    let upmap: HashMap<i64, i64> = extends
        .iter()
        .filter(|(pos, _, _)| positions.contains(pos))
        .map(|(_, base, fid)| (*base, *fid))
        .collect();
    if upmap.is_empty() {
        return factors.to_vec();
    }
    factors.iter().map(|f| *upmap.get(f).unwrap_or(f)).collect()
}

/// 分類後依 藍→紅→遺傳子→綠→覺醒→白 排序（穩定排序，組內維持原順序）
fn enrich(factors: Vec<i64>) -> Vec<Factor> {
    let mut v: Vec<Factor> = factors.into_iter().map(|id| Factor { id, ty: classify(id) }).collect();
    v.sort_by_key(sort_rank);
    v
}

impl Maps {
    /// 隱藏因子（賽事／事件／稱號）不顯示、也不計入白因數
    fn is_hidden(&self, factor_id: i64) -> bool {
        let g = (factor_id / 100) as u32;
        if SHOWN_TITLE_GROUPS.contains(&g) {
            return false;
        }
        if let Some(e) = self.factor.get(&g) {
            if let Some(t) = e.factor_type {
                return HIDDEN_FACTOR_TYPES.contains(&t);
            }
        }
        (10001..=10035).contains(&g) || (40001..=40007).contains(&g) || (50011..=50061).contains(&g)
    }
}

fn fb_class(f: &Factor) -> FbClass {
    let g = f.group();
    if T0_GROUPS.contains(&g) {
        FbClass::T0
    } else if T1_GROUPS.contains(&g) {
        FbClass::T1
    } else if is_gene_group(g) {
        FbClass::Red
    } else if is_awaken_group(g) {
        FbClass::T1
    } else if is_race_factor_group(g) {
        FbClass::WhiteOut
    } else {
        match f.ty {
            FactorType::Blue => FbClass::Blue,
            FactorType::Red => FbClass::Red,
            FactorType::Green => FbClass::Green,
            FactorType::White => FbClass::White,
        }
    }
}

// ── JSON 取值小工具 ─────────────────────────────────────────

fn factor_ids(v: &Value) -> Vec<i64> {
    v["factor_info_array"]
        .as_array()
        .map(|a| a.iter().filter_map(|f| f["factor_id"].as_i64()).collect())
        .unwrap_or_default()
}

impl CardData {
    /// `entry` = `trained_chara_array` 裡的一筆
    pub fn from_trained_chara(entry: &Value, maps: &Maps) -> CardData {
        CardData::from_trained_chara_with(entry, maps, Extras::default())
    }

    pub fn from_trained_chara_with(entry: &Value, maps: &Maps, extras: Extras) -> CardData {
        // (position_id, base_factor_id, factor_id)
        let extends: Vec<(i64, i64, i64)> = entry["factor_extend_array"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some((
                            e["position_id"].as_i64()?,
                            e["base_factor_id"].as_i64()?,
                            e["factor_id"].as_i64()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 本體用 position < 10 的 extend
        let main_pos: BTreeSet<i64> = extends.iter().map(|(p, _, _)| *p).filter(|p| *p < 10).collect();
        let own = enrich(apply_extends(&factor_ids(entry), &extends, &main_pos));

        // 父(position 10) / 母(position 20)
        let mut parents: HashMap<i64, (i64, Vec<Factor>)> = HashMap::new();
        if let Some(arr) = entry["succession_chara_array"].as_array() {
            for sc in arr {
                let (Some(pos), Some(cid)) = (sc["position_id"].as_i64(), sc["card_id"].as_i64()) else {
                    continue;
                };
                let pos_set: BTreeSet<i64> = [pos].into_iter().collect();
                parents.insert(pos, (cid, enrich(apply_extends(&factor_ids(sc), &extends, &pos_set))));
            }
        }
        let father = parents.get(&10);
        let mother = parents.get(&20);

        // ── 統計：全部改成「本體/父/母」三方分布 ──
        let sides: [Option<&Vec<Factor>>; 3] =
            [Some(&own), father.map(|(_, f)| f), mother.map(|(_, f)| f)];

        // 本體／父／母三方；沒有父／母就填 —
        let per_side = |f: &dyn Fn(&Vec<Factor>) -> Vec<Seg>| -> StatValue {
            let mut out: [Vec<Seg>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for (i, side) in sides.iter().enumerate() {
                out[i] = match side {
                    Some(v) => f(v),
                    None => vec![Seg::new("—".to_owned(), Tone::Default)],
                };
            }
            StatValue::PerSide(out)
        };
        // 例：速度★2＋持久力★1 → 「2速1耐」
        let abbr_list = |v: &Vec<Factor>, ty: FactorType, abbr: fn(u32) -> &'static str| -> String {
            let s: String = v
                .iter()
                .filter(|f| f.ty == ty && !is_gene_group(f.group()))
                .map(|f| format!("{}{}", f.stars(), abbr(f.group())))
                .collect();
            if s.is_empty() { "—".to_owned() } else { s }
        };
        let gene_list = |v: &Vec<Factor>| -> String {
            let s: String = v
                .iter()
                .filter(|f| is_gene_group(f.group()))
                .map(|f| format!("{}{}", f.stars(), gene_abbr(f.group())))
                .collect();
            if s.is_empty() { "—".to_owned() } else { s }
        };
        let green_stars = |v: &Vec<Factor>| -> String {
            let n: u32 = v.iter().filter(|f| f.ty == FactorType::Green).map(|f| f.stars()).sum();
            if n == 0 { "—".to_owned() } else { n.to_string() }
        };
        // 有效白因＝實際顯示出來的白因，扣掉遺傳子（覺醒等隱藏技能仍計入）
        let effective_white = |v: &Vec<Factor>| -> String {
            v.iter()
                .filter(|f| {
                    f.ty == FactorType::White && !maps.is_hidden(f.id) && !is_gene_group(f.group())
                })
                .count()
                .to_string()
        };

        let stats = vec![
            Stat {
                key: "藍/紅/綠/有效白分布",
                value: per_side(&|v| {
                    vec![
                        Seg::new(abbr_list(v, FactorType::Blue, blue_abbr), Tone::Blue),
                        Seg::new(abbr_list(v, FactorType::Red, red_abbr), Tone::Red),
                        Seg::new(green_stars(v), Tone::Green),
                        Seg::new(effective_white(v), Tone::Default),
                    ]
                }),
                tone: Tone::Default,
            },
            Stat {
                key: "遺傳子分布",
                value: per_side(&|v| vec![Seg::new(gene_list(v), Tone::Red)]),
                tone: Tone::Default,
            },
            Stat {
                key: "G1勝場",
                value: StatValue::Single(
                    extras.g1_wins.unwrap_or_else(|| g1_wins(entry, maps) as i64).to_string(),
                ),
                tone: Tone::Default,
            },
        ];

        // ── 三方共鳴白因：本體/父/母 都有的可見白因 group ──
        let visible_white_groups = |v: &[Factor]| -> BTreeSet<u32> {
            v.iter()
                .filter(|f| f.ty == FactorType::White && !maps.is_hidden(f.id))
                .map(|f| f.group())
                .collect()
        };
        let (sw, fw, mw) = (
            visible_white_groups(&own),
            father.map(|(_, f)| visible_white_groups(f)).unwrap_or_default(),
            mother.map(|(_, f)| visible_white_groups(f)).unwrap_or_default(),
        );
        let max_in_group = |v: Option<&Vec<Factor>>, g: u32| -> u32 {
            v.map(|v| {
                v.iter()
                    .filter(|f| f.ty == FactorType::White && f.group() == g)
                    .map(|f| f.stars())
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
        };
        let resonance: Vec<Resonance> = sw
            .iter()
            .filter(|g| fw.contains(g) && mw.contains(g))
            .map(|&g| Resonance {
                name: maps.factor.get(&g).map(|e| e.name.clone()).unwrap_or_else(|| g.to_string()),
                class: if T0_GROUPS.contains(&g) {
                    FbClass::T0
                } else if T1_GROUPS.contains(&g) {
                    FbClass::T1
                } else if is_gene_group(g) {
                    FbClass::Red
                } else if is_awaken_group(g) {
                    FbClass::T1
                } else {
                    FbClass::White
                },
                self_stars: max_in_group(Some(&own), g),
                father_stars: max_in_group(father.map(|(_, f)| f), g),
                mother_stars: max_in_group(mother.map(|(_, f)| f), g),
            })
            .collect();

        // ── 三個區塊 ──
        let to_badges = |v: &[Factor]| -> Vec<Badge> {
            v.iter()
                .filter(|f| !maps.is_hidden(f.id))
                .map(|f| Badge { name: maps.factor_name(f.id), stars: f.stars(), class: fb_class(f) })
                .collect()
        };
        let card_id = entry["card_id"].as_i64().unwrap_or(0);
        let mut blocks = vec![Block {
            label: "本體",
            card_id: Some(card_id),
            name: Some(maps.card_name(card_id)),
            badges: to_badges(&own),
        }];
        for (label, p) in [("父", father), ("母", mother)] {
            blocks.push(match p {
                Some((cid, f)) => Block {
                    label,
                    card_id: Some(*cid),
                    name: Some(maps.card_name(*cid)),
                    badges: to_badges(f),
                },
                None => Block { label, card_id: None, name: None, badges: Vec::new() },
            });
        }

        CardData {
            name: maps.card_name(card_id),
            trained_chara_id: entry["trained_chara_id"].as_i64().unwrap_or(0),
            viewer_id: entry["viewer_id"].as_i64().unwrap_or(0),
            follower_num: extras.follower_num,
            card_id,
            stats,
            resonance,
            blocks,
        }
    }
}

/// G1 勝場：`race_result_list` 中 grade==100 且 race_group!=7 的**不重複**賽事數
fn g1_wins(entry: &Value, maps: &Maps) -> usize {
    let mut names = BTreeSet::new();
    if let Some(arr) = entry["race_result_list"].as_array() {
        for race in arr {
            let Some(pid) = race["program_id"].as_i64() else { continue };
            if let Some(info) = maps.race.get(&(pid as u32)) {
                if info.grade == 100 && info.race_group != 7 {
                    names.insert(info.name.clone());
                }
            }
        }
    }
    names.len()
}
