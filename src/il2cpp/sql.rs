//! 遊戲 SQLite 查詢攔截的共用型別，以及「技能資料說明」（skill_data_desc）。
//!
//! skill_data_desc 移植自 Hachimi-Edge（2b490c4 / c971890 / 4834434）：讀 master.mdb 的 `skill_data`，
//! 把發動條件（condition / precondition）與效果（ability_type / value / target）組成人看得懂的文字，
//! 在遊戲查 `text_data` category 48（技能說明）時換掉原本的說明。字串在 locale 的 `skill_data_desc.*`。
//! 原本整套翻譯用的 SQL 查詢替換已在 bc3f711 拔掉，這裡只保留技能說明需要的部分。

use std::ptr;
use fnv::FnvHashMap;
use sqlparser::ast;
use once_cell::sync::OnceCell;
use crate::{
    core::Hachimi,
    il2cpp::{ext::{StringExt, Il2CppStringExt}, hook::LibNative_Runtime::Sqlite3::{Connection, Query}, types::{Il2CppObject, Il2CppString}}
};
use rust_i18n::locale;

/// master.mdb 位置：`<exe 目錄>/<exe 名>_Data/Persistent/master/master.mdb`
/// （台服 `komoeumamusume_Data\Persistent\master\master.mdb`）。
fn get_masterdb_path() -> String {
    #[cfg(target_os = "windows")]
    {
        let exe = crate::windows::utils::get_exec_path();
        let mut data_dir = exe.file_stem().unwrap_or_default().to_owned();
        data_dir.push("_Data");
        exe.parent().unwrap_or(std::path::Path::new("."))
            .join(data_dir).join("Persistent").join("master").join("master.mdb")
            .to_string_lossy().into_owned()
    }
    #[cfg(not(target_os = "windows"))]
    {
        String::new()
    }
}

/// 第一次用到時才從 master.mdb 建。必須在 il2cpp 執行緒上（查詢走遊戲自己的 Sqlite3 wrapper），
/// 而呼叫點本來就在遊戲查 text_data 的當下（Query::GetText hook 裡）。
/// 注意：這時 GetText hook 正鎖著 SELECT_QUERIES，所以建表時的 GetText／Dispose 一律走 `*_orig`，
/// 不能再經過 hook。只建一次；失敗（空表）就記一筆 log，之後不再重試。
static SKILL_DATA_DESC: OnceCell<Option<SkillDataDesc>> = OnceCell::new();

fn skill_data_desc() -> Option<&'static SkillDataDesc> {
    SKILL_DATA_DESC.get_or_init(|| {
        let data = SkillDataDesc::load_from_db();
        if data.descs.is_empty() {
            warn!("[skill_data_desc] master.mdb 讀不到 skill_data（{}）", get_masterdb_path());
            None
        }
        else {
            info!("[skill_data_desc] 已建立 {} 筆技能說明", data.descs.len());
            Some(data)
        }
    }).as_ref()
}

/// 台服的技能名稱（text_data category 47），給「使用了技能 X」條件用；locale 裡寫死的是簡中名稱。
static SKILL_NAMES: OnceCell<FnvHashMap<i32, String>> = OnceCell::new();

fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// 這段文字是不是我們產生的技能說明（給換行 hook 判斷要不要接手）。說明還沒建好時一律 false。
pub fn is_skill_data_desc_text(text: &str) -> bool {
    SKILL_DATA_DESC.get()
        .and_then(|d| d.as_ref())
        .is_some_and(|d| d.text_hashes.contains(&text_hash(text)))
}

pub trait SelectQueryState {
    /// Adds a column to the query.
    ///
    /// Implementers are expected to only track the index of columns that they need.
    fn add_column(&mut self, idx: i32, name: &str);

    /// Adds a placeholder parameter to the query (WHERE param = ?).
    ///
    /// Index starts at 1.
    fn add_param(&mut self, idx: i32, name: &str);

    /// Bind an int value to a placeholder.
    ///
    /// Index starts at 1.
    fn bind_int(&mut self, idx: i32, value: i32);

    /// Gets the resulting string on the current row's column.
    fn get_text(&self, query: *mut Il2CppObject, idx: i32) -> Option<*mut Il2CppString>;
}

#[derive(Default)]
struct Column {
    /// Index of the column in the SELECT statement.
    ///
    /// Can be used to query the value later if needed.
    select_idx: Option<i32>,

    /// Index of the placeholder param for this column.
    ///
    /// If this column's value is already binded as a param in the query, we won't need to query it later.
    param_idx: Option<i32>,

    /// The int value binded to this column as a parameter.
    int_value: Option<i32>
}

impl Column {
    fn is_select_idx(&self, idx: i32) -> bool {
        if let Some(i) = self.select_idx {
            idx == i
        }
        else {
            false
        }
    }

    fn is_param_idx(&self, idx: i32) -> bool {
        if let Some(i) = self.param_idx {
            idx == i
        }
        else {
            false
        }
    }

    fn try_bind_int(&mut self, idx: i32, value: i32) {
        if self.is_param_idx(idx) {
            self.int_value = Some(value);
        }
    }
}

/// 一組效果（condition_N + 最多 3 個效果）拆好的各部分，組合時才決定要印哪些。
struct GroupParts {
    /// 拆出來的硬限制（跑法／距離／場地／賽場）：外層是選項（或），內層是同時成立的項目（且），
    /// 例：[[後追, 中距], [後追, 長距]]。保留結構是為了能算兩組效果的共同限制。
    restrictions: Vec<Vec<String>>,
    effects: Vec<String>,
    /// 「持續X秒」「立即發動」…
    time: String,
    /// 「（冷卻X秒）」，沒有就空字串
    cd: String,
    /// 「 條件：…」「 附加：…」
    cond: String,
    /// 拆掉硬限制後的原始條件式（condition, precondition），給加強版比對「多了哪幾項」用
    cond_raw: (String, String),
}

#[derive(Default)]
pub struct SkillDataDesc {
    pub descs: FnvHashMap<i32, String>,
    /// descs 每段文字的雜湊，見 [`is_skill_data_desc_text`]
    text_hashes: fnv::FnvHashSet<u64>
}

struct SkillDataDescRow {
    id: i32,
    precondition_1: String,
    condition_1: String,
    ability_time_1: i32,
    cooldown_time_1: i32,
    precondition_2: String,
    condition_2: String,
    ability_time_2: i32,
    cooldown_time_2: i32,
    slots: [SkillDataDescSlot; 6]
}

#[derive(Clone, Copy, Default)]
struct SkillDataDescSlot {
    ability_type: i32,
    ability_value: i32,
    ability_value_usage: i32,
    additional_activate_type: i32,
    target_type: i32,
    target_value: i32
}

impl SkillDataDesc {
    pub fn load_from_db() -> Self {
        let mut descs = FnvHashMap::default();

        let db_path = get_masterdb_path();
        let conn = Connection::new();

        if Connection::Open(conn, db_path.to_il2cpp_string(), ptr::null_mut(), ptr::null_mut(), 0) {
            // 技能名稱要先有，格式化條件時會用到
            let mut names = FnvHashMap::default();
            let name_query = Connection::Query_orig(conn, "SELECT \"index\", text FROM text_data WHERE category = 47".to_il2cpp_string());
            if !name_query.is_null() {
                while Query::Step(name_query) {
                    names.insert(Query::GetInt(name_query, 0), Self::get_data_text(name_query, 1));
                }
                Query::Dispose_orig(name_query);
            }
            let _ = SKILL_NAMES.set(names);

            let sql = "SELECT id, \
                precondition_1, condition_1, float_ability_time_1, float_cooldown_time_1, \
                ability_type_1_1, ability_value_usage_1_1, additional_activate_type_1_1, float_ability_value_1_1, target_type_1_1, target_value_1_1, \
                ability_type_1_2, ability_value_usage_1_2, additional_activate_type_1_2, float_ability_value_1_2, target_type_1_2, target_value_1_2, \
                ability_type_1_3, ability_value_usage_1_3, additional_activate_type_1_3, float_ability_value_1_3, target_type_1_3, target_value_1_3, \
                precondition_2, condition_2, float_ability_time_2, float_cooldown_time_2, \
                ability_type_2_1, ability_value_usage_2_1, additional_activate_type_2_1, float_ability_value_2_1, target_type_2_1, target_value_2_1, \
                ability_type_2_2, ability_value_usage_2_2, additional_activate_type_2_2, float_ability_value_2_2, target_type_2_2, target_value_2_2, \
                ability_type_2_3, ability_value_usage_2_3, additional_activate_type_2_3, float_ability_value_2_3, target_type_2_3, target_value_2_3 \
                FROM skill_data";
            let query = Connection::Query_orig(conn, sql.to_il2cpp_string());

            if !query.is_null() {
                while Query::Step(query) {
                    let row = Self::get_data_row(query);
                    let desc = Self::format_data_desc(&row);
                    // 產生不出說明（例：活動專用技能）就不收，遊戲照原本顯示
                    if !desc.is_empty() {
                        descs.insert(row.id, desc);
                    }
                }
                Query::Dispose_orig(query);
            }
            Connection::CloseDB(conn);
        }

        let text_hashes = descs.values().map(|t| text_hash(t)).collect();
        SkillDataDesc { descs, text_hashes }
    }

