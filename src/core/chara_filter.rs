//! 馬娘篩選：用包包封包的資料篩練成角色，點結果直接開遊戲原生的詳細視窗。
//!
//! - 資料：進包包時遊戲會下載整份練成角色（因子卡片已經在收，見 [`super::factor_card`]），
//!   這裡把它轉成好篩的 [`Horse`]，另外收「我的最愛」圖示與備忘錄。
//! - 哪些是自己包包裡現有的：問遊戲本身的清單（`WorkTrainedCharaData.List`），收集表裡
//!   別人的馬（好友、對手）和已經放生的馬就不會出現。
//! - 開視窗：`DialogTrainedCharacterDetail.Open`，不連線。
//!
//! 介面在設定視窗的「馬娘篩選」分頁（[`ui`]），跑在沒有 attach il2cpp 的執行緒上，
//! 所以碰遊戲的事一律用 [`super::main_thread::post`] 排到遊戲主執行緒。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use fnv::FnvHashMap;
use once_cell::sync::{Lazy, OnceCell};
use serde_json::Value;

use super::factor_card;

// —— 資料 ——

/// 適性欄位，順序同原生篩選器：場地、距離、腳質
pub const APTITUDES: [(&str, &str); 10] = [
    ("proper_ground_turf", "草地"),
    ("proper_ground_dirt", "沙地"),
    ("proper_distance_short", "短距離"),
    ("proper_distance_mile", "一哩"),
    ("proper_distance_middle", "中距離"),
    ("proper_distance_long", "長距離"),
    ("proper_running_style_nige", "領頭"),
    ("proper_running_style_senko", "前列"),
    ("proper_running_style_sashi", "居中"),
    ("proper_running_style_oikomi", "後追"),
];
/// 適性等級 1～8 = G～S
const APTITUDE_RANKS: [&str; 9] = ["不限", "G", "F", "E", "D", "C", "B", "A", "S"];

const STATS: [(&str, &str); 5] = [
    ("speed", "速度"),
    ("stamina", "持久力"),
    ("power", "力量"),
    ("guts", "意志力"),
    ("wiz", "智力"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    Blue,
    Red,
    Green,
    White,
}

impl Color {
    const ALL: [Color; 4] = [Color::Blue, Color::Red, Color::Green, Color::White];

    fn label(self) -> &'static str {
        match self {
            Color::Blue => "藍",
            Color::Red => "紅",
            Color::Green => "綠",
            Color::White => "白",
        }
    }

    fn fill(self) -> egui::Color32 {
        match self {
            Color::Blue => egui::Color32::from_rgb(0x2f, 0x6f, 0xc4),
            Color::Red => egui::Color32::from_rgb(0xc4, 0x3b, 0x4a),
            Color::Green => egui::Color32::from_rgb(0x3a, 0x9a, 0x4a),
            Color::White => egui::Color32::from_gray(0x55),
        }
    }

    /// 由 factor_id 判斷顏色（同因子卡片的分法）
    fn of(factor_id: i64) -> Color {
        if factor_id < 1000 {
            Color::Blue
        } else if factor_id < 10000 {
            Color::Red
        } else if factor_id >= 10_000_000 {
            Color::Green
        } else {
            Color::White
        }
    }
}

/// 一顆因子：group = factor_id / 100（同一種因子不分星數），stars = factor_id % 10
#[derive(Clone, Copy)]
struct Factor {
    group: u32,
    stars: u8,
    color: Color,
}

fn factors_of(v: &Value) -> Vec<Factor> {
    v.get("factor_info_array")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|f| f.get("factor_id")?.as_i64())
                .map(|id| Factor { group: (id / 100) as u32, stars: (id % 10) as u8, color: Color::of(id) })
                .collect()
        })
        .unwrap_or_default()
}

/// 篩選用的一隻練成角色
pub struct Horse {
    pub id: i64,
    name: String,
    rank_score: i64,
    stats: [i64; 5],
    aptitude: [u8; 10],
    /// [本人, 父, 母]（繼承來源 = succession_chara_array 的 position 10、20）
    factors: [Vec<Factor>; 3],
    skills: Vec<i32>,
    create_time: String,
}

