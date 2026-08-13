//! `UnityEngine.Transform` 綁定。純粹的方法位址包裝，本身不 hook 任何東西。
//!
//! 位置／旋轉一律走 `*_Injected` 變體：IL2CPP 會把回傳結構的屬性改寫成「用 by-ref 參數
//! 收結果」的形式，`_Injected` 才是實際存在的那個。簽章都對過繁中服的 dump：
//!
//! ```text
//! UnityEngine.Transform::get_position_Injected(UnityEngine.Vector3& ret) -> System.Void
//! UnityEngine.Transform::set_position_Injected(UnityEngine.Vector3& value) -> System.Void
//! ```

use crate::il2cpp::{symbols::get_method_addr, types::*};

static mut GET_PARENT_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_parent, GET_PARENT_ADDR, *mut Il2CppObject, this: *mut Il2CppObject);

static mut GET_CHILDCOUNT_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_childCount, GET_CHILDCOUNT_ADDR, i32, this: *mut Il2CppObject);

static mut GETCHILD_ADDR: usize = 0;
impl_addr_wrapper_fn!(GetChild, GETCHILD_ADDR, *mut Il2CppObject, this: *mut Il2CppObject, index: i32);

static mut GET_FORWARD_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_forward, GET_FORWARD_ADDR, Vector3_t, this: *mut Il2CppObject);

static mut GET_POSITION_INJECTED_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_position_Injected, GET_POSITION_INJECTED_ADDR, (),
    this: *mut Il2CppObject, ret: *mut Vector3_t);

static mut SET_POSITION_INJECTED_ADDR: usize = 0;
impl_addr_wrapper_fn!(set_position_Injected, SET_POSITION_INJECTED_ADDR, (),
    this: *mut Il2CppObject, value: *mut Vector3_t);

static mut GET_LOCALPOSITION_INJECTED_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_localPosition_Injected, GET_LOCALPOSITION_INJECTED_ADDR, (),
    this: *mut Il2CppObject, ret: *mut Vector3_t);

static mut SET_LOCALPOSITION_INJECTED_ADDR: usize = 0;
impl_addr_wrapper_fn!(set_localPosition_Injected, SET_LOCALPOSITION_INJECTED_ADDR, (),
    this: *mut Il2CppObject, value: *mut Vector3_t);

static mut GET_ROTATION_INJECTED_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_rotation_Injected, GET_ROTATION_INJECTED_ADDR, (),
    this: *mut Il2CppObject, ret: *mut Quaternion_t);

static mut SET_ROTATION_INJECTED_ADDR: usize = 0;
impl_addr_wrapper_fn!(set_rotation_Injected, SET_ROTATION_INJECTED_ADDR, (),
    this: *mut Il2CppObject, value: *mut Quaternion_t);

static mut GET_LOCALROTATION_INJECTED_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_localRotation_Injected, GET_LOCALROTATION_INJECTED_ADDR, (),
    this: *mut Il2CppObject, ret: *mut Quaternion_t);

static mut SET_LOCALROTATION_INJECTED_ADDR: usize = 0;
impl_addr_wrapper_fn!(set_localRotation_Injected, SET_LOCALROTATION_INJECTED_ADDR, (),
    this: *mut Il2CppObject, value: *mut Quaternion_t);

// 上面那組 by-ref 介面用起來很吵，包成一般的取值／設值。
// `Vector3_t` / `Quaternion_t` 來自產生的 types.rs，沒有 Default，就地建構即可。
//
// ⚠️ 要做旋轉運算前先確認 `Quaternion_t` 的欄位順序：types.rs 裡是 `w, x, y, z`，
// 但 Unity 的 Quaternion 在記憶體中是 `x, y, z, w`。單純取值／設值原封傳遞不受影響，
// 一旦要自己算四元數就會對不上。

fn zero_vec() -> Vector3_t { Vector3_t { x: 0.0, y: 0.0, z: 0.0 } }
fn zero_quat() -> Quaternion_t { Quaternion_t { w: 0.0, x: 0.0, y: 0.0, z: 0.0 } }

pub fn get_position(this: *mut Il2CppObject) -> Vector3_t {
    let mut v = zero_vec();
    get_position_Injected(this, &mut v);
    v
}

pub fn set_position(this: *mut Il2CppObject, mut value: Vector3_t) {
    set_position_Injected(this, &mut value);
}

pub fn get_localPosition(this: *mut Il2CppObject) -> Vector3_t {
    let mut v = zero_vec();
    get_localPosition_Injected(this, &mut v);
    v
}

pub fn set_localPosition(this: *mut Il2CppObject, mut value: Vector3_t) {
    set_localPosition_Injected(this, &mut value);
}

pub fn get_rotation(this: *mut Il2CppObject) -> Quaternion_t {
    let mut q = zero_quat();
    get_rotation_Injected(this, &mut q);
    q
}

pub fn set_rotation(this: *mut Il2CppObject, mut value: Quaternion_t) {
    set_rotation_Injected(this, &mut value);
}

pub fn get_localRotation(this: *mut Il2CppObject) -> Quaternion_t {
    let mut q = zero_quat();
    get_localRotation_Injected(this, &mut q);
    q
}

pub fn set_localRotation(this: *mut Il2CppObject, mut value: Quaternion_t) {
    set_localRotation_Injected(this, &mut value);
}

pub fn init(UnityEngine_CoreModule: *const Il2CppImage) {
    get_class_or_return!(UnityEngine_CoreModule, UnityEngine, Transform);

    unsafe {
        GET_PARENT_ADDR = get_method_addr(Transform, c"get_parent", 0);
        GET_CHILDCOUNT_ADDR = get_method_addr(Transform, c"get_childCount", 0);
        GETCHILD_ADDR = get_method_addr(Transform, c"GetChild", 1);
        GET_FORWARD_ADDR = get_method_addr(Transform, c"get_forward", 0);

        GET_POSITION_INJECTED_ADDR = get_method_addr(Transform, c"get_position_Injected", 1);
        SET_POSITION_INJECTED_ADDR = get_method_addr(Transform, c"set_position_Injected", 1);
        GET_LOCALPOSITION_INJECTED_ADDR = get_method_addr(Transform, c"get_localPosition_Injected", 1);
        SET_LOCALPOSITION_INJECTED_ADDR = get_method_addr(Transform, c"set_localPosition_Injected", 1);
        GET_ROTATION_INJECTED_ADDR = get_method_addr(Transform, c"get_rotation_Injected", 1);
        SET_ROTATION_INJECTED_ADDR = get_method_addr(Transform, c"set_rotation_Injected", 1);
        GET_LOCALROTATION_INJECTED_ADDR = get_method_addr(Transform, c"get_localRotation_Injected", 1);
        SET_LOCALROTATION_INJECTED_ADDR = get_method_addr(Transform, c"set_localRotation_Injected", 1);
    }
}