    pub fn get_desc(&self, id: i32) -> Option<&String> {
        self.descs.get(&id)
    }
    
    fn get_data_slot(query: *mut Il2CppObject, base: i32) -> SkillDataDescSlot {
        SkillDataDescSlot {
            ability_type: Query::GetInt(query, base),
            ability_value_usage: Query::GetInt(query, base + 1),
            additional_activate_type: Query::GetInt(query, base + 2),
            ability_value: Query::GetInt(query, base + 3),
            target_type: Query::GetInt(query, base + 4),
            target_value: Query::GetInt(query, base + 5)
        }
    }

    fn get_data_text(query: *mut Il2CppObject, idx: i32) -> String {
        let text_ptr = Query::GetText_orig(query, idx);
        unsafe { text_ptr.as_ref() }.map(|s| s.as_utf16str().to_string()).unwrap_or_default()
    }

    fn get_data_row(query: *mut Il2CppObject) -> SkillDataDescRow {
        SkillDataDescRow {
            id: Query::GetInt(query, 0),
            precondition_1: Self::get_data_text(query, 1),
            condition_1: Self::get_data_text(query, 2),
            ability_time_1: Query::GetInt(query, 3),
            cooldown_time_1: Query::GetInt(query, 4),
            precondition_2: Self::get_data_text(query, 23),
            condition_2: Self::get_data_text(query, 24),
            ability_time_2: Query::GetInt(query, 25),
            cooldown_time_2: Query::GetInt(query, 26),
            slots: [
                Self::get_data_slot(query, 5), Self::get_data_slot(query, 11), Self::get_data_slot(query, 17),
                Self::get_data_slot(query, 27), Self::get_data_slot(query, 33), Self::get_data_slot(query, 39)
            ]
        }
    }

    fn round_ties_up(value: i32, units: i32) -> i32 {
        let rem = value.rem_euclid(units);
        let base = value - rem;
        if rem * 2 >= units { base + units } else { base }
    }

    fn format_data_number(value: i32, div: i32, decimals: usize) -> String {
        let units = div / 10i32.pow(decimals as u32);
        let rounded = Self::round_ties_up(value, units);
        let neg = rounded < 0;
        let abs = rounded.unsigned_abs() as u64;
        let div = div as u64;
        let whole = abs / div;
        let frac = (abs % div) / (div / 10u64.pow(decimals as u32));

        let mut out = String::new();
        if neg {
            out.push('-');
        }
        out.push_str(&whole.to_string());
        if frac > 0 {
            out.push('.');
            let frac_str = format!("{:0width$}", frac, width = decimals);
            out.push_str(frac_str.trim_end_matches('0'));
        }
        out
    }

    fn str(key: &str) -> Option<String> {
        let full_key = format!("skill_data_desc.{key}");
        let locale = locale();
        crate::_rust_i18n_try_translate(&locale, full_key.as_str()).map(|text| text.to_string())
    }

    fn data_fmt(key: &str, value: &str) -> Option<String> {
        Self::str(key).map(|text| text.replace("%{v}", value))
    }

    fn op_tag(op: &str) -> &str {
        match op {
            "==" => "eq",
            "!=" => "ne",
            "<=" => "le",
            ">=" => "ge",
            "<" => "lt",
            ">" => "gt",
            _ => "op"
        }
    }

    fn format_effect(slot: SkillDataDescSlot) -> Option<String> {
        let (name_key, unit_key, div, decimals) = match slot.ability_type {
            1 => ("speed_stat", "stat", 10000, 2),
            2 => ("stamina_stat", "stat", 10000, 2),
            3 => ("power_stat", "stat", 10000, 2),
            4 => ("guts_stat", "stat", 10000, 2),
            5 => ("wit_stat", "stat", 10000, 2),
            8 => ("field_of_view", "deg", 10000, 2),
            9 => ("current_hp", "percent", 100, 1),
            13 => ("rushed_time", "second", 10000, 2),
            14 => ("delay_start", "second", 10000, 2),
            21 => ("current_speed", "mps", 10000, 2),
            22 => ("current_speed_natural_decel", "mps", 10000, 2),
            27 => ("target_speed", "mps", 10000, 2),
            28 => ("lane_movement_speed", "percent", 100, 1),
            29 => ("rushed_chance", "stat", 10000, 2),
            31 => ("acceleration", "mps2", 10000, 2),
            32 => ("all_stats", "stat", 10000, 2),
            35 => ("target_lane", "stat", 10000, 2),
            37 => ("activate_rare_skill", "stat", 10000, 2),
            // 42：此技能與已進化技能的效果時間倍率（台服僅 Weaving History，數值 2.0 ＝ 2 倍）
            42 => match Self::data_fmt("effect.fixed.duration_mult", &Self::format_data_number(slot.ability_value, 10000, 2)) {
                Some(text) => return Some(text),
                None => ("special", "stat", 10000, 2)
            },
            48 | 49 => ("special", "stat", 10000, 2),
            // 501～503：活動（競速狂歡節）專用，實際數值由活動決定、master.mdb 沒有 → 不產生說明，保留遊戲原文
            501 | 502 => return None,

            6 => return Self::str("effect.fixed.aggressive_strategy"),
            38 => return Self::str("effect.fixed.debuff_immunity"),
            41 => return Self::str("effect.fixed.sympathy_all"),
            502 => return Self::str("effect.fixed.loh_stat"),
            10 => return Self::str(&format!("effect.start_reaction.{}", slot.ability_value)),
            503 | _ => return None,
        };

        let name = Self::str(&format!("effect.name.{name_key}"))?;
        let unit = Self::str(&format!("effect.unit.{unit_key}")).unwrap_or_default();
        let value = Self::format_data_number(slot.ability_value, div, decimals);
        let sign = if slot.ability_value > 0 { " +" } else { " " };
        let mut out = format!("{name}{sign}{value}{unit}");
        if slot.ability_value_usage == 19 {
            out.push_str(&Self::str("effect.usage19_suffix").unwrap_or_default());
        }

        let star = match slot.additional_activate_type {
            1 => Self::str("star.activate.1"),
            2 => Self::str("star.activate.2"),
            3 => Self::str("star.activate.3"),
            _ => None
        }.or_else(|| {
            if slot.ability_value_usage != 1 {
                Self::str(&format!("star.usage.{}", slot.ability_value_usage))
            } else {
                None
            }
        });
        if let Some(star) = star {
            out.push_str(&Self::str("sep.star").unwrap_or_default());
            out.push_str(&star);
        }

        if slot.target_type != 1 {
            let target = match slot.target_type {
                4 => Self::str("target.all_in_fov"),
                7 => Self::data_fmt("target.leading", &(slot.target_value - 1).to_string()),
                9 => if slot.target_value == 18 {
                    Self::str("target.all_ahead")
                } else {
                    Self::data_fmt("target.closest_ahead", &slot.target_value.to_string())
                },
                10 => if slot.target_value == 18 {
                    Self::str("target.all_behind")
                } else {
                    Self::data_fmt("target.closest_behind", &slot.target_value.to_string())
                },
                11 => Self::str("target.team"),
                18 => match Self::str(&format!("target.style.{}", slot.target_value)) {
                    Some(text) => Some(text),
                    None => return None
                },
                19 => Self::data_fmt("target.random_rushed_ahead", &slot.target_value.to_string()),
                20 => Self::data_fmt("target.random_rushed_behind", &slot.target_value.to_string()),
                21 => match Self::str(&format!("target.style_rushed.{}", slot.target_value)) {
                    Some(text) => Some(text),
                    None => return None
                },
                22 => Self::str("target.suzuka"),
                23 => Self::data_fmt("target.random_recovery_users", &slot.target_value.to_string()),
                24 => Self::str("target.unknown"),
                _ => None
            };
            if let Some(target) = target {
                // locale 有 sep.target_wrap（例「（%{v}）」）就整段包起來，沒有就照舊「 to 對象」
                match Self::data_fmt("sep.target_wrap", &target) {
                    Some(wrapped) => out.push_str(&wrapped),
                    None => {
                        out.push_str(&Self::str("sep.to").unwrap_or_default());
                        out.push_str(&target);
                    }
                }
            }
        }

        Some(out)
    }

