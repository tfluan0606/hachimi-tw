//! 育成訓練演出加速（按下訓練後的 3D 演出、友情訓練 cut-in）。
//!
//! 這些演出不是 DOTween，`ui_animation_scale` 管不到，速度全由遊戲自己的
//! 「演出速度」設定（`StoryManager.GetSavedHighSpeedSetting`）決定：
//!
//! - 3D 訓練演出（Cutt timeline）：`SingleModeMainTrainingCuttController.FixedUpdateTrainingCutIn`
//!   每個 FixedUpdate 讀 helper 的虛擬方法 `GetTargetSpeed()`，大於 1 就呼叫
//!   `CutInHelper.FixedUpdateForHighSpeed(speed)`，在同一幀多跑 `(int)speed - 1` 次 timeline 步進
//!   （每步固定時間，跑到結尾前一格就停）。原速時回 1.0；高速設定＝2 時回 `總長 / 0.5`，
//!   也就是整段壓成約 0.5 秒。
//! - 友情訓練 cut-in：`get_MotionSpeed` 只有「高速模式＋超高速略過」都開時回 2.0，否則 1.0，
//!   拿去當 cut-in 動作、開頭文字 flash、粒子的播放速度。
//!
//! 所以加速做法就是把這兩個回傳值乘上倍率。步進次數會被遊戲取整數，3D 演出的倍率只有整數有效。

use crate::{core::Hachimi, il2cpp::{symbols::get_method_addr, types::*}};

fn speed_mult() -> f32 {
    Hachimi::instance().config.load().training_cut_speed
}

type GetTargetSpeedFn = extern "C" fn(this: *mut Il2CppObject) -> f32;
extern "C" fn GetTargetSpeed(this: *mut Il2CppObject) -> f32 {
    let res = get_orig_fn!(GetTargetSpeed, GetTargetSpeedFn)(this);
    let mult = speed_mult();
    if mult > 1.0 {
        // 原速回 1.0 時遊戲不走高速步進，所以先墊到 1 再乘
        res.max(1.0) * mult
    }
    else {
        res
    }
}

type MainViewTagMotionSpeedFn = extern "C" fn(this: *mut Il2CppObject) -> f32;
extern "C" fn MainViewTag_get_MotionSpeed(this: *mut Il2CppObject) -> f32 {
    let res = get_orig_fn!(MainViewTag_get_MotionSpeed, MainViewTagMotionSpeedFn)(this);
    let mult = speed_mult();
    if mult > 1.0 { res * mult } else { res }
}

// 無人島劇本的友情 cut-in 是另一個類別，邏輯一樣
type PioneerTagMotionSpeedFn = extern "C" fn(this: *mut Il2CppObject) -> f32;
extern "C" fn PioneerTag_get_MotionSpeed(this: *mut Il2CppObject) -> f32 {
    let res = get_orig_fn!(PioneerTag_get_MotionSpeed, PioneerTagMotionSpeedFn)(this);
    let mult = speed_mult();
    if mult > 1.0 { res * mult } else { res }
}

fn init_training_cut(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, SingleModeTrainingCutInHelper);

    let GetTargetSpeed_addr = get_method_addr(SingleModeTrainingCutInHelper, c"GetTargetSpeed", 0);
    new_hook!(GetTargetSpeed_addr, GetTargetSpeed);
}

fn init_main_view_tag(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, SingleModeMainViewTagTrainingCutInPlayer);

    let MainViewTag_get_MotionSpeed_addr = get_method_addr(SingleModeMainViewTagTrainingCutInPlayer, c"get_MotionSpeed", 0);
    new_hook!(MainViewTag_get_MotionSpeed_addr, MainViewTag_get_MotionSpeed);
}

fn init_pioneer_tag(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, SingleModePioneerTagTrainingCutInPlayer);

    let PioneerTag_get_MotionSpeed_addr = get_method_addr(SingleModePioneerTagTrainingCutInPlayer, c"get_MotionSpeed", 0);
    new_hook!(PioneerTag_get_MotionSpeed_addr, PioneerTag_get_MotionSpeed);
}

pub fn init(umamusume: *const Il2CppImage) {
    init_training_cut(umamusume);
    init_main_view_tag(umamusume);
    init_pioneer_tag(umamusume);
}
