#![allow(non_snake_case, non_upper_case_globals)]

// version.dll proxy —— 讓本 DLL 命名為 version.dll 劫持載入（UnityPlayer.dll 靜態依賴 version.dll）。
// 17 個匯出用 proxy_proc! 的裸 jmp stub 轉發到**真** version.dll（System32 全路徑，與我們同名不同路徑 → 零遞迴）。
// init() 必須在 DllMain 最前面無條件呼叫（早於遊戲任何 version 呼叫，且不依賴 Hachimi 已初始化）。

use widestring::U16CString;
use windows::{core::PCWSTR, Win32::System::LibraryLoader::LoadLibraryW};

use crate::windows::utils;

proxy_proc!(GetFileVersionInfoA, GetFileVersionInfoA_orig);
proxy_proc!(GetFileVersionInfoByHandle, GetFileVersionInfoByHandle_orig);
proxy_proc!(GetFileVersionInfoExA, GetFileVersionInfoExA_orig);
proxy_proc!(GetFileVersionInfoExW, GetFileVersionInfoExW_orig);
proxy_proc!(GetFileVersionInfoSizeA, GetFileVersionInfoSizeA_orig);
proxy_proc!(GetFileVersionInfoSizeExA, GetFileVersionInfoSizeExA_orig);
proxy_proc!(GetFileVersionInfoSizeExW, GetFileVersionInfoSizeExW_orig);
proxy_proc!(GetFileVersionInfoSizeW, GetFileVersionInfoSizeW_orig);
proxy_proc!(GetFileVersionInfoW, GetFileVersionInfoW_orig);
proxy_proc!(VerFindFileA, VerFindFileA_orig);
proxy_proc!(VerFindFileW, VerFindFileW_orig);
proxy_proc!(VerInstallFileA, VerInstallFileA_orig);
proxy_proc!(VerInstallFileW, VerInstallFileW_orig);
proxy_proc!(VerLanguageNameA, VerLanguageNameA_orig);
proxy_proc!(VerLanguageNameW, VerLanguageNameW_orig);
proxy_proc!(VerQueryValueA, VerQueryValueA_orig);
proxy_proc!(VerQueryValueW, VerQueryValueW_orig);

pub fn init() {
    let mut path = utils::_get_system_directory().to_string();
    path.push_str("\\version.dll");
    let Ok(path_cstr) = U16CString::from_str(&path) else { return };
    let handle = match unsafe { LoadLibraryW(PCWSTR(path_cstr.as_ptr())) } {
        Ok(h) => h,
        Err(_) => return
    };
    unsafe {
        GetFileVersionInfoA_orig = utils::get_proc_address(handle, c"GetFileVersionInfoA");
        GetFileVersionInfoByHandle_orig = utils::get_proc_address(handle, c"GetFileVersionInfoByHandle");
        GetFileVersionInfoExA_orig = utils::get_proc_address(handle, c"GetFileVersionInfoExA");
        GetFileVersionInfoExW_orig = utils::get_proc_address(handle, c"GetFileVersionInfoExW");
        GetFileVersionInfoSizeA_orig = utils::get_proc_address(handle, c"GetFileVersionInfoSizeA");
        GetFileVersionInfoSizeExA_orig = utils::get_proc_address(handle, c"GetFileVersionInfoSizeExA");
        GetFileVersionInfoSizeExW_orig = utils::get_proc_address(handle, c"GetFileVersionInfoSizeExW");
        GetFileVersionInfoSizeW_orig = utils::get_proc_address(handle, c"GetFileVersionInfoSizeW");
        GetFileVersionInfoW_orig = utils::get_proc_address(handle, c"GetFileVersionInfoW");
        VerFindFileA_orig = utils::get_proc_address(handle, c"VerFindFileA");
        VerFindFileW_orig = utils::get_proc_address(handle, c"VerFindFileW");
        VerInstallFileA_orig = utils::get_proc_address(handle, c"VerInstallFileA");
        VerInstallFileW_orig = utils::get_proc_address(handle, c"VerInstallFileW");
        VerLanguageNameA_orig = utils::get_proc_address(handle, c"VerLanguageNameA");
        VerLanguageNameW_orig = utils::get_proc_address(handle, c"VerLanguageNameW");
        VerQueryValueA_orig = utils::get_proc_address(handle, c"VerQueryValueA");
        VerQueryValueW_orig = utils::get_proc_address(handle, c"VerQueryValueW");
    }
}
