use std::{borrow::Cow, ops::RangeInclusive, sync::{atomic::{self, AtomicBool}, Arc, Mutex}, thread, time::Instant};

use fnv::FnvHashSet;
use once_cell::sync::OnceCell;
use rust_i18n::t;

use crate::il2cpp::{
    hook::{
        umamusume::{CySpringController::SpringUpdateMode, GameSystem, GraphicSettings::GraphicsQuality},
    },
    symbols::Thread
};

#[cfg(not(target_os = "windows"))]
use crate::il2cpp::hook::umamusume::WebViewManager;


use super::{hachimi::{self, Language}, Hachimi};

macro_rules! add_font {
    ($fonts:expr, $family_fonts:expr, $filename:literal) => {
        $fonts.font_data.insert(
            $filename.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!(concat!("../../assets/fonts/", $filename))))
        );
        $family_fonts.push($filename.to_owned());
    };
}

type BoxedWindow = Box<dyn Window + Send + Sync>;
pub struct Gui {
    pub context: egui::Context,
    pub input: egui::RawInput,
    pub start_time: Instant,
    pub prev_main_axis_size: i32,
    last_fps_update: Instant,
    tmp_frame_count: u32,
    fps_text: String,

    show_menu: bool,

    splash_visible: bool,
    splash_tween: TweenInOutWithDelay,
    splash_sub_str: String,

    menu_visible: bool,
    menu_anim_time: Option<Instant>,

    notifications: Vec<Notification>,
    windows: Vec<BoxedWindow>
}

const PIXELS_PER_POINT_RATIO: f32 = 3.0/1080.0;
const BACKGROUND_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(27, 27, 27, 220);
const TEXT_COLOR: egui::Color32 = egui::Color32::from_gray(170);

static INSTANCE: OnceCell<Mutex<Gui>> = OnceCell::new();
static IS_CONSUMING_INPUT: AtomicBool = AtomicBool::new(false);
static mut DISABLED_GAME_UIS: once_cell::unsync::Lazy<FnvHashSet<*mut crate::il2cpp::types::Il2CppObject>> =
    once_cell::unsync::Lazy::new(|| FnvHashSet::default());

impl Gui {
    // Call this from the render thread!
    pub fn instance_or_init(open_key_id: &str) -> &Mutex<Gui> {
        if let Some(instance) = INSTANCE.get() {
            return instance;
        }

        let hachimi = Hachimi::instance();

        let context = egui::Context::default();
        Self::setup_context(&context);

        let windows: Vec<BoxedWindow> = Vec::new();

        let now = Instant::now();
        let instance = Gui {
            context,
            input: egui::RawInput::default(),
            start_time: now,
            prev_main_axis_size: 1,
            last_fps_update: now,
            tmp_frame_count: 0,
            fps_text: "FPS: 0".to_string(),

            show_menu: false,

            splash_visible: true,
            splash_tween: TweenInOutWithDelay::new(0.8, 3.0, Easing::OutQuad),
            splash_sub_str: t!("splash_sub", open_key_str = t!(open_key_id)).into_owned(),

            menu_visible: false,
            menu_anim_time: None,

            notifications: Vec::new(),
            windows
        };
        unsafe {
            INSTANCE.set(Mutex::new(instance)).unwrap_unchecked();

            // Doing auto update check here to ensure that the updater can access the gui
            hachimi.run_auto_update_check();

            INSTANCE.get().unwrap_unchecked()
        }
    }