    /// 「硬限制」：跑法、距離、場地、賽場（只認 `==`）。這些決定技能「能不能用」，跟發動時機無關，
    /// 所以從條件裡拆出來，放在說明最前面醒目標示（遊戲原文是放在最後的「＜一哩/中距離＞」）。
    const HARD_TOKENS: [&'static str; 4] = ["running_style", "distance_type", "ground_type", "track_id"];

    fn is_hard_atom(atom: &str) -> bool {
        Self::HARD_TOKENS.iter().any(|t| {
            atom.strip_prefix(t).is_some_and(|rest| rest.starts_with("=="))
        })
    }

    /// 把條件拆成（剩下的條件原始字串, 硬限制文字清單）。
    /// 兩種情況才拆：各分支硬限制完全相同；或「每個 OR 分支拿掉硬限制後剩下的條件都一樣」（例：`一哩&第3彎道@中距離&第3彎道`
    /// → 限制「一哩／中距離」、條件「第3彎道」）；分支之間其他條件不同時硬拆會弄錯配對，就不拆。
    fn split_hard_conditions(condition: &str) -> (String, Vec<Vec<String>>) {
        if condition.is_empty() {
            return (String::new(), Vec::new());
        }
        // 硬限制排固定順序（賽場 → 距離 → 場地 → 跑法），同樣的組合才會寫得一樣
        const ORDER: [&str; 4] = ["track_id", "distance_type", "ground_type", "running_style"];
        let rank = |a: &str| ORDER.iter().position(|t| a.starts_with(t)).unwrap_or(ORDER.len());
        let branches: Vec<(Vec<&str>, Vec<&str>)> = condition.split('@')
            .map(|g| {
                let (mut hard, soft): (Vec<&str>, Vec<&str>) = g.split('&').partition(|a| Self::is_hard_atom(a));
                hard.sort_by_key(|a| rank(a));
                (hard, soft)
            })
            .collect();
        if branches.iter().all(|(hard, _)| hard.is_empty()) {
            return (condition.to_string(), Vec::new());
        }
        // 每個分支的硬限制都一樣（例：`超越中&一哩@被超越&一哩`）→ 直接拆出，各分支保留自己的其他條件
        let first_hard = &branches[0].0;
        if branches.iter().all(|(hard, _)| hard == first_hard) {
            let label: Vec<String> = first_hard.iter().map(|a| Self::format_tag_atom(a)).collect();
            let rest = branches.iter().map(|(_, soft)| soft.join("&")).collect::<Vec<_>>().join("@");
            return (rest, vec![label]);
        }
        let first_soft = &branches[0].1;
        if !branches.iter().all(|(_, soft)| soft == first_soft) {
            return (condition.to_string(), Vec::new());
        }
        let mut labels: Vec<Vec<String>> = Vec::new();
        for (hard, _) in &branches {
            let label: Vec<String> = hard.iter().map(|a| Self::format_tag_atom(a)).collect();
            if !label.is_empty() && !labels.contains(&label) {
                labels.push(label);
            }
        }
        (first_soft.join("&"), labels)
    }

    /// 拆原始條件 `token op value`（例 `order_rate>=40` → ("order_rate", ">=", 40)）。
    fn parse_atom(atom: &str) -> (&str, &str, i32) {
        let token_end = atom.find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')).unwrap_or(atom.len());
        let rest = &atom[token_end..];
        let op_end = rest.find(|c: char| !matches!(c, '=' | '!' | '<' | '>')).unwrap_or(rest.len());
        (&atom[..token_end], &rest[..op_end], rest[op_end..].parse().unwrap_or(0))
    }

    /// 一組 AND 條件。同一個條件同時有 `>=` 下限與 `<=` 上限、且 locale 有對應的 `range` 字串時，
    /// 合併成區間（「名次40～70%」「競賽距離50～60%」「剩餘距離199～201米」）；其餘照原本不等式顯示。
    /// range 字串：`cond.order_rate.range`，其他在 `cond.template.<token>.range`。
    fn format_and_group(atoms: &[&str], and_sep: &str) -> String {
        let parsed: Vec<(&str, &str, i32)> = atoms.iter().map(|a| Self::parse_atom(a)).collect();
        let mut merged: Vec<(usize, usize, String)> = Vec::new(); // (前面那個的位置, 後面那個的位置, 區間文字)
        // 賽場編號 10001～10010 正好是 JRA 的 10 個賽場：`track_id>=10001&track_id<=10010` 整組就是「JRA賽場」
        // （單看 >=10001 會被翻成「任意賽場」，拆開寫變成「在任意賽場、在JRA賽場」）
        let lo = parsed.iter().position(|(t, op, v)| *t == "track_id" && *op == ">=" && *v == 10001);
        let hi = parsed.iter().position(|(t, op, v)| *t == "track_id" && *op == "<=" && *v == 10010);
        if let (Some(lo), Some(hi), Some(text)) = (lo, hi, Self::str("cond.track_id.jra")) {
            merged.push((lo.min(hi), lo.max(hi), text));
        }
        // 「最後彎道起」＋「在彎道上／在直線上」＝「在最後彎道上／在最後直線上」
        // （分開寫成「最後彎道/直線、在彎道上」讀起來像直線也算）
        if let Some(fc) = parsed.iter().position(|(t, op, v)| *t == "is_finalcorner" && *op == "==" && *v == 1) {
            for (op, key) in [("!=", "cond.final_corner.on_corner"), ("==", "cond.final_corner.on_straight")] {
                let pos = parsed.iter().position(|(t, o, v)| *t == "corner" && *o == op && *v == 0);
                if let (Some(pos), Some(text)) = (pos, Self::str(key)) {
                    merged.push((fc.min(pos), fc.max(pos), text));
                    break;
                }
            }
        }
        // 能力值：比較方式與數值相同的合併、取首字縮寫（速耐力根智）；五項全有寫「五圍」。
        // 例：基礎速度≥1000、…、基礎智力≥1000 → 五圍≥1000；基礎速度≥1200、基礎力量≥1200 → 速力≥1200
        // locale 沒有 cond.base_stat.* 就照原本逐項寫。多項合併時，第 3 項起用 usize::MAX 當「前面」只用來跳過。
        const STATS: [&str; 5] = ["base_speed", "base_stamina", "base_power", "base_guts", "base_wiz"];
        if Self::str("cond.base_stat.base_speed").is_some() {
            let mut groups: Vec<((&str, i32), Vec<usize>)> = Vec::new();
            for (i, (t, op, v)) in parsed.iter().enumerate() {
                if !STATS.contains(t) {
                    continue;
                }
                match groups.iter_mut().find(|(k, _)| *k == (*op, *v)) {
                    Some((_, idxs)) => idxs.push(i),
                    None => groups.push(((*op, *v), vec![i])),
                }
            }
            for ((op, value), mut idxs) in groups {
                idxs.sort_by_key(|&i| STATS.iter().position(|s| *s == parsed[i].0));
                idxs.dedup_by_key(|i| parsed[*i].0);
                let names = if idxs.len() == STATS.len() {
                    Self::str("cond.base_stat.all").unwrap_or_default()
                } else {
                    idxs.iter().filter_map(|&i| Self::str(&format!("cond.base_stat.{}", parsed[i].0))).collect()
                };
                let sym = match op { ">=" => "≥", "<=" => "≤", ">" => "﹥", "<" => "﹤", "!=" => "≠", _ => "=" };
                let text = format!("{names}{sym}{value}");
                let first = *idxs.iter().min().unwrap();
                let others: Vec<usize> = idxs.iter().copied().filter(|&i| i != first).collect();
                merged.push((first, others.first().copied().unwrap_or(usize::MAX), text));
                for &o in others.iter().skip(1) {
                    merged.push((usize::MAX, o, String::new()));
                }
            }
        }
        for (lo, (token, op, _)) in parsed.iter().enumerate() {
            if *op != ">=" || merged.iter().any(|(a, b, _)| *a == lo || *b == lo) {
                continue;
            }
            let Some(hi) = parsed.iter().position(|(t, op, _)| t == token && *op == "<=") else {
                continue;
            };
            let key = if *token == "order_rate" { "cond.order_rate.range".to_string() } else { format!("cond.template.{token}.range") };
            let range_sep = Self::str("sep.range").unwrap_or_else(|| "~".into());
            let range = format!("{}{range_sep}{}", parsed[lo].2, parsed[hi].2);
            if let Some(text) = Self::data_fmt(&key, &range) {
                merged.push((lo.min(hi), lo.max(hi), text));
            }
        }
        // (是否時間類, 文字)；時間類（比賽進行到哪：階段、進度%、剩餘距離、經過秒數）排最前面並另外上色，
        // 其他照原本順序。例：「後期或更晚、最後彎道過半之後」而不是反過來（有些賽道最後彎道不在後期）
        let mut out: Vec<(bool, String)> = Vec::with_capacity(atoms.len());
        for (i, atom) in atoms.iter().enumerate() {
            let timing = Self::is_timing_token(parsed[i].0);
            if let Some((_, _, text)) = merged.iter().find(|(first, _, _)| *first == i) {
                out.push((timing, text.clone()));
            }
            else if !merged.iter().any(|(_, second, _)| *second == i) {
                out.push((timing, Self::format_data_atom(atom)));
            }
        }
        out.sort_by_key(|(timing, _)| !*timing);
        out.into_iter()
            .map(|(timing, text)| match timing.then(|| Self::data_fmt("cond.time_wrap", &text)).flatten() {
                Some(wrapped) => wrapped,
                None => text
            })
            .collect::<Vec<_>>()
            .join(and_sep)
    }

    /// 描述「比賽進行到什麼時候」的條件：階段（phase*）、進度百分比（distance_rate*）、剩餘距離、經過秒數。
    /// 「被堵≥2秒」這類持續時間、「正在最終衝刺中」這類狀態不算。
    fn is_timing_token(token: &str) -> bool {
        token.starts_with("phase")
            || token.starts_with("distance_rate")
            || token.starts_with("remain_distance")
            || token == "accumulatetime"
    }

    /// 限制標籤裡的單項：套 locale `group.tag_strip`（以 | 分隔；`A` = 拿掉、`A=B` = 換成 B），
    /// zh-tw 例：「在中山賽場」→「中山」、「中距離」→「中距」。處理完變空就用原文。
    fn format_tag_atom(atom: &str) -> String {
        let text = Self::format_data_atom(atom);
        let Some(strip) = Self::str("group.tag_strip") else {
            return text;
        };
        let short = strip.split('|').filter(|w| !w.is_empty()).fold(text.clone(), |t, rule| match rule.split_once('=') {
            Some((from, to)) => t.replace(from, to),
            None => t.replace(rule, ""),
        });
        if short.is_empty() { text } else { short }
    }

    /// 合成的條件 token：「最後彎道過半之後」＝ `is_finalcorner_laterhalf==1`（最後彎道後半）
    /// 或 `is_finalcorner==1&corner==0`（進入過最後彎道、且已不在彎道上＝最後彎道之後）。
    /// 遊戲資料一律用這兩個分支成對表達原文的「最後彎道過半之後」（台服 10 組、後者從不單獨出現）。
    const LATERHALF_AFTER: &'static str = "hachimi_finalcorner_laterhalf_after==1";

    /// 兩個分支除了「最後彎道後半」／「最後彎道之後」以外都一樣時，合併成一個分支＋合成 token。
    /// locale 沒有 `cond.final_corner.laterhalf_after` 就不合併（照原樣顯示兩個分支）。
    fn merge_laterhalf_after(condition: &str) -> Option<String> {
        Self::str("cond.final_corner.laterhalf_after")?;
        let groups: Vec<Vec<&str>> = condition.split('@').map(|g| g.split('&').collect()).collect();
        let is_lh = |g: &Vec<&str>| g.contains(&"is_finalcorner_laterhalf==1");
        let is_fc0 = |g: &Vec<&str>| g.contains(&"is_finalcorner==1") && g.contains(&"corner==0");
        let a = groups.iter().position(|g| is_lh(g))?;
        let b = groups.iter().position(|g| is_fc0(g))?;
        let rest_a: Vec<&str> = groups[a].iter().copied().filter(|x| *x != "is_finalcorner_laterhalf==1").collect();
        let rest_b: Vec<&str> = groups[b].iter().copied().filter(|x| *x != "is_finalcorner==1" && *x != "corner==0").collect();
        let (mut sa, mut sb) = (rest_a.clone(), rest_b.clone());
        sa.sort();
        sb.sort();
        if sa != sb {
            return None;
        }
        let merged: Vec<&str> = std::iter::once(Self::LATERHALF_AFTER).chain(rest_a.iter().copied()).collect();
        let out: Vec<String> = groups.iter().enumerate()
            .filter(|(i, _)| *i != b)
            .map(|(i, g)| if i == a { merged.join("&") } else { g.join("&") })
            .collect();
        Some(out.join("@"))
    }

    fn format_data_conditions(condition: &str) -> String {
        if let Some(merged) = Self::merge_laterhalf_after(condition) {
            return Self::format_data_conditions(&merged);
        }
        let or_sep = Self::str("sep.or").unwrap_or_default();
        let and_sep = Self::str("sep.and").unwrap_or_default();
        let fmt_and = |atoms: &[&str]| Self::format_and_group(atoms, &and_sep);

        let groups: Vec<Vec<&str>> = condition.split('@').map(|g| g.split('&').collect()).collect();

        // 各 OR 分支共有的條件抽出來，只寫一次：
        //   A&B&C / D&B&C → (A / D) & B & C；若某分支就只剩共同條件（X / X&Y）→ 整體等於 X
        if groups.len() > 1 {
            let common: Vec<&str> = groups[0].iter().copied()
                .filter(|a| groups[1..].iter().all(|g| g.contains(a)))
                .collect();
            if !common.is_empty() {
                let diffs: Vec<Vec<&str>> = groups.iter()
                    .map(|g| g.iter().copied().filter(|a| !common.contains(a)).collect())
                    .collect();
                if diffs.iter().any(|d| d.is_empty()) {
                    return fmt_and(&common);
                }
                let open = Self::str("sep.group_open").unwrap_or_else(|| "(".into());
                let close = Self::str("sep.group_close").unwrap_or_else(|| ")".into());
                let alts = diffs.iter().map(|d| fmt_and(d)).collect::<Vec<_>>().join(&or_sep);
                // 共同條件裡的時間類放在括號前面（時間先講），其餘放後面
                let (timing, rest): (Vec<&str>, Vec<&str>) = common.iter()
                    .partition(|a| Self::is_timing_token(Self::parse_atom(a).0));
                let mut out = String::new();
                if !timing.is_empty() {
                    out.push_str(&fmt_and(&timing));
                    out.push_str(&and_sep);
                }
                out.push_str(&format!("{open}{alts}{close}"));
                if !rest.is_empty() {
                    out.push_str(&and_sep);
                    out.push_str(&fmt_and(&rest));
                }
                return out;
            }
        }

        groups.iter().map(|g| fmt_and(g)).collect::<Vec<_>>().join(&or_sep)
    }

    /// zh-tw 時用遊戲資料庫的技能名稱（跟遊戲顯示一致）；其他語系沿用 locale 的名稱。
    fn db_skill_name(id: i32) -> Option<String> {
        if !locale().starts_with("zh-tw") {
            return None;
        }
        SKILL_NAMES.get()?.get(&id).filter(|n| !n.is_empty()).cloned()
    }

    fn format_data_atom(atom: &str) -> String {
        let bytes = atom.as_bytes();
        let mut token_end = 0;
        while token_end < bytes.len() && (bytes[token_end].is_ascii_lowercase() || bytes[token_end] == b'_' || bytes[token_end].is_ascii_digit()) {
            token_end += 1;
        }
        let op_start = token_end;
        let mut op_end = op_start;
        while op_end < bytes.len() && (bytes[op_end] == b'=' || bytes[op_end] == b'!' || bytes[op_end] == b'<' || bytes[op_end] == b'>') {
            op_end += 1;
        }
        let token = &atom[..token_end];
        let op = &atom[op_start..op_end];
        let value = atom[op_end..].parse::<i32>().unwrap_or(0);

        if token == "order_rate" {
            let text = match op {
                ">" => Self::data_fmt("cond.order_rate.gt", &value.to_string()),
                ">=" => Self::data_fmt("cond.order_rate.ge", &value.to_string()),
                "<=" => Self::data_fmt("cond.order_rate.le", &value.to_string()),
                "<" => Self::data_fmt("cond.order_rate.lt", &value.to_string()),
                _ => None
            };
            if let Some(text) = text {
                return text;
            }
        }

        if token == "corner" {
            let text = match (op, value) {
                ("==", 0) => Self::str("cond.corner.straight"),
                ("==", _) => Self::data_fmt("cond.corner.corner", &value.to_string()),
                ("!=", 0) => Self::str("cond.corner.any"),
                ("!=", _) => Self::data_fmt("cond.corner.not", &value.to_string()),
                _ => None
            };
            if let Some(text) = text {
                return text;
            }
        }

        if token == "phase" && matches!(op, "==" | "!=" | "<=" | ">=") {
            // 特定組合的慣用說法（例：phase>=2 就是玩家說的「後期」，含最後那段；不是「後期或更晚」）
            if let Some(text) = Self::str(&format!("cond.phase.special.{}_{value}", Self::op_tag(op))) {
                return text;
            }
            if let Some(name) = Self::str(&format!("cond.phase.name.{value}")) {
                return match op {
                    "==" => name,
                    "!=" => Self::data_fmt("cond.negate", &name).unwrap_or_default(),
                    "<=" => Self::data_fmt("cond.phase.le", &name).unwrap_or_default(),
                    _ => Self::data_fmt("cond.phase.ge", &name).unwrap_or_default()
                };
            }
        }

        if token == "ground_condition" && matches!(op, "==" | "!=" | "<=" | ">=") {
            if let Some(name) = Self::str(&format!("cond.ground_condition.name.{value}")) {
                if let Some(text) = Self::data_fmt(&format!("cond.ground_condition.{}", Self::op_tag(op)), &name) {
                    return text;
                }
            }
        }

        if atom == Self::LATERHALF_AFTER {
            if let Some(text) = Self::str("cond.final_corner.laterhalf_after") {
                return text;
            }
        }

        if token == "track_id" {
            if op == "<=" && value == 10010 {
                if let Some(text) = Self::str("cond.track_id.jra") {
                    return text;
                }
            }
            if op == ">=" && value == 10001 {
                if let Some(text) = Self::str("cond.track_id.any") {
                    return text;
                }
            }
            if op == "==" || op == "!=" {
                if let Some(name) = Self::str(&format!("cond.track_name.{value}")) {
                    let key = if op == "==" { "cond.track_id.at" } else { "cond.track_id.not_at" };
                    if let Some(text) = Self::data_fmt(key, &name) {
                        return text;
                    }
                }
            }
        }

        if token == "same_skill_horse_count" && op == "==" {
            let text = if value == 1 {
                Self::str("cond.same_skill_horse_count.unique")
            } else {
                Self::data_fmt("cond.same_skill_horse_count.count", &value.to_string())
            };
            if let Some(text) = text {
                return text;
            }
        }

        if token == "near_infront_count" && op == "==" {
            let text = if value == 0 {
                Self::str("cond.near_infront_count.none")
            } else {
                Self::data_fmt("cond.near_infront_count.count", &value.to_string())
            };
            if let Some(text) = text {
                return text;
            }
        }

        if let Some(text) = Self::data_condition_enum(token, op, value) {
            return text;
        }

        if op == "!=" {
            if let Some(text) = Self::data_condition_enum(token, "==", value) {
                if let Some(negated) = Self::data_fmt("cond.negate", &text) {
                    return negated;
                }
            }
        }

        if let Some(text) = Self::data_condition_fixed(token, op) {
            return text;
        }

        if token == "distance_diff_top_float" && op == "<=" {
            if let Some(text) = Self::data_fmt("cond.template.distance_diff_top_float.le", &Self::format_data_number(value, 10, 1)) {
                return text;
            }
        }

        if let Some(text) = Self::data_condition_template(token, op, value) {
            return text;
        }

        if token == "furlong" && op == "==" {
            if let Some(text) = Self::data_fmt("cond.furlong", &(value + 1).to_string()) {
                return text;
            }
        }

        if token == "is_used_skill_id" && op == "==" {
            if let Some(text) = Self::db_skill_name(value).and_then(|n| Self::data_fmt("cond.used_skill.template", &n)) {
                return text;
            }
            if let Some(text) = Self::str(&format!("cond.used_skill.{value}")) {
                return text;
            }
            if let Some(text) = Self::data_fmt("cond.used_skill.template", &value.to_string()) {
                return text;
            }
        }

        if token == "is_used_skill_id_with_detail_one" && op == "==" {
            if let Some(text) = Self::db_skill_name(value).and_then(|n| Self::data_fmt("cond.used_skill_detail_one.template", &n)) {
                return text;
            }
            if let Some(text) = Self::str(&format!("cond.used_skill_detail_one.{value}")) {
                return text;
            }
            if let Some(text) = Self::data_fmt("cond.used_skill_detail_one.template", &value.to_string()) {
                return text;
            }
        }

        if token == "is_popularity_top_character_activate_advantage_skill" && op == "==" {
            if value == -1 {
                if let Some(text) = Self::str("cond.popularity_top.any") {
                    return text;
                }
            }
            if let Some(text) = Self::data_fmt("cond.popularity_top.count", &value.to_string()) {
                return text;
            }
        }

        format!("{token} {op} {value}")
    }

    fn data_condition_enum(token: &str, op: &str, value: i32) -> Option<String> {
        Self::str(&format!("cond.enum.{token}.{}.{}", Self::op_tag(op), value))
    }

    fn data_condition_fixed(token: &str, op: &str) -> Option<String> {
        Self::str(&format!("cond.fixed.{token}.{}", Self::op_tag(op)))
    }

    fn data_condition_template(token: &str, op: &str, value: i32) -> Option<String> {
        Self::str(&format!("cond.template.{token}.{}", Self::op_tag(op)))
            .map(|text| text.replace("%{v}", &value.to_string()))
    }

    /// 回傳這組拆好的各部分；怎麼組成一行由 [`Self::format_data_desc`] 依兩組的關係決定。
    fn format_data_group(condition: &str, precondition: &str, ability_time: i32, cooldown_time: i32, slots: &[SkillDataDescSlot]) -> Option<GroupParts> {
        let mut effects: Vec<String> = Vec::new();
        for slot in slots {
            if slot.ability_type == 0 && slot.ability_value == 0 {
                continue;
            }
            if let Some(effect) = Self::format_effect(*slot) {
                effects.push(effect);
            }
        }
        if effects.is_empty() {
            return None;
        }

        let first_type = slots.first().map(|s| s.ability_type).unwrap_or(0);
        let first_value = slots.first().map(|s| s.ability_value).unwrap_or(0);
        let time_suffix = if ability_time > 0 {
            Self::data_fmt("group.duration", &Self::format_data_number(ability_time, 10000, 2))
        } else if ability_time == 0 {
            Self::str("group.immediate")
        } else if first_type == 21 && first_value < 0 {
            Self::str("group.long_negative")
        } else {
            Self::str("group.indefinite")
        }.unwrap_or_default();

        let cd = if cooldown_time > 0 && cooldown_time < 5000000 {
            Self::data_fmt("group.cd", &format!("{:.1}", cooldown_time as f64 / 10000.0)).unwrap_or_default()
        } else {
            String::new()
        };
        let mut line = String::new();
        let (condition, cond_restrictions) = Self::split_hard_conditions(condition);
        let (precondition, pre_restrictions) = Self::split_hard_conditions(precondition);
        // 條件與附加條件的限制是「且」：兩邊都有時交叉組合（後追 × 中距／長距 → 後追、中距／後追、長距），
        // 不能直接串成一串選項（會變成「後追／中距／長距」三選一）
        let restrictions: Vec<Vec<String>> = match (cond_restrictions.is_empty(), pre_restrictions.is_empty()) {
            (false, false) => {
                let mut out: Vec<Vec<String>> = Vec::new();
                for c in &cond_restrictions {
                    for p in &pre_restrictions {
                        let mut label = c.clone();
                        label.extend(p.iter().filter(|a| !c.contains(a)).cloned());
                        if !out.contains(&label) {
                            out.push(label);
                        }
                    }
                }
                out
            }
            (false, true) => cond_restrictions,
            _ => pre_restrictions,
        };
        // 原本整句只有硬限制（拆完沒剩）→ 視同「隨時」，不再多印一次
        if !condition.is_empty() || restrictions.is_empty() {
            line.push_str(&Self::str("group.when").unwrap_or_default());
            line.push_str(&Self::format_data_conditions(if condition.is_empty() { "always==1" } else { &condition }));
        }
        if !precondition.is_empty() {
            line.push_str(&Self::str("group.after").unwrap_or_default());
            line.push_str(&Self::format_data_conditions(&precondition));
        }
        Some(GroupParts { restrictions, effects, time: time_suffix, cd, cond: line, cond_raw: (condition, precondition) })
    }

    /// 加強版比一般版「多出來的條件」（原始條件式）。兩邊都是單純 AND（或 OR 部分完全相同，只差 AND 項）
    /// 才比；一般版有、加強版沒有的條件若只是硬限制的反面（例：一般版的 `distance_type!=3`）就忽略。
    /// 情況複雜時回 None，呼叫端就印完整條件。
    fn extra_conditions<'a>(base: &'a str, v: &'a str) -> Option<Vec<&'a str>> {
        if base.contains('@') || v.contains('@') {
            return None;
        }
        let split = |c: &'a str| -> Vec<&'a str> { c.split('&').filter(|a| !a.is_empty()).collect() };
        let (b, x) = (split(base), split(v));
        let missing_ok = b.iter().filter(|a| !x.contains(a)).all(|a| {
            let (token, op, _) = Self::parse_atom(a);
            op == "!=" && Self::HARD_TOKENS.contains(&token)
        });
        missing_ok.then(|| x.into_iter().filter(|a| !b.contains(a)).collect())
    }

    /// 多個選項共同的開頭／結尾只寫一次：
    /// 「在札幌賽場／在函館賽場」→「在札幌／函館賽場」、「在東京賽場、短距離／在東京賽場、一哩」→「在東京賽場、短距離／一哩」。
    /// 抽完若有選項變空字串（例：「一哩／一哩、草地」）或選項本身還是組合，就不抽，照原樣接起來。
    fn compact_alternatives(labels: &[String], sep: &str) -> String {
        if labels.len() < 2 {
            return labels.join(sep);
        }
        if let Some(product) = Self::compact_product(labels, sep) {
            return product;
        }
        let chars: Vec<Vec<char>> = labels.iter().map(|l| l.chars().collect()).collect();
        let min_len = chars.iter().map(|c| c.len()).min().unwrap_or(0);
        let prefix = (0..min_len).take_while(|&i| chars.iter().all(|c| c[i] == chars[0][i])).count();
        let suffix = (0..min_len - prefix)
            .take_while(|&i| chars.iter().all(|c| c[c.len() - 1 - i] == chars[0][chars[0].len() - 1 - i]))
            .count();
        if prefix + suffix == 0 {
            return labels.join(sep);
        }
        let middles: Vec<String> = chars.iter().map(|c| c[prefix..c.len() - suffix].iter().collect()).collect();
        // 選項本身是組合（中間還有「、」）時抽了會切得亂七八糟（「中山賽場、中／阪神賽場、長…」），就不抽
        let and_sep = Self::str("sep.and").unwrap_or_default();
        if middles.iter().any(|m| m.is_empty() || (!and_sep.is_empty() && m.contains(and_sep.as_str()))) {
            return labels.join(sep);
        }
        let head: String = chars[0][..prefix].iter().collect();
        let tail: String = chars[0][chars[0].len() - suffix..].iter().collect();
        format!("{head}{}{tail}", middles.join(sep))
    }

    /// 選項剛好是兩類的所有組合時合併：「中山、中／中山、長／阪神、中／阪神、長」→「中山／阪神、中／長」。
    /// 每個選項都要恰好兩項、且 選項集合 = 第一項集合 × 第二項集合，否則回 None。
    fn compact_product(labels: &[String], sep: &str) -> Option<String> {
        let and_sep = Self::str("sep.and").unwrap_or_default();
        if and_sep.is_empty() {
            return None;
        }
        let pairs: Vec<(&str, &str)> = labels.iter().map(|l| l.split_once(and_sep.as_str())).collect::<Option<_>>()?;
        if pairs.iter().any(|(_, b)| b.contains(and_sep.as_str())) {
            return None;
        }
        let mut firsts: Vec<&str> = Vec::new();
        let mut seconds: Vec<&str> = Vec::new();
        for (a, b) in &pairs {
            if !firsts.contains(a) { firsts.push(a); }
            if !seconds.contains(b) { seconds.push(b); }
        }
        if firsts.len() < 2 || seconds.len() < 2 || firsts.len() * seconds.len() != pairs.len() {
            return None;
        }
        let all = firsts.iter().all(|a| seconds.iter().all(|b| pairs.contains(&(*a, *b))));
        all.then(|| format!("{}{and_sep}{}", firsts.join(sep), seconds.join(sep)))
    }

    /// v 的條件是否就是 base 的條件再多幾項（條件或附加條件其中一邊多、另一邊相同）；是的話回傳多出來那幾項的文字。
    fn situational_extra(base: &GroupParts, v: &GroupParts) -> Option<String> {
        let and_sep = Self::str("sep.and").unwrap_or_default();
        let (b, x) = (&base.cond_raw, &v.cond_raw);
        let extra = if b.1 == x.1 && b.0 != x.0 {
            Self::extra_conditions(&b.0, &x.0)?
        } else if b.0 == x.0 && b.1 != x.1 {
            Self::extra_conditions(&b.1, &x.1)?
        } else {
            return None;
        };
        if extra.is_empty() {
            return None;
        }
        Some(Self::format_and_group(&extra, &and_sep))
    }

    fn join_effects(effects: &[String]) -> String {
        effects.join(&Self::str("sep.effect").unwrap_or_else(|| ", ".into()))
    }

    /// 完整一行：效果 時間（冷卻） 條件
    fn render_group(g: &GroupParts) -> String {
        format!("<b>{} {}</b>{}{}", Self::join_effects(&g.effects), g.time, g.cd, g.cond)
    }

    /// 加強版那一行只寫跟一般版不同的部分：
    /// - 效果：一般版的效果它全都有 → 只列多出來的（「另加 …」）；否則整組列出（「改為 …」）；一樣就不列
    /// - 時間／冷卻、條件：跟一般版一樣就省略
    fn render_variant_diff(base: &GroupParts, v: &GroupParts, skip_cond: bool) -> String {
        // 條件跟一般版比不出「只多幾項」（例：接在條件1之後才發動的另一段效果）→ 它是獨立的效果，
        // 不是一般版的加強／替換，整行完整寫出、不加「另加／改為」
        let independent = [(&base.cond_raw.0, &v.cond_raw.0), (&base.cond_raw.1, &v.cond_raw.1)]
            .iter()
            .any(|(b, x)| b != x && !x.is_empty() && Self::extra_conditions(b, x).is_none());
        if independent {
            return Self::render_group(v);
        }

        let mut out = String::new();
        if v.effects != base.effects {
            let extra: Vec<String> = v.effects.iter().filter(|e| !base.effects.contains(e)).cloned().collect();
            let (prefix, shown) = if base.effects.iter().all(|e| v.effects.contains(e)) {
                (Self::str("group.extra").unwrap_or_default(), extra)
            } else {
                (Self::str("group.replace").unwrap_or_default(), v.effects.clone())
            };
            out.push_str(&prefix);
            out.push_str(&format!("<b>{}</b>", Self::join_effects(&shown)));
        }
        if v.time != base.time || v.cd != base.cd {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&format!("<b>{}</b>{}", v.time, v.cd));
        }
        if v.cond != base.cond && !skip_cond {
            let and_sep = Self::str("sep.and").unwrap_or_default();
            // 條件、附加條件各自比：一樣就省略，只多幾項就寫「另需：」，比不出來才寫完整
            for (b, x, full_key, extra_key) in [
                (&base.cond_raw.0, &v.cond_raw.0, "group.when", "group.also_when"),
                (&base.cond_raw.1, &v.cond_raw.1, "group.after", "group.also_after"),
            ] {
                if b == x {
                    continue;
                }
                match Self::extra_conditions(b, x) {
                    Some(extra) if extra.is_empty() => {}
                    Some(extra) => {
                        // 一般版這一段根本是空的（例：一般版沒有前提）→ 不是「另需」，直接寫「前提：」
                        let key = if b.is_empty() { full_key } else { extra_key };
                        out.push_str(&Self::str(key).unwrap_or_default());
                        out.push_str(&Self::format_and_group(&extra, &and_sep));
                    }
                    None if x.is_empty() => {}
                    None => {
                        out.push_str(&Self::str(full_key).unwrap_or_default());
                        out.push_str(&Self::format_data_conditions(x));
                    }
                }
            }
        }
        if out.is_empty() {
            // 理論上不會發生（兩組完全一樣）；保險起見印完整一行
            return Self::render_group(v);
        }
        out
    }

