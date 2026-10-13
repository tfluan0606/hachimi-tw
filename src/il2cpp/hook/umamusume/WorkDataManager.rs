//! 遊戲端的玩家資料（`Gallop.WorkDataManager`）。馬娘篩選用它取包包裡現有的練成角色。
//!
//! 進包包時資料就整份下載到這裡了，之後點開詳細視窗、換頁都不會再連線，
//! 所以只要拿到 `TrainedCharaData` 物件就能直接開原生的詳細視窗。
//! 這裡的函式都會呼叫 il2cpp，**只能在遊戲主執行緒呼叫**。

use std::ffi::{c_void, CStr};
use std::ptr::null_mut;

use crate::il2cpp::{
    ext::Il2CppObjectExt,
    symbols::{get_method_addr, IList, SingletonLike},
    types::*,
};

static mut CLASS: *mut Il2CppClass = 0 as _;
static mut GET_TRAINEDCHARADATA_ADDR: usize = 0;

fn instance() -> *mut Il2CppObject {
    let Some(singleton) = SingletonLike::new(unsafe { CLASS }) else {
        return null_mut();
    };
    singleton.instance()
}

/// `WorkDataManager.Instance.TrainedCharaData`（`Gallop.WorkTrainedCharaData`）
fn work_trained_chara() -> *mut Il2CppObject {
    let this = instance();
    let addr = unsafe { GET_TRAINEDCHARADATA_ADDR };
    if this.is_null() || addr == 0 {
        return null_mut();
    }
    let f: extern "C" fn(*mut Il2CppObject, *const c_void) -> *mut Il2CppObject = unsafe { std::mem::transmute(addr) };
    f(this, std::ptr::null())
}

/// 從物件本身的 class 找方法（含父類別），找不到回 0
fn method_of(obj: *mut Il2CppObject, name: &CStr, args: i32) -> usize {
    if obj.is_null() {
        return 0;
    }
    get_method_addr(unsafe { (*obj).klass() }, name, args)
}

/// 用 trained_chara_id 取包包裡那隻的 `TrainedCharaData`；不在包包裡（放生、轉讓掉了）回 null。
pub fn get_trained_chara(trained_chara_id: i32) -> *mut Il2CppObject {
    let work = work_trained_chara();
    // Get(int trainedCharaId, bool all)
    let addr = method_of(work, c"Get", 2);
    if addr == 0 {
        return null_mut();
    }
    let f: extern "C" fn(*mut Il2CppObject, i32, bool, *const c_void) -> *mut Il2CppObject =
        unsafe { std::mem::transmute(addr) };
    f(work, trained_chara_id, true, std::ptr::null())
}

/// 包包裡現有的全部 trained_chara_id；還沒登入完、拿不到清單時回 `None`。
pub fn trained_chara_ids() -> Option<Vec<i64>> {
    let work = work_trained_chara();
    let get_list = method_of(work, c"get_List", 0);
    if get_list == 0 {
        return None;
    }
    let get_list: extern "C" fn(*mut Il2CppObject, *const c_void) -> *mut Il2CppObject =
        unsafe { std::mem::transmute(get_list) };
    let list = IList::<*mut Il2CppObject>::new(get_list(work, std::ptr::null()))?;

    let mut ids = Vec::with_capacity(list.count().max(0) as usize);
    let mut get_id: Option<extern "C" fn(*mut Il2CppObject, *const c_void) -> i32> = None;
    for chara in list.iter() {
        if chara.is_null() {
            continue;
        }
        let f = match get_id {
            Some(f) => f,
            None => {
                let addr = method_of(chara, c"get_Id", 0);
                if addr == 0 {
                    return None;
                }
                let f = unsafe { std::mem::transmute(addr) };
                get_id = Some(f);
                f
            }
        };
        ids.push(f(chara, std::ptr::null()) as i64);
    }
    Some(ids)
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, WorkDataManager);

    unsafe {
        CLASS = WorkDataManager;
        GET_TRAINEDCHARADATA_ADDR = get_method_addr(WorkDataManager, c"get_TrainedCharaData", 0);
    }
}
