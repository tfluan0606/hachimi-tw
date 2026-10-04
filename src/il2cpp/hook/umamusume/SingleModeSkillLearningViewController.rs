//! 育成技能學習頁的「自動學習」按鈕。
//!
//! 開技能頁時（`BeginView`）把「重置」鈕複製一份擺到「決定」左邊，字改成「一鍵學習」。按下去
//! 先跳遊戲原生的兩鈕對話框（主要／次要），選了之後：清掉目前的選擇 → 依目前設定檔的那份清單
//! （技能名稱或 ID）順序呼叫遊戲自己的 `SingleModeSkillLearningModel::Select`，點數不夠的跳過 →
//! 呼叫「決定」鈕本來的處理函式 `OnClickDecideButton`，跳出遊戲自己的確認視窗。要不要送 request
//! 仍由使用者在確認視窗決定，我們不碰任何封包。
//!
//! 簽章對過繁中服 dump：
//!
//! ```text
//! Gallop.SingleModeSkillLearningViewController::BeginView() -> System.Void
//! Gallop.SingleModeSkillLearningViewController::OnSelectUpdate() -> System.Void
//! Gallop.SingleModeSkillLearningViewController::OnClickDecideButton() -> System.Void
//! Gallop.SingleModeSkillLearningView::get_ResetButton() -> Gallop.ButtonCommon
//! Gallop.SingleModeSkillLearningModel::get_SkillInfoList() -> List<SingleModeSkillLearningSkillInfo>
//! Gallop.SingleModeSkillLearningModel::get_RemainingPoint() -> System.Int32
//! Gallop.SingleModeSkillLearningModel::Select / Deselect / IsSelect(PartsSingleModeSkillLearningListItem.Info)
//! Gallop.SingleModeSkillLearningSkillInfo::get_SkillList() -> List<PartsSingleModeSkillLearningListItem.Info>
//! Gallop.DialogCommon.Data::SetSimpleTwoButtonMessage(String header, String message, Action<DialogCommon> onRight,
//!     TextId leftTextId, TextId rightTextId, Action<DialogCommon> onLeft, DialogCommonBase.FormType) -> Data
//! Gallop.DialogCommon.Data::AddOpenCallback(Action<DialogCommon>) -> Void
//! Gallop.DialogManager::PushDialog(DialogCommon.Data) -> DialogCommon [static]
//! Gallop.DialogCommon::GetButtonObj(DialogCommon.ButtonIndex) -> ButtonCommon
//! ```
//!
//! controller 持有 model／view 的欄位名稱不在 dump 裡，改用「欄位型別」去找（往父類別找），
//! 不依賴名稱。

use std::{ffi::c_void, ptr::null_mut, sync::Mutex};

use widestring::Utf16Str;

use crate::{
    core::{Gui, Hachimi},
    il2cpp::{
        api::*,
        ext::{Il2CppObjectExt, Il2CppStringExt, StringExt},
        hook::UnityEngine_CoreModule::{Component, GameObject, Transform},
        symbols::{
            create_delegate, find_nested_class, get_assembly_image, get_class, get_method, get_method_addr,
            get_method_overload_addr, GCHandle, IList, Thread,
        },
        types::*,
    },
};

struct Classes {
    view: *mut Il2CppClass,
    model: *mut Il2CppClass,
    text_common: *mut Il2CppClass,
    unity_action: *mut Il2CppClass,
    button_clicked_event: *mut Il2CppClass,
    dialog_data: *mut Il2CppClass,
    /// `Action<DialogCommon>`（從 SetSimpleTwoButtonMessage 的參數型別取得的泛型實例）
    action_dialog: *mut Il2CppClass,
}
unsafe impl Send for Classes {}

static CLASSES: Mutex<Option<Classes>> = Mutex::new(None);
/// 目前開著的技能頁 controller（每次開頁換掉）
static CONTROLLER: Mutex<Option<GCHandle>> = Mutex::new(None);

static mut INSTANTIATE_ADDR: usize = 0;
static mut GET_TRANSFORM_ADDR: usize = 0;
static mut SET_TWO_BUTTON_ADDR: usize = 0;
static mut PUSH_DIALOG_ADDR: usize = 0;