impl Horse {
    fn from_json(id: i64, v: &Value) -> Horse {
        let int = |k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
        let parent = |pos: i64| {
            v.get("succession_chara_array")
                .and_then(|a| a.as_array())
                .and_then(|a| a.iter().find(|s| s.get("position_id").and_then(|p| p.as_i64()) == Some(pos)))
                .map(factors_of)
                .unwrap_or_default()
        };
        Horse {
            id,
            name: factor_card::maps().card_name(int("card_id")),
            rank_score: int("rank_score"),
            stats: STATS.map(|(k, _)| int(k)),
            aptitude: APTITUDES.map(|(k, _)| int(k).clamp(0, 8) as u8),
            factors: [factors_of(v), parent(10), parent(20)],
            skills: v.get("skill_array")
                .and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|s| s.get("skill_id")?.as_i64()).map(|i| i as i32).collect())
                .unwrap_or_default(),
            create_time: v.get("create_time").and_then(|x| x.as_str()).unwrap_or_default().to_owned(),
        }
    }

    /// 某一份因子（本人或某個父母）裡符合 target 的星數合計
    fn stars_in(list: &[Factor], target: Target) -> u32 {
        list.iter()
            .filter(|f| match target {
                Target::Factor(g) => f.group == g,
                Target::Color(c) => f.color == c,
            })
            .map(|f| f.stars as u32)
            .sum()
    }

    fn stars(&self, target: Target, scope: Scope) -> u32 {
        let each = self.factors.each_ref().map(|l| Self::stars_in(l, target));
        match scope {
            Scope::Own => each[0],
            Scope::Any => each.into_iter().max().unwrap_or(0),
            Scope::Sum => each.into_iter().sum(),
        }
    }

    fn white_count(&self) -> usize {
        self.factors[0].iter().filter(|f| f.color == Color::White).count()
    }
}

/// 收集表轉成的索引，收集表有變才重建
static INDEX: Lazy<Mutex<(u64, Vec<Horse>)>> = Lazy::new(|| Mutex::new((u64::MAX, Vec::new())));

fn with_index<R>(f: impl FnOnce(&[Horse]) -> R) -> R {
    let mut index = INDEX.lock().unwrap();
    let generation = factor_card::trained_generation();
    if index.0 != generation {
        index.1 = factor_card::with_trained(|all| all.iter().map(|(id, v)| Horse::from_json(*id, v)).collect());
        index.0 = generation;
    }
    f(&index.1)
}

/// 我的最愛圖示（0 = 未設定）與備忘錄，key = trained_chara_id
static FAVORITES: Lazy<Mutex<HashMap<i64, (i64, String)>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// 從一包已解碼的 response 收「我的最愛」與備忘錄（跟練成角色清單一起下來）
pub fn store_response(json: &Value) {
    let Some(arr) = json.get("data").and_then(|d| d.get("trained_chara_favorite_array")).and_then(|v| v.as_array()) else {
        return;
    };
    let mut fav = FAVORITES.lock().unwrap();
    for e in arr {
        let Some(id) = e.get("trained_chara_id").and_then(|v| v.as_i64()) else { continue };
        let icon = e.get("icon_type").and_then(|v| v.as_i64()).unwrap_or(0);
        let memo = e.get("memo").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
        fav.insert(id, (icon, memo));
    }
}

/// 包包裡現有的 trained_chara_id（問遊戲本身的清單）；還沒問過是 `None`
static OWNED: Mutex<Option<HashSet<i64>>> = Mutex::new(None);

/// 排程到遊戲主執行緒重新讀包包清單
fn request_owned_refresh() {
    super::main_thread::post(|| {
        if let Some(ids) = crate::il2cpp::hook::umamusume::WorkDataManager::trained_chara_ids() {
            *OWNED.lock().unwrap() = Some(ids.into_iter().collect());
        }
    });
}

/// 技能 id → 名稱（master.mdb，第一次要用時在主執行緒載入）
static SKILL_NAMES: OnceCell<FnvHashMap<i32, String>> = OnceCell::new();
static SKILL_NAMES_REQUESTED: Mutex<bool> = Mutex::new(false);

fn skill_names() -> Option<&'static FnvHashMap<i32, String>> {
    if let Some(m) = SKILL_NAMES.get() {
        return Some(m);
    }
    let mut requested = SKILL_NAMES_REQUESTED.lock().unwrap();
    if !*requested {
        *requested = true;
        super::main_thread::post(|| {
            _ = SKILL_NAMES.set(crate::il2cpp::sql::load_skill_names());
        });
    }
    None
}

/// 等著在主執行緒開詳細視窗的那隻（0 = 沒有）
static PENDING_OPEN: AtomicI64 = AtomicI64::new(0);
static STATUS: Mutex<String> = Mutex::new(String::new());