    pub fn instance() -> Option<&'static Mutex<Gui>> {
        INSTANCE.get()
    }

    /// 字型、樣式、圖片載入器。遊戲內選單和獨立設定視窗共用，外觀才一致。
    pub(crate) fn setup_context(context: &egui::Context) {
        egui_extras::install_image_loaders(context);

        context.set_fonts(Self::get_font_definitions());

        let mut style = egui::Style::default();
        style.spacing.button_padding = egui::Vec2::new(8.0, 5.0);
        style.interaction.selectable_labels = false;
        context.set_global_style(style);

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BACKGROUND_COLOR;
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_COLOR);
        context.set_visuals(visuals);
    }

    fn get_font_definitions() -> egui::FontDefinitions {
        let mut fonts = egui::FontDefinitions::default();
        let proportional_fonts = fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap();

        add_font!(fonts, proportional_fonts, "AlibabaPuHuiTi-3-45-Light.otf");
        add_font!(fonts, proportional_fonts, "NotoSans-Light.ttf");
        add_font!(fonts, proportional_fonts, "FontAwesome.otf");

        fonts
    }

    pub fn set_screen_size(&mut self, width: i32, height: i32) {
        let main_axis_size = if width < height { width } else { height };
        // 每幀都會走到這裡，所以縮放直接乘進來就能即時生效，不必額外記狀態。
        //
        // 倍率當成「螢幕 DPI」（native_pixels_per_point）給 egui，zoom_factor 維持 1。
        // 不能用 set_pixels_per_point：那會換算成 zoom_factor，而 egui-directx11 0.13 畫頂點時
        // pixels_per_point 和 zoom_factor 各乘一次，縮放被套兩次（畫面放大、字糊、滑鼠對不準）。
        // input.rs 換算滑鼠座標也用同一個 pixels_per_point。
        let gui_scale = Hachimi::instance().config.load().gui_scale.clamp(0.5, 3.0);
        let pixels_per_point = main_axis_size as f32 * PIXELS_PER_POINT_RATIO * gui_scale;
        self.input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point =
            Some(pixels_per_point);

        self.input.screen_rect = Some(egui::Rect {
            min: egui::Pos2::default(),
            max: egui::Pos2::new(width as f32 / pixels_per_point, height as f32 / pixels_per_point)
        });

        self.prev_main_axis_size = main_axis_size;
    }

    fn take_input(&mut self) -> egui::RawInput {
        self.input.time = Some(self.start_time.elapsed().as_secs_f64());
        self.input.take()
    }

    fn update_fps(&mut self) {
        let delta = self.last_fps_update.elapsed().as_secs_f64();
        if delta > 0.5 {
            let fps = (self.tmp_frame_count as f64 * (0.5 / delta) * 2.0).round();
            self.fps_text = t!("menu.fps_text", fps = fps).into_owned();
            self.tmp_frame_count = 1;
            self.last_fps_update = Instant::now();
        }
        else {
            self.tmp_frame_count += 1;
        }
    }

    pub fn run(&mut self) -> egui::FullOutput {
        self.update_fps();
        let input = self.take_input();

        self.context.begin_pass(input);

        if self.menu_visible {
            // egui 0.35 的 Panel 只能放在 Ui 裡；照 Context::run_ui 的做法建一個蓋滿畫面的根 Ui
            let mut root_ui = egui::Ui::new(
                self.context.clone(),
                egui::Id::new("hachimi_root_ui"),
                egui::UiBuilder::new()
                    .layer_id(egui::LayerId::background())
                    .max_rect(self.context.viewport_rect()),
            );
            self.run_menu(&mut root_ui);
        }

        self.run_windows();
        self.run_notifications();
        super::settings::autosave_tick();
        super::main_thread::drain();

        if self.splash_visible { self.run_splash(); }

        // Store this as an atomic value so the input thread can check it without locking the gui
        #[cfg(target_os = "windows")]
        let consuming = self.is_consuming_input() || crate::windows::settings_window::is_foreground();
        #[cfg(not(target_os = "windows"))]
        let consuming = self.is_consuming_input();
        IS_CONSUMING_INPUT.store(consuming, atomic::Ordering::Relaxed);

        // 輸入框有焦點時替遊戲視窗打開輸入法（Unity 平常會把它關掉）
        #[cfg(target_os = "windows")]
        crate::windows::wnd_hook::set_ime_wanted(self.context.egui_wants_keyboard_input());

        self.context.end_pass()
    }

    const ICON_IMAGE: egui::ImageSource<'static> = egui::include_image!("../../assets/icon.png");
    const BANNER_IMAGE: egui::ImageSource<'static> = egui::include_image!("../../assets/ASKRNB_ver2.png");
    fn icon<'a>() -> egui::Image<'a> {
        egui::Image::new(Self::ICON_IMAGE)
        .fit_to_exact_size(egui::Vec2::new(24.0, 24.0))
    }

    fn icon_2x<'a>() -> egui::Image<'a> {
        egui::Image::new(Self::ICON_IMAGE)
        .fit_to_exact_size(egui::Vec2::new(48.0, 48.0))
    }

    fn run_splash(&mut self) {
        let ctx = &self.context;

        let id = egui::Id::from("splash");
        let Some(tween_val) = self.splash_tween.run(ctx, id.with("tween")) else {
            self.splash_visible = false;
            return;
        };

        egui::Area::new(id)
        .fixed_pos(egui::Pos2 {
            x: -250.0 * (1.0 - tween_val),
            y: 16.0
        })
        .show(ctx, |ui| {
            egui::Frame::NONE
            .fill(BACKGROUND_COLOR)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(Self::icon());
                    ui.heading(t!("hachimi"));
                    // splash／關於只顯示乾淨版本號；完整含 git hash 的版本只寫在 log 開頭（除錯用）
                    ui.label(concat!("v", env!("CARGO_PKG_VERSION")));
                });
                ui.label(&self.splash_sub_str);
            });
        });
    }

    /// 遊戲內選單：只放玩的當下會用到的東西，其餘都在「設定」裡。
    /// 這裡的開關和設定視窗改的是同一份 config（走 settings），改了即時生效、自動存檔。
    fn run_menu(&mut self, root_ui: &mut egui::Ui) {
        use super::settings;

        let ctx = &self.context.clone();
        let config = Hachimi::instance().config.load_full();

        let mut show_notification: Option<Cow<'_, str>> = None;
        let mut show_window: Option<BoxedWindow> = None;
        let mut show_menu = self.show_menu;
        egui::Panel::left("hachimi_menu").show_collapsible(root_ui, &mut show_menu, |ui| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::TOP), |ui| {
                // ASKR 牛逼！banner（也是「有載入成功」的記號）
                ui.vertical_centered(|ui| {
                    ui.add(egui::Image::new(Self::BANNER_IMAGE).fit_to_exact_size(egui::Vec2::splat(72.0)));
                });
                ui.horizontal(|ui| {
                    ui.add(Self::icon());
                    ui.vertical(|ui| {
                        ui.heading(t!("hachimi"));
                        ui.small(format!("v{} · {}", env!("CARGO_PKG_VERSION"), self.fps_text));
                    });
                });
                ui.horizontal(|ui| {
                    if ui.button("\u{f013} 設定").clicked() {
                        #[cfg(target_os = "windows")]
                        if config.windows.settings_in_game {
                            show_window = Some(Box::new(ConfigEditor::new()));
                        }
                        else {
                            crate::windows::settings_window::open();
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            show_window = Some(Box::new(ConfigEditor::new()));
                        }
                    }
                    if ui.button(t!("menu.close_menu")).clicked() {
                        self.show_menu = false;
                        self.menu_anim_time = None;
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    menu_heading(ui, "常用");
                    ui.horizontal(|ui| {
                        ui.label("FPS 上限");
                        // None＝交給遊戲；滑桿一動就改成指定值
                        let mut fps = config.target_fps.unwrap_or(60);
                        if ui.add(egui::Slider::new(&mut fps, 30..=240)).changed() {
                            settings::update(|c| c.target_fps = Some(fps));
                        }
                    });
                    if config.target_fps.is_none() {
                        ui.small("目前沒有限制（遊戲預設）");
                    }
                    #[cfg(target_os = "windows")]
                    {
                        let mut topmost = config.windows.window_always_on_top;
                        if ui.checkbox(&mut topmost, t!("menu.stay_on_top")).changed() {
                            settings::update(|c| c.windows.window_always_on_top = topmost);
                        }
                        let mut hide_cursor = config.windows.disable_game_cursor;
                        if ui.checkbox(&mut hide_cursor, "隱藏遊戲游標").changed() {
                            settings::update(|c| c.windows.disable_game_cursor = hide_cursor);
                        }
                    }

                    #[cfg(target_os = "windows")]
                    {
                        use crate::core::factor_card;

                        menu_heading(ui, "因子卡片");
                        ui.label(match factor_card::last_viewed_label() {
                            Some(label) => format!("目標：{label}"),
                            None => "目標：（先點開一隻馬的詳細視窗）".to_owned()
                        });
                        ui.horizontal(|ui| {
                            if ui.button("擷取卡片").clicked() {
                                capture_factor_card();
                            }
                            if ui.button("開資料夾").clicked() {
                                open_folder(&factor_card::output_dir());
                            }
                        });
                    }

                    {
                        use crate::core::api_packet::practice_race;

                        menu_heading(ui, "比賽擷取");
                        let mut on = practice_race::capture_enabled();
                        if ui.checkbox(&mut on, "自動存練習賽封包").changed() {
                            practice_race::set_capture_enabled(on);
                            show_notification = Some(if on { "比賽擷取已開啟".into() } else { "比賽擷取已關閉".into() });
                        }
                        if on {
                            ui.horizontal(|ui| {
                                ui.small(format!("本次已存 {} 場", practice_race::capture_count()));
                                #[cfg(target_os = "windows")]
                                if ui.small_button("開資料夾").clicked() {
                                    open_folder(&practice_race::capture_dir());
                                }
                            });
                        }
                    }

                    // 全量 API 擷取（datamine）：分享版不編入，選單看不到。
                    #[cfg(feature = "datamine")]
                    {
                        use crate::core::api_packet;

                        menu_heading(ui, "API 擷取");
                        let mut on = api_packet::capture_enabled();
                        if ui.checkbox(&mut on, "把 API 回傳的 JSON 全部存檔").changed() {
                            api_packet::set_capture_enabled(on);
                            show_notification = Some(if on { "API 擷取已開啟".into() } else { "API 擷取已關閉".into() });
                        }
                        if on {
                            ui.small(format!("已抓 {} 筆。檔案很大而且含帳號明文資料，用完記得關", api_packet::capture_count()));
                        }
                    }

                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("更多").default_open(false).show(ui, |ui| {
                        if ui.button(t!("menu.toggle_game_ui")).clicked() {
                            Thread::main_thread().schedule(Self::toggle_game_ui);
                        }
                        #[cfg(not(target_os = "windows"))]
                        if ui.button(t!("menu.open_in_game_browser")).clicked() {
                            show_window = Some(Box::new(SimpleYesNoDialog::new(&t!("confirm_dialog_title"), &t!("in_game_browser_confirm_content"), |ok| {
                                if !ok { return; }
                                Thread::main_thread().schedule(|| {
                                    WebViewManager::quick_open(&t!("browser_dialog_title"), &Hachimi::instance().config.load().open_browser_url);
                                });
                            })));
                        }
                        if ui.button(format!("{}…", t!("menu.soft_restart"))).clicked() {
                            show_window = Some(Box::new(SimpleYesNoDialog::new(&t!("confirm_dialog_title"), &t!("soft_restart_confirm_content"), |ok| {
                                if !ok { return; }
                                Thread::main_thread().schedule(|| {
                                    GameSystem::SoftwareReset(GameSystem::instance());
                                });
                            })));
                        }
                    });
                });
            });
        });

        if !self.show_menu {
            if let Some(time) = self.menu_anim_time {
                if time.elapsed().as_secs_f32() >= ctx.global_style().animation_time {
                    self.menu_visible = false;
                }
            }
            else {
                self.menu_anim_time = Some(Instant::now());
            }
        }

        if let Some(content) = show_notification {
            self.show_notification(content.as_ref());
        }

        if let Some(window) = show_window {
            self.show_window(window);
        }
    }

    fn toggle_game_ui() {
        use crate::il2cpp::hook::{
            UnityEngine_CoreModule::{Object, Behaviour},
            UnityEngine_UIModule::Canvas
        };

        let canvas_array = Object::FindObjectsOfType(Canvas::type_object(), true);
        let canvas_iter = unsafe { canvas_array.as_slice().iter() };

        if unsafe { DISABLED_GAME_UIS.is_empty() } {
            for canvas in canvas_iter {
                if Behaviour::get_enabled(*canvas) {
                    Behaviour::set_enabled(*canvas, false);
                    unsafe { DISABLED_GAME_UIS.insert(*canvas); }
                }
            }
        }
        else {
            for canvas in canvas_iter {
                if unsafe { DISABLED_GAME_UIS.contains(canvas) } {
                    Behaviour::set_enabled(*canvas, true);
                }
            }
            unsafe { DISABLED_GAME_UIS.clear(); }
        }
    }

    #[cfg(target_os = "windows")]
    fn run_vsync_combo(ui: &mut egui::Ui, value: &mut i32) {
        Self::run_combo(ui, "vsync_combo", value, &[
            (-1, &t!("default")),
            (0, &t!("off")),
            (1, &t!("on")),
            (2, "1/2"),
            (3, "1/3"),
            (4, "1/4")
        ]);
    }

    fn run_combo<T: PartialEq + Copy>(
        ui: &mut egui::Ui,
        id_child: impl std::hash::Hash + std::fmt::Debug,
        value: &mut T,
        choices: &[(T, &str)]
    ) -> bool {
        let mut selected = "Unknown";
        for choice in choices.iter() {
            if *value == choice.0 {
                selected = choice.1;
            }
        }

        let mut changed = false;
        egui::ComboBox::new(ui.id().with(id_child), "")
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for choice in choices.iter() {
                changed |= ui.selectable_value(value, choice.0, choice.1).changed();
            }
        });

        changed
    }

    fn run_notifications(&mut self) {
        let mut offset: f32 = -16.0;
        self.notifications.retain_mut(|n| n.run(&self.context, &mut offset));
    }

    fn run_windows(&mut self) {
        self.windows.retain_mut(|w| w.run(&self.context));
    }

    pub fn is_empty(&self) -> bool {
        !self.splash_visible && !self.menu_visible &&
        self.notifications.is_empty() && self.windows.is_empty()
    }

    pub fn is_consuming_input(&self) -> bool {
        self.menu_visible || !self.windows.is_empty()
    }

    pub fn is_consuming_input_atomic() -> bool {
        IS_CONSUMING_INPUT.load(atomic::Ordering::Relaxed)
    }

    pub fn toggle_menu(&mut self) {
        self.show_menu = !self.show_menu;
        // Menu is always visible on show, but not immediately invisible on hide
        if self.show_menu {
            self.menu_visible = true;
        }
        else {
            self.menu_anim_time = None;
        }
    }

    pub fn show_notification(&mut self, content: &str) {
        self.notifications.push(Notification::new(content.to_owned()));
    }

    pub fn show_window(&mut self, window: BoxedWindow) {
        self.windows.push(window);
    }
}

