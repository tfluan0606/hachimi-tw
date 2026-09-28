use crate::{core::Hachimi, il2cpp::{api::il2cpp_object_get_class, ext::{Il2CppStringExt, LocalizedDataExt, StringExt}, hook::UnityEngine_UI::Text, sql, symbols::{get_method_addr, get_method_addr_cached}, types::*}};

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
/// 技能資料說明（skill_data_desc）放不下（會被截斷）時，依序試這些字級比例，放得下就停（最小 75%）。
const SKILL_DESC_FONT_SCALES: [f32; 4] = [0.9, 0.85, 0.8, 0.75];

/// 這個 Text 目前的內容需要的高度是否超過它的框（= 會被截斷）。
/// 方法從物件本身的類別往上找（il2cpp 會沿父類別查），取不到就當作放得下。
fn text_overflows(this: *mut Il2CppObject) -> bool {
    type PreferredHeightFn = extern "C" fn(this: *mut Il2CppObject) -> f32;
    type RectTransformFn = extern "C" fn(this: *mut Il2CppObject) -> *mut Il2CppObject;
    type GetRectFn = extern "C" fn(this: *mut Il2CppObject) -> Rect_t;

    let class = il2cpp_object_get_class(this);
    let preferred_addr = get_method_addr_cached(class, c"get_preferredHeight", 0);
    let rect_transform_addr = get_method_addr_cached(class, c"get_rectTransform", 0);
    if preferred_addr == 0 || rect_transform_addr == 0 {
        return false;
    }
    let preferred = unsafe { std::mem::transmute::<usize, PreferredHeightFn>(preferred_addr) }(this);
    let rect_transform = unsafe { std::mem::transmute::<usize, RectTransformFn>(rect_transform_addr) }(this);
    if rect_transform.is_null() {
        return false;
    }
    let get_rect_addr = get_method_addr_cached(il2cpp_object_get_class(rect_transform), c"get_rect", 0);
    if get_rect_addr == 0 {
        return false;
    }
    let rect = unsafe { std::mem::transmute::<usize, GetRectFn>(get_rect_addr) }(rect_transform);
    rect.height > 0.0 && preferred > rect.height + 0.5
}

// 台服的文字元件是舊版 UI.Text：rich text 的 <size> 只吃絕對字級（寫 <size=90%> 會被當成 90），
// 所以要縮就在設定文字的當下讀這個元件的 fontSize 換算後包 <size=N>。
// 只處理我們產生的說明：先照原字級放，放不下（會被截斷）才縮小重放。
type SetTextFn = extern "C" fn(this: *mut Il2CppObject, value: *mut Il2CppString);
extern "C" fn set_text(this: *mut Il2CppObject, value: *mut Il2CppString) {
    get_orig_fn!(set_text, SetTextFn)(this, value);

    if value.is_null() || !Hachimi::instance().config.load().skill_data_desc {
        return;
    }
    let text = unsafe { (*value).as_utf16str() }.to_string();
    if !sql::is_skill_data_desc_text(&text) || !text_overflows(this) {
        return;
    }
    let font_size = Text::get_fontSize(this) as f32;
    for scale in SKILL_DESC_FONT_SCALES {
        let size = (font_size * scale).round() as i32;
        if size <= 0 {
            break;
        }
        let sized = format!("<size={size}>{text}</size>").to_il2cpp_string();
        get_orig_fn!(set_text, SetTextFn)(this, sized);
        if !text_overflows(this) {
            break;
        }
    }
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, TextCommon);

    let Awake_addr = get_method_addr(TextCommon, c"Awake", 0);

    new_hook!(Awake_addr, Awake);

    let set_text_addr = get_method_addr(TextCommon, c"set_text", 1);
    new_hook!(set_text_addr, set_text);
}