/// 對話框用到的 enum 值（runtime 從 enum 欄位讀，不寫死數字）
#[derive(Clone, Copy, Default)]
struct DialogEnums {
    text_id: i32,
    form_type: i32,
    button_left: i32,
    button_right: i32,
}
static DIALOG_ENUMS: Mutex<Option<DialogEnums>> = Mutex::new(None);

const BUTTON_LABEL: &str = "一鍵學習";
const PRIMARY_LABEL: &str = "主要";
const SECONDARY_LABEL: &str = "次要";

// ---- 小工具 ----

/// 呼叫 `this` 上的無參數 instance method（method 由物件本身的 class 解析，含父類別）。
fn call0<R>(this: *mut Il2CppObject, name: &std::ffi::CStr) -> Option<R> {
    if this.is_null() {
        return None;
    }
    let addr = get_method_addr(unsafe { (*this).klass() }, name, 0);
    if addr == 0 {
        return None;
    }
    let f: extern "C" fn(*mut Il2CppObject, *const c_void) -> R = unsafe { std::mem::transmute(addr) };
    Some(f(this, null_mut()))
}

fn call1<A, R>(this: *mut Il2CppObject, name: &std::ffi::CStr, arg: A) -> Option<R> {
    if this.is_null() {
        return None;
    }
    let addr = get_method_addr(unsafe { (*this).klass() }, name, 1);
    if addr == 0 {
        return None;
    }
    let f: extern "C" fn(*mut Il2CppObject, A, *const c_void) -> R = unsafe { std::mem::transmute(addr) };
    Some(f(this, arg, null_mut()))
}

/// 在 `obj` 的 class（含父類別）找第一個型別為 `target` 的 instance 欄位，回傳其值。
fn find_field_object_by_type(obj: *mut Il2CppObject, target: *mut Il2CppClass) -> *mut Il2CppObject {
    if obj.is_null() || target.is_null() {
        return null_mut();
    }
    let mut klass = unsafe { (*obj).klass() };
    while !klass.is_null() {
        let mut iter: *mut c_void = null_mut();
        loop {
            let field = il2cpp_class_get_fields(klass, &mut iter);
            if field.is_null() {
                break;
            }
            const FIELD_ATTRIBUTE_STATIC: i32 = 0x0010;
            if il2cpp_field_get_flags(field) & FIELD_ATTRIBUTE_STATIC != 0 {
                continue;
            }
            let ty = il2cpp_field_get_type(field);
            if !ty.is_null() && il2cpp_class_from_type(ty) == target {
                return unsafe {
                    *((obj as *const u8).add(il2cpp_field_get_offset(field)) as *const *mut Il2CppObject)
                };
            }
        }
        klass = il2cpp_class_get_parent(klass);
    }
    null_mut()
}

fn type_object(class: *mut Il2CppClass) -> *mut Il2CppObject {
    il2cpp_type_get_object(il2cpp_class_get_type(class))
}

fn il2cpp_str(s: *mut Il2CppString) -> String {
    if s.is_null() {
        return String::new();
    }
    let s: &Utf16Str = unsafe { (*s).as_utf16str() };
    s.to_string()
}

fn notify(msg: String) {
    // 這裡跑在遊戲主執行緒，GUI 鎖丟到別的執行緒拿，避免跟渲染互卡
    std::thread::spawn(move || {
        if let Some(mutex) = Gui::instance() {
            mutex.lock().unwrap().show_notification(&msg);
        }
    });
}

/// 設定裡的一行：純數字當技能 ID，其餘當技能名稱（完全比對）
enum Wanted {
    Id(i32),
    Name(String),
}

fn wanted_list(secondary: bool) -> Vec<Wanted> {
    let config = Hachimi::instance().config.load();
    let Some(profile) = config.active_auto_skill_profile() else {
        return Vec::new();
    };
    let list = if secondary { &profile.secondary } else { &profile.primary };
    list
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| match s.parse::<i32>() {
            Ok(id) => Wanted::Id(id),
            Err(_) => Wanted::Name(s.to_owned()),
        })
        .collect()
}

// ---- 選技能 ----

struct Entry {
    info: *mut Il2CppObject,
    skill_info: *mut Il2CppObject,
    skill_id: i32,
    name: String,
}

