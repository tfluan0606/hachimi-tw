//! Hook 練成角色詳細視窗的開啟，記下「使用者最後點開的是哪一隻」。
//!
//! 馬房裡點開一隻馬並不會發 API（資料早就隨清單一起下載了），所以光看封包沒辦法知道
//! 現在看的是誰。這裡攔 `Gallop.DialogTrainedCharacterDetail::CreateSetupParameter(...)`
//! ——所有開啟詳細視窗的路徑都會經過它——從第一個參數 `TrainedCharaData` 取 `Id`
//! （＝`trained_chara_id`），交給 [`factor_card`] 當作截圖目標。

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    core::factor_card,
    il2cpp::{api::il2cpp_object_get_class, ext::{Il2CppObjectExt, StringExt}, symbols::{get_method_addr, GCHandle}, types::*},
};
use std::sync::Mutex;

use super::WorkDataManager;

// Open(TrainedCharaData, String trainerName, Action<bool> onClose, Action<(bool,bool)> onChangePartner) [static]
static OPEN_ADDR: AtomicUsize = AtomicUsize::new(0);

/// 上一個由 [`open`] 開的視窗（弱參考：視窗被遊戲銷毀後就取不到）
static LAST_OPENED: Mutex<Option<GCHandle>> = Mutex::new(None);

/// 呼叫物件上的無參數方法（含父類別），找不到回 `None`
fn call0<R>(obj: *mut Il2CppObject, name: &std::ffi::CStr) -> Option<R> {
    let addr = get_method_addr(unsafe { (*obj).klass() }, name, 0);
    if addr == 0 {
        return None;
    }
    let f: extern "C" fn(*mut Il2CppObject, *const std::ffi::c_void) -> R = unsafe { std::mem::transmute(addr) };
    Some(f(obj, std::ptr::null()))
}

/// 上一個由 [`open`] 開的視窗還開著的話關掉
fn close_last_opened() {
    let Some(handle) = LAST_OPENED.lock().unwrap().take() else { return };
    let dialog = handle.target();
    if !dialog.is_null() && call0::<bool>(dialog, c"get_IsOpen") == Some(true) {
        call0::<()>(dialog, c"Close");
    }
}

/// 開包包裡那隻馬的原生詳細視窗（不連線），上一個用這裡開的視窗會先關掉。
/// 不在包包裡、或方法沒解到時回 `false`。會呼叫 il2cpp，**只能在遊戲主執行緒呼叫**。
pub fn open(trained_chara_id: i32) -> bool {
    let addr = OPEN_ADDR.load(Ordering::Relaxed);
    if addr == 0 {
        return false;
    }
    let chara = WorkDataManager::get_trained_chara(trained_chara_id);
    if chara.is_null() {
        return false;
    }
    let f: extern "C" fn(
        *mut Il2CppObject,
        *mut Il2CppString,
        *mut Il2CppObject,
        *mut Il2CppObject,
        *const std::ffi::c_void,
    ) -> *mut Il2CppObject = unsafe { std::mem::transmute(addr) };
    close_last_opened();
    // 訓練師名給空字串（自己的馬）；關閉、換練習夥伴兩個 callback 用不到，傳 null
    let dialog = f(chara, "".to_il2cpp_string(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
    if !dialog.is_null() {
        *LAST_OPENED.lock().unwrap() = Some(GCHandle::new_weak_ref(dialog, false));
    }
    true
}

/// `TrainedCharaData::get_Id()` 的位址（第一次用到時從物件本身的 class 解出來並快取）
static GET_ID_ADDR: AtomicUsize = AtomicUsize::new(0);

fn record(trained_chara_data: *mut Il2CppObject) {
    if trained_chara_data.is_null() {
        return;
    }
    let mut addr = GET_ID_ADDR.load(Ordering::Relaxed);
    if addr == 0 {
        addr = get_method_addr(il2cpp_object_get_class(trained_chara_data), c"get_Id", 0);
        if addr == 0 {
            return;
        }
        GET_ID_ADDR.store(addr, Ordering::Relaxed);
    }
    let get_id: extern "C" fn(*mut Il2CppObject) -> i32 = unsafe { std::mem::transmute(addr) };
    factor_card::set_last_viewed(get_id(trained_chara_data) as i64);
}

// CreateSetupParameter(TrainedCharaData, String trainerName, Action<(bool,bool)>, bool isSingleMode)
type CreateSetupParameterFn = extern "C" fn(
    *mut Il2CppObject,
    *mut Il2CppString,
    *mut Il2CppObject,
    bool,
) -> *mut Il2CppObject;
extern "C" fn CreateSetupParameter(
    trained_chara_data: *mut Il2CppObject,
    trainer_name: *mut Il2CppString,
    on_change_partner: *mut Il2CppObject,
    is_single_mode: bool,
) -> *mut Il2CppObject {
    record(trained_chara_data);
    get_orig_fn!(CreateSetupParameter, CreateSetupParameterFn)(
        trained_chara_data,
        trainer_name,
        on_change_partner,
        is_single_mode,
    )
}

// CreateSetupParameterWithCharaSwitchButtonAsync(TrainedCharaData, int, int, Action<...>, Action<bool>,
//                                                Action, UpdateCharacterButtonFrame, String)
type CreateSetupParameterAsyncFn = extern "C" fn(
    *mut Il2CppObject,
    i32,
    i32,
    *mut Il2CppObject,
    *mut Il2CppObject,
    *mut Il2CppObject,
    *mut Il2CppObject,
    *mut Il2CppString,
) -> *mut Il2CppObject;
#[allow(clippy::too_many_arguments)]
extern "C" fn CreateSetupParameterWithCharaSwitchButtonAsync(
    trained_chara_data: *mut Il2CppObject,
    current_chara_index: i32,
    total_chara_count: i32,
    get_indexed_setup_param: *mut Il2CppObject,
    on_close_dialog: *mut Il2CppObject,
    on_change_chara: *mut Il2CppObject,
    update_chara_button_frame: *mut Il2CppObject,
    trainer_name: *mut Il2CppString,
) -> *mut Il2CppObject {
    record(trained_chara_data);
    get_orig_fn!(CreateSetupParameterWithCharaSwitchButtonAsync, CreateSetupParameterAsyncFn)(
        trained_chara_data,
        current_chara_index,
        total_chara_count,
        get_indexed_setup_param,
        on_close_dialog,
        on_change_chara,
        update_chara_button_frame,
        trainer_name,
    )
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, DialogTrainedCharacterDetail);

    // 同名多載以參數數量區分：(TrainedCharaData, string, Action, bool) = 4
    let CreateSetupParameter_addr = get_method_addr(DialogTrainedCharacterDetail, c"CreateSetupParameter", 4);
    let CreateSetupParameterAsync_addr =
        get_method_addr(DialogTrainedCharacterDetail, c"CreateSetupParameterWithCharaSwitchButtonAsync", 8);

    OPEN_ADDR.store(get_method_addr(DialogTrainedCharacterDetail, c"Open", 4), Ordering::Relaxed);

    new_hook!(CreateSetupParameter_addr, CreateSetupParameter);
    new_hook!(CreateSetupParameterAsync_addr, CreateSetupParameterWithCharaSwitchButtonAsync);
}
