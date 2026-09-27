use crate::{core::Hachimi, il2cpp::{ext::{Il2CppStringExt, LocalizedDataExt, StringExt}, hook::UnityEngine_UI::Text, sql, symbols::get_method_addr, types::*}};

type AwakeFn = extern "C" fn(this: *mut Il2CppObject);
extern "C" fn Awake(this: *mut Il2CppObject) {
    get_orig_fn!(Awake, AwakeFn)(this);

    let localized_data = Hachimi::instance().localized_data.load();

    let font = localized_data.load_replacement_font();
    if !font.is_null() {
        Text::set_font(this, font);
    }

    if localized_data.config.text_common_allow_overflow {
        Text::set_horizontalOverflow(this, 1);
        Text::set_verticalOverflow(this, 1);
    }
}

/// 技能資料說明（skill_data_desc）比原文長，縮小到原字級的這個比例才塞得下。
const SKILL_DESC_FONT_SCALE: f32 = 0.95;

// 台服的文字元件是舊版 UI.Text：rich text 的 <size> 只吃絕對字級（寫 <size=90%> 會被當成 90），
// 所以在設定文字的當下讀這個元件的 fontSize，換算後包 <size=N>。只包我們產生的說明。
type SetTextFn = extern "C" fn(this: *mut Il2CppObject, value: *mut Il2CppString);
extern "C" fn set_text(this: *mut Il2CppObject, value: *mut Il2CppString) {
    if !value.is_null() && Hachimi::instance().config.load().skill_data_desc {
        let text = unsafe { (*value).as_utf16str() }.to_string();
        if sql::is_skill_data_desc_text(&text) {
            let size = (Text::get_fontSize(this) as f32 * SKILL_DESC_FONT_SCALE).round() as i32;
            if size > 0 {
                let sized = format!("<size={size}>{text}</size>").to_il2cpp_string();
                return get_orig_fn!(set_text, SetTextFn)(this, sized);
            }
        }
    }
    get_orig_fn!(set_text, SetTextFn)(this, value);
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, TextCommon);

    let Awake_addr = get_method_addr(TextCommon, c"Awake", 0);

    new_hook!(Awake_addr, Awake);

    let set_text_addr = get_method_addr(TextCommon, c"set_text", 1);
    new_hook!(set_text_addr, set_text);
}