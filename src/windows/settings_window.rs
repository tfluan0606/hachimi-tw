//! 獨立的 Hachimi 設定視窗：一般的 Windows 視窗，不畫在遊戲畫面裡。
//!
//! 遊戲內的 overlay 被限制在遊戲視窗大小裡，直式視窗時很擠；設定這種要打字、要排清單的東西
//! 放在獨立視窗比較好用，也能拖到副螢幕。內容跟遊戲內設定視窗是同一套（`gui::ConfigEditor`），
//! 改設定一樣走 `settings`。
//!
//! 架構：自己一條執行緒，自己的視窗、D3D11 裝置、swap chain 和 egui Context，跟遊戲的渲染
//! 完全分開。輸入法、貼上都是標準 Windows 行為，不必跟 Unity 搶（遊戲內 overlay 那些麻煩都沒有）。
//!
//! 刻意**不**把遊戲視窗設成 owner：跨執行緒的 owner 關係會讓 Windows 把兩條執行緒的輸入佇列
//! 綁在一起（AttachThreadInput），實測兩個視窗一起「沒有回應」。遊戲置頂時改成這個視窗也置頂。
//! 也不讓 DXGI 監看這個視窗（MakeWindowAssociation），渲染不等垂直同步，自己控制幀率。

use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, AtomicIsize, Ordering},
    time::{Duration, Instant},
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDeviceAndSwapChain, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
                ID3D11Texture2D, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION
            },
            Dxgi::{
                Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC},
                IDXGIFactory, IDXGISwapChain, DXGI_MWA_NO_ALT_ENTER, DXGI_MWA_NO_WINDOW_CHANGES, DXGI_PRESENT,
                DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT
            }
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, IsIconic,
                LoadCursorW, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassExW, SetCursor,
                SetForegroundWindow, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
                HTCLIENT, IDC_ARROW, IDC_HAND, IDC_IBEAM, IDC_SIZEALL, IDC_SIZENS, IDC_SIZEWE, MINMAXINFO,
                MSG, PM_REMOVE, SW_RESTORE, SW_SHOW, WM_APP, WM_CLOSE, WM_DESTROY, WM_GETMINMAXINFO,
                WM_QUIT, WM_SETCURSOR, WM_SIZE, WNDCLASSEXW, WS_EX_APPWINDOW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW
            }
        }
    }
};

use crate::core::{gui::ConfigEditor, settings, Gui, Hachimi};

use super::{gui_impl::input, wnd_hook};

/// 已經開著時，請視窗執行緒把它拉到最前面
const WM_HACHIMI_SHOW: u32 = WM_APP + 1;

static RUNNING: AtomicBool = AtomicBool::new(false);
static PANEL_HWND: AtomicIsize = AtomicIsize::new(0);

const BG_COLOR: [f32; 4] = [27.0 / 255.0, 27.0 / 255.0, 27.0 / 255.0, 1.0];

/// 開啟設定視窗；已經開著就拉到最前面。
pub fn open() {
    if RUNNING.swap(true, Ordering::AcqRel) {
        let hwnd = HWND(PANEL_HWND.load(Ordering::Acquire) as *mut _);
        if !hwnd.is_invalid() {
            unsafe { _ = PostMessageW(Some(hwnd), WM_HACHIMI_SHOW, WPARAM(0), LPARAM(0)); }
        }
        return;
    }

    std::thread::spawn(|| {
        // 刻意不 attach il2cpp（見 core::main_thread）：要主執行緒做的事都走 main_thread::post
        info!("[SettingsWindow] thread started");
        if let Err(e) = run() {
            error!("Settings window: {}", e);
        }
        settings::flush();
        PANEL_HWND.store(0, Ordering::Release);
        RUNNING.store(false, Ordering::Release);
    });
}

/// 設定視窗是不是前景視窗。是的話遊戲那邊也當成「Hachimi 在用輸入」，擋掉遊戲的 Raw Input，
/// 免得在這裡打字觸發遊戲快捷鍵（遊戲若在背景也收 Raw Input 的話）。
pub fn is_foreground() -> bool {
    let hwnd = PANEL_HWND.load(Ordering::Acquire);
    hwnd != 0 && unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }.0 as isize == hwnd
}

/// 視窗程序和渲染迴圈在同一條執行緒，用 thread_local 交換資料
struct Shared {
    input: egui::RawInput,
    pixels_per_point: f32,
    resized: bool,
    cursor: egui::CursorIcon,
}

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::new(Shared {
        input: egui::RawInput::default(),
        pixels_per_point: 1.0,
        resized: false,
        cursor: egui::CursorIcon::Default,
    });
}

