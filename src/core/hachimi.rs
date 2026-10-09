use std::{fs, path::{Path, PathBuf}, process, sync::{atomic::{self, AtomicBool, AtomicI32}, Arc, Mutex}};
use arc_swap::ArcSwap;
use fnv::FnvHashSet;
use once_cell::sync::OnceCell;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{core::plugin_api::Plugin, gui_impl, hachimi_impl, il2cpp::{self, hook::umamusume::{CySpringController::SpringUpdateMode, GameSystem}}};

use super::{game::Game, utils, Error, Interceptor};

pub struct Hachimi {
    // Hooking stuff
    pub interceptor: Interceptor,
    pub hooking_finished: AtomicBool,
    pub plugins: Mutex<Vec<Plugin>>,

    // 字體／顯示用的在地化資料（翻譯已移除，只留字體相關）
    pub localized_data: ArcSwap<LocalizedData>,

    // Shared properties
    pub game: Game,
    pub config: ArcSwap<Config>,

    /// -1 = default
    pub target_fps: AtomicI32,

    #[cfg(target_os = "windows")]
    pub vsync_count: AtomicI32,

    #[cfg(target_os = "windows")]
    pub window_always_on_top: AtomicBool,

    #[cfg(target_os = "windows")]
    pub disable_game_cursor: AtomicBool,

    #[cfg(target_os = "windows")]
    pub updater: Arc<crate::windows::updater::Updater>
}

static INSTANCE: OnceCell<Arc<Hachimi>> = OnceCell::new();

impl Hachimi {
    pub fn init() -> bool {
        if INSTANCE.get().is_some() {
            warn!("Hachimi should be initialized only once");
            return true;
        }

        let instance = match Self::new() {
            Ok(v) => v,
            Err(e) => {
                super::log::init(false); // early init to log error
                error!("Init failed: {}", e);
                return false;
            }
        };

        let config = instance.config.load();
        if config.disable_gui_once {
            let mut config = config.as_ref().clone();
            config.disable_gui_once = false;
            _ = instance.save_config(&config);

            config.disable_gui = true;
            instance.config.store(Arc::new(config));
        }

        super::log::init(instance.config.load().debug_mode);

        info!("Hachimi TW {}", env!("HACHIMI_DISPLAY_VERSION"));
        info!("Game region: {}", instance.game.region);
        instance.load_localized_data();

        INSTANCE.set(Arc::new(instance)).is_ok()
    }

    pub fn instance() -> Arc<Hachimi> {
        INSTANCE.get().unwrap_or_else(|| {
            error!("FATAL: Attempted to get Hachimi instance before initialization");
            process::exit(1);
        }).clone()
    }

    pub fn is_initialized() -> bool {
        INSTANCE.get().is_some()
    }

    fn new() -> Result<Hachimi, Error> {
        let game = Game::init();
        let config = Self::load_config(&game.data_dir)?;

        config.language.set_locale();

        Ok(Hachimi {
            interceptor: Interceptor::default(),
            hooking_finished: AtomicBool::new(false),
            plugins: Mutex::default(),

            // Don't load localized data initially since it might fail, logging the error is not possible here
            localized_data: ArcSwap::default(),

            game,

            target_fps: AtomicI32::new(config.target_fps.unwrap_or(-1)),

            #[cfg(target_os = "windows")]
            vsync_count: AtomicI32::new(config.windows.vsync_count),

            #[cfg(target_os = "windows")]
            window_always_on_top: AtomicBool::new(config.windows.window_always_on_top),

            #[cfg(target_os = "windows")]
            disable_game_cursor: AtomicBool::new(config.windows.disable_game_cursor),

            #[cfg(target_os = "windows")]
            updater: Arc::default(),

            config: ArcSwap::new(Arc::new(config))
        })
    }

    fn load_config(data_dir: &Path) -> Result<Config, Error> {
        let config_path = data_dir.join("config.json");
        if fs::metadata(&config_path).is_ok() {
            let json = fs::read_to_string(&config_path)?;
            let mut config: Config = serde_json::from_str(&json)?;
            config.migrate();
            Ok(config)
        }
        else {
            Ok(Config::default())
        }
    }

    pub fn reload_config(&self) {
        let new_config = match Self::load_config(&self.game.data_dir) {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to reload config: {}", e);
                return;
            }
        };