/// 把 model 裡所有技能攤平，保留「同組（同一列）」的先後順序：○ 在 ◎ 前面。
fn collect_groups(model: *mut Il2CppObject) -> Vec<Vec<Entry>> {
    let mut groups = Vec::new();
    let Some(list) = call0::<*mut Il2CppObject>(model, c"get_SkillInfoList").and_then(IList::<*mut Il2CppObject>::new)
    else {
        return groups;
    };
    for skill_info in list.iter() {
        let Some(infos) = call0::<*mut Il2CppObject>(skill_info, c"get_SkillList").and_then(IList::<*mut Il2CppObject>::new)
        else {
            continue;
        };
        let mut group = Vec::new();
        for info in infos.iter() {
            if info.is_null() {
                continue;
            }
            group.push(Entry {
                info,
                skill_info,
                skill_id: call0::<i32>(info, c"get_SkillId").unwrap_or(0),
                name: il2cpp_str(call0::<*mut Il2CppString>(info, c"get_Name").unwrap_or(null_mut())),
            });
        }
        groups.push(group);
    }
    groups
}

/// 試著選進 `entry`；失敗（不能學、點數不夠）就退回原狀。
fn try_select(model: *mut Il2CppObject, entry: &Entry) -> bool {
    let info = entry.info;
    let mut available = call0::<bool>(info, c"get_IsAvailable").unwrap_or(false);
    if !available {
        // 上一階剛選進去時可用狀態可能還沒刷新
        call0::<()>(entry.skill_info, c"UpdateAvailabilityInfo");
        available = call0::<bool>(info, c"get_IsAvailable").unwrap_or(false);
    }
    let need = call0::<i32>(info, c"get_CurrentNeedPoint").unwrap_or(i32::MAX);
    let remain = call0::<i32>(model, c"get_RemainingPoint").unwrap_or(0);
    if !available || need > remain {
        debug!("[AutoSkill] skip {} ({}) available={} need={} remain={}", entry.name, entry.skill_id, available, need, remain);
        return false;
    }

    call1::<_, ()>(model, c"Select", info);
    let remain_after = call0::<i32>(model, c"get_RemainingPoint").unwrap_or(-1);
    if remain_after < 0 || !call1::<_, bool>(model, c"IsSelect", info).unwrap_or(false) {
        call1::<_, ()>(model, c"Deselect", info);
        debug!("[AutoSkill] rollback {} ({}) remain_after={}", entry.name, entry.skill_id, remain_after);
        return false;
    }
    info!("[AutoSkill] select {} ({}) need={} remain {} -> {}", entry.name, entry.skill_id, need, remain, remain_after);
    true
}

fn apply_primary() {
    apply_list(false);
}

fn apply_secondary() {
    apply_list(true);
}

fn apply_list(secondary: bool) {
    let controller = match CONTROLLER.lock().unwrap().as_ref() {
        Some(h) => h.target(),
        None => null_mut(),
    };
    let model_class = match CLASSES.lock().unwrap().as_ref() {
        Some(c) => c.model,
        None => return,
    };
    let model = find_field_object_by_type(controller, model_class);
    if model.is_null() {
        error!("[AutoSkill] model not found (controller={:p})", controller);
        notify("一鍵學習失敗：找不到技能資料".into());
        return;
    }

    let wanted = wanted_list(secondary);
    let label = if secondary { SECONDARY_LABEL } else { PRIMARY_LABEL };
    if wanted.is_empty() {
        notify(format!("{label}清單是空的，請到設定編輯器的「遊戲」分頁填寫"));
        return;
    }
    info!("[AutoSkill] apply {} list ({} entries)", label, wanted.len());

    call0::<()>(model, c"ResetSelected");
    let groups = collect_groups(model);

    let mut picked = 0;
    for w in &wanted {
        // 找到目標在哪一列、第幾階
        let found = groups.iter().find_map(|g| {
            g.iter()
                .position(|e| match w {
                    Wanted::Id(id) => e.skill_id == *id,
                    Wanted::Name(n) => e.name.trim() == n,
                })
                .map(|i| (g, i))
        });
        let Some((group, idx)) = found else { continue };

        // 同一列由低到高逐階選，跟按「＋」的行為一樣；任何一階失敗就把這次加的全部退掉
        let mut added = Vec::new();
        let mut ok = true;
        for e in &group[..=idx] {
            if call0::<bool>(e.info, c"get_IsAcquired").unwrap_or(false)
                || call1::<_, bool>(model, c"IsSelect", e.info).unwrap_or(false)
            {
                continue;
            }
            if try_select(model, e) {
                added.push(e.info);
            }
            else {
                ok = false;
                break;
            }
        }
        if ok {
            picked += added.len();
        }
        else {
            for info in added.into_iter().rev() {
                call1::<_, ()>(model, c"Deselect", info);
            }
        }
    }

    call0::<()>(controller, c"OnSelectUpdate");

    if picked == 0 || !call0::<bool>(model, c"get_HasSelectedSkill").unwrap_or(false) {
        notify(format!("{label}清單裡沒有現在能學的技能（或點數不夠）"));
        return;
    }
    call0::<()>(controller, c"OnClickDecideButton");
}