fn run() -> Result<(), String> {
    let hinstance = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
    let class_name = w!("HachimiSettingsWindow");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    // 第二次開會回 0（類別已存在），不影響
    unsafe { RegisterClassExW(&wc); }

    // 遊戲置頂時這個視窗也要置頂，不然會被遊戲蓋住（沒有 owner 關係可以靠）
    let topmost = Hachimi::instance().window_always_on_top.load(Ordering::Relaxed);
    let ex_style = if topmost { WS_EX_APPWINDOW | WS_EX_TOPMOST } else { WS_EX_APPWINDOW };
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style, class_name, w!("Hachimi TW 設定"), WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT, CW_USEDEFAULT, WINDOW_SIZE.0, WINDOW_SIZE.1,
            None, None, Some(hinstance.into()), None
        )
    }.map_err(|e| e.to_string())?;
    center_on_game(hwnd);
    PANEL_HWND.store(hwnd.0 as isize, Ordering::Release);
    info!("[SettingsWindow] window created");

    let (swap_chain, device) = create_device(hwnd)?;
    info!("[SettingsWindow] device created");
    let context: ID3D11DeviceContext = unsafe { device.GetImmediateContext() }.map_err(|e| e.to_string())?;
    let mut renderer = egui_directx11::Renderer::new(&device).map_err(|e| e.to_string())?;
    let mut render_target = create_render_target(&swap_chain, &device);

    let ctx = egui::Context::default();
    Gui::setup_context(&ctx);
    let mut editor = ConfigEditor::new();

    unsafe { _ = ShowWindow(hwnd, SW_SHOW); }

    let start = Instant::now();
    let mut first_frame = true;
    let mut msg = MSG::default();
    loop {
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                return Ok(());
            }
            unsafe {
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if unsafe { IsIconic(hwnd) }.as_bool() {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        // 視窗大小變了：先放掉 render target 才能 ResizeBuffers
        if SHARED.with_borrow_mut(|s| std::mem::take(&mut s.resized)) {
            render_target = None;
            unsafe {
                // context 上還繫結著舊的 render target 時 ResizeBuffers 會失敗
                context.OMSetRenderTargets(None, None);
                _ = swap_chain.ResizeBuffers(0, 0, 0, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0));
            }
            render_target = create_render_target(&swap_chain, &device);
        }

        let mut rect = RECT::default();
        unsafe { _ = GetClientRect(hwnd, &mut rect); }
        let (width, height) = ((rect.right - rect.left).max(1), (rect.bottom - rect.top).max(1));
        let pixels_per_point = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;

        let mut raw = SHARED.with_borrow_mut(|s| {
            s.pixels_per_point = pixels_per_point;
            s.input.take()
        });
        raw.time = Some(start.elapsed().as_secs_f64());
        raw.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width as f32 / pixels_per_point, height as f32 / pixels_per_point)
        ));
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(pixels_per_point);

        let output = ctx.run_ui(raw, |ui| {
            egui::Frame::NONE
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| editor.show(ui));
        });

        SHARED.with_borrow_mut(|s| s.cursor = output.platform_output.cursor_icon);
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                input::set_clipboard_text(text);
            }
        }

        if let Some(rtv) = &render_target {
            unsafe { context.ClearRenderTargetView(rtv, &BG_COLOR); }
            let renderer_output = egui_directx11::split_output(output).0;
            if let Err(e) = renderer.render(&context, rtv, &ctx, renderer_output) {
                error!("Settings window render: {}", e);
            }
        }
        unsafe { _ = swap_chain.Present(0, DXGI_PRESENT(0)); }
        if first_frame {
            first_frame = false;
            info!("[SettingsWindow] first frame presented");
        }
        // 設定視窗不需要高幀率：約 60fps
        std::thread::sleep(Duration::from_millis(16));

        settings::autosave_tick();
    }
}

fn create_device(hwnd: HWND) -> Result<(IDXGISwapChain, ID3D11Device), String> {
    let desc = DXGI_SWAP_CHAIN_DESC {
        BufferCount: 1,
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        OutputWindow: hwnd,
        Windowed: true.into(),
        SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_DESC {
            // egui-directx11 要求 gamma 空間、非 sRGB 的 render target
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut swap_chain = None;
    let mut device = None;
    unsafe {
        D3D11CreateDeviceAndSwapChain(
            None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0),
            Some(&[D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION, Some(&desc),
            Some(&mut swap_chain), Some(&mut device), None, None
        )
    }.map_err(|e| e.to_string())?;
    let swap_chain: IDXGISwapChain = swap_chain.ok_or("no swap chain")?;
    // 不讓 DXGI 監看這個視窗（Alt+Enter 全螢幕切換之類），少一個跨執行緒互動的來源
    unsafe {
        if let Ok(factory) = swap_chain.GetParent::<IDXGIFactory>() {
            _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_WINDOW_CHANGES | DXGI_MWA_NO_ALT_ENTER);
        }
    }
    Ok((swap_chain, device.ok_or("no device")?))
}

fn create_render_target(swap_chain: &IDXGISwapChain, device: &ID3D11Device) -> Option<ID3D11RenderTargetView> {
    let backbuffer: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0) }.ok()?;
    let mut rtv = None;
    unsafe { device.CreateRenderTargetView(&backbuffer, None, Some(&mut rtv)) }.ok()?;
    rtv
}