struct TweenInOutWithDelay {
    tween_time: f32,
    delay_duration: f32,
    easing: Easing,

    started: bool,
    delay_start: Option<Instant>
}

enum Easing {
    //Linear,
    //InQuad,
    OutQuad
}

impl TweenInOutWithDelay {
    fn new(tween_time: f32, delay_duration: f32, easing: Easing) -> TweenInOutWithDelay {
        TweenInOutWithDelay {
            tween_time,
            delay_duration,
            easing,

            started: false,
            delay_start: None
        }
    }

    fn run(&mut self, ctx: &egui::Context, id: egui::Id) -> Option<f32> {
        let anim_dir = if let Some(start) = self.delay_start {
            // Hold animation at peak position until duration passes
            start.elapsed().as_secs_f32() < self.delay_duration
        }
        else {
            // On animation start, initialize to 0.0. Next calls will start tweening to 1.0
            let v = self.started;
            self.started = true;
            v
        };
        let tween_val = ctx.animate_bool_with_time(id, anim_dir, self.tween_time);

        // Switch on delay when animation hits peak (next call makes tween_val < 1.0)
        if tween_val == 1.0 && self.delay_start.is_none() {
            self.delay_start = Some(Instant::now());
        }
        // Check if everything's done
        else if tween_val == 0.0 && self.delay_start.is_some() {
            return None;
        }

        Some(
            match self.easing {
                //Easing::Linear => tween_val,
                //Easing::InQuad => tween_val * tween_val,
                Easing::OutQuad => 1.0 - (1.0 - tween_val) * (1.0 - tween_val)
            }
        )
    }
}