fn open_in_game(id: i64) {
    PENDING_OPEN.store(id, Ordering::Relaxed);
    super::main_thread::post(|| {
        let id = PENDING_OPEN.swap(0, Ordering::Relaxed);
        if id == 0 {
            return;
        }
        let opened = crate::il2cpp::hook::umamusume::DialogTrainedCharacterDetail::open(id as i32);
        *STATUS.lock().unwrap() = if opened {
            String::new()
        } else {
            // 多半是已經放生／轉讓掉了：順便重讀包包清單，讓它從結果消失
            request_owned_refresh();
            "開不了這隻（可能已經不在包包裡）".to_owned()
        };
    });
}

// —— 篩選條件 ——

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    /// 指定一種因子（group = factor_id / 100）
    Factor(u32),
    /// 某顏色的全部因子
    Color(Color),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// 只看本人
    Own,
    /// 本人或任一父母單獨達標（原生「繼承來源也包含」）
    Any,
    /// 本人＋父母合計
    Sum,
}

impl Scope {
    const ALL: [(Scope, &'static str); 3] = [
        (Scope::Own, "本人"),
        (Scope::Any, "本人或父母任一"),
        (Scope::Sum, "本人＋父母合計"),
    ];
}

#[derive(Clone)]
struct FactorCond {
    target: Target,
    min_stars: u32,
    scope: Scope,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Favorite {
    Any,
    Set,
    Unset,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    RankScore,
    Newest,
    BlueSum,
    WhiteCount,
}

impl Sort {
    const ALL: [(Sort, &'static str); 4] = [
        (Sort::RankScore, "評價分數"),
        (Sort::Newest, "取得時間（新→舊）"),
        (Sort::BlueSum, "藍因子星數（本人＋父母）"),
        (Sort::WhiteCount, "白因子數（本人）"),
    ];
}

struct Filter {
    name: String,
    aptitude_min: [u8; 10],
    stat_min: [i64; 5],
    rank_score_min: i64,
    conds: Vec<FactorCond>,
    skills: Vec<String>,
    favorite: Favorite,
    memo: String,
    sort: Sort,
    /// 加因子條件的搜尋框
    factor_query: String,
    skill_query: String,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            name: String::new(),
            aptitude_min: [0; 10],
            stat_min: [0; 5],
            rank_score_min: 0,
            conds: Vec::new(),
            skills: Vec::new(),
            favorite: Favorite::Any,
            memo: String::new(),
            sort: Sort::RankScore,
            factor_query: String::new(),
            skill_query: String::new(),
        }
    }
}

impl Filter {
    fn matches(&self, h: &Horse, fav: &HashMap<i64, (i64, String)>, skill_names: Option<&FnvHashMap<i32, String>>) -> bool {
        if !self.name.trim().is_empty() && !h.name.contains(self.name.trim()) {
            return false;
        }
        if h.aptitude.iter().zip(self.aptitude_min).any(|(a, min)| *a < min) {
            return false;
        }
        if h.stats.iter().zip(self.stat_min).any(|(s, min)| *s < min) || h.rank_score < self.rank_score_min {
            return false;
        }
        if !self.conds.iter().all(|c| h.stars(c.target, c.scope) >= c.min_stars) {
            return false;
        }
        if !self.skills.is_empty() {
            // 技能名稱還沒載入時先不擋
            if let Some(names) = skill_names {
                let owned: HashSet<&str> = h.skills.iter().filter_map(|id| names.get(id)).map(|s| s.as_str()).collect();
                if !self.skills.iter().all(|s| owned.contains(s.as_str())) {
                    return false;
                }
            }
        }
        let (icon, memo) = fav.get(&h.id).map(|(i, m)| (*i, m.as_str())).unwrap_or((0, ""));
        match self.favorite {
            Favorite::Any => {}
            Favorite::Set if icon == 0 => return false,
            Favorite::Unset if icon != 0 => return false,
            _ => {}
        }
        if !self.memo.trim().is_empty() && !memo.contains(self.memo.trim()) {
            return false;
        }
        true
    }

