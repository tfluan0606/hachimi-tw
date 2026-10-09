// Originally from sy1ntexx/egui-d3d11
use egui::{Event, ImeEvent, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, TouchPhase, Vec2};

use windows::Win32::{
    Foundation::{HGLOBAL, HWND},
    System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        Memory::{GlobalLock, GlobalUnlock},
        Ole::CF_UNICODETEXT,
        SystemServices::{MK_CONTROL, MK_SHIFT}
    },
    UI::{
        Input::Ime::{
            ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext,
            GCS_COMPSTR, GCS_RESULTSTR, IME_COMPOSITION_STRING
        },
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
            VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_LSHIFT, VK_NEXT, VK_PRIOR, VK_RETURN,
            VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
        },
        WindowsAndMessaging::{
            WHEEL_DELTA, WM_CHAR, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION,
            WM_KEYDOWN, WM_KEYUP,
            WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN,
            WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDBLCLK,
            WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    },
};

/// High-level overview of recognized `WndProc` messages.
#[repr(u8)]
pub enum InputResult {
    Unknown,
    MouseMove,
    MouseLeft,
    MouseRight,
    MouseMiddle,
    Character,
    Scroll,
    Zoom,
    Key,
}

pub fn process(input: &mut RawInput, pixels_per_point: f32, umsg: u32, wparam: usize, lparam: isize) -> InputResult {
    match umsg {
        WM_MOUSEMOVE => {
            input.events.push(Event::PointerMoved(get_pos(lparam) / pixels_per_point));
            InputResult::MouseMove
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / pixels_per_point,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseLeft
        }
        WM_LBUTTONUP => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / pixels_per_point,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseLeft
        }
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / pixels_per_point,
                button: PointerButton::Secondary,
                pressed: true,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseRight
        }
        WM_RBUTTONUP => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / pixels_per_point,
                button: PointerButton::Secondary,
                pressed: false,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseRight
        }
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / pixels_per_point,
                button: PointerButton::Middle,
                pressed: true,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseMiddle
        }
        WM_MBUTTONUP => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / pixels_per_point,
                button: PointerButton::Middle,
                pressed: false,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseMiddle
        }
        WM_CHAR => {
            if let Some(ch) = char::from_u32(wparam as _) {
                if !ch.is_control() {
                    input.events.push(Event::Text(ch.into()));
                }
            }
            InputResult::Character
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam >> 16) as i16 as f32 * 10. / WHEEL_DELTA as f32;

            if wparam & MK_CONTROL.0 as usize != 0 {
                input.events.push(Event::Zoom(if delta > 0. { 1.5 } else { 0.5 }));
                InputResult::Zoom
            } else {
                input.events.push(wheel_event(Vec2::new(0., delta), wparam));
                InputResult::Scroll
            }
        }
        WM_MOUSEHWHEEL => {
            let delta = (wparam >> 16) as i16 as f32 * 10. / WHEEL_DELTA as f32;

            if wparam & MK_CONTROL.0 as usize != 0 {
                input.events.push(Event::Zoom(if delta > 0. { 1.5 } else { 0.5 }));
                InputResult::Zoom
            } else {
                input.events.push(wheel_event(Vec2::new(delta, 0.), wparam));
                InputResult::Scroll
            }
        }
        msg @ (WM_KEYDOWN | WM_SYSKEYDOWN) => {
            if let Some(key) = get_key(wparam) {
                let events = &mut input.events;
                let mods = get_key_modifiers(msg);

                if key == Key::Space {
                    events.push(Event::Text(String::from(" ")));
                } else if key == Key::V && mods.ctrl {
                    if let Some(clipboard) = get_clipboard_text() {
                        events.push(Event::Text(clipboard));
                    }
                } else if key == Key::C && mods.ctrl {
                    events.push(Event::Copy);
                } else if key == Key::X && mods.ctrl {
                    events.push(Event::Cut);
                } else {
                    events.push(Event::Key {
                        key,
                        pressed: true,
                        modifiers: get_key_modifiers(msg),
                        physical_key: None,
                        repeat: false,
                    });
                }
            }
            InputResult::Key
        }
        msg @ (WM_KEYUP | WM_SYSKEYUP) => {
            if let Some(key) = get_key(wparam) {
                input.events.push(Event::Key {
                    key,
                    pressed: false,
                    modifiers: get_key_modifiers(msg),
                    physical_key: None,
                    repeat: false,
                });
            }
            InputResult::Key
        }
        _ => InputResult::Unknown,
    }
}