// quick n dirty random id generator
fn random_id() -> egui::Id {
    egui::Id::new(std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), 0))
}

struct Notification {
    content: String,
    tween: TweenInOutWithDelay,
    id: egui::Id
}

impl Notification {
    fn new(content: String) -> Notification {
        Notification {
            content,
            tween: TweenInOutWithDelay::new(0.2, 3.0, Easing::OutQuad),
            id: random_id()
        }
    }

    const WIDTH: f32 = 150.0;
    fn run(&mut self, ctx: &egui::Context, offset: &mut f32) -> bool {
        let Some(tween_val) = self.tween.run(ctx, self.id.with("tween")) else {
            return false;
        };

        let frame_rect = egui::Area::new(self.id)
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::Vec2::new(
                Self::WIDTH * (1.0 - tween_val),
                *offset
            )
        )
        .show(ctx, |ui| {
            egui::Frame::NONE
            .fill(BACKGROUND_COLOR)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(Self::WIDTH);
                ui.label(&self.content);
            }).response.rect
        }).inner;

        *offset -= 2.0 + frame_rect.height() * tween_val;
        true
    }
}

pub trait Window {
    fn run(&mut self, ctx: &egui::Context) -> bool;
}

// Shared window creation function
fn new_window<'a>(ctx: &egui::Context, title: impl Into<egui::WidgetText>) -> egui::Window<'a> {
    // 遊戲視窗很窄（直式）或介面縮放調大時，固定尺寸會超出畫面；上限跟著畫面大小走
    let screen = ctx.content_rect();
    egui::Window::new(title)
    .pivot(egui::Align2::CENTER_CENTER)
    .fixed_pos(screen.max / 2.0)
    .max_width(320.0f32.min(screen.width() - 24.0))
    .max_height(250.0f32.min(screen.height() - 80.0))
    .collapsible(false)
    .resizable(false)
}

/// 選單上的小標題
fn menu_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).strong());
}

#[cfg(target_os = "windows")]
fn open_folder(dir: &std::path::Path) {
    _ = std::fs::create_dir_all(dir);
    _ = std::process::Command::new("explorer").arg(dir).spawn();
}

/// 擷取目前目標的因子卡片。下載立繪＋繪圖要一點時間，丟到背景執行緒。
#[cfg(target_os = "windows")]
fn capture_factor_card() {
    std::thread::spawn(|| {
        let msg = match super::factor_card::capture(None) {
            Ok(path) => format!(
                "因子卡片已存到 {}",
                path.file_name().map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned())
            ),
            Err(e) => format!("因子卡片失敗：{e}")
        };
        if let Some(mutex) = Gui::instance() {
            mutex.lock().unwrap().show_notification(&msg);
        }
    });
}

/// 置中的對話框（egui Modal：底下其他東西都點不到，按鈕列不會被擠掉）。
/// 回傳 `should_close`（按 Esc 或點背景）。
fn modal_dialog(
    ctx: &egui::Context, id: egui::Id, title: &str, content: &str, add_buttons: impl FnOnce(&mut egui::Ui)
) -> bool {
    egui::Modal::new(id).show(ctx, |ui| {
        ui.set_width(280.0f32.min(ctx.content_rect().width() - 48.0));
        ui.heading(title);
        ui.add_space(6.0);
        ui.label(content);
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add_buttons);
    }).should_close()
}

pub struct SimpleYesNoDialog {
    title: String,
    content: String,
    callback: fn(bool),
    id: egui::Id
}

impl SimpleYesNoDialog {
    pub fn new(title: &str, content: &str, callback: fn(bool)) -> SimpleYesNoDialog {
        SimpleYesNoDialog {
            title: title.to_owned(),
            content: content.to_owned(),
            callback,
            id: random_id()
        }
    }
}

impl Window for SimpleYesNoDialog {
    fn run(&mut self, ctx: &egui::Context) -> bool {
        let mut answer: Option<bool> = None;
        let dismissed = modal_dialog(ctx, self.id, &self.title, &self.content, |ui| {
            if ui.button(t!("yes")).clicked() {
                answer = Some(true);
            }
            if ui.button(t!("no")).clicked() {
                answer = Some(false);
            }
        });
        if dismissed && answer.is_none() {
            answer = Some(false);
        }

        match answer {
            Some(result) => {
                (self.callback)(result);
                false
            }
            None => true
        }
    }
}

pub struct SimpleOkDialog {
    title: String,
    content: String,
    callback: fn(),
    id: egui::Id
}

