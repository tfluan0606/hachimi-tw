//! 關掉遊戲內建的 Sentry 錯誤回報。
//!
//! 遊戲有兩條路會初始化 Sentry，兩條都先問 `SentryUnityOptionsExtensions.ShouldInitializeSdk`：
//!
//! ```text
//! Sentry.Unity.SentryInitialization::Init()   （RuntimeInitializeOnLoad，開機時）
//! SentryHelper_BUMA::Init()                    （遊戲自己的 helper）
//!   → ScriptableSentryUnityOptions.LoadSentryUnityOptions()
//!   → SentryUnityOptionsExtensions.ShouldInitializeSdk(options, appDomain)   ← 這裡回 false
//!   → Sentry.Unity.SentrySdk.Init(options)
//! ```
//!
//! 一般 hook 是等 cri_ware_unity.dll 載入才裝，那時開機那條早跑完了，所以這個 hook 改在
//! `il2cpp_init` 一返回就裝（[`hook_il2cpp_init`]），趕在 Unity 跑 RuntimeInitializeOnLoad 之前。
//! 晚載入時來不及擋，[`init`] 會在發現 Sentry 已啟用時呼叫 `SentrySdk.Close()` 關掉。

use std::{
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    core::Hachimi,
    il2cpp::{
        api::il2cpp_domain_get,
        symbols::{self, get_method_addr},
        types::*,
    },
};

static INSTALLED: AtomicBool = AtomicBool::new(false);

extern "C" fn ShouldInitializeSdk(_options: *mut Il2CppObject, _app_domain: *mut Il2CppObject, _method: *const c_void) -> bool {
    info!("[Sentry] blocked SDK initialization");
    false
}

/// 裝 `ShouldInitializeSdk` hook；裝過就不再裝。需要 il2cpp domain 已經可用。
fn install() {
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }
    get_assembly_image_or_return!(image, "Sentry.Unity.dll");
    get_class_or_return!(image, "Sentry.Unity", SentryUnityOptionsExtensions);
    let ShouldInitializeSdk_addr = get_method_addr(SentryUnityOptionsExtensions, c"ShouldInitializeSdk", 2);
    new_hook!(ShouldInitializeSdk_addr, ShouldInitializeSdk);
}

// ---- 早期：il2cpp_init 返回後立刻裝 ----

type Il2CppInitFn = extern "C" fn(domain_name: *const c_void) -> i32;

extern "C" fn il2cpp_init_hook(domain_name: *const c_void) -> i32 {
    let ret = get_orig_fn!(il2cpp_init_hook, Il2CppInitFn)(domain_name);
    install_after_runtime_init();
    ret
}

extern "C" fn il2cpp_init_utf16_hook(domain_name: *const c_void) -> i32 {
    let ret = get_orig_fn!(il2cpp_init_utf16_hook, Il2CppInitFn)(domain_name);
    install_after_runtime_init();
    ret
}

fn install_after_runtime_init() {
    // symbols::init() 之後在 on_hooking_finished 還會再跑一次，重複呼叫沒有副作用
    if il2cpp_domain_get().is_null() {
        warn!("[Sentry] il2cpp domain not ready after il2cpp_init");
        return;
    }
    symbols::init();
    info!("[Sentry] il2cpp runtime ready, installing early hook");
    install();
}

/// GameAssembly.dll 剛載入（handle 已設好）時呼叫：hook `il2cpp_init`／`il2cpp_init_utf16`。
pub fn hook_il2cpp_init() {
    let interceptor = &Hachimi::instance().interceptor;
    for (name, hook) in [
        ("il2cpp_init", il2cpp_init_hook as usize),
        ("il2cpp_init_utf16", il2cpp_init_utf16_hook as usize),
    ] {
        let addr = unsafe { symbols::dlsym(name) };
        if addr == 0 {
            continue;
        }
        match interceptor.hook(addr, hook) {
            Ok(_) => info!("[Sentry] hooked {}", name),
            Err(e) => error!("[Sentry] hook {}: {}", name, e),
        }
    }
}

// ---- 一般時機（晚載入的保險） ----

pub fn init() {
    let already = INSTALLED.load(Ordering::Acquire);
    install();
    if already {
        return;
    }

    // 早期 hook 沒裝到（晚載入），Sentry 可能已經初始化了：直接關掉
    get_assembly_image_or_return!(image, "Sentry.Unity.dll");
    get_class_or_return!(image, "Sentry.Unity", SentrySdk);
    let is_enabled_addr = get_method_addr(SentrySdk, c"get_IsEnabled", 0);
    let close_addr = get_method_addr(SentrySdk, c"Close", 0);
    if is_enabled_addr == 0 || close_addr == 0 {
        return;
    }
    let is_enabled: extern "C" fn(*const c_void) -> bool = unsafe { std::mem::transmute(is_enabled_addr) };
    if is_enabled(std::ptr::null()) {
        warn!("[Sentry] already initialized (late load), closing it");
        let close: extern "C" fn(*const c_void) = unsafe { std::mem::transmute(close_addr) };
        close(std::ptr::null());
    }
}