    /// 限制（選項 × 項目）→ 標籤文字：項目用「、」接，選項用「／」接並抽共同開頭結尾。
    fn restriction_label(alts: &[Vec<String>]) -> String {
        let and_sep = Self::str("sep.and").unwrap_or_default();
        let or_sep = Self::str("sep.or").unwrap_or_default();
        let labels: Vec<String> = alts.iter().map(|a| a.join(&and_sep)).collect();
        Self::compact_alternatives(&labels, &or_sep)
    }

    /// 兩組效果都有的限制項目（出現在兩組每一個選項裡的）。
    fn common_restrictions(a: &[Vec<String>], b: &[Vec<String>]) -> Vec<String> {
        // 任一組沒有限制就沒有「共同」可言（否則對空清單 all() 恆真，會把另一組的限制誤當成共同的）
        if a.is_empty() || b.is_empty() {
            return Vec::new();
        }
        let first = &a[0];
        first.iter()
            .filter(|atom| a.iter().chain(b.iter()).all(|alt| alt.contains(atom)))
            .cloned()
            .collect()
    }

    /// 拿掉共同限制後剩下的；任一選項被拿空（＝這組除了共同限制外沒有別的限制）就視為沒有剩。
    fn residual_restrictions(alts: &[Vec<String>], common: &[String]) -> Vec<Vec<String>> {
        let mut out: Vec<Vec<String>> = Vec::new();
        for alt in alts {
            let rest: Vec<String> = alt.iter().filter(|a| !common.contains(a)).cloned().collect();
            if rest.is_empty() {
                return Vec::new();
            }
            if !out.contains(&rest) {
                out.push(rest);
            }
        }
        out
    }

