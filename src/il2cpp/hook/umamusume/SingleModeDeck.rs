// Tier 2 訓練助手資料源：讀「當前這局養成」的育成角色 + 裝備的支援卡。
//
// 存取鏈（runtime dump 驗證過的型別圖）：
//   Singleton<WorkDataManager>._instance (static)
//     → WorkDataManager.get_SingleMode()          → WorkSingleModeData
//     → WorkSingleModeData.get_IsPlaying()         → bool（閘門！false=不在養成中，殘留資料無效）
//     → WorkSingleModeData.get_Character()         → WorkSingleModeCharaData
//     → get_CardId()/get_CharaId()                 → 育成卡/角色 id（getter 正常，沿用）
//     → get_EquipSupportCardArray()                → EquipSupportCard[]（6 個物件指標）
//
// 踩雷紀錄（實機定位）：
//   * WorkSingleModeData 是常駐單例、首頁也在且殘留上局資料 → 用 get_IsPlaying() 閘門。
//   * EquipSupportCard 陣列元素實測 klass 完全 match、指標合法，但呼叫其 property getter
//     （get_SupportCardId 等）會 native crash → 研判該類的小 getter 被 il2cpp inline 掉，
//     get_method_addr 拿到的是無效 stub。故 EquipSupportCard 的值改「直接讀 ObscuredInt 欄位」。
//   * CodeStage ObscuredInt = 兩個 int（key 與 加密值）做 XOR；XOR 可交換 →
//     欄位前 8 bytes 的 w0 ^ w1 即真值，連欄位順序都不用管。

use std::sync::Mutex;

use crate::il2cpp::{
    api::{il2cpp_array_element_size, il2cpp_array_length, il2cpp_class_is_valuetype, il2cpp_field_get_offset, il2cpp_object_get_class},
    symbols::{get_field_from_name, get_field_object_value, get_method_addr, SingletonLike, Thread},
    types::*,
};

static mut WORKDATAMANAGER_CLASS: *mut Il2CppClass = 0 as _;
static mut EQUIP_CLASS: *mut Il2CppClass = 0 as _;
static mut EQUIP_IS_VALUETYPE: bool = false;

// EquipSupportCard 的 ObscuredInt 欄位（直接讀，不走 getter）
static mut F_SUPPORTCARDID: *mut FieldInfo = 0 as _;
static mut F_POSITION: *mut FieldInfo = 0 as _;
static mut F_LIMITBREAK: *mut FieldInfo = 0 as _;

// WorkSingleModeCharaData._skillTipsList（當前已獲得的技能靈感）+ SkillTips 的 ObscuredInt 欄位
static mut F_SKILLTIPSLIST: *mut FieldInfo = 0 as _;
static mut F_TIP_GROUPID: *mut FieldInfo = 0 as _;
static mut F_TIP_RARITY: *mut FieldInfo = 0 as _;
static mut F_TIP_LEVEL: *mut FieldInfo = 0 as _;

static mut GET_SINGLEMODE_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_SingleMode, GET_SINGLEMODE_ADDR, *mut Il2CppObject, this: *mut Il2CppObject);

static mut GET_ISPLAYING_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_IsPlaying, GET_ISPLAYING_ADDR, bool, this: *mut Il2CppObject);

static mut GET_CHARACTER_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_Character, GET_CHARACTER_ADDR, *mut Il2CppObject, this: *mut Il2CppObject);

static mut GET_CARDID_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_CardId, GET_CARDID_ADDR, i32, this: *mut Il2CppObject);

static mut GET_CHARAID_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_CharaId, GET_CHARAID_ADDR, i32, this: *mut Il2CppObject);

static mut GET_EQUIP_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_EquipSupportCardArray, GET_EQUIP_ADDR, *mut Il2CppArray, this: *mut Il2CppObject);

#[derive(Clone, Debug)]
pub struct DeckCard {
    pub position: i32,
    pub support_card_id: i32,
    pub limit_break: i32,
}

/// 當前已獲得的技能靈感（來自訓練點擊或事件）。group_id = 技能家族 id。
#[derive(Clone, Debug)]
pub struct HintTip {
    pub group_id: i32,
    pub rarity: i32,
    pub level: i32,
}