/// 從 IME 取出來的一則事件。字串在視窗執行緒上就先抓好了。
pub enum ImeInput {
    Start,
    /// 組字中的預覽（注音、拼音那條）
    Update(String),
    /// 確定送出的字
    Commit(String)
}

/// **必須在擁有視窗的執行緒上呼叫**——`Imm*` 系列是綁執行緒的，從別的執行緒問會拿到空的。
/// 所以字串在 wndproc 當場取出，之後才丟給處理輸入的執行緒。
pub fn read_ime_event(hwnd: HWND, umsg: u32, lparam: isize) -> Option<ImeInput> {
    match umsg {
        WM_IME_STARTCOMPOSITION => Some(ImeInput::Start),
        WM_IME_ENDCOMPOSITION => Some(ImeInput::Update(String::new())),
        WM_IME_COMPOSITION => {
            let flags = lparam as u32;
            // 先看有沒有確定的字，有的話組字這輪就結束了
            if flags & GCS_RESULTSTR.0 != 0 {
                return get_composition_string(hwnd, GCS_RESULTSTR).map(ImeInput::Commit);
            }
            if flags & GCS_COMPSTR.0 != 0 {
                // 組字被清空時也要送空字串，否則預覽會留在畫面上
                return Some(ImeInput::Update(
                    get_composition_string(hwnd, GCS_COMPSTR).unwrap_or_default()
                ));
            }
            None
        }
        _ => None
    }
}

fn get_composition_string(hwnd: HWND, index: IME_COMPOSITION_STRING) -> Option<String> {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc.is_invalid() {
            return None;
        }

        // 先問長度（回傳的是位元組數，UTF-16 所以要除以 2）
        let byte_len = ImmGetCompositionStringW(himc, index, None, 0);
        let result = if byte_len > 0 {
            let mut buf = vec![0u16; byte_len as usize / 2];
            let written = ImmGetCompositionStringW(
                himc, index,
                Some(buf.as_mut_ptr() as *mut _),
                byte_len as u32
            );
            (written > 0).then(|| String::from_utf16_lossy(&buf))
        }
        else {
            None
        };

        _ = ImmReleaseContext(hwnd, himc);
        result
    }
}

pub fn push_ime(input: &mut RawInput, ime: ImeInput) {
    // egui 0.35 不再需要「開始組字」事件：Preedit 非空＝組字中，空字串＝取消
    let event = match ime {
        ImeInput::Start => return,
        ImeInput::Update(text) => ImeEvent::Preedit { text, active_range_chars: None },
        ImeInput::Commit(text) => ImeEvent::Commit(text)
    };
    input.events.push(Event::Ime(event));
}

/// 滾輪。舊版的 `Event::Scroll` 單位就是 point，沿用同樣的位移量。
fn wheel_event(delta: Vec2, wparam: usize) -> Event {
    Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta,
        phase: TouchPhase::Move,
        modifiers: get_modifiers(wparam)
    }
}

pub fn is_ime_msg(umsg: u32) -> bool {
    matches!(umsg, WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION)
}

pub fn is_handled_msg(umsg: u32) -> bool {
    match umsg {
        WM_CHAR | WM_KEYDOWN | WM_KEYUP |
        WM_LBUTTONDBLCLK | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDBLCLK | WM_MBUTTONDOWN |
        WM_MBUTTONUP | WM_MOUSEHWHEEL | WM_MOUSEMOVE | WM_MOUSEWHEEL | WM_RBUTTONDBLCLK |
        WM_RBUTTONDOWN | WM_RBUTTONUP | WM_SYSKEYDOWN | WM_SYSKEYUP => true,
        _ => false
    }
}