    fn format_data_desc(row: &SkillDataDescRow) -> String {
        let group1 = Self::format_data_group(&row.condition_1, &row.precondition_1, row.ability_time_1, row.cooldown_time_1, &row.slots[0..3]);
        let group2 = Self::format_data_group(&row.condition_2, &row.precondition_2, row.ability_time_2, row.cooldown_time_2, &row.slots[3..6]);

        let tag = |alts: &[Vec<String>]| -> String {
            if alts.is_empty() {
                return String::new();
            }
            Self::data_fmt("group.restriction", &Self::restriction_label(alts)).unwrap_or_default()
        };
        // 加強版接在一般效果後面的分隔（zh-tw「；」＝同一行接下去，省掉換行留下的空白；沒設定就換行）
        let variant_join = Self::str("group.variant_join").unwrap_or_else(|| "\n".into());

        match (group1, group2) {
            (Some(g1), Some(g2)) => {
                // 兩組共同的限制（例：兩組都要「後追」）抽出來，放在整段最前面
                let common = Self::common_restrictions(&g1.restrictions, &g2.restrictions);
                let r1 = Self::residual_restrictions(&g1.restrictions, &common);
                let r2 = Self::residual_restrictions(&g2.restrictions, &common);
                let head = if common.is_empty() { String::new() } else { tag(&[common.clone()]) };

                // 抽完只剩一組帶限制：那組是「特定情況下的加強版」（例：一般效果＋在中山賽場時另有加成）。
                // 一般效果排前面，加強版標「中山時：」，而且只寫跟一般版不同的部分。
                let variant = |base: &GroupParts, v: &GroupParts, rv: &[Vec<String>]| -> String {
                    match Self::data_fmt("group.variant", &Self::restriction_label(rv)) {
                        Some(label) => format!("{label}{}", Self::render_variant_diff(base, v, false)),
                        None => format!("{}{}", tag(rv), Self::render_group(v))
                    }
                };
                match (r1.is_empty(), r2.is_empty()) {
                    (true, false) => format!("{head}{}{variant_join}{}", Self::render_group(&g1), variant(&g1, &g2, &r2)),
                    (false, true) => format!("{head}{}{variant_join}{}", Self::render_group(&g2), variant(&g2, &g1, &r1)),
                    (true, true) => {
                        // 兩組限制相同，但一組條件只是另一組多幾項（例：多「距第1名≤5米」）→ 多的那幾項就是
                        // 觸發加強版的情境：一般效果在前，「距第1名≤5米時：改為 …」
                        let situational = |base: &GroupParts, v: &GroupParts| -> Option<String> {
                            let extra = Self::situational_extra(base, v)?;
                            let label = Self::data_fmt("group.variant", &extra)?;
                            Some(format!("{head}{}{variant_join}{label}{}", Self::render_group(base), Self::render_variant_diff(base, v, true)))
                        };
                        situational(&g2, &g1)
                            .or_else(|| situational(&g1, &g2))
                            .unwrap_or_else(|| format!("{head}{}{variant_join}{}", Self::render_group(&g1), Self::render_group(&g2)))
                    }
                    // 兩組各有不同限制（互斥的兩個版本）：各自完整標示
                    (false, false) => format!("{}{}{variant_join}{}{}",
                        tag(&g1.restrictions), Self::render_group(&g1), tag(&g2.restrictions), Self::render_group(&g2))
                }
            }
            (Some(g), None) | (None, Some(g)) => format!("{}{}", tag(&g.restrictions), Self::render_group(&g)),
            (None, None) => String::new()
        }
    }
}