        new_config.language.set_locale();
        self.config.store(Arc::new(new_config));
    }

    pub fn save_config(&self, config: &Config) -> Result<(), Error> {
        fs::create_dir_all(&self.game.data_dir)?;
        let config_path = self.get_data_path("config.json");
        utils::write_json_file(config, &config_path)?;

        Ok(())
    }

    pub fn save_and_reload_config(&self, config: Config) -> Result<(), Error> {
        self.save_config(&config)?;

        config.language.set_locale();
        self.config.store(Arc::new(config));
        Ok(())
    }

    pub fn load_localized_data(&self) {
        let new_data = match LocalizedData::new(&self.config.load(), &self.game.data_dir) {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to load localized data: {}", e);
                return;
            }
        };
        self.localized_data.store(Arc::new(new_data));
    }

    pub fn on_dlopen(&self, filename: &str, handle: usize) -> bool {
        // Prevent double initialization
        if self.hooking_finished.load(atomic::Ordering::Relaxed) { return false; }

        if hachimi_impl::is_il2cpp_lib(filename) {
            info!("Got il2cpp handle");
            il2cpp::symbols::set_handle(handle);
            false
        }
        else if hachimi_impl::is_criware_lib(filename) {
            self.on_hooking_finished();
            true
        }
        else {
            false
        }
    }

    pub fn on_hooking_finished(&self) {
        self.hooking_finished.store(true, atomic::Ordering::Relaxed);

        info!("GameAssembly finished loading");
        il2cpp::symbols::init();
        il2cpp::hook::init();

        // capture-only 建置：不做遊戲端初始化（UI scale）、不起 GUI、不起 IPC。
        // 這些依賴的 hook 在 capture-only 都沒裝，呼叫會踩空。
        #[cfg(not(feature = "capture-only"))]
        {
            // By the time it finished hooking the game will have already finished initializing
            GameSystem::on_game_initialized();

            let config = self.config.load();
            if !config.disable_gui {
                gui_impl::init();
            }
        }

        hachimi_impl::on_hooking_finished(self);

        for plugin in self.plugins.lock().unwrap().iter() {
            info!("Initializing plugin: {}", plugin.name);
            let res = plugin.init();
            if !res.is_ok() {
                info!("Plugin init failed");
            }
        }
    }

    pub fn get_data_path<P: AsRef<Path>>(&self, rel_path: P) -> PathBuf {
        self.game.data_dir.join(rel_path)
    }

    pub fn run_auto_update_check(&self) {
        // 每次啟動都在背景查一次（走 GitHub 網頁轉址，不吃 API 次數）。有新版時：auto_update 開就
        // 下載，關就只通知、選單按鈕變「有新版」。不再看 disable_auto_update_check——那是上游時代
        // 用來擋原版 Hachimi 查上游更新的，打包給朋友的 config 都是 true，結果關掉自動下載的人
        // 啟動時完全收不到新版通知。
        #[cfg(target_os = "windows")]
        self.updater.clone().check_for_updates(false);
    }
}

