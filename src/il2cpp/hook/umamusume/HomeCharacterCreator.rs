//! 首頁站在畫面上的馬娘。用來餵 Discord Rich Presence 的角色圖片。
//!
//! `CreateAdditionalStandCharacter` 在首頁擺放每一隻角色時被呼叫，參數帶著站位與
//! `CreateInfo`（chara id + dress id）。首頁可能同時站多隻，所以誰是「代表馬娘」得看站位——
//! 但 `HomeDefine.StandPos` 的實際數值 dump 裡查不到（只有方法簽章、沒有 enum 值）。
//!
//! 所以先不猜：**後報的蓋掉先報的**，並在 debug log 留下每一次的站位與 id。
//! 看過實機 log 知道有幾隻、站位各是多少之後，再改成挑指定站位。
//! 這個 hook 也可能根本不涵蓋主要角色（方法名是 "Additional"，主角色走
//! `InitFooterCharacter` 那條 coroutine）——log 會告訴我們，備援是 static 的
//! `GetFooterCharaInfo()`。

use crate::il2cpp::{
    symbols::{find_nested_class, get_method_addr, Dictionary, Il2CppDictionary},
    types::*
};

static mut GETCHARAID_ADDR: usize = 0;
impl_addr_wrapper_fn!(CreateInfo_get_CharaId, GETCHARAID_ADDR, i32, this: *mut Il2CppObject);

static mut GETDRESSID_ADDR: usize = 0;
impl_addr_wrapper_fn!(CreateInfo_get_DressId, GETDRESSID_ADDR, i32, this: *mut Il2CppObject);

type CreateAdditionalStandCharacterFn = extern "C" fn(
    this: *mut Il2CppObject, stand_pos: i32, chara_info: *mut Il2CppObject
);
extern "C" fn CreateAdditionalStandCharacter(
    this: *mut Il2CppObject, stand_pos: i32, chara_info: *mut Il2CppObject
) {
    get_orig_fn!(CreateAdditionalStandCharacter, CreateAdditionalStandCharacterFn)(
        this, stand_pos, chara_info
    );

    if chara_info.is_null() || unsafe { GETCHARAID_ADDR } == 0 {
        return;
    }

    let chara_id = CreateInfo_get_CharaId(chara_info);
    let dress_id = CreateInfo_get_DressId(chara_info);
    debug!("Home stand character: standPos={} charaId={} dressId={}", stand_pos, chara_id, dress_id);

    crate::windows::discord::on_home_chara(chara_id, dress_id);
}

static mut GETFOOTERCHARAINFO_ADDR: usize = 0;

/// 讀首頁站位角色表。**必須在遊戲主執行緒上跑**（由 discord worker 排程進來），
/// 因為會直接呼叫 il2cpp 方法。
///
/// `CreateAdditionalStandCharacter` 實測不會為主要角色觸發（首頁進去後 log 一片空白），
/// 所以改讀這個 static 的表。站位數值未知，先取第一筆有效的並記進 log。
pub fn read_footer_chara() {
    let addr = unsafe { GETFOOTERCHARAINFO_ADDR };
    if addr == 0 || unsafe { GETCHARAID_ADDR } == 0 {
        return;
    }

    let get_footer_chara_info: extern "C" fn() -> *mut Il2CppDictionary =
        unsafe { std::mem::transmute(addr) };
    let raw = get_footer_chara_info();
    if raw.is_null() {
        return;
    }

    let dict: Dictionary<i32, *mut Il2CppObject> = raw.into();
    let entries = dict.entries();
    // .NET Dictionary 的 entries 陣列是容量，只有前 count 格填過；再多讀就是未初始化的槽。
    let used = (dict.count().max(0) as usize).min(entries.len());
    if used == 0 {
        return;
    }

    for entry in unsafe { &entries.as_slice()[..used] } {
        let chara_info = entry.value;
        if chara_info.is_null() {
            continue;
        }
        let chara_id = CreateInfo_get_CharaId(chara_info);
        let dress_id = CreateInfo_get_DressId(chara_info);
        debug!("Home footer character: standPos={} charaId={} dressId={}",
            entry.key, chara_id, dress_id);

        crate::windows::discord::on_home_chara(chara_id, dress_id);
        return;
    }
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, HomeCharacterCreator);

    let CreateInfo = match find_nested_class(HomeCharacterCreator, c"CreateInfo") {
        Ok(v) => v,
        Err(e) => {
            error!("Failed to find HomeCharacterCreator.CreateInfo: {}", e);
            return;
        }
    };

    unsafe {
        GETCHARAID_ADDR = get_method_addr(CreateInfo, c"get_CharaId", 0);
        GETDRESSID_ADDR = get_method_addr(CreateInfo, c"get_DressId", 0);
        GETFOOTERCHARAINFO_ADDR = get_method_addr(HomeCharacterCreator, c"GetFooterCharaInfo", 0);
    }

    let CreateAdditionalStandCharacter_addr =
        get_method_addr(HomeCharacterCreator, c"CreateAdditionalStandCharacter", 2);

    new_hook!(CreateAdditionalStandCharacter_addr, CreateAdditionalStandCharacter);
}