// ---- 主要／次要對話框 ----

fn make_action(callback: fn()) -> *mut Il2CppObject {
    let class = CLASSES.lock().unwrap().as_ref().map(|c| c.action_dialog).unwrap_or(null_mut());
    if class.is_null() {
        return null_mut();
    }
    create_delegate(class, 1, callback).map(|d| d as *mut Il2CppObject).unwrap_or(null_mut())
}

/// 對話框開好時把兩顆鈕的字換成「次要」「主要」（TextId 只是佔位，真正的字在這裡設）
extern "C" fn on_dialog_open(_target: *mut Il2CppObject, dialog: *mut Il2CppObject) {
    let Some(enums) = *DIALOG_ENUMS.lock().unwrap() else { return };
    let text_common = CLASSES.lock().unwrap().as_ref().map(|c| c.text_common).unwrap_or(null_mut());
    for (index, label) in [(enums.button_left, SECONDARY_LABEL), (enums.button_right, PRIMARY_LABEL)] {
        let button = call1::<i32, *mut Il2CppObject>(dialog, c"GetButtonObj", index).unwrap_or(null_mut());
        if button.is_null() {
            warn!("[AutoSkill] dialog button {} not found", index);
            continue;
        }
        let go = Component::get_gameObject(button);
        let text = GameObject::GetComponentInChildren(go, type_object(text_common), true);
        if !text.is_null() {
            call1::<_, ()>(text, c"set_text", label.to_il2cpp_string());
        }
    }
}

fn on_click_auto() {
    let config = Hachimi::instance().config.load();
    let Some(profile) = config.active_auto_skill_profile() else {
        notify("還沒有一鍵學習的設定檔，請到設定編輯器的「遊戲」分頁建立".into());
        return;
    };
    let message = format!(
        "設定檔：{}\n主要 {} 項／次要 {} 項\n\n要套用哪一份清單？",
        profile.name,
        profile.primary.iter().filter(|s| !s.trim().is_empty()).count(),
        profile.secondary.iter().filter(|s| !s.trim().is_empty()).count(),
    );

    let (Some(enums), Some(data_class)) = (
        *DIALOG_ENUMS.lock().unwrap(),
        CLASSES.lock().unwrap().as_ref().map(|c| c.dialog_data).filter(|c| !c.is_null()),
    ) else {
        // 對話框準備失敗時退回直接套主要清單，至少功能還能用
        apply_list(false);
        return;
    };

    let on_right = make_action(apply_primary);
    let on_left = make_action(apply_secondary);
    // 開啟回呼要拿到 DialogCommon 參數，簽章跟 fn() 不同；delegate 呼叫時會傳 (target, dialog, method)
    let on_open = make_action(unsafe {
        std::mem::transmute::<extern "C" fn(*mut Il2CppObject, *mut Il2CppObject), fn()>(on_dialog_open)
    });
    if on_right.is_null() || on_left.is_null() {
        error!("[AutoSkill] failed to create dialog callbacks");
        apply_list(false);
        return;
    }

    let data = il2cpp_object_new(data_class);
    call0::<()>(data, c".ctor");
    let set_two_button: extern "C" fn(
        *mut Il2CppObject, *mut Il2CppString, *mut Il2CppString, *mut Il2CppObject, i32, i32, *mut Il2CppObject, i32,
        *const c_void,
    ) -> *mut Il2CppObject = unsafe { std::mem::transmute(SET_TWO_BUTTON_ADDR) };
    set_two_button(
        data,
        BUTTON_LABEL.to_il2cpp_string(),
        message.to_il2cpp_string(),
        on_right,
        enums.text_id,
        enums.text_id,
        on_left,
        enums.form_type,
        null_mut(),
    );
    if !on_open.is_null() {
        call1::<_, ()>(data, c"AddOpenCallback", on_open);
    }

    let push_dialog: extern "C" fn(*mut Il2CppObject, *const c_void) -> *mut Il2CppObject =
        unsafe { std::mem::transmute(PUSH_DIALOG_ADDR) };
    let dialog = push_dialog(data, null_mut());
    info!("[AutoSkill] dialog pushed: {:p}", dialog);
}

