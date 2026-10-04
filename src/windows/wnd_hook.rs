use std::{os::raw::c_uint, sync::atomic::{self, AtomicBool, AtomicI32, AtomicIsize, AtomicU32}};

use egui::mutex::Mutex;
use once_cell::sync::Lazy;
use widestring::U16CString;
use windows::{core::{w, PCWSTR}, Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Globalization::HIMC,
    UI::Input::Ime::{ImmAssociateContext, ImmAssociateContextEx, IACE_DEFAULT},
    UI::Input::{GetRawInputData, HRAWINPUT, RAWINPUTHEADER, RID_HEADER, RIM_TYPEKEYBOARD},
    System::Threading::GetCurrentThreadId,
    UI::Input::KeyboardAndMouse::{GetKeyNameTextW, MapVirtualKeyW, MAPVK_VK_TO_VSC},
    UI::WindowsAndMessaging::{
        CallNextHookEx, DefWindowProcW, FindWindowW, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
        PostMessageW, SetWindowsHookExW, SetWindowTextW, UnhookWindowsHookEx, GWLP_WNDPROC, HCBT_MINMAX, HHOOK,
        SW_RESTORE, WH_CBT, WM_APP, WM_CLOSE, WM_INPUT, WM_KEYDOWN, WM_SYSKEYDOWN, WM_SIZE, WNDPROC
    }
}};

use crate::{core::{game::Region, Gui, Hachimi}, il2cpp::{api::il2cpp_resolve_icall, hook::UnityEngine_CoreModule, symbols::Thread}, windows::utils};

use super::gui_impl::input;

/// 自訂訊息：請擁有視窗的執行緒去套用視窗標題。
const WM_HACHIMI_APPLY_TITLE: c_uint = WM_APP + 1;
/// 同上，套用「視窗置頂」。`wparam` 非 0 ＝置頂。
const WM_HACHIMI_APPLY_TOPMOST: c_uint = WM_APP + 2;
/// GUI 輸入框焦點變了，開關輸入法。`wparam` 非 0 ＝要打開。見 set_ime_wanted。
const WM_HACHIMI_SET_IME: c_uint = WM_APP + 3;

struct WndProcCall {
    hwnd: HWND,
    umsg: c_uint,
    wparam: WPARAM,
    lparam: LPARAM
}

// WM_SIZE 放行閘門。啟動極早期（第一次 Present 前）先緩衝 WM_SIZE，避免早期 init 出問題；
// 一旦開始 Present 就永久放行並補送緩衝內容。
//
// 原設計靠 SceneManager::ChangeView hook 偵測 splash 畫面來放行，但 TW(Komoe) client 的
// ChangeView 簽名不同、hook 失敗(log: "ChangeView_addr is null") → SPLASH_SHOWN 永遠 false
// → 每個 WM_SIZE 都被吞掉、Unity 收不到視窗縮放通知 → 輸入座標用舊尺寸映射 → 視窗縮放後點擊偏移。
// 改用「第一次 Present」當放行訊號，不再依賴那個會掛失敗的 hook。
static SIZE_READY: AtomicBool = AtomicBool::new(false);
pub fn mark_size_ready() {
    if SIZE_READY.swap(true, atomic::Ordering::AcqRel) {
        return;
    }
    drain_wm_size_buffer();
}

static WM_SIZE_BUFFER: Lazy<Mutex<Vec<WndProcCall>>> = Lazy::new(|| Mutex::default());
/// IME 事件依到達順序排隊（見 wndproc 裡的說明）
static IME_QUEUE: Lazy<Mutex<Vec<input::ImeInput>>> = Lazy::new(|| Mutex::default());
pub fn drain_wm_size_buffer() {
    let Some(orig_fn) = (unsafe { std::mem::transmute::<isize, WNDPROC>(WNDPROC_ORIG) }) else {
        return;
    };
    for call in WM_SIZE_BUFFER.lock().drain(..) {
        unsafe { orig_fn(call.hwnd, call.umsg, call.wparam, call.lparam); }
    }
}

// 熱鍵改鍵。直接在 wndproc 抓原始 VK code，不走 egui 的鍵位對應——那張表不完整，
// 而且我們要能設成任何鍵，包含 egui 不認得的。
static CAPTURING_KEY: AtomicBool = AtomicBool::new(false);
/// 抓到的 VK；`u32::MAX` 代表還沒抓到。
static CAPTURED_KEY: AtomicU32 = AtomicU32::new(u32::MAX);