#[derive(Clone, Debug)]
pub struct DeckInfo {
    pub chara_card_id: i32,
    pub chara_id: i32,
    pub cards: Vec<DeckCard>,
    pub hints: Vec<HintTip>,
}

static DECK: Mutex<Option<DeckInfo>> = Mutex::new(None);

/// 讀 obscured int 欄位（前 8 bytes 兩個 int 做 XOR = 真值）。回傳 (w0, w1, value)。
unsafe fn read_obscured_int(obj: *mut Il2CppObject, field: *mut FieldInfo) -> (i32, i32, i32) {
    if field.is_null() {
        return (0, 0, 0);
    }
    let off = il2cpp_field_get_offset(field);
    let base = (obj as *const u8).add(off);
    let w0 = *(base as *const i32);
    let w1 = *(base.add(4) as *const i32);
    (w0, w1, w0 ^ w1)
}

/// 讀 System.Collections.Generic.List<物件> 的元素指標。
/// List 記憶體佈局：_items(T[]) @+0x10、_size(int) @+0x18；陣列資料 @+32。
unsafe fn read_object_list(list: *mut Il2CppObject) -> Vec<*mut Il2CppObject> {
    let mut out = Vec::new();
    if list.is_null() {
        return out;
    }
    let items = *((list as *const u8).add(0x10) as *const *mut u8);
    let size = *((list as *const u8).add(0x18) as *const i32);
    if items.is_null() || size <= 0 {
        return out;
    }
    let data = items.add(32);
    for i in 0..(size.min(512) as usize) {
        out.push(*(data.add(i * 8) as *const *mut Il2CppObject));
    }
    out
}

/// 只能在 il2cpp 主執行緒呼叫。None = 未就緒 / 不在養成中。
fn read_current_deck() -> Option<DeckInfo> {
    let wdm_class = unsafe { WORKDATAMANAGER_CLASS };
    if wdm_class.is_null() {
        return None;
    }
    let wdm = SingletonLike::new(wdm_class)?.instance();
    if wdm.is_null() {
        return None;
    }

    let single_mode = get_SingleMode(wdm);
    if single_mode.is_null() {
        return None;
    }
    if !get_IsPlaying(single_mode) {
        info!("[TrainingHelper] IsPlaying=false（不在養成中），略過");
        return None;
    }

    let chara = get_Character(single_mode);
    if chara.is_null() {
        info!("[TrainingHelper] IsPlaying=true 但 Character=null");
        return None;
    }

    let chara_card_id = get_CardId(chara);
    let chara_id = get_CharaId(chara);
    info!("[TrainingHelper] chara_card={} chara={}", chara_card_id, chara_id);

    let mut cards = Vec::new();
    let arr = get_EquipSupportCardArray(chara);
    if !arr.is_null() {
        let count = unsafe { il2cpp_array_length(arr) } as usize;
        let arr_class = unsafe { il2cpp_object_get_class(arr as *mut Il2CppObject) };
        let esize = unsafe { il2cpp_array_element_size(arr_class) } as usize;
        let expect_class = unsafe { EQUIP_CLASS } as usize;
        info!("[TrainingHelper] equip count={} elem_size={}", count, esize);

        let data_ptr = unsafe { (arr as *mut u8).add(32) };
        let capped = count.min(12);
        for i in 0..capped {
            let this = unsafe { *(data_ptr.add(i * esize) as *const *mut Il2CppObject) };
            if this.is_null() {
                continue;
            }
            // 合法性檢查：物件首欄位 = klass，須 match EquipSupportCard
            let klass = unsafe { *(this as *const usize) };
            if klass != expect_class {
                info!("[TrainingHelper] elem[{}] klass mismatch, skip", i);
                continue;
            }

            let (s0, s1, sid) = unsafe { read_obscured_int(this, F_SUPPORTCARDID) };
            let (_, _, pos) = unsafe { read_obscured_int(this, F_POSITION) };
            let (_, _, lb) = unsafe { read_obscured_int(this, F_LIMITBREAK) };
            info!("[TrainingHelper] elem[{}] sid_raw=({},{}) sid={} pos={} lb={}", i, s0, s1, sid, pos, lb);

            if sid == 0 {
                continue;
            }
            cards.push(DeckCard { position: pos, support_card_id: sid, limit_break: lb });
        }
        cards.sort_by_key(|c| c.position);
    }

    // 當前已獲得的技能靈感（_skillTipsList）
    let mut hints = Vec::new();
    unsafe {
        let list = get_field_object_value::<Il2CppObject>(chara, F_SKILLTIPSLIST);
        for tip in read_object_list(list) {
            if tip.is_null() {
                continue;
            }
            let (_, _, gid) = read_obscured_int(tip, F_TIP_GROUPID);
            let (_, _, rarity) = read_obscured_int(tip, F_TIP_RARITY);
            let (_, _, level) = read_obscured_int(tip, F_TIP_LEVEL);
            info!("[TrainingHelper] hint gid={} rarity={} level={}", gid, rarity, level);
            hints.push(HintTip { group_id: gid, rarity, level });
        }
    }
    info!("[TrainingHelper] hints_total={}", hints.len());

    Some(DeckInfo { chara_card_id, chara_id, cards, hints })
}