/// 讀 enum 常數的值；`names` 空＝回傳第一個。找不到就把前幾個名稱印出來方便對照
fn enum_value(class: *mut Il2CppClass, names: &[&str]) -> Option<i32> {
    let mut found_names = Vec::new();
    let mut iter: *mut c_void = null_mut();
    loop {
        let field = il2cpp_class_get_fields(class, &mut iter);
        if field.is_null() {
            break;
        }
        const FIELD_ATTRIBUTE_LITERAL: i32 = 0x0040;
        if il2cpp_field_get_flags(field) & FIELD_ATTRIBUTE_LITERAL == 0 {
            continue;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(il2cpp_field_get_name(field)) }.to_string_lossy().into_owned();
        if names.is_empty() || names.iter().any(|n| *n == name) {
            let mut value: i32 = 0;
            il2cpp_field_static_get_value(field, &mut value as *mut i32 as *mut c_void);
            return Some(value);
        }
        if found_names.len() < 20 {
            found_names.push(name);
        }
    }
    warn!("[AutoSkill] enum {:?} not found; first names: {:?}", names, found_names);
    None
}

// ---- 按鈕 ----

fn add_button(controller: *mut Il2CppObject) {
    let (view_class, text_common, unity_action, clicked_event) = match CLASSES.lock().unwrap().as_ref() {
        Some(c) => (c.view, c.text_common, c.unity_action, c.button_clicked_event),
        None => return,
    };

    let view = find_field_object_by_type(controller, view_class);
    info!("[AutoSkill] view={:p}", view);
    let reset_button = call0::<*mut Il2CppObject>(view, c"get_ResetButton").unwrap_or(null_mut());
    if reset_button.is_null() {
        error!("[AutoSkill] reset button not found (view={:p})", view);
        return;
    }

    // Component 專用（傳 GameObject 進去會讀錯物件直接 crash）
    let get_transform: extern "C" fn(*mut Il2CppObject) -> *mut Il2CppObject =
        unsafe { std::mem::transmute(GET_TRANSFORM_ADDR) };
    let instantiate: extern "C" fn(*mut Il2CppObject, *mut Il2CppObject, bool, *const c_void) -> *mut Il2CppObject =
        unsafe { std::mem::transmute(INSTANTIATE_ADDR) };

    // 直接複製 ButtonCommon 這個 component：Unity 會複製整個 GameObject，回傳複製品上的 ButtonCommon，
    // 之後 transform／gameObject 都從它拿
    let reset_tr = get_transform(reset_button);
    let parent = Transform::get_parent(reset_tr);
    info!("[AutoSkill] reset_button={:p} parent={:p}", reset_button, parent);
    let button = instantiate(reset_button, parent, false, null_mut());
    if button.is_null() {
        error!("[AutoSkill] Instantiate failed");
        return;
    }
    let clone_go = Component::get_gameObject(button);
    info!("[AutoSkill] cloned button={:p} go={:p}", button, clone_go);

    // 擺到「決定」左邊：「重置」在右邊，對中線鏡射過去
    let clone_tr = get_transform(button);
    let mut pos = Transform::get_localPosition(reset_tr);
    info!("[AutoSkill] reset button localPosition = ({}, {}, {})", pos.x, pos.y, pos.z);
    pos.x = -pos.x;
    Transform::set_localPosition(clone_tr, pos);

    // 換字
    let text = GameObject::GetComponentInChildren(clone_go, type_object(text_common), true);
    info!("[AutoSkill] label text={:p}", text);
    if !text.is_null() {
        call1::<_, ()>(text, c"set_text", BUTTON_LABEL.to_il2cpp_string());
    }

    // 換掉 onClick：先整個換成新的空事件（連 prefab 裡序列化的 listener 一起清掉），再掛我們的
    let event = il2cpp_object_new(clicked_event);
    call0::<()>(event, c".ctor");
    call1::<_, ()>(button, c"set_onClick", event);
    match create_delegate(unity_action, 0, on_click_auto) {
        Some(d) => {
            call1::<_, ()>(button, c"SetOnClick", d);
        }
        None => error!("[AutoSkill] failed to create UnityAction"),
    }
    info!("[AutoSkill] button added");
}