fn default_serde_instance<'a, T: Deserialize<'a>>() -> Option<T> {
    let empty_data = std::iter::empty::<((), ())>();
    let empty_deserializer = serde::de::value::MapDeserializer::<_, serde::de::value::Error>::new(empty_data);
    T::deserialize(empty_deserializer).ok()
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub debug_mode: bool,
    #[serde(default)]
    pub disable_gui: bool,
    #[serde(default)]
    pub disable_gui_once: bool,
    pub localized_data_dir: Option<String>,
    pub target_fps: Option<i32>,
    #[serde(default = "Config::default_open_browser_url")]
    pub open_browser_url: String,
    #[serde(default = "Config::default_virtual_res_mult")]
    pub virtual_res_mult: f32,
    /// 已不使用（見 `run_auto_update_check`）；留著只為了讀得進舊 config
    #[serde(default)]
    pub disable_auto_update_check: bool,
    /// 測試用：把「最新 release」查詢網址換掉（例 `http://127.0.0.1:8765/latest.json`），
    /// 回傳格式同 GitHub `releases/latest`。未設定＝查 GitHub。設定編輯器不顯示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_check_url: Option<String>,
    /// 有新版時自動下載：開＝啟動時查到新版就背景下載、完成後通知重開；關＝只通知有新版，
    /// 由使用者在選單按「更新」再裝。不管開關，啟動時都會檢查。
    #[serde(default)]
    pub auto_update: bool,
    /// 因子卡片用亮色主題（預設暗色）
    #[serde(default)]
    pub factor_card_light_theme: bool,
    /// 因子卡片輸出資料夾（未設定＝`<data>/factor_card`）
    pub factor_card_output_dir: Option<String>,
    /// 把每個遊戲 API response 解出來的 JSON 全量落檔到 `<data>/api_capture/`。
    /// 撈 API 資料用，會產生大量檔案（單檔可到十幾 MB），預設關。
    #[serde(default)]
    pub api_capture: bool,
    /// 只把「練習賽結果」的 response 落檔到 `<data>/race_capture/`，檔名帶本機時間與場地距離
    /// （例 `20260906_231914_大井_ダート2000m.json`）。與 `api_capture` 獨立，預設關。
    #[serde(default)]
    pub practice_race_capture: bool,
    #[serde(default = "Config::default_ui_scale")]
    pub ui_scale: f32,
    /// Hachimi 自己這層介面的縮放（不是遊戲畫面）。高解析度螢幕上選單會太小，這個放大它。
    #[serde(default = "Config::default_gui_scale")]
    pub gui_scale: f32,
    /// 演唱會播放速度倍率（1.0＝原速）
    #[serde(default = "Config::default_live_playback_speed")]
    pub live_playback_speed: f32,
    #[serde(default)]
    pub graphics_quality: crate::il2cpp::hook::umamusume::GraphicSettings::GraphicsQuality,
    #[serde(default = "Config::default_story_choice_auto_select_delay")]
    pub story_choice_auto_select_delay: f32,
    #[serde(default = "Config::default_story_tcps_multiplier")]
    pub story_tcps_multiplier: f32,
    #[serde(default)]
    pub force_allow_dynamic_camera: bool,
    /// 技能說明改成顯示實際發動條件與效果數值（讀 master.mdb 的 skill_data，移植自 Edge）。預設關。
    #[serde(default)]
    pub skill_data_desc: bool,
    /// 舊版（單一清單）的一鍵學習設定，只讀不寫：載入時搬進 `auto_skill_profiles` 的第一個設定檔。
    #[serde(default, skip_serializing)]
    pub auto_skill_list: Vec<String>,
    /// 育成技能學習頁「一鍵學習」的設定檔。每個設定檔有主要／次要兩份清單，依優先順序；
    /// 每一項純數字＝技能 ID，其餘＝技能名稱（完全比對）。
    #[serde(default)]
    pub auto_skill_profiles: Vec<AutoSkillProfile>,
    /// 目前使用的設定檔（`auto_skill_profiles` 的索引）
    #[serde(default)]
    pub auto_skill_active_profile: usize,
    #[serde(default)]
    pub live_theater_allow_same_chara: bool,
    #[serde(default)]
    pub language: Language,
    pub physics_update_mode: Option<SpringUpdateMode>,
    #[serde(default = "Config::default_ui_animation_scale")]
    pub ui_animation_scale: f32,
    #[serde(default)]
    pub disabled_hooks: FnvHashSet<String>,

    #[cfg(target_os = "windows")]
    #[serde(flatten)]
    pub windows: hachimi_impl::Config,

    #[cfg(target_os = "android")]
    #[serde(flatten)]
    pub android: hachimi_impl::Config
}

impl Config {
    fn default_open_browser_url() -> String { "https://www.google.com/".to_owned() }
    fn default_virtual_res_mult() -> f32 { 1.0 }
    fn default_ui_scale() -> f32 { 1.0 }
    fn default_gui_scale() -> f32 { 1.0 }
    fn default_live_playback_speed() -> f32 { 1.0 }
    fn default_story_choice_auto_select_delay() -> f32 { 0.75 }
    fn default_story_tcps_multiplier() -> f32 { 1.0 }
    fn default_ui_animation_scale() -> f32 { 1.0 }
}

/// 一鍵學習的一組清單（例：「長距離逃」「短英大賽」）
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct AutoSkillProfile {
    pub name: String,
    #[serde(default)]
    pub primary: Vec<String>,
    #[serde(default)]
    pub secondary: Vec<String>,
}

