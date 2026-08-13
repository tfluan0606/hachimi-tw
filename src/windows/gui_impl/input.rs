// Originally from sy1ntexx/egui-d3d11
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Vec2};
use std::ffi::CStr;
use windows::Win32::{
    Foundation::HWND,
    System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        Ole::CF_TEXT,
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

pub fn process(input: &mut RawInput, zoom_factor: f32, umsg: u32, wparam: usize, lparam: isize) -> InputResult {
    match umsg {
        WM_MOUSEMOVE => {
            input.events.push(Event::PointerMoved(get_pos(lparam) / zoom_factor));
            InputResult::MouseMove
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / zoom_factor,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseLeft
        }
        WM_LBUTTONUP => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / zoom_factor,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseLeft
        }
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / zoom_factor,
                button: PointerButton::Secondary,
                pressed: true,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseRight
        }
        WM_RBUTTONUP => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / zoom_factor,
                button: PointerButton::Secondary,
                pressed: false,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseRight
        }
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / zoom_factor,
                button: PointerButton::Middle,
                pressed: true,
                modifiers: get_modifiers(wparam),
            });
            InputResult::MouseMiddle
        }
        WM_MBUTTONUP => {
            input.events.push(Event::PointerButton {
                pos: get_pos(lparam) / zoom_factor,
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
                input.events.push(Event::Scroll(Vec2::new(0., delta)));
                InputResult::Scroll
            }
        }
        WM_MOUSEHWHEEL => {
            let delta = (wparam >> 16) as i16 as f32 * 10. / WHEEL_DELTA as f32;

            if wparam & MK_CONTROL.0 as usize != 0 {
                input.events.push(Event::Zoom(if delta > 0. { 1.5 } else { 0.5 }));
                InputResult::Zoom
            } else {
                input.events.push(Event::Scroll(Vec2::new(delta, 0.)));
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
        if himc.0 == 0 {
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
    input.events.push(match ime {
        ImeInput::Start => Event::CompositionStart,
        ImeInput::Update(text) => Event::CompositionUpdate(text),
        ImeInput::Commit(text) => Event::CompositionEnd(text)
    });
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
        0x30..=0x39 => unsafe { Some(std::mem::transmute::<_, Key>(wparam as u8 - 0x21)) },
        0x41..=0x5A => unsafe { Some(std::mem::transmute::<_, Key>(wparam as u8 - 0x28)) },
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

fn get_clipboard_text() -> Option<String> {
    unsafe {
        if OpenClipboard(HWND::default()).is_ok() {
            if let Ok(handle) = GetClipboardData(CF_TEXT.0 as u32) {
                let txt = handle.0 as *const i8;
                let data = Some(CStr::from_ptr(txt).to_str().ok()?.to_string());
                CloseClipboard().ok();
                return data;
            }
        }

        None
    }
}