    fn sort(&self, list: &mut Vec<&Horse>) {
        match self.sort {
            Sort::RankScore => list.sort_by_key(|h| std::cmp::Reverse(h.rank_score)),
            Sort::Newest => list.sort_by(|a, b| b.create_time.cmp(&a.create_time)),
            Sort::BlueSum => list.sort_by_key(|h| std::cmp::Reverse(h.stars(Target::Color(Color::Blue), Scope::Sum))),
            Sort::WhiteCount => list.sort_by_key(|h| std::cmp::Reverse(h.white_count())),
        }
    }
}

static FILTER: Lazy<Mutex<Filter>> = Lazy::new(|| Mutex::new(Filter::default()));
/// 第一次打開分頁時讀一次包包清單
static FIRST_SHOW: std::sync::Once = std::sync::Once::new();

fn target_label(t: Target) -> String {
    match t {
        Target::Factor(g) => factor_card::maps().factor.get(&g).map(|e| e.name.clone()).unwrap_or_else(|| g.to_string()),
        Target::Color(c) => format!("{}因子（全部）", c.label()),
    }
}

fn target_color(t: Target) -> Color {
    match t {
        // 用 group 還原成一個 factor_id 判顏色
        Target::Factor(g) => Color::of(g as i64 * 100 + 1),
        Target::Color(c) => c,
    }
}

fn chip(ui: &mut egui::Ui, color: Color, text: &str) {
    egui::Frame::new()
        .fill(color.fill())
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(4, 1))
        .show(ui, |ui| ui.label(egui::RichText::new(text).color(egui::Color32::WHITE).small()));
}

// —— 介面 ——

