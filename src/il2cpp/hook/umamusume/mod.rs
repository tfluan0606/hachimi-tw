// 字體／顯示相關（保留，未來自訂字體功能要用）
pub mod TextFrame;
mod TextFontManager;
mod TextFormat;
mod TextCommon;
mod TextMeshProUguiCommon;

// 玩法／畫面／擷取等非翻譯 hook（保留）
mod UIManager;
pub mod GraphicSettings;
mod CameraController;
pub mod SingleModeStartResultCharaViewer;
pub mod GameSystem;
pub mod Screen;
mod StoryChoiceController;
mod StoryViewController;
mod DialogRaceOrientation;
mod RaceInfo;
mod RaceUtil;
mod SaveDataManager;
mod ApplicationSettingSaveLoader;
mod LiveTheaterCharaSelect;
mod LiveTheaterViewController;
pub mod CySpringController;

pub mod Director;

#[cfg(target_os = "windows")]
pub mod SceneDefine;

#[cfg(target_os = "windows")]
pub mod HomeCharacterCreator;

#[cfg(target_os = "windows")]
pub mod SceneManager;

#[cfg(target_os = "windows")]
mod PaymentUtility;

#[cfg(target_os = "windows")]
pub mod DialogTrainedCharacterDetail;

// 育成技能學習頁的「自動學習」按鈕
#[cfg(target_os = "windows")]
pub mod SingleModeSkillLearningViewController;

pub mod HttpHelper;

// 技能資料說明（skill_data_desc）的換行：避免遊戲把 rich text 標籤切斷
mod GallopUtil;

pub fn init() {
    get_assembly_image_or_return!(image, "umamusume.dll");

    // 封包擷取的唯一 choke point，capture-only 建置也只裝這一個。
    HttpHelper::init(image);

    // 以下是字體／玩法／畫面等遊戲改動 hook，capture-only 建置一律不裝。
    #[cfg(not(feature = "capture-only"))]
    {
    #[cfg(target_os = "windows")]
    DialogTrainedCharacterDetail::init(image);

    // 字體／顯示
    TextFrame::init(image);
    TextFontManager::init(image);
    TextFormat::init(image);
    TextCommon::init(image);
    TextMeshProUguiCommon::init(image);

    // 玩法／畫面
    UIManager::init(image);
    GraphicSettings::init(image);
    CameraController::init(image);
    SingleModeStartResultCharaViewer::init(image);
    GameSystem::init(image);
    Screen::init(image);
    StoryChoiceController::init(image);
    StoryViewController::init(image);
    DialogRaceOrientation::init(image);
    RaceInfo::init(image);
    RaceUtil::init(image);
    SaveDataManager::init(image);
    ApplicationSettingSaveLoader::init(image);
    LiveTheaterCharaSelect::init(image);
    LiveTheaterViewController::init(image);
    CySpringController::init(image);
    Director::init(image);
    GallopUtil::init(image);

    #[cfg(target_os = "windows")]
    {
        SceneManager::init(image);
        HomeCharacterCreator::init(image);
        PaymentUtility::init(image);
        SingleModeSkillLearningViewController::init(image);
    }
    } // end #[cfg(not(feature = "capture-only"))]
}