fn get_pos(lparam: isize) -> Pos2 {
    let x = (lparam & 0xFFFF) as i16 as f32;
    let y = (lparam >> 16 & 0xFFFF) as i16 as f32;

    Pos2::new(x, y)
}

fn get_modifiers(wparam: usize) -> Modifiers {
    Modifiers {
        alt: false,
        ctrl: (wparam & MK_CONTROL.0 as usize) != 0,
        shift: (wparam & MK_SHIFT.0 as usize) != 0,
        mac_cmd: false,
        command: (wparam & MK_CONTROL.0 as usize) != 0,
    }
}

fn get_key_modifiers(msg: u32) -> Modifiers {
    let ctrl = unsafe { GetAsyncKeyState(VK_CONTROL.0 as _) != 0 };
    let shift = unsafe { GetAsyncKeyState(VK_LSHIFT.0 as _) != 0 };

    Modifiers {
        alt: msg == WM_SYSKEYDOWN,
        mac_cmd: false,
        command: ctrl,
        shift,
        ctrl,
    }
}

fn get_key(wparam: usize) -> Option<Key> {
    match wparam {
        // 以前用「VK 減一個偏移量再 transmute」，那是舊版 egui 的 Key 排列；0.27 已經不同，
        // 字母全部錯位（V 變 E），Ctrl+V/C/X 因此從來沒作用過。改成照名稱查。
        0x30..=0x39 | 0x41..=0x5A => Key::from_name(&(wparam as u8 as char).to_string()),
        _ => match VIRTUAL_KEY(wparam as u16) {
            VK_DOWN => Some(Key::ArrowDown),
            VK_LEFT => Some(Key::ArrowLeft),
            VK_RIGHT => Some(Key::ArrowRight),
            VK_UP => Some(Key::ArrowUp),
            VK_ESCAPE => Some(Key::Escape),
            VK_TAB => Some(Key::Tab),
            VK_BACK => Some(Key::Backspace),
            VK_RETURN => Some(Key::Enter),
            VK_SPACE => Some(Key::Space),
            VK_INSERT => Some(Key::Insert),
            VK_DELETE => Some(Key::Delete),
            VK_HOME => Some(Key::Home),
            VK_END => Some(Key::End),
            VK_PRIOR => Some(Key::PageUp),
            VK_NEXT => Some(Key::PageDown),
            _ => None,
        },
    }
}

/// 把文字放進剪貼簿（egui 的複製）。
pub fn set_clipboard_text(text: &str) {
    use windows::Win32::{
        Foundation::HANDLE,
        System::{DataExchange::{EmptyClipboard, SetClipboardData}, Memory::{GlobalAlloc, GMEM_MOVEABLE}},
    };
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(None).is_err() {
            return;
        }
        _ = EmptyClipboard();
        if let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, utf16.len() * 2) {
            let ptr = GlobalLock(mem) as *mut u16;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
                _ = GlobalUnlock(mem);
                // 成功後記憶體歸剪貼簿所有，不要自己釋放
                _ = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(mem.0)));
            }
        }
        _ = CloseClipboard();
    }
}

/// 讀剪貼簿文字。要用 `CF_UNICODETEXT`：`CF_TEXT` 是系統 ANSI 碼頁（繁中 Windows＝Big5），
/// 中文不是合法 UTF-8，以前整段解碼失敗就貼不上。
fn get_clipboard_text() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let data = GetClipboardData(CF_UNICODETEXT.0 as u32).ok().and_then(|handle| {
            let ptr = GlobalLock(HGLOBAL(handle.0 as _)) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let mut len = 0;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            _ = GlobalUnlock(HGLOBAL(handle.0 as _));
            // egui 的單行輸入框不吃換行，貼一行字時常會帶到行尾的 \r\n
            Some(text.replace("\r\n", "\n"))
        });
        // 不論成功與否都要關，否則剪貼簿會被一直佔住
        _ = CloseClipboard();
        data
    }
}