/// 設定視窗「馬娘篩選」分頁
pub fn ui(ui: &mut egui::Ui) {
    FIRST_SHOW.call_once(request_owned_refresh);
    let skill_names = skill_names();
    let mut filter = FILTER.lock().unwrap();
    let filter = &mut *filter;

    ui.label("進一次遊戲的包包（練成角色清單）就有資料；點結果會在遊戲裡打開那隻的詳細資料，不會連線。");
    ui.add_space(4.0);

    // 條件區最多佔一半高度，剩下留給結果清單
    egui::ScrollArea::vertical()
        .id_salt("chara_filter_conds")
        .max_height(ui.available_height() * 0.5)
        .auto_shrink([false, true])
        .show(ui, |ui| {
        // 角色、排序
        ui.horizontal_wrapped(|ui| {
            ui.label("角色／服裝");
            ui.add(egui::TextEdit::singleline(&mut filter.name).hint_text("名稱包含…").desired_width(160.0));
            ui.label("排序");
            egui::ComboBox::from_id_salt("chara_filter_sort")
                .selected_text(Sort::ALL.iter().find(|(s, _)| *s == filter.sort).map(|(_, l)| *l).unwrap_or(""))
                .show_ui(ui, |ui| {
                    for (s, l) in Sort::ALL {
                        ui.selectable_value(&mut filter.sort, s, l);
                    }
                });
            if ui.button("重置條件").clicked() {
                *filter = Filter::default();
            }
        });

        egui::CollapsingHeader::new("適性（最低等級）").default_open(false).show(ui, |ui| {
            egui::Grid::new("chara_filter_aptitude").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
                for (i, (_, label)) in APTITUDES.iter().enumerate() {
                    ui.label(*label);
                    egui::ComboBox::from_id_salt(("chara_filter_apt", i))
                        .width(60.0)
                        .selected_text(APTITUDE_RANKS[filter.aptitude_min[i] as usize])
                        .show_ui(ui, |ui| {
                            for (r, name) in APTITUDE_RANKS.iter().enumerate() {
                                ui.selectable_value(&mut filter.aptitude_min[i], r as u8, *name);
                            }
                        });
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        });

        egui::CollapsingHeader::new("五維／評價（最低值）").default_open(false).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, (_, label)) in STATS.iter().enumerate() {
                    ui.label(*label);
                    ui.add(egui::DragValue::new(&mut filter.stat_min[i]).range(0..=2000).speed(10));
                }
                ui.label("評價分數");
                ui.add(egui::DragValue::new(&mut filter.rank_score_min).range(0..=50000).speed(100));
            });
        });

        egui::CollapsingHeader::new(format!("因子條件（{}）", filter.conds.len())).default_open(true).show(ui, |ui| {
            ui.small("每一條都要符合。「本人或父母任一」＝原生的「繼承來源也包含」；「合計」是本人＋兩個父母的星數加總。");
            let mut remove = None;
            for (i, c) in filter.conds.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    chip(ui, target_color(c.target), &target_label(c.target));
                    egui::ComboBox::from_id_salt(("chara_filter_scope", i))
                        .selected_text(Scope::ALL.iter().find(|(s, _)| *s == c.scope).map(|(_, l)| *l).unwrap_or(""))
                        .show_ui(ui, |ui| {
                            for (s, l) in Scope::ALL {
                                ui.selectable_value(&mut c.scope, s, l);
                            }
                        });
                    ui.label("≥");
                    ui.add(egui::DragValue::new(&mut c.min_stars).range(1..=60).suffix("★"));
                    if ui.small_button("\u{f00d}").on_hover_text("移除").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                filter.conds.remove(i);
            }

            ui.horizontal_wrapped(|ui| {
                ui.label("加條件：");
                for c in Color::ALL {
                    if ui.button(format!("{}因子合計", c.label())).clicked() {
                        filter.conds.push(FactorCond { target: Target::Color(c), min_stars: 1, scope: Scope::Own });
                    }
                }
            });
            ui.add(egui::TextEdit::singleline(&mut filter.factor_query).hint_text("搜尋因子名稱（速度、中距離、固有、技能白因子…）").desired_width(f32::INFINITY));
            let q = filter.factor_query.trim();
            if !q.is_empty() {
                let mut hits: Vec<(u32, &str)> = factor_card::maps().factor.iter()
                    .filter(|(_, e)| matches!(e.factor_type, Some(1..=4)) && e.name.contains(q))
                    .map(|(g, e)| (*g, e.name.as_str()))
                    .collect();
                hits.sort_by_key(|(g, n)| (!n.starts_with(q), *g));
                hits.truncate(30);
                let mut picked = None;
                ui.horizontal_wrapped(|ui| {
                    for (g, name) in &hits {
                        let color = Color::of(*g as i64 * 100 + 1);
                        if ui.add(egui::Button::new(egui::RichText::new(*name).color(egui::Color32::WHITE)).fill(color.fill())).clicked() {
                            picked = Some(*g);
                        }
                    }
                    if hits.is_empty() {
                        ui.weak("找不到");
                    }
                });
                if let Some(g) = picked {
                    filter.conds.push(FactorCond { target: Target::Factor(g), min_stars: 1, scope: Scope::Own });
                    filter.factor_query.clear();
                }
            }
        });

        egui::CollapsingHeader::new(format!("技能（{}）", filter.skills.len())).default_open(false).show(ui, |ui| {
            let Some(names) = skill_names else {
                ui.weak("技能名稱載入中…");
                return;
            };
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                for (i, s) in filter.skills.iter().enumerate() {
                    if ui.button(format!("{s} \u{f00d}")).on_hover_text("移除").clicked() {
                        remove = Some(i);
                    }
                }
            });
            if let Some(i) = remove {
                filter.skills.remove(i);
            }
            ui.add(egui::TextEdit::singleline(&mut filter.skill_query).hint_text("搜尋技能名稱，要全部都有").desired_width(f32::INFINITY));
            let q = filter.skill_query.trim();
            if !q.is_empty() {
                let mut hits: Vec<&str> = names.values().map(|s| s.as_str()).filter(|n| n.contains(q)).collect();
                hits.sort_by_key(|n| (!n.starts_with(q), n.len()));
                hits.dedup();
                hits.truncate(30);
                let mut picked = None;
                ui.horizontal_wrapped(|ui| {
                    for n in &hits {
                        if ui.button(*n).clicked() {
                            picked = Some(n.to_string());
                        }
                    }
                });
                if let Some(n) = picked {
                    if !filter.skills.contains(&n) {
                        filter.skills.push(n);
                    }
                    filter.skill_query.clear();
                }
            }
        });

        egui::CollapsingHeader::new("我的最愛／備忘錄").default_open(false).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut filter.favorite, Favorite::Any, "不限");
                ui.selectable_value(&mut filter.favorite, Favorite::Set, "有設我的最愛");
                ui.selectable_value(&mut filter.favorite, Favorite::Unset, "沒設我的最愛");
                ui.label("備忘錄");
                ui.add(egui::TextEdit::singleline(&mut filter.memo).hint_text("包含…").desired_width(140.0));
            });
        });
    });

    ui.separator();

    // —— 結果 ——
    let owned = OWNED.lock().unwrap().clone();
    let fav = FAVORITES.lock().unwrap();
    with_index(|horses| {
        let mine: Vec<&Horse> = horses.iter()
            .filter(|h| owned.as_ref().map_or(true, |o| o.contains(&h.id)))
            .collect();
        let mut hits: Vec<&Horse> = mine.iter().copied().filter(|h| filter.matches(h, &fav, skill_names)).collect();
        filter.sort(&mut hits);

        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("符合 {} 隻", hits.len())).strong());
            match &owned {
                Some(o) => {
                    ui.weak(format!("（包包 {} 隻，有資料 {} 隻）", o.len(), mine.len()));
                    if mine.len() < o.len() {
                        ui.weak("缺的進一次包包就會補上");
                    }
                }
                None => {
                    ui.weak("（還沒讀到遊戲的包包清單，暫時列出全部收到的資料）");
                }
            }
            if ui.small_button("重新整理").clicked() {
                request_owned_refresh();
            }
        });
        let status = STATUS.lock().unwrap().clone();
        if !status.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, status);
        }

        let row_h = 44.0;
        egui::ScrollArea::vertical()
            .id_salt("chara_filter_results")
            .max_height(ui.available_height().max(240.0))
            .auto_shrink([false, true])
            .show_rows(ui, row_h, hits.len(), |ui, range| {
                for h in &hits[range] {
                    result_row(ui, h, row_h);
                }
            });
    });
}

