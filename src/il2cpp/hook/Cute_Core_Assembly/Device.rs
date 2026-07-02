use crate::il2cpp::{symbols::get_method_addr, types::*};

type IsIllegalUserFn = extern "C" fn() -> bool;

// 保險 + 可觀察版：平常透明放行（實測 TW PC 正常流程根本不呼叫它、或回 false）；
// 只有在原函式「真的想回 true（想 flag 我們）」時才介入——強制 false 並留 log。
// 正常情況零行為改變；萬一某個沒測到的流程觸發它，才自動中和。
extern "C" fn IsIllegalUser() -> bool {
    let orig = get_orig_fn!(IsIllegalUser, IsIllegalUserFn)();
    if orig {
        warn!("[DISGUISE] Cute.Core.Device::IsIllegalUser -> true, forcing false");
        return false;
    }
    orig
}

pub fn init(Cute_Core_Assembly: *const Il2CppImage) {
    get_class_or_return!(Cute_Core_Assembly, "Cute.Core", Device);

    let IsIllegalUser_addr = get_method_addr(Device, c"IsIllegalUser", 0);

    new_hook!(IsIllegalUser_addr, IsIllegalUser);
}