type BeginViewFn = extern "C" fn(this: *mut Il2CppObject);
extern "C" fn BeginView(this: *mut Il2CppObject) {
    get_orig_fn!(BeginView, BeginViewFn)(this);

    info!("[AutoSkill] BeginView controller={:p}", this);
    *CONTROLLER.lock().unwrap() = Some(GCHandle::new(this, false));
    add_button(this);
}

// ---- 匯入目前技能頁的技能（給設定編輯器用） ----

/// `None`＝還沒有結果；`Some(Err)`＝失敗原因
static IMPORT_RESULT: Mutex<Option<Result<Vec<String>, String>>> = Mutex::new(None);

/// 要求在主執行緒讀一次「最後開過的技能頁」上還沒學的技能，結果用 [`take_import_result`] 拿。
pub fn request_import() {
    *IMPORT_RESULT.lock().unwrap() = None;
    Thread::main_thread().schedule(|| {
        *IMPORT_RESULT.lock().unwrap() = Some(read_learnable());
    });
}

pub fn take_import_result() -> Option<Result<Vec<String>, String>> {
    IMPORT_RESULT.lock().unwrap().take()
}

fn read_learnable() -> Result<Vec<String>, String> {
    let controller = match CONTROLLER.lock().unwrap().as_ref() {
        Some(h) => h.target(),
        None => null_mut(),
    };
    if controller.is_null() {
        return Err("還沒開過技能學習頁，先進一次技能頁再匯入".into());
    }
    let model_class = CLASSES.lock().unwrap().as_ref().map(|c| c.model).unwrap_or(null_mut());
    let model = find_field_object_by_type(controller, model_class);
    if model.is_null() {
        return Err("找不到技能資料".into());
    }

    // 照遊戲列表的順序；已學會的不列
    let mut names = Vec::new();
    for group in collect_groups(model) {
        for e in group {
            if e.name.is_empty() || call0::<bool>(e.info, c"get_IsAcquired").unwrap_or(false) {
                continue;
            }
            names.push(e.name.trim().to_owned());
        }
    }
    info!("[AutoSkill] import: {} skills", names.len());
    Ok(names)
}

