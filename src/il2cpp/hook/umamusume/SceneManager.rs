use std::sync::atomic::{self, AtomicBool};

use crate::{il2cpp::{symbols::get_method_addr, types::*}, windows::wnd_hook};

static SPLASH_SHOWN: AtomicBool = AtomicBool::new(false);
pub fn is_splash_shown() -> bool {
    SPLASH_SHOWN.load(atomic::Ordering::Acquire)
}

// 繁中(Komoe) client 的 ChangeView 是 **6** 個參數（多了尾端的 isFastDestroy），
// 日服是 7、Edge 猜非日服 5 —— 兩邊都對不上，之前用 5 導致 hook 拿到 null。
// 依 il2cpp dump：
//   Gallop.SceneManager::ChangeView(Gallop.SceneDefine.ViewId nextViewId, Gallop.IViewInfo viewInfo,
//       System.Action callbackOnChangeViewCancel, System.Action callbackOnChangeViewAccept,
//       System.Boolean forceChange, System.Boolean isFastDestroy) -> System.Void
type ChangeViewFn = extern "C" fn(
    this: *mut Il2CppObject, next_view_id: i32, view_info: *mut Il2CppObject,
    callback_on_change_view_cancel: *mut Il2CppObject, callback_on_change_view_accept: *mut Il2CppObject,
    force_change: bool, is_fast_destroy: bool
);
extern "C" fn ChangeView(
    this: *mut Il2CppObject, next_view_id: i32, view_info: *mut Il2CppObject,
    callback_on_change_view_cancel: *mut Il2CppObject, callback_on_change_view_accept: *mut Il2CppObject,
    force_change: bool, is_fast_destroy: bool
) {
    get_orig_fn!(ChangeView, ChangeViewFn)(
        this, next_view_id, view_info, callback_on_change_view_cancel, callback_on_change_view_accept,
        force_change, is_fast_destroy
    );
    if next_view_id == 1 { // ViewId.Splash
        SPLASH_SHOWN.store(true, atomic::Ordering::Release);
        wnd_hook::drain_wm_size_buffer();
    }
}

static mut GETCURRENTVIEWID_ADDR: usize = 0;
impl_addr_wrapper_fn!(GetCurrentViewId, GETCURRENTVIEWID_ADDR, i32, this: *mut Il2CppObject);

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, SceneManager);

    let ChangeView_addr = get_method_addr(SceneManager, c"ChangeView", 6);

    unsafe {
        GETCURRENTVIEWID_ADDR = get_method_addr(SceneManager, c"GetCurrentViewId", 0);
    }

    new_hook!(ChangeView_addr, ChangeView);
}