fn result_row(ui: &mut egui::Ui, h: &Horse, height: f32) {
    ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
        ui.horizontal(|ui| {
            if ui.button("開啟").on_hover_text("在遊戲裡打開詳細資料").clicked() {
                open_in_game(h.id);
            }
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&h.name).strong());
                    ui.weak(format!("評價 {}　{}", h.rank_score,
                        h.stats.iter().map(|s| s.to_string()).collect::<Vec<_>>().join("/")));
                });
                ui.horizontal(|ui| {
                    for c in [Color::Blue, Color::Red, Color::Green] {
                        for f in h.factors[0].iter().filter(|f| f.color == c) {
                            let name = factor_card::maps().factor.get(&f.group).map(|e| e.name.as_str()).unwrap_or("?");
                            chip(ui, c, &format!("{name}{}", f.stars));
                        }
                    }
                    let white = h.white_count();
                    if white > 0 {
                        chip(ui, Color::White, &format!("白×{white}"));
                    }
                    let blue_sum = h.stars(Target::Color(Color::Blue), Scope::Sum);
                    ui.weak(format!("藍合計 {blue_sum}★"));
                });
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用實際抓到的包包封包驗證：解析、因子星數三種算法、條件組合。檔案不在就跳過。
    #[test]
    fn filters_captured_bag() {
        let path = r"G:\komoemumamusume\komoemumamusume Game\hachimi\api_capture_20260901\0886_room_match_entry_chara_id_array_trained_chara_array_trained_.json";
        let Ok(text) = std::fs::read_to_string(path) else { eprintln!("skip: fixture missing"); return; };
        let json: Value = serde_json::from_str(&text).unwrap();
        let horses: Vec<Horse> = json["data"]["trained_chara_array"].as_array().unwrap().iter()
            .map(|v| Horse::from_json(v["trained_chara_id"].as_i64().unwrap(), v))
            .collect();
        assert_eq!(horses.len(), 352);

        // trained_chara_id 916：本人 速度1★、父母各有因子
        let h = horses.iter().find(|h| h.id == 916).unwrap();
        assert_eq!(h.stars(Target::Factor(1), Scope::Own), 1);
        let blue = Target::Color(Color::Blue);
        let (own, any, sum) = (h.stars(blue, Scope::Own), h.stars(blue, Scope::Any), h.stars(blue, Scope::Sum));
        assert!(own <= any && any <= sum, "{own} {any} {sum}");

        let fav = HashMap::new();
        let count = |f: &Filter| horses.iter().filter(|h| f.matches(h, &fav, None)).count();
        let all = count(&Filter::default());
        assert_eq!(all, 352);

        let mut f = Filter::default();
        f.conds.push(FactorCond { target: blue, min_stars: 3, scope: Scope::Own });
        let own3 = count(&f);
        f.conds[0].scope = Scope::Sum;
        let sum3 = count(&f);
        f.conds[0].min_stars = 7;
        let sum7 = count(&f);
        f.aptitude_min[4] = 7; // 中距離 ≥ A
        let sum7_mid = count(&f);
        eprintln!("全部 {all}、藍≥3（本人）{own3}、藍≥3（合計）{sum3}、藍≥7（合計）{sum7}、再加中距離A {sum7_mid}");
        assert!(own3 <= sum3 && sum7 <= sum3 && sum7_mid <= sum7);
    }
}
