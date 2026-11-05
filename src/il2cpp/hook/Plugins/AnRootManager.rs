//! A2U Flash 在育成畫面跟著 `ui_animation_scale` 加速。
//!
//! 育成的 UI 等待（結果 flash 後的停留、數值面板、換頁）多半是 DOTween，`ui_animation_scale`
//! 會縮短；但同時在播的 A2U flash 不是 DOTween，原本不會變快，倍率一高 flash 就被提早切掉。
//! 這裡把 A2U 的時間也乘同一個倍率，兩邊一起縮。
//!
//! `AnRootManager.Update()` 每幀先算 `CurrentDeltaTime`（`Time.deltaTime`）寫進欄位，
//! 接著呼叫 `_UpdateScreenSize()`，然後才逐一 `AnRoot._UpdateRoot` 讀那個欄位推進時間。
//! 所以在 `_UpdateScreenSize` 回來時（只限從 `Update` 進來的那次）改欄位，所有 flash 都吃得到。
//!
//! 欄位沒有名字可查（dump 只有方法），offset 從 getter `get_CurrentDeltaTime` 的
//! `movss xmm0, [reg+disp32]` 讀出來，改版時不必手改。只在育成畫面（view id 1000–1799）生效。

use std::sync::atomic::{self, AtomicBool, AtomicUsize};

use crate::{
    core::Hachimi,
    il2cpp::{hook::umamusume::SceneManager, symbols::get_method_addr, types::*}
};

/// `CurrentDeltaTime` 欄位 offset，0＝沒找到（不改）
static DELTA_TIME_OFFSET: AtomicUsize = AtomicUsize::new(0);
static IN_UPDATE: AtomicBool = AtomicBool::new(false);

fn is_single_mode_view() -> bool {
    matches!(SceneManager::current_view_id(), 1000..=1799)
}

type UpdateFn = extern "C" fn(this: *mut Il2CppObject);
extern "C" fn Update(this: *mut Il2CppObject) {
    IN_UPDATE.store(true, atomic::Ordering::Relaxed);
    get_orig_fn!(Update, UpdateFn)(this);
    IN_UPDATE.store(false, atomic::Ordering::Relaxed);
}

type UpdateScreenSizeFn = extern "C" fn(this: *mut Il2CppObject);
extern "C" fn _UpdateScreenSize(this: *mut Il2CppObject) {
    get_orig_fn!(_UpdateScreenSize, UpdateScreenSizeFn)(this);

    // _OnInitialize / OptimizeAll 也會呼叫，那時欄位不是剛算好的 delta，不碰
    if !IN_UPDATE.load(atomic::Ordering::Relaxed) || this.is_null() {
        return;
    }
    let offset = DELTA_TIME_OFFSET.load(atomic::Ordering::Relaxed);
    if offset == 0 {
        return;
    }
    let scale = Hachimi::instance().config.load().ui_animation_scale;
    if scale != 1.0 && is_single_mode_view() {
        unsafe { *(this as *mut u8).add(offset).cast::<f32>() *= scale; }
    }
}

/// 在 getter 開頭找 `movss xmm0, [reg+disp32]`（F3 0F 10 /r，mod=10、reg=xmm0）取 disp32
fn find_field_offset(getter: usize) -> Option<usize> {
    if getter == 0 {
        return None;
    }
    let code = unsafe { std::slice::from_raw_parts(getter as *const u8, 96) };
    for i in 0..code.len() - 8 {
        let modrm = code[i + 3];
        if code[i] == 0xF3 && code[i + 1] == 0x0F && code[i + 2] == 0x10
            && modrm >> 6 == 0b10 && (modrm >> 3) & 0b111 == 0 && modrm & 0b111 != 0b100
        {
            let disp = i32::from_le_bytes(code[i + 4..i + 8].try_into().unwrap());
            if (0x10..0x1000).contains(&disp) {
                return Some(disp as usize);
            }
        }
    }
    None
}

pub fn init(Plugins: *const Il2CppImage) {
    get_class_or_return!(Plugins, AnimateToUnity, AnRootManager);

    let getter = get_method_addr(AnRootManager, c"get_CurrentDeltaTime", 0);
    match find_field_offset(getter) {
        Some(offset) => {
            info!("AnRootManager.CurrentDeltaTime offset = {:#x}", offset);
            DELTA_TIME_OFFSET.store(offset, atomic::Ordering::Relaxed);
        }
        None => {
            error!("AnRootManager.CurrentDeltaTime offset not found, A2U speed-up disabled");
            return;
        }
    }

    let Update_addr = get_method_addr(AnRootManager, c"Update", 0);
    let _UpdateScreenSize_addr = get_method_addr(AnRootManager, c"_UpdateScreenSize", 0);

    new_hook!(Update_addr, Update);
    new_hook!(_UpdateScreenSize_addr, _UpdateScreenSize);
}