pub fn begin_key_capture() {
    CAPTURED_KEY.store(u32::MAX, atomic::Ordering::Release);
    CAPTURING_KEY.store(true, atomic::Ordering::Release);
}

pub fn cancel_key_capture() {
    CAPTURING_KEY.store(false, atomic::Ordering::Release);
}

pub fn is_capturing_key() -> bool {
    CAPTURING_KEY.load(atomic::Ordering::Acquire)
}

/// 取走抓到的按鍵（只會回傳一次）。
pub fn take_captured_key() -> Option<u16> {
    let key = CAPTURED_KEY.swap(u32::MAX, atomic::Ordering::AcqRel);
    (key != u32::MAX).then_some(key as u16)
}

/// 按鍵的顯示名稱。用系統的鍵盤配置去問，所以會跟著使用者的鍵盤語系走。
pub fn key_display_name(vk: u16) -> String {
    unsafe {
        let scan_code = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC);
        if scan_code != 0 {
            let mut buf = [0u16; 64];
            // 方向鍵那類 extended key 要把 bit24 設起來，否則會拿到數字鍵盤的名字
            let extended = matches!(vk as u32, 0x21..=0x28 | 0x2D | 0x2E | 0x24 | 0x23);
            let lparam = ((scan_code as i32) << 16) | if extended { 1 << 24 } else { 0 };
            let len = GetKeyNameTextW(lparam, &mut buf);
            if len > 0 {
                return String::from_utf16_lossy(&buf[..len as usize]);
            }
        }
    }
    format!("VK {}", vk)
}

static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);
pub fn get_target_hwnd() -> HWND {
    HWND(TARGET_HWND.load(atomic::Ordering::Relaxed))
}

/// GUI 的輸入框有沒有焦點（上一次送出的狀態，避免每幀都 post）
static IME_WANTED: AtomicBool = AtomicBool::new(false);
/// 是我們替遊戲打開的輸入法（離開輸入框時要關回去）
static IME_ENABLED_BY_US: AtomicBool = AtomicBool::new(false);

/// GUI 每幀呼叫：輸入框取得／失去焦點時通知視窗執行緒開關輸入法。
///
/// Unity 在遊戲內沒有選中輸入框時會把視窗的 IME context 拿掉，這時 Windows 根本不送組字訊息，
/// 注音只會變成英文字母。Imm* 綁視窗執行緒，所以跟視窗標題一樣 post 過去做。
pub fn set_ime_wanted(wanted: bool) {
    if IME_WANTED.swap(wanted, atomic::Ordering::AcqRel) == wanted {
        return;
    }
    let hwnd = get_target_hwnd();
    if hwnd.0 != 0 {
        unsafe { _ = PostMessageW(hwnd, WM_HACHIMI_SET_IME, WPARAM(wanted as usize), LPARAM(0)); }
    }
}

/// 打開輸入法前遊戲原本的 `Input.imeCompositionMode`（Auto=0 / On=1 / Off=2），關回去時還原
static IME_PREV_MODE: AtomicI32 = AtomicI32::new(0);

/// 在視窗執行緒（＝Unity 主執行緒）上開關輸入法。
///
/// 只用 Win32 `ImmAssociateContextEx` 不夠：Unity 下一幀就會依 `Input.imeCompositionMode`
/// 把 context 拿掉（實測打開後下一個按鍵 `ImmGetContext` 就是空的）。所以主要靠 Unity 自己的
/// API 設成 On，Win32 那步只是讓它立即生效。
fn apply_ime(hwnd: HWND, wanted: bool) {
    static GET_MODE: Lazy<usize> = Lazy::new(|| il2cpp_resolve_icall(
        c"UnityEngine.Input::get_imeCompositionMode()".as_ptr()));
    static SET_MODE: Lazy<usize> = Lazy::new(|| il2cpp_resolve_icall(
        c"UnityEngine.Input::set_imeCompositionMode(UnityEngine.IMECompositionMode)".as_ptr()));
    const IME_MODE_ON: i32 = 1;

    let (get_mode, set_mode) = (*GET_MODE, *SET_MODE);
    unsafe {
        if wanted {
            if IME_ENABLED_BY_US.swap(true, atomic::Ordering::AcqRel) {
                return;
            }
            if get_mode != 0 && set_mode != 0 {
                let get: extern "C" fn() -> i32 = std::mem::transmute(get_mode);
                let set: extern "C" fn(i32) = std::mem::transmute(set_mode);
                IME_PREV_MODE.store(get(), atomic::Ordering::Release);
                set(IME_MODE_ON);
            }
            let ok = ImmAssociateContextEx(hwnd, HIMC(0), IACE_DEFAULT).as_bool();
            debug!("IME: on (unity api={}, prev mode={}, win32={})",
                set_mode != 0, IME_PREV_MODE.load(atomic::Ordering::Relaxed), ok);
        }
        else if IME_ENABLED_BY_US.swap(false, atomic::Ordering::AcqRel) {
            if set_mode != 0 {
                let set: extern "C" fn(i32) = std::mem::transmute(set_mode);
                set(IME_PREV_MODE.load(atomic::Ordering::Acquire));
            }
            else {
                ImmAssociateContext(hwnd, HIMC(0));
            }
            debug!("IME: restored");
        }
    }
}

