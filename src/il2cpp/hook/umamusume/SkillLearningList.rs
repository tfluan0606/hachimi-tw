// Tier 2（B 方案）：精準鏡像「技能學習頁」清單。
// 該清單（SingleModeSkillLearningSkillInfo.SkillList : List<...Info>）只在開技能頁時臨時建。
//
// 作法：hook 其 .ctor（arity 0，已知）→ 用 GCHandle 記住「最新建立的實例」（保住不被 GC）。
// 建構後遊戲會用 AddInfo 把清單填滿；等玩家開我們的選單刷新時，清單早已完整，
// 於是 refresh()（主執行緒）直接讀該實例的 <SkillList> 欄位，逐筆取 Info.SkillId / IsAvailable。
// 這避開了 AddInfo 未知 arity、及 UpdateAvailabilityInfo 不觸發的問題。GUI 只讀 cached()。

use std::sync::Mutex;

use crate::il2cpp::{
    api::il2cpp_field_get_offset,
    symbols::{get_field_from_name, get_field_object_value, get_method_addr, GCHandle, Thread},
    types::*,
};

#[derive(Clone, Debug)]
pub struct CapturedSkill {
    pub skill_id: i32,
    pub available: bool,
}

// 最新建立的 SingleModeSkillLearningSkillInfo 實例（GCHandle 保活）
static LATEST: Mutex<Option<GCHandle>> = Mutex::new(None);
// 供 GUI 讀的快取
static ACCUM: Mutex<Vec<CapturedSkill>> = Mutex::new(Vec::new());

static mut F_SKILLLIST: *mut FieldInfo = 0 as _;
static mut F_INFO_SKILLID: *mut FieldInfo = 0 as _;
static mut F_INFO_AVAIL: *mut FieldInfo = 0 as _;

unsafe fn read_i32(obj: *mut Il2CppObject, field: *mut FieldInfo) -> i32 {
    if field.is_null() || obj.is_null() {
        return 0;
    }
    *((obj as *const u8).add(il2cpp_field_get_offset(field)) as *const i32)
}

unsafe fn read_bool(obj: *mut Il2CppObject, field: *mut FieldInfo) -> bool {
    if field.is_null() || obj.is_null() {
        return true;
    }
    *((obj as *const u8).add(il2cpp_field_get_offset(field)) as *const u8) != 0
}

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

// .ctor：記住這個新實例（GCHandle 保活，換掉舊的）
type CtorFn = extern "C" fn(this: *mut Il2CppObject);
extern "C" fn ctor(this: *mut Il2CppObject) {
    get_orig_fn!(ctor, CtorFn)(this);
    if !this.is_null() {
        *LATEST.lock().unwrap() = Some(GCHandle::new(this, false));
    }
}

/// 主執行緒讀取最新技能學習清單並更新快取。
pub fn refresh() {
    Thread::main_thread().schedule(|| {
        let this = {
            let guard = LATEST.lock().unwrap();
            match guard.as_ref() {
                Some(h) => h.target(),
                None => std::ptr::null_mut(),
            }
        };
        let mut v = Vec::new();
        if !this.is_null() {
            unsafe {
                let list = get_field_object_value::<Il2CppObject>(this, F_SKILLLIST);
                for info in read_object_list(list) {
                    if info.is_null() {
                        continue;
                    }
                    let sid = read_i32(info, F_INFO_SKILLID);
                    if sid == 0 {
                        continue;
                    }
                    v.push(CapturedSkill { skill_id: sid, available: read_bool(info, F_INFO_AVAIL) });
                }
            }
        }
        info!("[SkillLearning] refresh: {} skills (instance={:p})", v.len(), this);
        *ACCUM.lock().unwrap() = v;
    });
}

/// GUI 讀取用。空 = 尚未開過技能頁 / 尚未刷新。
pub fn cached() -> Vec<CapturedSkill> {
    ACCUM.lock().unwrap().clone()
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, SingleModeSkillLearningSkillInfo);
    get_class_or_return!(umamusume, Gallop, PartsSingleModeSkillLearningListItem);
    find_nested_class_or_return!(PartsSingleModeSkillLearningListItem, Info);

    unsafe {
        F_SKILLLIST = get_field_from_name(SingleModeSkillLearningSkillInfo, c"<SkillList>k__BackingField");
        F_INFO_SKILLID = get_field_from_name(Info, c"<SkillId>k__BackingField");
        F_INFO_AVAIL = get_field_from_name(Info, c"<IsAvailable>k__BackingField");
        if F_SKILLLIST.is_null() || F_INFO_SKILLID.is_null() {
            error!("[SkillLearning] required fields missing, abort");
            return;
        }
    }

    let ctor_addr = get_method_addr(SingleModeSkillLearningSkillInfo, c".ctor", 0);
    new_hook!(ctor_addr, ctor);
}
