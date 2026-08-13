//! 演唱會播放控制（`Gallop.Live.Director`）。
//!
//! 繁中服的 Director 是**比較舊的 API 形狀**，不能照抄 Edge：Edge hook 的是
//! `AlterUpdate(this, deltaTime, isUpdateDeltaTime)`（2 參數），這裡是
//!
//! ```text
//! AlterUpdateTimeline(System.Single& timescale)   ← 1 參數，by-ref
//! get_LiveCurrentTime() / get_LiveTotalTime()     ← 0 參數
//! PauseLive(System.Boolean) / IsPauseLive() [static]
//! ```
//!
//! 舊形狀對我們反而更順手：`timescale` 是 by-ref 傳的，直接改那個值就控制得了播放速度，
//! 不必自己算 delta time。
//!
//! 播放時間在 hook 裡就讀好存成 atomic，**不讓 GUI 去呼叫 il2cpp**——overlay 跑在
//! Present 執行緒上，那不保證是遊戲主執行緒，從那裡碰 il2cpp 會出事。

use std::sync::atomic::{self, AtomicU32, AtomicU64};

use crate::{
    core::Hachimi,
    il2cpp::{symbols::get_method_addr, types::*}
};

/// 目前播放時間與總長度（f32 的 bit pattern）。
static CURRENT_TIME: AtomicU32 = AtomicU32::new(0);
static TOTAL_TIME: AtomicU32 = AtomicU32::new(0);

/// 上次收到 timeline 更新的時間戳（毫秒）。用來判斷「現在是不是真的在播演唱會」——
/// 演唱會結束後 hook 就不再被呼叫，時間戳會停住。
static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 目前是否在播演唱會。超過半秒沒有 timeline 更新就當作沒有。
pub fn is_live_active() -> bool {
    let last = LAST_TICK_MS.load(atomic::Ordering::Relaxed);
    last != 0 && now_ms().saturating_sub(last) < 500
}

/// (目前秒數, 總秒數)
pub fn live_progress() -> (f32, f32) {
    (
        f32::from_bits(CURRENT_TIME.load(atomic::Ordering::Relaxed)),
        f32::from_bits(TOTAL_TIME.load(atomic::Ordering::Relaxed))
    )
}

static mut GETLIVECURRENTTIME_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_LiveCurrentTime, GETLIVECURRENTTIME_ADDR, f32, this: *mut Il2CppObject);

static mut GETLIVETOTALTIME_ADDR: usize = 0;
impl_addr_wrapper_fn!(get_LiveTotalTime, GETLIVETOTALTIME_ADDR, f32, this: *mut Il2CppObject);

type AlterUpdateTimelineFn = extern "C" fn(this: *mut Il2CppObject, timescale: *mut f32);
extern "C" fn AlterUpdateTimeline(this: *mut Il2CppObject, timescale: *mut f32) {
    // by-ref 的 timescale 直接改，遊戲拿去算這一幀要推進多少
    if !timescale.is_null() {
        let speed = Hachimi::instance().config.load().live_playback_speed;
        if speed != 1.0 {
            unsafe { *timescale *= speed.clamp(0.1, 4.0); }
        }
    }

    get_orig_fn!(AlterUpdateTimeline, AlterUpdateTimelineFn)(this, timescale);

    // 趁還在正確的執行緒上，把播放進度抓下來
    if !this.is_null() && unsafe { GETLIVECURRENTTIME_ADDR } != 0 {
        CURRENT_TIME.store(get_LiveCurrentTime(this).to_bits(), atomic::Ordering::Relaxed);
        TOTAL_TIME.store(get_LiveTotalTime(this).to_bits(), atomic::Ordering::Relaxed);
        LAST_TICK_MS.store(now_ms(), atomic::Ordering::Relaxed);
    }
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, "Gallop.Live", Director);

    unsafe {
        GETLIVECURRENTTIME_ADDR = get_method_addr(Director, c"get_LiveCurrentTime", 0);
        GETLIVETOTALTIME_ADDR = get_method_addr(Director, c"get_LiveTotalTime", 0);
    }

    let AlterUpdateTimeline_addr = get_method_addr(Director, c"AlterUpdateTimeline", 1);
    new_hook!(AlterUpdateTimeline_addr, AlterUpdateTimeline);
}