impl SimpleOkDialog {
    pub fn new(title: &str, content: &str, callback: fn()) -> SimpleOkDialog {
        SimpleOkDialog {
            title: title.to_owned(),
            content: content.to_owned(),
            callback,
            id: random_id()
        }
    }
}

impl Window for SimpleOkDialog {
    fn run(&mut self, ctx: &egui::Context) -> bool {
        let mut done = false;
        let dismissed = modal_dialog(ctx, self.id, &self.title, &self.content, |ui| {
            if ui.button(t!("ok")).clicked() {
                done = true;
            }
        });

        if done || dismissed {
            (self.callback)();
            false
        }
        else {
            true
        }
    }
}

pub struct PersistentMessageWindow {
    id: egui::Id,
    title: String,
    content: String,
    show: Arc<AtomicBool>
}

impl PersistentMessageWindow {
    pub fn new(title: &str, content: &str, show: Arc<AtomicBool>) -> PersistentMessageWindow {
        PersistentMessageWindow {
            id: random_id(),
            title: title.to_owned(),
            content: content.to_owned(),
            show
        }
    }
}

impl Window for PersistentMessageWindow {
    fn run(&mut self, ctx: &egui::Context) -> bool {
        // 沒有按鈕、也不能用 Esc 關：由呼叫端把 show 設成 false 才消失
        modal_dialog(ctx, self.id, &self.title, &self.content, |_| {});
        self.show.load(atomic::Ordering::Relaxed)
    }
}

/// 設定的內容（導覽＋各頁）。每個設定只在這裡有一個家；改了即時生效、自動存檔（走 settings），
/// 沒有儲存／取消。遊戲內是包在 egui Window 裡（`impl Window`），獨立設定視窗則直接畫 `show`。
pub(crate) struct ConfigEditor {
    id: egui::Id,
    page: SettingsPage
}

#[derive(Eq, PartialEq, Clone, Copy)]
pub(crate) enum SettingsPage {
    General,
    Display,
    Game,
    #[cfg(target_os = "windows")]
    AutoSkill,
    #[cfg(target_os = "windows")]
    FactorCard,
    Capture,
    About
}

impl SettingsPage {
    fn all() -> Vec<(SettingsPage, &'static str)> {
        vec![
            (SettingsPage::General, "一般"),
            (SettingsPage::Display, "畫面"),
            (SettingsPage::Game, "遊戲"),
            #[cfg(target_os = "windows")]
            (SettingsPage::AutoSkill, "一鍵學習"),
            #[cfg(target_os = "windows")]
            (SettingsPage::FactorCard, "因子卡片"),
            (SettingsPage::Capture, "擷取"),
            (SettingsPage::About, "關於"),
        ]
    }
}

/// 兩欄（標籤｜控制項）的設定列表
fn settings_grid(ui: &mut egui::Ui, id: egui::Id, add_rows: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
    .striped(true)
    .num_columns(2)
    .spacing([24.0, 8.0])
    .show(ui, add_rows);
}

impl ConfigEditor {
    pub(crate) fn new() -> ConfigEditor {
        ConfigEditor {
            id: random_id(),
            page: SettingsPage::General
        }
    }

    /// 畫導覽和目前這一頁，有改就 commit。寬度不夠放左側導覽時改成上方分頁。
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        let before = Hachimi::instance().config.load_full();
        // 每幀從目前的設定拷一份來改，選單那邊同時改的值也看得到，不會被這裡蓋回去
        let mut config = (*before).clone();