/// 這則 WM_INPUT 是不是鍵盤。遊戲用 Unity 新版 Input System，它讀鍵盤走 Raw Input，
/// 不經過我們已經攔下的 WM_KEYDOWN——所以 Hachimi 開著時要另外擋。
fn is_raw_keyboard(lparam: LPARAM) -> bool {
    let mut header = RAWINPUTHEADER::default();
    let mut size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
    let read = unsafe {
        GetRawInputData(HRAWINPUT(lparam.0), RID_HEADER, Some(&mut header as *mut _ as *mut _), &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32)
    };
    read != u32::MAX && header.dwType == RIM_TYPEKEYBOARD.0
}

// Safety: only modified once on init
static mut WNDPROC_ORIG: isize = 0;
static mut WNDPROC_RECALL: usize = 0;
extern "system" fn wnd_proc(hwnd: HWND, umsg: c_uint, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(orig_fn) = (unsafe { std::mem::transmute::<isize, WNDPROC>(WNDPROC_ORIG) }) else {
        return unsafe { DefWindowProcW(hwnd, umsg, wparam, lparam) };
    };

    match umsg {
        // Check for Home key presses
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            // 改鍵模式：吃掉這次按鍵當成新的熱鍵，不要順便把選單開關掉
            if CAPTURING_KEY.swap(false, atomic::Ordering::AcqRel) {
                CAPTURED_KEY.store(wparam.0 as u32, atomic::Ordering::Release);
                return LRESULT(0);
            }
            if wparam.0 as u16 == Hachimi::instance().config.load().windows.menu_open_key {
                let Some(mut gui) = Gui::instance().map(|m| m.lock().unwrap()) else {
                    return unsafe { orig_fn(hwnd, umsg, wparam, lparam) };
                };

                gui.toggle_menu();
                return LRESULT(0);
            }
        },
        // 視窗標題的實際讀寫。GetWindowTextW / SetWindowTextW 都是 SendMessage，跨執行緒呼叫會
        // 卡住等對方抽訊息；我們的呼叫端（Present、GUI）都在 render thread，所以一律 post 過來，
        // 在擁有視窗的這條執行緒上做。詳見 apply_custom_title。
        WM_HACHIMI_APPLY_TITLE => {
            apply_custom_title_now(hwnd);
            return LRESULT(0);
        },
        // SetWindowPos 同樣會往擁有視窗的執行緒送 WM_WINDOWPOSCHANGING。
        WM_HACHIMI_APPLY_TOPMOST => {
            unsafe { _ = utils::set_window_topmost(hwnd, wparam.0 != 0); }
            return LRESULT(0);
        },
        WM_HACHIMI_SET_IME => {
            apply_ime(hwnd, wparam.0 != 0);
            return LRESULT(0);
        },
        // Hachimi 選單／視窗開著時，鍵盤的 Raw Input 不給遊戲（否則打字會觸發遊戲的快捷鍵）。
        // WM_INPUT 不交給原 wndproc 時仍要走 DefWindowProc 做清理。
        WM_INPUT if Gui::is_consuming_input_atomic() && is_raw_keyboard(lparam) => {
            return unsafe { DefWindowProcW(hwnd, umsg, wparam, lparam) };
        },
        WM_CLOSE => {
            if let Some(hook) = Hachimi::instance().interceptor.unhook(wnd_proc as _) {
                unsafe { WNDPROC_RECALL = hook.orig_addr; }
                Thread::main_thread().schedule(|| {
                    unsafe {
                        let orig_fn = std::mem::transmute::<usize, WNDPROC>(WNDPROC_RECALL).unwrap();
                        orig_fn(get_target_hwnd(), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                });
            }
            return LRESULT(0);
        },
        WM_SIZE => {
            if SIZE_READY.load(atomic::Ordering::Acquire) {
                return unsafe { orig_fn(hwnd, umsg, wparam, lparam) };
            }
            else {
                WM_SIZE_BUFFER.lock().push(WndProcCall {
                    hwnd, umsg, wparam, lparam
                });
                return LRESULT(0);
            }
        }
        _ => ()
    }

    // Only capture input if gui needs it
    if !Gui::is_consuming_input_atomic() {
        return unsafe { orig_fn(hwnd, umsg, wparam, lparam) };
    }

    // IME 要在這條執行緒上處理——Imm* 是綁視窗執行緒的，丟到別的執行緒去問會拿到空字串。
    // 這裡取完字串才交給下面的處理執行緒。訊息**不能**往下傳給遊戲：轉出去的話
    // Windows 會為了組好的字再送一次 WM_CHAR，變成重複輸入。
    // 先依到達順序排進佇列，再由拿到 GUI 鎖的那條執行緒整批照順序送出。以前一則訊息開一條執行緒
    // 各自搶鎖，順序不保證：「送出」若比「開始組字」先到，egui 會把字丟掉。
    if input::is_ime_msg(umsg) {
        if let Some(ime) = input::read_ime_event(hwnd, umsg, lparam.0) {
            IME_QUEUE.lock().push(ime);
            std::thread::spawn(|| {
                let Some(mut gui) = Gui::instance().map(|m| m.lock().unwrap()) else {
                    return;
                };
                for ime in IME_QUEUE.lock().drain(..) {
                    input::push_ime(&mut gui.input, ime);
                }
            });
        }
        return LRESULT(0);
    }

    // Check if the input processor handles this message
    if !input::is_handled_msg(umsg) {
        return unsafe { orig_fn(hwnd, umsg, wparam, lparam) };
    }

    // A deadlock would *sometimes* consistently occur if this was done on the current thread
    // (when moving the window, etc.)
    // I assume that SwapChain::Present and WndProc are running on the same thread
    std::thread::spawn(move || {
        let Some(mut gui) = Gui::instance().map(|m| m.lock().unwrap()) else {
            return;
        };

        let zoom_factor = gui.context.zoom_factor();
        input::process(&mut gui.input, zoom_factor, umsg, wparam.0, lparam.0);
    });

    LRESULT(0)
}

static mut HCBTHOOK: HHOOK = HHOOK(0);
extern "system" fn cbt_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if ncode == HCBT_MINMAX as i32 &&
        lparam.0 as i32 != SW_RESTORE.0 &&
        Hachimi::instance().config.load().windows.block_minimize_in_full_screen &&
        UnityEngine_CoreModule::Screen::get_fullScreen()
    {
        return LRESULT(1);
    }

    unsafe { CallNextHookEx(HCBTHOOK, ncode, wparam, lparam) }
}