impl Config {
    fn migrate(&mut self) {
        if !self.auto_skill_list.is_empty() && self.auto_skill_profiles.is_empty() {
            self.auto_skill_profiles.push(AutoSkillProfile {
                name: "預設".to_owned(),
                primary: std::mem::take(&mut self.auto_skill_list),
                secondary: Vec::new(),
            });
        }
    }

    /// 目前使用的一鍵學習設定檔（索引越界時退回第一個）
    pub fn active_auto_skill_profile(&self) -> Option<&AutoSkillProfile> {
        self.auto_skill_profiles.get(self.auto_skill_active_profile)
            .or_else(|| self.auto_skill_profiles.first())
    }
}

impl Default for Config {
    fn default() -> Self {
        default_serde_instance().expect("default instance")
    }
}

#[derive(Deserialize, Default, Clone)]
pub struct OsOption<T> {
    #[cfg(target_os = "android")]
    android: Option<T>,

    #[cfg(target_os = "windows")]
    windows: Option<T>
}

impl<T> OsOption<T> {
    pub fn as_ref(&self) -> Option<&T> {
        #[cfg(target_os = "android")]
        return self.android.as_ref();

        #[cfg(target_os = "windows")]
        return self.windows.as_ref();
    }
}

#[derive(Default, Copy, Clone, Eq, PartialEq, Deserialize, Serialize)]
#[allow(non_camel_case_types)]
pub enum Language {
    #[serde(rename = "en")]
    English,

    #[serde(rename = "zh-tw")]
    #[default] TChinese,

    #[serde(rename = "zh-cn")]
    SChinese,

    #[serde(rename = "vi")]
    Vietnamese
}

impl Language {
    pub const CHOICES: &[(Self, &'static str)] = &[
        Self::English.choice(),
        Self::TChinese.choice(),
        Self::SChinese.choice(),
        Self::Vietnamese.choice()
    ];

    pub fn set_locale(&self) {
        rust_i18n::set_locale(self.locale_str());
    }

    pub const fn locale_str(&self) -> &'static str {
        match self {
            Language::English => "en",
            Language::TChinese => "zh-tw",
            Language::SChinese => "zh-cn",
            Language::Vietnamese => "vi"
        }
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Language::English => "English",
            Language::TChinese => "繁體中文",
            Language::SChinese => "简体中文",
            Language::Vietnamese => "Tiếng Việt"
        }
    }

    pub const fn choice(self) -> (Self, &'static str) {
        (self, self.name())
    }
}

// 文字翻譯已移除，這裡只保留「字體／顯示」相關的設定與資產路徑（給字體載入器用）。
#[derive(Default)]
pub struct LocalizedData {
    pub config: LocalizedDataConfig,
    path: Option<PathBuf>,
    assets_path: Option<PathBuf>
}

impl LocalizedData {
    fn new(config: &Config, data_dir: &Path) -> Result<LocalizedData, Error> {
        let path: Option<PathBuf>;
        let config: LocalizedDataConfig = if let Some(ld_dir) = &config.localized_data_dir {
            let ld_path = Path::new(data_dir).join(ld_dir);

            // Create .nomedia
            #[cfg(target_os = "android")]
            { _ = fs::OpenOptions::new().create_new(true).write(true).open(ld_path.join(".nomedia")); }

            let ld_config_path = ld_path.join("config.json");
            path = Some(ld_path);

            if fs::metadata(&ld_config_path).is_ok() {
                let json = fs::read_to_string(&ld_config_path)?;
                serde_json::from_str(&json)?
            }
            else {
                warn!("Localized data config not found");
                LocalizedDataConfig::default()
            }
        }
        else {
            path = None;
            LocalizedDataConfig::default()
        };

        Ok(LocalizedData {
            assets_path: path.as_ref()
                .map(|p| config.assets_dir.as_ref()
                    .map(|dir| p.join(dir))
                )
                .unwrap_or_default(),

            config,
            path
        })
    }

    fn load_dict_static_ex<T: DeserializeOwned, P: AsRef<Path>>(ld_path_opt: &Option<PathBuf>, rel_path_opt: Option<P>, silent_fs_error: bool) -> Option<T> {
        let Some(ld_path) = ld_path_opt else {
            return None;
        };
        let Some(rel_path) = rel_path_opt else {
            return None;
        };

        let path = ld_path.join(rel_path);
        let json = match fs::read_to_string(&path) {
            Ok(v) => v,
            Err(e) => {
                if !silent_fs_error {
                    error!("Failed to read '{}': {}", path.display(), e);
                }
                return None;
            }
        };

        let dict = match serde_json::from_str::<T>(&json) {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to parse '{}': {}", path.display(), e);
                return None;
            }
        };