        if ui.available_width() < 440.0 {
            ui.horizontal_wrapped(|ui| {
                for (page, label) in SettingsPage::all() {
                    ui.selectable_value(&mut self.page, page, label);
                }
            });
            ui.separator();
            self.run_page_scroll(ui, &mut config);
        }
        else {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(92.0);
                    for (page, label) in SettingsPage::all() {
                        if ui.add_sized([92.0, 30.0], egui::Button::selectable(self.page == page, label)).clicked() {
                            self.page = page;
                        }
                    }
                });
                ui.separator();
                // 外層是水平排列（導覽｜內容），內容要自己開一個垂直排列，否則整頁會排成一長條橫列
                ui.vertical(|ui| self.run_page_scroll(ui, &mut config));
            });
        }

        // 有改才 commit（比對序列化結果，Config 沒有實作 PartialEq）
        if serde_json::to_string(&*before).ok() != serde_json::to_string(&config).ok() {
            super::settings::commit(config);
        }
    }

    /// 內容區：只上下捲動、填滿剩下的空間，文字依寬度自動換行
    fn run_page_scroll(&mut self, ui: &mut egui::Ui, config: &mut hachimi::Config) {
        egui::ScrollArea::vertical()
        .id_salt("settings_body")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width());
            self.run_page(ui, config);
        });
    }

    fn run_page(&mut self, ui: &mut egui::Ui, config: &mut hachimi::Config) {
        let grid_id = self.id.with(("grid", self.page as u8));
        match self.page {
            SettingsPage::General => settings_grid(ui, grid_id, |ui| Self::page_general(ui, config)),
            SettingsPage::Display => settings_grid(ui, grid_id, |ui| Self::page_display(ui, config)),
            SettingsPage::Game => settings_grid(ui, grid_id, |ui| Self::page_game(ui, config)),
            #[cfg(target_os = "windows")]
            SettingsPage::AutoSkill => Self::page_auto_skill(ui, config),
            #[cfg(target_os = "windows")]
            SettingsPage::FactorCard => Self::page_factor_card(ui),
            SettingsPage::Capture => Self::page_capture(ui),
            SettingsPage::About => Self::page_about(ui),
        }
    }

    fn page_general(ui: &mut egui::Ui, config: &mut hachimi::Config) {
        ui.label(t!("config_editor.language"));
        Gui::run_combo(ui, "language", &mut config.language, Language::CHOICES);
        ui.end_row();

        #[cfg(target_os = "windows")]
        {
            use crate::windows::wnd_hook;

            ui.label("開啟選單的按鍵");
            // 按下按鈕後由 wndproc 攔下一個按鍵，這裡每幀輪詢結果
            if let Some(key) = wnd_hook::take_captured_key() {
                config.windows.menu_open_key = key;
            }
            let capturing = wnd_hook::is_capturing_key();
            let label = if capturing {
                "請按下新按鍵…".to_owned()
            }
            else {
                wnd_hook::key_display_name(config.windows.menu_open_key)
            };
            if ui.button(label).clicked() {
                if capturing { wnd_hook::cancel_key_capture(); }
                else { wnd_hook::begin_key_capture(); }
            }
            ui.end_row();
        }

        ui.label("自動更新")
            .on_hover_text("開：發現新版自動背景下載，完成後通知重開遊戲。\n關：只在右下角通知有新版，不下載。");
        ui.checkbox(&mut config.auto_update, "");
        ui.end_row();

        #[cfg(target_os = "windows")]
        {
            ui.label("視窗標題");
            let mut title = config.windows.custom_title_name.clone().unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut title).desired_width(150.0)).changed() {
                config.windows.custom_title_name =
                    (!title.trim().is_empty()).then(|| title.trim().to_owned());
            }
            ui.end_row();

            ui.label("Discord 顯示遊戲活動");
            ui.checkbox(&mut config.windows.enable_discord_rpc, "");
            ui.end_row();
        }

        ui.label("選單縮放");
        ui.add(egui::Slider::new(&mut config.gui_scale, 0.5..=3.0).step_by(0.05));
        ui.end_row();

        #[cfg(target_os = "windows")]
        {
            ui.label("設定開在遊戲內")
                .on_hover_text("預設會開一個獨立的設定視窗。用獨佔全螢幕時切到別的視窗遊戲會縮小，可以改開在遊戲畫面裡。");
            ui.checkbox(&mut config.windows.settings_in_game, "");
            ui.end_row();
        }

        ui.label(t!("config_editor.disable_overlay"));
        if ui.checkbox(&mut config.disable_gui, "").clicked() && config.disable_gui {
            thread::spawn(|| {
                Gui::instance().unwrap()
                .lock().unwrap()
                .show_window(Box::new(SimpleOkDialog::new(
                    &t!("warning"),
                    &t!("config_editor.disable_overlay_warning"),
                    || {}
                )));
            });
        }
        ui.end_row();

        ui.label(t!("config_editor.debug_mode"));
        ui.checkbox(&mut config.debug_mode, "");
        ui.end_row();

        ui.label("");
        if ui.button("從 config.json 重新載入")
            .on_hover_text("手動改過 config.json 時用")
            .clicked()
        {
            Hachimi::instance().reload_config();
        }
        ui.end_row();
    }

    fn page_display(ui: &mut egui::Ui, config: &mut hachimi::Config) {
        Self::option_slider(ui, &t!("config_editor.target_fps"), &mut config.target_fps, 30..=240);

        #[cfg(target_os = "windows")]
        {
            use crate::windows::hachimi_impl::{FullScreenMode, ResolutionScaling};

            ui.label(t!("config_editor.vsync"));
            Gui::run_vsync_combo(ui, &mut config.windows.vsync_count);
            ui.end_row();

            ui.label(t!("config_editor.window_always_on_top"));
            ui.checkbox(&mut config.windows.window_always_on_top, "");
            ui.end_row();

            ui.label("隱藏遊戲游標");
            ui.checkbox(&mut config.windows.disable_game_cursor, "");
            ui.end_row();

            ui.label(t!("config_editor.auto_full_screen"));
            ui.checkbox(&mut config.windows.auto_full_screen, "");
            ui.end_row();

            ui.label(t!("config_editor.full_screen_mode"));
            Gui::run_combo(ui, "full_screen_mode", &mut config.windows.full_screen_mode, &[
                (FullScreenMode::ExclusiveFullScreen, &t!("config_editor.full_screen_mode_exclusive")),
                (FullScreenMode::FullScreenWindow, &t!("config_editor.full_screen_mode_borderless"))
            ]);
            ui.end_row();

            ui.label(t!("config_editor.block_minimize_in_full_screen"));
            ui.checkbox(&mut config.windows.block_minimize_in_full_screen, "");
            ui.end_row();

            ui.label(t!("config_editor.resolution_scaling"));
            Gui::run_combo(ui, "resolution_scaling", &mut config.windows.resolution_scaling, &[
                (ResolutionScaling::Default, &t!("config_editor.resolution_scaling_default")),
                (ResolutionScaling::ScaleToScreenSize, &t!("config_editor.resolution_scaling_ssize")),
                (ResolutionScaling::ScaleToWindowSize, &t!("config_editor.resolution_scaling_wsize"))
            ]);
            ui.end_row();
        }

        ui.label(t!("config_editor.virtual_resolution_multiplier"));
        ui.add(egui::Slider::new(&mut config.virtual_res_mult, 1.0..=4.0).step_by(0.1));
        ui.end_row();

        ui.label(t!("config_editor.ui_scale"));
        ui.add(egui::Slider::new(&mut config.ui_scale, 0.1..=10.0).step_by(0.05));
        ui.end_row();

        ui.label(t!("config_editor.ui_animation_scale"));
        ui.add(egui::Slider::new(&mut config.ui_animation_scale, 0.1..=10.0).step_by(0.1));
        ui.end_row();

        ui.label(t!("config_editor.graphics_quality"));
        Gui::run_combo(ui, "graphics_quality", &mut config.graphics_quality, &[
            (GraphicsQuality::Default, &t!("default")),
            (GraphicsQuality::Toon1280, "Toon1280"),
            (GraphicsQuality::Toon1280x2, "Toon1280x2"),
            (GraphicsQuality::Toon1280x4, "Toon1280x4"),
            (GraphicsQuality::ToonFull, "ToonFull"),
            (GraphicsQuality::Max, "Max")
        ]);
        ui.end_row();
    }

    fn page_game(ui: &mut egui::Ui, config: &mut hachimi::Config) {
        ui.label(t!("config_editor.story_text_speed_multiplier"));
        ui.add(egui::Slider::new(&mut config.story_tcps_multiplier, 0.1..=10.0).step_by(0.1));
        ui.end_row();

        ui.label(t!("config_editor.story_choice_auto_select_delay"));
        ui.add(egui::Slider::new(&mut config.story_choice_auto_select_delay, 0.1..=10.0).step_by(0.05));
        ui.end_row();

        ui.label(t!("config_editor.skill_data_desc"))
            .on_hover_text(t!("config_editor.skill_data_desc_hint"));
        ui.checkbox(&mut config.skill_data_desc, "");
        ui.end_row();

        ui.label(t!("config_editor.force_allow_dynamic_camera"));
        ui.checkbox(&mut config.force_allow_dynamic_camera, "");
        ui.end_row();

        ui.label(t!("config_editor.physics_update_mode"));
        Gui::run_combo(ui, "physics_update_mode", &mut config.physics_update_mode, &[
            (None, &t!("default")),
            (SpringUpdateMode::ModeNormal.into(), "ModeNormal"),
            (SpringUpdateMode::Mode60FPS.into(), "Mode60FPS"),
            (SpringUpdateMode::SkipFrame.into(), "SkipFrame"),
            (SpringUpdateMode::SkipFramePostAlways.into(), "SkipFramePostAlways")
        ]);
        ui.end_row();

        ui.label(t!("config_editor.live_theater_allow_same_chara"));
        ui.checkbox(&mut config.live_theater_allow_same_chara, "");
        ui.end_row();

        ui.label("演唱會播放速度");
        ui.add(egui::Slider::new(&mut config.live_playback_speed, 0.1..=4.0).step_by(0.05));
        ui.end_row();

        // 只在真的在播的時候顯示進度，其他畫面掛一條不動的條沒有意義
        {
            use crate::il2cpp::hook::umamusume::Director;
            if Director::is_live_active() {
                let (current, total) = Director::live_progress();
                ui.label("播放進度");
                ui.label(format!("{:.0}:{:02.0} / {:.0}:{:02.0}",
                    current / 60.0, current % 60.0, total / 60.0, total % 60.0));
                ui.end_row();
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn page_auto_skill(ui: &mut egui::Ui, config: &mut hachimi::Config) {
        ui.label("在育成技能頁按「一鍵學習」→ 選主要或次要，依清單由上往下點技能，點數不夠的跳過，最後跳出遊戲的確認視窗。");
        ui.add_space(6.0);
        ui.label(egui::RichText::new("設定檔").strong());
        Self::run_auto_skill_profiles(ui, config);

        let active = config.auto_skill_active_profile;
        if let Some(profile) = config.auto_skill_profiles.get_mut(active) {
            ui.add_space(8.0);
            ui.label(egui::RichText::new("主要清單").strong())
                .on_hover_text("「從技能頁匯入」會把最後開過的技能頁上還沒學的技能加進來（已在清單的不重複）。");
            Self::run_auto_skill_list(ui, "primary", &mut profile.primary);
            ui.add_space(8.0);
            ui.label(egui::RichText::new("次要清單").strong());
            Self::run_auto_skill_list(ui, "secondary", &mut profile.secondary);
        }
    }

    #[cfg(target_os = "windows")]
    fn page_factor_card(ui: &mut egui::Ui) {
        use crate::core::factor_card;

        ui.label(format!("已收集 {} 隻練成角色", factor_card::stored_count()));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("卡片主題");
            let mut light = factor_card::light_theme();
            let before = light;
            ui.selectable_value(&mut light, false, "暗色");
            ui.selectable_value(&mut light, true, "亮色");
            if light != before {
                factor_card::set_light_theme(light);
            }
        });
        ui.add_space(6.0);
        ui.label("輸出位置（留空＝預設 hachimi\\factor_card）");
        // 打字中先放在 egui 暫存，按「套用」才寫進設定，免得每打一個字就改一次資料夾
        let id = ui.id().with("factor_card_dir");
        let mut path = ui.data_mut(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| Hachimi::instance().config.load().factor_card_output_dir.clone().unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut path).desired_width(f32::INFINITY));
        ui.horizontal(|ui| {
            if ui.button("套用").clicked() {
                factor_card::set_output_dir(&path);
            }
            if ui.button("開資料夾").clicked() {
                open_folder(&factor_card::output_dir());
            }
        });
        ui.data_mut(|d| d.insert_temp(id, path));
    }

    fn page_capture(ui: &mut egui::Ui) {
        use crate::core::api_packet::practice_race;

        let mut on = practice_race::capture_enabled();
        if ui.checkbox(&mut on, "跑練習賽／自訂配對賽時自動存下該場封包").changed() {
            practice_race::set_capture_enabled(on);
        }
        ui.small("練習賽、自訂配對賽、群英聯賽；逐幀資料已解好放進 JSON，其他封包不理。");
        #[cfg(target_os = "windows")]
        if ui.button("開啟練習賽資料夾").clicked() {
            open_folder(&practice_race::capture_dir());
        }

        #[cfg(feature = "datamine")]
        {
            use crate::core::api_packet;

            ui.separator();
            let mut on = api_packet::capture_enabled();
            if ui.checkbox(&mut on, "把 API 回傳的 JSON 全部存檔").changed() {
                api_packet::set_capture_enabled(on);
            }
            ui.small("檔案很大而且含帳號明文資料，用完記得關。");
            #[cfg(target_os = "windows")]
            if ui.button("開啟擷取資料夾").clicked() {
                open_folder(&api_packet::capture_dir());
            }
        }
    }

    fn page_about(ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(Gui::icon_2x());
            ui.vertical(|ui| {
                ui.heading(t!("hachimi"));
                ui.label(concat!("v", env!("CARGO_PKG_VERSION")));
            });
        });
        ui.label(t!("about.copyright"));
        ui.horizontal(|ui| {
            if ui.button(t!("about.view_license")).clicked() {
                thread::spawn(|| {
                    Gui::instance().unwrap()
                    .lock().unwrap()
                    .show_window(Box::new(LicenseWindow::new()));
                });
            }
            #[cfg(target_os = "windows")]
            if ui.button(t!("about.check_for_updates")).clicked() {
                Hachimi::instance().updater.clone().check_for_updates(true);
            }
        });
    }

    fn option_slider<Num: egui::emath::Numeric>(ui: &mut egui::Ui, label: &str, value: &mut Option<Num>, range: RangeInclusive<Num>) {
        let mut checked = value.is_some();
        ui.label(label);
        ui.checkbox(&mut checked, t!("enable"));
        ui.end_row();

        if checked && value.is_none() {
            *value = Some(*range.start())
        }
        else if !checked && value.is_some() {
            *value = None;
        }

        if let Some(num) = value.as_mut() {
            ui.label("");
            ui.add(egui::Slider::new(num, range));
            ui.end_row();
        }
    }

    /// 一鍵學習：設定檔選擇／新增／複製／改名／刪除
    #[cfg(target_os = "windows")]
    fn run_auto_skill_profiles(ui: &mut egui::Ui, config: &mut hachimi::Config) {
        use hachimi::AutoSkillProfile;

        let profiles = &mut config.auto_skill_profiles;
        if profiles.is_empty() {
            profiles.push(AutoSkillProfile { name: "預設".to_owned(), ..Default::default() });
        }
        let active = &mut config.auto_skill_active_profile;
        if *active >= profiles.len() {
            *active = 0;
        }

        ui.vertical(|ui| {
            egui::ComboBox::from_id_salt("auto_skill_profile")
                .width(140.0)
                .selected_text(profiles[*active].name.clone())
                .show_ui(ui, |ui| {
                    for (i, p) in profiles.iter().enumerate() {
                        ui.selectable_value(active, i, p.name.clone());
                    }
                });
            ui.horizontal(|ui| {
                if ui.button("新增").clicked() {
                    profiles.push(AutoSkillProfile {
                        name: format!("設定檔 {}", profiles.len() + 1),
                        ..Default::default()
                    });
                    *active = profiles.len() - 1;
                }
                if ui.button("複製").clicked() {
                    let mut copy = profiles[*active].clone();
                    copy.name = format!("{} 複本", copy.name);
                    profiles.push(copy);
                    *active = profiles.len() - 1;
                }
                if ui.add_enabled(profiles.len() > 1, egui::Button::new("刪除")).clicked() {
                    profiles.remove(*active);
                    *active = active.saturating_sub(1);
                }
            });
            ui.horizontal(|ui| {
                ui.label("名稱");
                ui.add(egui::TextEdit::singleline(&mut profiles[*active].name).desired_width(140.0));
            });
        });
    }

    /// 一鍵學習的一份清單：一列一個技能，可上下移、刪除；可手動加一筆或從技能頁匯入。
    /// `key` 區分主要／次要（各自的輸入框與匯入目標）。
    #[cfg(target_os = "windows")]
    fn run_auto_skill_list(ui: &mut egui::Ui, key: &'static str, list: &mut Vec<String>) {
        use crate::il2cpp::hook::umamusume::SingleModeSkillLearningViewController as auto_skill;

        // 匯入結果在主執行緒算完後才回來，這裡每幀輪詢；只有發出匯入的那份清單收結果
        static IMPORT_TARGET: Mutex<Option<&'static str>> = Mutex::new(None);
        static STATUS: Mutex<(&'static str, String)> = Mutex::new(("", String::new()));
        if *IMPORT_TARGET.lock().unwrap() == Some(key) {
            if let Some(res) = auto_skill::take_import_result() {
                *IMPORT_TARGET.lock().unwrap() = None;
                let msg = match res {
                    Ok(names) => {
                        let before = list.len();
                        for n in names {
                            if !list.iter().any(|s| s.trim() == n) {
                                list.push(n);
                            }
                        }
                        format!("加入 {} 個技能", list.len() - before)
                    }
                    Err(e) => e,
                };
                *STATUS.lock().unwrap() = (key, msg);
            }
        }
        list.retain(|s| !s.trim().is_empty());

        let entry_id = ui.id().with(("auto_skill_entry", key));
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui.button("從技能頁匯入").clicked() {
                    *IMPORT_TARGET.lock().unwrap() = Some(key);
                    *STATUS.lock().unwrap() = (key, "讀取中…".into());
                    auto_skill::request_import();
                }
                if ui.button("清空").clicked() {
                    list.clear();
                }
            });
            {
                let status = STATUS.lock().unwrap();
                if status.0 == key && !status.1.is_empty() {
                    ui.label(status.1.clone());
                }
            }

            let mut action: Option<(usize, i32)> = None; // (index, -1 上移 / 1 下移 / 0 刪除)
            egui::ScrollArea::vertical()
                .id_salt(("auto_skill_list", key))
                .max_height(200.0)
                .show(ui, |ui| {
                    for (i, name) in list.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}.", i + 1));
                            // FontAwesome：arrow-up / arrow-down / times
                            if ui.small_button("\u{f062}").clicked() { action = Some((i, -1)); }
                            if ui.small_button("\u{f063}").clicked() { action = Some((i, 1)); }
                            if ui.small_button("\u{f00d}").clicked() { action = Some((i, 0)); }
                            ui.label(name);
                        });
                    }
                });
            match action {
                Some((i, 0)) => { list.remove(i); }
                Some((i, -1)) if i > 0 => list.swap(i, i - 1),
                Some((i, 1)) if i + 1 < list.len() => list.swap(i, i + 1),
                _ => {}
            }

            ui.horizontal(|ui| {
                let mut entry = ui.data_mut(|d| d.get_temp::<String>(entry_id)).unwrap_or_default();
                ui.add(egui::TextEdit::singleline(&mut entry).hint_text("技能名稱或 ID").desired_width(120.0));
                if ui.button("加入").clicked() && !entry.trim().is_empty() {
                    list.push(entry.trim().to_owned());
                    entry.clear();
                }
                ui.data_mut(|d| d.insert_temp(entry_id, entry));
            });
        });
    }

}

impl Window for ConfigEditor {
    fn run(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let screen = ctx.content_rect();
        let size = egui::vec2(
            560.0f32.min(screen.width() - 24.0),
            480.0f32.min(screen.height() - 80.0)
        );

        egui::Window::new(t!("config_editor.title"))
        .id(self.id)
        .pivot(egui::Align2::CENTER_CENTER)
        .fixed_pos(screen.center())
        .fixed_size(size)
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| self.show(ui));

        if !open {
            super::settings::flush();
        }
        open
    }
}

struct LicenseWindow {
    id: egui::Id
}

impl LicenseWindow {
    fn new() -> LicenseWindow {
        LicenseWindow {
            id: random_id()
        }
    }
}

impl Window for LicenseWindow {
    fn run(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;

        new_window(ctx, t!("license.title"))
        .id(self.id)
        .open(&mut open)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label(include_str!("../../LICENSE"));
            });
        });

        open
    }
}