/// wndproc + CBT hook 只裝一次的閘門。
static INSTALLED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    // 早期嘗試：用 class+title 找視窗，找到就先裝好（能盡早支援 always-on-top / CBT）。
    // 找不到也**不是錯誤**——render_hook 第一次 Present 會用 swapchain 的 OutputWindow 補裝，
    // 那才是最可靠的來源。遊戲更新後視窗標題／建立時序改變，FindWindowW 會失效（＝之前的
    // 「Failed to find game window」，連帶 overlay 因 TARGET_HWND=0 而完全不渲染）。
    let hwnd = find_window_by_title();
    if hwnd.0 != 0 {
        ensure_installed(hwnd);
    }
    else {
        info!("Game window not found by title yet; will install on first Present");
    }
}

/// 沿用原本的 class+title 精確比對（早期最佳嘗試）。
fn find_window_by_title() -> HWND {
    let game = &Hachimi::instance().game;
    let window_name = if game.region == Region::Japan && game.is_steam_release {
        w!("UmamusumePrettyDerby_Jpn")
    }
    else if game.region == Region::Taiwan {
        w!("komoeumamusume")
    }
    else {
        w!("umamusume")
    };
    unsafe { FindWindowW(w!("UnityWndClass"), window_name) }
}

/// 記住目標視窗並裝上 wndproc + CBT hook。**冪等**——只有第一次真正執行。
/// 由 [`init`] 早期嘗試，或（更可靠地）由 render_hook 首次 Present 用 swapchain 的
/// OutputWindow 呼叫。
pub fn ensure_installed(hwnd: HWND) {
    if hwnd.0 == 0 {
        return;
    }
    if INSTALLED.swap(true, atomic::Ordering::AcqRel) {
        return; // 已裝過
    }
    unsafe {
        let hachimi = Hachimi::instance();
        TARGET_HWND.store(hwnd.0, atomic::Ordering::Relaxed);

        info!("Hooking WndProc");
        let wnd_proc_addr = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
        match hachimi.interceptor.hook(wnd_proc_addr as _, wnd_proc as _) {
            Ok(trampoline_addr) => WNDPROC_ORIG = trampoline_addr as _,
            Err(e) => error!("Failed to hook WndProc: {}", e)
        }

        info!("Adding CBT hook");
        if let Ok(hhook) = SetWindowsHookExW(WH_CBT, Some(cbt_proc), None, GetCurrentThreadId()) {
            HCBTHOOK = hhook;
        }

        // Apply always on top（同樣不能在這條執行緒上直接做，見 apply_custom_title）
        if hachimi.window_always_on_top.load(atomic::Ordering::Relaxed) {
            _ = PostMessageW(hwnd, WM_HACHIMI_APPLY_TOPMOST, WPARAM(1), LPARAM(0));
        }
    }

    // 標題不在這裡讀寫——這個函式通常是從 Present 裡（render thread）呼叫的，
    // 而視窗屬於主執行緒。丟訊息過去讓它自己做。
    apply_custom_title();
}