/// 排到主執行緒讀取一次並更新快取。可從任意執行緒（含 GUI render thread）安全呼叫。
pub fn refresh() {
    Thread::main_thread().schedule(|| {
        let deck = read_current_deck();
        match &deck {
            Some(d) => info!(
                "[TrainingHelper] deck OK: chara_card={} support_cards={:?}",
                d.chara_card_id,
                d.cards.iter().map(|c| c.support_card_id).collect::<Vec<_>>()
            ),
            None => info!("[TrainingHelper] deck=None"),
        }
        *DECK.lock().unwrap() = deck;
    });
}

/// GUI 讀取用（clone 出快取）。
pub fn cached() -> Option<DeckInfo> {
    DECK.lock().unwrap().clone()
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, WorkDataManager);
    get_class_or_return!(umamusume, Gallop, WorkSingleModeData);
    get_class_or_return!(umamusume, Gallop, WorkSingleModeCharaData);
    find_nested_class_or_return!(WorkSingleModeCharaData, EquipSupportCard);
    find_nested_class_or_return!(WorkSingleModeCharaData, SkillTips);

    let is_vt = unsafe { il2cpp_class_is_valuetype(EquipSupportCard) };

    unsafe {
        WORKDATAMANAGER_CLASS = WorkDataManager;
        EQUIP_CLASS = EquipSupportCard;
        EQUIP_IS_VALUETYPE = is_vt;

        F_SUPPORTCARDID = get_field_from_name(EquipSupportCard, c"<SupportCardId>k__BackingField");
        F_POSITION = get_field_from_name(EquipSupportCard, c"<Position>k__BackingField");
        F_LIMITBREAK = get_field_from_name(EquipSupportCard, c"<LimitBreakCount>k__BackingField");

        F_SKILLTIPSLIST = get_field_from_name(WorkSingleModeCharaData, c"_skillTipsList");
        F_TIP_GROUPID = get_field_from_name(SkillTips, c"<GroupId>k__BackingField");
        F_TIP_RARITY = get_field_from_name(SkillTips, c"<Rarity>k__BackingField");
        F_TIP_LEVEL = get_field_from_name(SkillTips, c"<Level>k__BackingField");
        info!(
            "[TrainingHelper] EquipSupportCard vt={} sid_off={} pos_off={} lb_off={}",
            is_vt,
            il2cpp_field_get_offset(F_SUPPORTCARDID),
            il2cpp_field_get_offset(F_POSITION),
            il2cpp_field_get_offset(F_LIMITBREAK)
        );

        GET_SINGLEMODE_ADDR = get_method_addr(WorkDataManager, c"get_SingleMode", 0);
        GET_ISPLAYING_ADDR = get_method_addr(WorkSingleModeData, c"get_IsPlaying", 0);
        GET_CHARACTER_ADDR = get_method_addr(WorkSingleModeData, c"get_Character", 0);
        GET_CARDID_ADDR = get_method_addr(WorkSingleModeCharaData, c"get_CardId", 0);
        GET_CHARAID_ADDR = get_method_addr(WorkSingleModeCharaData, c"get_CharaId", 0);
        GET_EQUIP_ADDR = get_method_addr(WorkSingleModeCharaData, c"get_EquipSupportCardArray", 0);
    }
}
