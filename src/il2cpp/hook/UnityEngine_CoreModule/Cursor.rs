// 關閉遊戲內建游標（PC 版更新後新增的自訂滑鼠游標）。
// 遊戲用 `UnityEngine.Cursor.SetCursor(texture, hotspot, mode)` 把系統游標換成自訂圖，
// 我們攔截底層 icall `SetCursor_Injected`（hotspot 走指標，避開 Vector2 傳值的 ABI 問題），
// 開關開啟時強制傳 null texture → 還原成 Windows 預設箭頭。遊戲之後每次重設游標都會被還原。
use std::sync::atomic;

use crate::{
    core::Hachimi,
    il2cpp::{api::il2cpp_resolve_icall, symbols::Thread, types::*},
};

// CursorMode.Auto：交給系統決定硬體/軟體游標
const CURSOR_MODE_AUTO: i32 = 0;

static mut SETCURSOR_INJECTED_ADDR: usize = 0;

fn disabled() -> bool {
    Hachimi::instance().disable_game_cursor.load(atomic::Ordering::Relaxed)
}

type SetCursorInjectedFn = extern "C" fn(texture: *mut Il2CppObject, hotspot: *mut Vector2_t, cursor_mode: i32);
extern "C" fn SetCursor_Injected(texture: *mut Il2CppObject, hotspot: *mut Vector2_t, cursor_mode: i32) {
    if disabled() {
        // null texture + hotspot 歸零 + Auto = 還原成系統預設游標
        let mut zero = Vector2_t::default();
        get_orig_fn!(SetCursor_Injected, SetCursorInjectedFn)(0 as _, &mut zero, CURSOR_MODE_AUTO);
        return;
    }

    get_orig_fn!(SetCursor_Injected, SetCursorInjectedFn)(texture, hotspot, cursor_mode);
}

/// 立即套用（主執行緒）：開關開著就把當前游標換回系統預設，讓切換不必重開遊戲。
/// 遊戲會在下次自然重設游標時被 hook 攔下，所以這裡只需處理「已經是自訂游標」的當下狀態。
pub fn apply() {
    if !disabled() {
        return;
    }
    Thread::main_thread().schedule(|| {
        let addr = unsafe { SETCURSOR_INJECTED_ADDR };
        if addr == 0 {
            return;
        }
        let set_cursor: SetCursorInjectedFn = unsafe { std::mem::transmute(addr) };
        let mut zero = Vector2_t::default();
        set_cursor(0 as _, &mut zero, CURSOR_MODE_AUTO);
    });
}

pub fn init(_UnityEngine_CoreModule: *const Il2CppImage) {
    let SetCursor_Injected_addr = il2cpp_resolve_icall(
        c"UnityEngine.Cursor::SetCursor_Injected(UnityEngine.Texture2D,\
        UnityEngine.Vector2&,UnityEngine.CursorMode)".as_ptr()
    );

    if SetCursor_Injected_addr == 0 {
        error!("Failed to resolve UnityEngine.Cursor::SetCursor_Injected");
        return;
    }

    new_hook!(SetCursor_Injected_addr, SetCursor_Injected);

    unsafe {
        SETCURSOR_INJECTED_ADDR = SetCursor_Injected_addr;
    }
}
