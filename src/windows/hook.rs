#![allow(non_snake_case)]

use std::path::Path;

use windows::{core::{w, PCWSTR}, Win32::{Foundation::HMODULE, System::LibraryLoader::GetModuleHandleW}};

use crate::{core::{Error, Hachimi}, windows::steamworks};

use super::{hachimi_impl, proxy, ffi};

type LoadLibraryWFn = extern "C" fn(filename: PCWSTR) -> HMODULE;
extern "C" fn LoadLibraryW(filename: PCWSTR) -> HMODULE {
    let hachimi = Hachimi::instance();
    let orig_fn: LoadLibraryWFn = unsafe {
        std::mem::transmute(hachimi.interceptor.get_trampoline_addr(LoadLibraryW as usize))
    };

    let handle = orig_fn(filename);
    let filename_str = unsafe { filename.to_string().expect("valid utf-16 filename") };

    if hachimi_impl::is_criware_lib(&filename_str) {
        // Manually trigger a GameAssembly.dll load anyways since hachimi might have been loaded later
        let assembly_module = orig_fn(w!("GameAssembly.dll")).0 as usize;
        if assembly_module != 0 {
            hachimi.on_dlopen("GameAssembly.dll", assembly_module);
        }
    }

    let needs_init_steamworks = steamworks::is_overlay_conflicting(&hachimi);
    if hachimi.on_dlopen(&filename_str, handle.0 as usize) {
        if !needs_init_steamworks {
            hachimi.interceptor.unhook(LoadLibraryW as usize);
        }
    }
    else if needs_init_steamworks &&
        Path::new(&filename_str).file_name().is_some_and(|name| name == "steam_api64.dll")
    {
        steamworks::init(handle);
        hachimi.interceptor.unhook(LoadLibraryW as usize);
    }
    handle
}

fn init_internal() -> Result<(), Error> {
    let hachimi = Hachimi::instance();
    if let Ok(handle) = unsafe { GetModuleHandleW(w!("GameAssembly.dll")) } {
        info!("Late loading detected");
        if steamworks::is_overlay_conflicting(&hachimi) {
            info!("Hooking LoadLibraryW");
            hachimi.interceptor.hook(ffi::LoadLibraryW as usize, LoadLibraryW as usize)?;
        }
        else {
            info!("Skipping LoadLibraryW hook");
        }

        info!("Init cri_mana_vpx.dll proxy");
        proxy::cri_mana_vpx::init();

        hachimi.on_dlopen("GameAssembly.dll", handle.0 as _);
        hachimi.on_hooking_finished();   
    }
    else {
        // 早期注入（version.dll 劫持）：轉發已在 DllMain 的 proxy::version::init() 裝好，
        // 這裡只需 hook LoadLibraryW，等 cri_ware_unity.dll 載入時觸發 il2cpp hook 安裝。
        info!("Early load via version.dll proxy; hooking LoadLibraryW");
        hachimi.interceptor.hook(ffi::LoadLibraryW as usize, LoadLibraryW as usize)?;
    }

    Ok(())
}

pub fn init() {
    init_internal().unwrap_or_else(|e| {
        error!("Init failed: {}", e);
    });
}