// text_data：只接 category 48（技能說明）
#[derive(Default)]
pub struct TextDataQuery {
    // SELECT
    text: Column,

    // WHERE
    category: Column,
    index: Column
}

impl TextDataQuery {
    fn get_skill_desc(index: i32) -> Option<*mut Il2CppString> {
        if !Hachimi::instance().config.load().skill_data_desc {
            return None;
        }
        skill_data_desc()?.get_desc(index).map(|desc| desc.to_il2cpp_string())
    }
}

impl SelectQueryState for TextDataQuery {
    fn add_column(&mut self, idx: i32, name: &str) {
        if name == "text" {
            self.text.select_idx = Some(idx)
        }
    }

    fn add_param(&mut self, idx: i32, name: &str) {
        match name {
            "category" => self.category.param_idx = Some(idx),
            "index" => self.index.param_idx = Some(idx),
            _ => ()
        }
    }

    fn bind_int(&mut self, idx: i32, value: i32) {
        self.category.try_bind_int(idx, value);
        self.index.try_bind_int(idx, value);
    }

    fn get_text(&self, _query: *mut Il2CppObject, idx: i32) -> Option<*mut Il2CppString> {
        if !self.text.is_select_idx(idx) {
            return None;
        }
        match (self.category.int_value, self.index.int_value) {
            (Some(48), Some(index)) => Self::get_skill_desc(index),
            _ => None
        }
    }
}