/// 遊戲原本的視窗標題，第一次裝 hook 時記下來，讓使用者清空設定後能還原。
static ORIGINAL_TITLE: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

fn read_window_title(hwnd: HWND) -> Option<String> {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let written = GetWindowTextW(hwnd, &mut buf);
        (written > 0).then(|| String::from_utf16_lossy(&buf[..written as usize]))
    }
}

/// 套用自訂視窗標題。設定改動時也會呼叫，所以不用重開遊戲。
///
/// **從任何執行緒呼叫都安全**：只 post 一個訊息，實際動作在 [`apply_custom_title_now`]，
/// 由擁有視窗的執行緒在 wndproc 裡執行。
///
/// 一開始是直接在這裡呼叫 GetWindowTextW / SetWindowTextW 的，結果遊戲一開就沒有回應——
/// 那兩個 API 是 SendMessage，跨執行緒會阻塞到對方抽訊息為止。我們的呼叫點（首次 Present
/// 補裝 hook、選單改設定）都在 render thread，而此時主執行緒正等著 render thread 交件，
/// 兩邊互等就是死鎖。繁中服的 FindWindowW 查不到視窗（見 docs/tw-compat-notes.md），
/// 所以裝 hook 一定走 Present 那條路，一定會踩到。
pub fn apply_custom_title() {
    let hwnd = get_target_hwnd();
    if hwnd.0 == 0 {
        return;
    }
    unsafe {
        _ = PostMessageW(hwnd, WM_HACHIMI_APPLY_TITLE, WPARAM(0), LPARAM(0));
    }
}

/// 實際讀寫視窗標題。**只能在擁有視窗的執行緒上呼叫**（我們的 wndproc 裡）。
/// 清空設定會還原成遊戲原本的標題，而不是留著上一次設的，所以第一次先把原標題記下來。
fn apply_custom_title_now(hwnd: HWND) {
    {
        let mut original = ORIGINAL_TITLE.lock();
        if original.is_none() {
            *original = read_window_title(hwnd);
        }
    }

    let custom = Hachimi::instance().config.load().windows.custom_title_name.clone();
    let title = match custom {
        Some(title) => title,
        None => match ORIGINAL_TITLE.lock().clone() {
            Some(original) => original,
            None => return // 沒記到原標題就別亂改
        }
    };

    let Ok(title_cstr) = U16CString::from_str(&title) else {
        return;
    };
    unsafe {
        _ = SetWindowTextW(hwnd, PCWSTR(title_cstr.as_ptr()));
    }
}

pub fn uninit() {
    unsafe {
        if HCBTHOOK.0 != 0 {
            info!("Removing CBT hook");
            if let Err(e) = UnhookWindowsHookEx(HCBTHOOK) {
                error!("Failed to remove CBT hook: {}", e);
            }
            HCBTHOOK = HHOOK(0);
        }
    }
}