/// 預設大小（外框，像素）
const WINDOW_SIZE: (i32, i32) = (760, 600);

/// 移到遊戲視窗正中間（每次打開都會，之後使用者可以自己拖走），並夾在該螢幕的可用範圍內。
fn center_on_game(hwnd: HWND) {
    use windows::Win32::{
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::WindowsAndMessaging::{GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER},
    };

    let game = wnd_hook::get_target_hwnd();
    if game.is_invalid() {
        return;
    }
    unsafe {
        let (mut game_rect, mut own) = (RECT::default(), RECT::default());
        if GetWindowRect(game, &mut game_rect).is_err() || GetWindowRect(hwnd, &mut own).is_err() {
            return;
        }
        let (w, h) = (own.right - own.left, own.bottom - own.top);
        let mut x = (game_rect.left + game_rect.right) / 2 - w / 2;
        let mut y = (game_rect.top + game_rect.bottom) / 2 - h / 2;

        // 遊戲視窗貼著螢幕邊緣時，置中會跑出螢幕外：夾回遊戲所在螢幕的工作區
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(MonitorFromWindow(game, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            let work = info.rcWork;
            x = x.min(work.right - w).max(work.left);
            y = y.min(work.bottom - h).max(work.top);
        }
        _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn set_cursor(icon: egui::CursorIcon) {
    use egui::CursorIcon as C;
    let id: PCWSTR = match icon {
        C::Text | C::VerticalText => IDC_IBEAM,
        C::PointingHand => IDC_HAND,
        C::ResizeHorizontal | C::ResizeColumn | C::ResizeEast | C::ResizeWest => IDC_SIZEWE,
        C::ResizeVertical | C::ResizeRow | C::ResizeNorth | C::ResizeSouth => IDC_SIZENS,
        C::Move | C::Grab | C::Grabbing => IDC_SIZEALL,
        _ => IDC_ARROW,
    };
    unsafe {
        if let Ok(cursor) = LoadCursorW(None, id) {
            SetCursor(Some(cursor));
        }
    }
}

extern "system" fn wnd_proc(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match umsg {
        WM_HACHIMI_SHOW => {
            unsafe {
                _ = ShowWindow(hwnd, SW_RESTORE);
                center_on_game(hwnd);
                _ = SetForegroundWindow(hwnd);
            }
            return LRESULT(0);
        }
        WM_SIZE => {
            SHARED.with_borrow_mut(|s| s.resized = true);
            return LRESULT(0);
        }
        WM_GETMINMAXINFO => {
            let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            info.ptMinTrackSize.x = 420;
            info.ptMinTrackSize.y = 320;
            return LRESULT(0);
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            set_cursor(SHARED.with_borrow(|s| s.cursor));
            return LRESULT(1);
        }
        WM_CLOSE => {
            unsafe { _ = DestroyWindow(hwnd); }
            return LRESULT(0);
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0); }
            return LRESULT(0);
        }
        _ => {}
    }

    // 改選單熱鍵時，按鍵是按在這個視窗上的
    if umsg == windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN && wnd_hook::capture_key_if_active(wparam.0 as u16) {
        return LRESULT(0);
    }

    // 輸入法：這是我們自己的視窗，Unity 不會關掉它，直接讀組字結果就好。
    // 組字訊息不交給 DefWindowProc，否則確定的字會再變成一次 WM_CHAR（重複輸入）。
    if input::is_ime_msg(umsg) {
        if let Some(ime) = input::read_ime_event(hwnd, umsg, lparam.0) {
            SHARED.with_borrow_mut(|s| input::push_ime(&mut s.input, ime));
        }
        return LRESULT(0);
    }

    if input::is_handled_msg(umsg) {
        SHARED.with_borrow_mut(|s| {
            let ppp = s.pixels_per_point;
            input::process(&mut s.input, ppp, umsg, wparam.0, lparam.0);
        });
        // 系統按鍵（Alt+F4 等）照常交給 Windows
        if !matches!(umsg, windows::Win32::UI::WindowsAndMessaging::WM_SYSKEYDOWN | windows::Win32::UI::WindowsAndMessaging::WM_SYSKEYUP) {
            return LRESULT(0);
        }
    }

    unsafe { DefWindowProcW(hwnd, umsg, wparam, lparam) }
}