pub fn init(umamusume: *const Il2CppImage) {
    get_class_or_return!(umamusume, Gallop, SingleModeSkillLearningViewController);
    get_class_or_return!(umamusume, Gallop, SingleModeSkillLearningView);
    get_class_or_return!(umamusume, Gallop, SingleModeSkillLearningModel);
    get_class_or_return!(umamusume, Gallop, TextCommon);

    // 主要／次要對話框（失敗不影響按鈕本身，按了會退回直接套主要清單）
    let mut dialog_data = null_mut();
    let mut action_dialog = null_mut();
    let dialog_lookup = || -> Result<(*mut Il2CppClass, *mut Il2CppClass, DialogEnums), crate::core::Error> {
        let dialog_common = get_class(umamusume, c"Gallop", c"DialogCommon")?;
        let dialog_common_base = get_class(umamusume, c"Gallop", c"DialogCommonBase")?;
        let dialog_manager = get_class(umamusume, c"Gallop", c"DialogManager")?;
        let text_id = get_class(umamusume, c"Gallop", c"TextId")?;
        let data = find_nested_class(dialog_common, c"Data")?;
        let button_index = find_nested_class(dialog_common, c"ButtonIndex")?;
        let form_type = find_nested_class(dialog_common_base, c"FormType")?;

        let set_two = get_method(data, c"SetSimpleTwoButtonMessage", 7)?;
        let action = il2cpp_class_from_type(il2cpp_method_get_param(set_two, 2));
        unsafe {
            SET_TWO_BUTTON_ADDR = (*set_two).methodPointer;
            PUSH_DIALOG_ADDR = get_method_addr(dialog_manager, c"PushDialog", 1);
        }

        let enums = DialogEnums {
            text_id: enum_value(text_id, &["Common0004", "Common0001"]).or_else(|| enum_value(text_id, &[])).unwrap_or(0),
            form_type: enum_value(form_type, &["SMALL_TWO_BUTTON", "MIDDLE_TWO_BUTTON"]).or_else(|| enum_value(form_type, &[])).unwrap_or(0),
            button_left: enum_value(button_index, &["Left", "LEFT"]).unwrap_or(0),
            button_right: enum_value(button_index, &["Right", "RIGHT"]).unwrap_or(2),
        };
        info!(
            "[AutoSkill] dialog enums: text_id={} form_type={} left={} right={}",
            enums.text_id, enums.form_type, enums.button_left, enums.button_right
        );
        Ok((data, action, enums))
    };
    match dialog_lookup() {
        Ok((data, action, enums)) if unsafe { PUSH_DIALOG_ADDR != 0 } && !action.is_null() => {
            dialog_data = data;
            action_dialog = action;
            *DIALOG_ENUMS.lock().unwrap() = Some(enums);
        }
        Ok(_) => error!("[AutoSkill] dialog setup incomplete"),
        Err(e) => error!("[AutoSkill] dialog setup: {}", e),
    }

    let lookup = || -> Result<(*mut Il2CppClass, *mut Il2CppClass, *mut Il2CppClass), crate::core::Error> {
        let core = get_assembly_image(c"UnityEngine.CoreModule.dll")?;
        let ui = get_assembly_image(c"UnityEngine.UI.dll")?;
        let unity_action = get_class(core, c"UnityEngine.Events", c"UnityAction")?;
        let object = get_class(core, c"UnityEngine", c"Object")?;
        let button = get_class(ui, c"UnityEngine.UI", c"Button")?;
        let clicked_event = find_nested_class(button, c"ButtonClickedEvent")?;
        Ok((unity_action, object, clicked_event))
    };
    let (unity_action, object, clicked_event) = match lookup() {
        Ok(v) => v,
        Err(e) => {
            error!("[AutoSkill] {}", e);
            return;
        }
    };

    unsafe {
        // 非泛型的 Instantiate(Object, Transform, Boolean)；泛型版的第一個參數是 MVAR，不會撞到
        INSTANTIATE_ADDR = get_method_overload_addr(
            object,
            "Instantiate",
            &[Il2CppTypeEnum_IL2CPP_TYPE_CLASS, Il2CppTypeEnum_IL2CPP_TYPE_CLASS, Il2CppTypeEnum_IL2CPP_TYPE_BOOLEAN],
        );
        GET_TRANSFORM_ADDR = il2cpp_resolve_icall(c"UnityEngine.Component::get_transform()".as_ptr());
        if GET_TRANSFORM_ADDR == 0 {
            if let Ok(component) = get_assembly_image(c"UnityEngine.CoreModule.dll")
                .and_then(|core| get_class(core, c"UnityEngine", c"Component"))
            {
                GET_TRANSFORM_ADDR = get_method_addr(component, c"get_transform", 0);
            }
        }
        if INSTANTIATE_ADDR == 0 || GET_TRANSFORM_ADDR == 0 {
            error!("[AutoSkill] Instantiate/get_transform not found");
            return;
        }
    }

    *CLASSES.lock().unwrap() = Some(Classes {
        view: SingleModeSkillLearningView,
        model: SingleModeSkillLearningModel,
        text_common: TextCommon,
        unity_action,
        button_clicked_event: clicked_event,
        dialog_data,
        action_dialog,
    });

    let BeginView_addr = get_method_addr(SingleModeSkillLearningViewController, c"BeginView", 0);
    new_hook!(BeginView_addr, BeginView);
}