pub trait SelectExt {
    fn get_first_table_name(&self) -> Option<&String>;
}

impl SelectExt for ast::Select {
    fn get_first_table_name(&self) -> Option<&String> {
        if let Some(table_with_joins) = self.from.get(0) {
            if let ast::TableFactor::Table { name: object_name, .. } = &table_with_joins.relation {
                if let Some(ident) = object_name.0.get(0) {
                    return Some(&ident.value);
                }
            }
        }

        None
    }
}

pub trait SelectItemExt {
    fn get_unnamed_expr_ident(&self) -> Option<&String>;
}

impl SelectItemExt for ast::SelectItem {
    fn get_unnamed_expr_ident(&self) -> Option<&String> {
        if let ast::SelectItem::UnnamedExpr(expr) = self {
            return expr.get_ident_value();
        }

        None
    }
}

pub trait ExprExt {
    fn binary_op_iter<'a>(&'a self) -> BinaryOpIter<'a>;
    fn get_ident_value(&self) -> Option<&String>;
    fn is_placeholder_value(&self) -> bool;
}

impl ExprExt for ast::Expr {
    fn binary_op_iter<'a>(&'a self) -> BinaryOpIter<'a> {
        BinaryOpIter { stack: vec![self] }
    }

    fn get_ident_value(&self) -> Option<&String> {
        if let ast::Expr::Identifier(ident) = self {
            return Some(&ident.value);
        }

        None
    }

    fn is_placeholder_value(&self) -> bool {
        if let ast::Expr::Value(value) = self {
            if let ast::Value::Placeholder(_) = value {
                return true;
            }
        }

        false
    }
}

