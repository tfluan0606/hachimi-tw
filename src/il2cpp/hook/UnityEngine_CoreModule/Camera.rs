//! `UnityEngine.Camera` 綁定。純粹的方法位址包裝，本身不 hook 任何東西。
//!
//! `get_main` 是 static，回傳目前的主攝影機；沒有主攝影機時會是 null，呼叫端要自己擋。

use crate::il2cpp::{symbols::get_method_addr, types::*};

static mut GET_MAIN_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_main, GET_MAIN_ADDR, *mut Il2CppObject,);

static mut GET_FIELDOFVIEW_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_fieldOfView, GET_FIELDOFVIEW_ADDR, f32, this: *mut Il2CppObject);

static mut SET_FIELDOFVIEW_ADDR: usize = 0;
impl_addr_wrapper_fn!(set_fieldOfView, SET_FIELDOFVIEW_ADDR, (), this: *mut Il2CppObject, value: f32);

pub fn init(UnityEngine_CoreModule: *const Il2CppImage) {
    get_class_or_return!(UnityEngine_CoreModule, UnityEngine, Camera);

    unsafe {
        GET_MAIN_ADDR = get_method_addr(Camera, c"get_main", 0);
        GET_FIELDOFVIEW_ADDR = get_method_addr(Camera, c"get_fieldOfView", 0);
        SET_FIELDOFVIEW_ADDR = get_method_addr(Camera, c"set_fieldOfView", 1);
    }
}
