//! 技能資料說明（skill_data_desc）的換行。
//!
//! 遊戲的 `LineHeadWrapCommon` 照字數硬切、插換行，會把我們塞的 `<color=…>`／`<b>` 標籤從中間切斷
//! （標籤一斷，整段 rich text 都變成原樣印出），而且照全形字數估算、每行都提早換行。
//! 所以「我們產生的說明文字」（[`sql::is_skill_data_desc_text`]）一律不插換行、原樣回傳，
//! 交給文字元件依實際寬度自動折行（實測會填滿整行）；其他文字照舊走遊戲原本的換行。

use crate::{core::Hachimi, il2cpp::{ext::Il2CppStringExt, sql, symbols::get_method_addr, types::*}};

/// 是我們的說明就原樣回傳（不插換行），否則 None（交給原函式）。
fn skip_wrap(s: *mut Il2CppString) -> Option<*mut Il2CppString> {
    if s.is_null() || !Hachimi::instance().config.load().skill_data_desc {
        return None;
    }
    let text = unsafe { (*s).as_utf16str() }.to_string();
    sql::is_skill_data_desc_text(&text).then_some(s)
}

type LineHeadWrapCommonFn = extern "C" fn(
    s: *mut Il2CppString, line_char_count: i32, handling_type: i32, is_match_delegate: *mut Il2CppDelegate
) -> *mut Il2CppString;
extern "C" fn LineHeadWrapCommon(
    s: *mut Il2CppString, line_char_count: i32, handling_type: i32, is_match_delegate: *mut Il2CppDelegate
) -> *mut Il2CppString {
    if let Some(unwrapped) = skip_wrap(s) {
        return unwrapped;
    }
    get_orig_fn!(LineHeadWrapCommon, LineHeadWrapCommonFn)(s, line_char_count, handling_type, is_match_delegate)
}

type LineHeadWrapCommonWithColorTagFn = extern "C" fn(
    s: *mut Il2CppString, line_char_count: i32, is_count_single_char: bool, is_match_delegate: *mut Il2CppDelegate
) -> *mut Il2CppString;
extern "C" fn LineHeadWrapCommonWithColorTag(
    s: *mut Il2CppString, line_char_count: i32, is_count_single_char: bool, is_match_delegate: *mut Il2CppDelegate
) -> *mut Il2CppString {
    if let Some(unwrapped) = skip_wrap(s) {
        return unwrapped;
    }
    get_orig_fn!(LineHeadWrapCommonWithColorTag, LineHeadWrapCommonWithColorTagFn)(
        s, line_char_count, is_count_single_char, is_match_delegate
    )
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, GallopUtil);

    let LineHeadWrapCommon_addr = get_method_addr(GallopUtil, c"LineHeadWrapCommon", 4);
    let LineHeadWrapCommonWithColorTag_addr = get_method_addr(GallopUtil, c"LineHeadWrapCommonWithColorTag", 4);

    new_hook!(LineHeadWrapCommon_addr, LineHeadWrapCommon);
    new_hook!(LineHeadWrapCommonWithColorTag_addr, LineHeadWrapCommonWithColorTag);
}

