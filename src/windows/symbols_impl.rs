use std::{ffi::CString, os::raw::c_void};
use windows::Win32::Foundation::HMODULE;

use crate::windows::utils;

// 繁中服（TW / Komoe）的 GameAssembly.dll 用「標準」il2cpp_* 匯出名（未混淆），
// 直接 GetProcAddress 即可解到（已驗證：242 個標準 il2cpp_ 匯出齊全）。
//
// 對比：原版 Hachimi 針對日/國際服，那邊 il2cpp 匯出名被隨機混淆，故它從 UnityPlayer.dll
// 一張寫死 RVA(0x7834a2)/步長(0x28,0x26) 的表還原混淆名。該 RVA 綁定特定 UnityPlayer.dll build，
// 在 TW 的不同 build 上會越界 panic（"bounds check failed"）→ 這裡整段拿掉、改直接解標準名。
pub unsafe fn dlsym(handle: *mut c_void, name: &str) -> usize {
    debug_assert!(!handle.is_null());
    let Ok(cname) = CString::new(name) else { return 0 };
    utils::get_proc_address(HMODULE(handle as _), &cname)
}