pub struct BinaryOpIter<'a> {
    stack: Vec<&'a ast::Expr>
}

pub struct BinaryOpRef<'a> {
    pub left: &'a Box<ast::Expr>,
    pub op: &'a ast::BinaryOperator,
    pub right: &'a Box<ast::Expr>
}

impl<'a> Iterator for BinaryOpIter<'a> {
    type Item = BinaryOpRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let Some(expr) = self.stack.pop() else {
                return None;
            };

            let ast::Expr::BinaryOp { left, op, right } = expr else {
                continue;
            };

            self.stack.push(right);
            self.stack.push(left); // left will be pop'd first

            return Some(BinaryOpRef { left, op, right })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用台服 master.mdb 匯出的 skill_data（JSON）跑一遍格式化：數有幾筆還殘留原始條件式
    /// （= 沒有對應的說明字串），並印幾筆樣本對照遊戲原文。沒有匯出檔就跳過。
    /// 匯出：HACHIMI_SKILL_JSON=<檔> ，內容 {"rows": [[id, precondition_1, ...同 load_from_db 的 SELECT 順序]], "names": {...}, "descs": {...}}
    #[test]
    fn formats_tw_skill_data() {
        let Ok(path) = std::env::var("HACHIMI_SKILL_JSON") else { eprintln!("skip: HACHIMI_SKILL_JSON not set"); return; };
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        rust_i18n::set_locale("zh-tw");
        let _ = SKILL_NAMES.set(json["names"].as_object().unwrap().iter()
            .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_string()))).collect());

        let int = |v: &serde_json::Value| v.as_i64().unwrap_or(0) as i32;
        let text = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
        let slot = |r: &[serde_json::Value], b: usize| SkillDataDescSlot {
            ability_type: int(&r[b]), ability_value_usage: int(&r[b + 1]), additional_activate_type: int(&r[b + 2]),
            ability_value: int(&r[b + 3]), target_type: int(&r[b + 4]), target_value: int(&r[b + 5]),
        };
        let raw_atom = regex_lite_like;

        let rows = json["rows"].as_array().unwrap();
        let (mut raw, mut empty) = (Vec::new(), 0usize);
        let mut samples = Vec::new();
        let mut dump = String::new();
        let (mut tagged, mut hard_left) = (0usize, Vec::new());
        let (mut variants, mut variant_full) = (0usize, Vec::<(i32, String)>::new());
        for r in rows {
            let r = r.as_array().unwrap();
            let row = SkillDataDescRow {
                id: int(&r[0]),
                precondition_1: text(&r[1]), condition_1: text(&r[2]), ability_time_1: int(&r[3]), cooldown_time_1: int(&r[4]),
                precondition_2: text(&r[23]), condition_2: text(&r[24]), ability_time_2: int(&r[25]), cooldown_time_2: int(&r[26]),
                slots: [slot(r, 5), slot(r, 11), slot(r, 17), slot(r, 27), slot(r, 33), slot(r, 39)],
            };
            let desc = SkillDataDesc::format_data_desc(&row);
            if desc.is_empty() { empty += 1; }
            dump.push_str(&format!("{}\t{}\t{}\n", row.id, json["names"][row.id.to_string()].as_str().unwrap_or(""), desc.replace('\n', "⏎")));
            if desc.contains("＜") || desc.contains("時：</color>") { tagged += 1; }
            for line in desc.split('\n').filter(|l| l.contains("時：</color>")) {
                variants += 1;
                if line.contains("條件</color>：") { variant_full.push((row.id, line.to_string())); }
            }
            // 各組「條件：」之後、到下一組限制標籤之前的文字
            let cond_part = desc.split("條件</color>：").skip(1)
                .map(|p| p.split("＜").next().unwrap_or("").split('\n').next().unwrap_or(""))
                .collect::<Vec<_>>().join(" ");
            if ["領頭", "前列", "居中", "後追", "短距離", "一哩", "中距離", "長距離", "草地", "沙地"].iter().any(|k| cond_part.contains(k)) {
                hard_left.push((row.id, desc.clone()));
            }
            if raw_atom(&desc) { raw.push((row.id, desc.clone())); }
            if [110321, 910321, 120041, 100131, 100281, 120671].contains(&row.id) { samples.push((row.id, desc)); }
        }
        for (id, d) in &samples {
            eprintln!("== {id} {} | 原文：{}\n{d}\n", json["names"][id.to_string()], json["descs"][id.to_string()]);
        }
        // HACHIMI_SKILL_DUMP=<檔>：把全部輸出寫成 TSV（id、名稱、說明，說明內換行記成 ⏎），改格式時可前後比對
        if let Ok(out) = std::env::var("HACHIMI_SKILL_DUMP") {
            std::fs::write(out, &dump).unwrap();
        }
        eprintln!("total {} / empty {} / 殘留原始條件式 {}", rows.len(), empty, raw.len());
        eprintln!("拆出硬限制 {tagged} / 硬限制仍留在條件裡 {}", hard_left.len());
        eprintln!("加強版 {variants} 行 / 仍印完整條件 {}", variant_full.len());
        for (id, l) in variant_full.iter().take(5) {
            eprintln!("  full {id} {}: {l}", json["names"][id.to_string()]);
        }
        for (id, d) in hard_left.iter().take(6) {
            eprintln!("  left {id} {}: {}", json["names"][id.to_string()], d.replace('\n', " ⏎ "));
        }
        for (id, d) in raw.iter().take(15) {
            eprintln!("  raw {id}: {}", d.replace('\n', " ⏎ "));
        }
    }

    /// 是否含 `foo_bar==3` 這類沒被翻成人話的原始條件式
    fn regex_lite_like(s: &str) -> bool {
        let b = s.as_bytes();
        (0..b.len()).any(|i| {
            let op = &b[i..];
            (op.starts_with(b"==") || op.starts_with(b">=") || op.starts_with(b"<=") || op.starts_with(b"!="))
                && i > 0 && (b[i - 1].is_ascii_lowercase() || b[i - 1] == b'_')
        })
    }
}
