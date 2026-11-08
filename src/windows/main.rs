use std::os::raw::{c_ulong, c_void};

use widestring::U16CString;
use windows::{core::{BOOL, PCWSTR}, Win32::{Foundation::{HMODULE, TRUE}, System::LibraryLoader::LoadLibraryW}};

use crate::{core::{plugin_api::Plugin, Hachimi}, windows::utils};

use super::{crash_guard, discord, game_impl, hook, proxy, updater, wnd_hook};

const DLL_PROCESS_ATTACH: c_ulong = 1;
const DLL_PROCESS_DETACH: c_ulong = 0;

pub fn load_libraries() -> Vec<Plugin> {
    let mut plugins = Vec::new();
    for name in Hachimi::instance().config.load().windows.load_libraries.iter() {
        let Ok(name_cstr) = U16CString::from_str(name) else {
            warn!("Invalid library name: {}", name);
            continue;
        };
        let res = unsafe { LoadLibraryW(PCWSTR(name_cstr.as_ptr())) };

        if let Ok(handle) = res {
            if !handle.is_invalid() {
                info!("Loaded library: {}", name);

                let hachimi_init_addr = utils::get_proc_address(handle, c"hachimi_init");
                if hachimi_init_addr != 0 {
                    plugins.push(Plugin {
                        name: name.clone(),
                        init_fn: unsafe { std::mem::transmute(hachimi_init_addr) }
                    });
                }

                continue;
            }
        }

        warn!("Failed to load library: {}", name);
    }

    plugins
}

fn is_game_process() -> bool {
    game_impl::get_region(&game_impl::get_package_name()) != crate::core::game::Region::Unknown
}

pub static mut DLL_HMODULE: HMODULE = HMODULE(0 as _);

#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn DllMain(hmodule: HMODULE, call_reason: c_ulong, _reserved: *mut c_void) -> BOOL {
    if call_reason == DLL_PROCESS_ATTACH {
        unsafe { DLL_HMODULE = hmodule; }
        // 先無條件裝好 version.dll 轉發（即使 Hachimi::init 失敗也要，否則遊戲解不到 version 函式會崩）。
        proxy::version::init();
        // version.dll 也會被同目錄的其他程式載入（UnityCrashHandler64.exe 每次開遊戲都會起）。
        // 不是遊戲本體就只當轉發用，不初始化、不 hook、不寫 log。
        if !is_game_process() {
            return TRUE;
        }
        if !Hachimi::init() {
            return TRUE;
        }

        // 清掉上次自動更新留下的 version.dll.old（現在它已不再被載入）。
        updater::cleanup_old_dll();

        let hachimi = Hachimi::instance();
        // 上次下載好的外掛更新（.new）在載入外掛之前換上
        super::plugins::apply_pending();
        *hachimi.plugins.lock().unwrap() = load_libraries();

        crash_guard::init();
        hook::init();
        info!("Attach completed");
    }
    else if call_reason == DLL_PROCESS_DETACH && Hachimi::is_initialized() {
        discord::stop();
        wnd_hook::uninit();

        info!("Unhooking everything");
        Hachimi::instance().interceptor.unhook_all();
    }
    TRUE
}