        Some(dict)
    }

    fn load_dict_static<T: DeserializeOwned, P: AsRef<Path>>(ld_path_opt: &Option<PathBuf>, rel_path_opt: Option<P>) -> Option<T> {
        Self::load_dict_static_ex(ld_path_opt, rel_path_opt, false)
    }

    pub fn load_dict<T: DeserializeOwned, P: AsRef<Path>>(&self, rel_path_opt: Option<P>) -> Option<T> {
        Self::load_dict_static(&self.path, rel_path_opt)
    }

    pub fn load_assets_dict<T: DeserializeOwned, P: AsRef<Path>>(&self, rel_path_opt: Option<P>) -> Option<T> {
        Self::load_dict_static_ex(&self.assets_path, rel_path_opt, true)
    }

    pub fn get_assets_path<P: AsRef<Path>>(&self, rel_path: P) -> Option<PathBuf> {
        self.assets_path.as_ref().map(|p| p.join(rel_path))
    }

    pub fn get_data_path<P: AsRef<Path>>(&self, rel_path: P) -> Option<PathBuf> {
        self.path.as_ref().map(|p| p.join(rel_path))
    }

    pub fn load_asset_metadata<P: AsRef<Path>>(&self, rel_path: P) -> AssetMetadata {
        let mut path = rel_path.as_ref().to_owned();
        path.set_extension("json");
        self.load_assets_dict(Some(path)).unwrap_or_else(|| AssetInfo::<()>::default()).metadata()
    }

    pub fn load_asset_info<P: AsRef<Path>, T: DeserializeOwned>(&self, rel_path: P) -> AssetInfo<T> {
        let mut path = rel_path.as_ref().to_owned();
        path.set_extension("json");
        self.load_assets_dict(Some(path)).unwrap_or_else(|| AssetInfo::default())
    }
}

// 文字翻譯已移除，這裡只保留「字體／顯示」相關設定（給字體載入器與少數顯示 hook 用）。
#[derive(Deserialize, Clone)]
pub struct LocalizedDataConfig {
    pub assets_dir: Option<String>,
    #[serde(default)]
    pub extra_asset_bundle: OsOption<String>,
    pub replacement_font_name: Option<String>,

    // 文字換行（給 core::utils 的斷行工具用；目前沒有 hook 呼叫，保留供未來字體/排版微調）
    #[serde(default)]
    pub use_text_wrapper: bool,
    // Predefined line widths are counts of cjk characters.
    // 1 cjk char = 2 columns, so setting this value to 2 replicates the default behaviour.
    pub line_width_multiplier: Option<f32>,

    pub text_frame_line_spacing_multiplier: Option<f32>,
    #[serde(default)]
    pub text_common_allow_overflow: bool,

    // RESERVED
    #[serde(default)]
    pub _debug: i32
}

impl Default for LocalizedDataConfig {
    fn default() -> Self {
        default_serde_instance().expect("default instance")
    }
}

#[derive(Deserialize)]
pub struct AssetInfo<T> {
    #[cfg(target_os = "android")]
    #[serde(default)]
    android: AssetMetadata,

    #[cfg(target_os = "windows")]
    #[serde(default)]
    windows: AssetMetadata,

    pub data: Option<T>
}

// Can't derive(Default), see rust-lang/rust#26925
impl<T> Default for AssetInfo<T> {
    fn default() -> Self {
        Self {
            #[cfg(target_os = "android")]
            android: Default::default(),

            #[cfg(target_os = "windows")]
            windows: Default::default(),

            data: None
        }
    }
}

impl<T> AssetInfo<T> {
    pub fn metadata(self) -> AssetMetadata {
        #[cfg(target_os = "android")]
        return self.android;

        #[cfg(target_os = "windows")]
        return self.windows;
    }

    pub fn metadata_ref(&self) -> &AssetMetadata {
        #[cfg(target_os = "android")]
        return &self.android;

        #[cfg(target_os = "windows")]
        return &self.windows;
    }
}

#[derive(Deserialize, Clone, Default)]
pub struct AssetMetadata {
    pub bundle_name: Option<String>
}