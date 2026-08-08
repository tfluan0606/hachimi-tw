//! `Gallop.SceneDefine.ViewId` —— 遊戲畫面（View）的識別碼。
//!
//! `SceneManager::ChangeView` 的第一個參數就是這個 enum。dump 只含方法簽章、不含 enum 值，
//! 所以下面的數值來自日服（Edge 的 `SceneDefine.rs`），**繁中服未逐項實測**。
//!
//! 因此對外的 [`describe`] 刻意以**區段**為主、單一數值為輔：Cygames 的 ViewId 是按功能分段
//! 配號（育成 1000+、團隊競技場 4000+、冠軍賽 5900+…），新功能往段尾追加。段界比個別數值
//! 穩定得多，日／繁版號差異落在段內時仍能給出正確的大分類。
//!
//! 遇到沒認出來的 id 會走 fallback 並在 debug log 留下數值，照那個補進來即可。

/// 只列實際會用到的；完整日服清單見 `edge/main:src/il2cpp/hook/umamusume/SceneDefine.rs`。
pub mod view_id {
    pub const SPLASH: i32 = 1;
    pub const HOME: i32 = 100;
}

/// 一個畫面對外的描述。`group` 是大分類，`detail` 是細節（沒有就 `None`），
/// `asset_key` 對應 Discord 應用程式裡上傳的圖片名稱。
#[derive(Copy, Clone, Eq, PartialEq)]
pub struct Scene {
    pub group: &'static str,
    pub detail: Option<&'static str>,
    pub asset_key: &'static str
}

const fn s(group: &'static str, asset_key: &'static str) -> Scene {
    Scene { group, detail: None, asset_key }
}

const fn sd(group: &'static str, detail: &'static str, asset_key: &'static str) -> Scene {
    Scene { group, detail: Some(detail), asset_key }
}

pub fn describe(view_id: i32) -> Scene {
    match view_id {
        // 0–99 啟動流程（splash / 標題 / 下載 / 教學）
        0..=99 => s("啟動中", "loading"),

        // 100+ 首頁
        100..=199 => s("首頁", "home"),

        // 200+ 演唱會（從首頁直接播的）
        200..=299 => s("觀看演唱會", "live"),

        // 300+ 轉蛋
        300..=399 => s("轉蛋", "gacha"),

        // 400+ 單場賽事
        401 => sd("賽事", "賽事分析", "race"),
        400..=499 => s("賽事", "race"),

        // 1000+ 育成（SingleMode）—— 這段最長，細節值得分出來
        1200 | 1210 | 1642 => sd("育成", "出賽準備", "training"),
        1300 | 1301 => sd("育成", "育成結束", "training"),
        1400 => sd("育成", "技能學習", "training"),
        1500 | 1501 => sd("育成", "繼承", "training"),
        1600..=1609 => sd("育成", "隊伍競賽", "training"),
        1620 | 1621 => sd("育成", "劇本演唱會", "training"),
        1000..=1799 => s("育成", "training"),

        // 3000+ 劇情
        3000..=3999 => s("觀看劇情", "story"),

        // 4000+ 團隊競技場
        4000..=4099 => s("團隊競技場", "arena"),

        // 5000–5899 選單／收藏／圖鑑
        5100..=5199 => sd("選單", "育成馬娘一覽", "menu"),
        5210..=5229 => sd("選單", "馬娘手帳", "menu"),
        5310..=5329 => sd("選單", "個人檔案", "menu"),
        5400..=5499 => sd("選單", "任務", "menu"),
        5500..=5599 => sd("選單", "卡片一覽", "menu"),
        5600..=5699 => s("每日賽事", "race"),
        5710 => sd("選單", "演唱會劇場", "live"),
        5730 => sd("選單", "商店", "menu"),
        5800..=5899 => s("社團", "menu"),
        5000..=5899 => s("選單", "menu"),

        // 5900+ 冠軍賽
        5900..=5999 => s("冠軍賽", "arena"),

        // 6000+ 各種對戰／小遊戲
        6000..=6099 => s("夾娃娃機", "minigame"),
        6100..=6199 => s("房間對戰", "arena"),
        6200..=6299 => s("練習賽", "race"),
        6300..=6399 => s("訓練挑戰賽", "arena"),
        6450..=6499 => s("打工", "menu"),
        6500..=6599 => s("行程表", "menu"),
        6600..=6699 => s("育成", "training"),

        // 7000+ 夾娃娃機（新版）
        7000..=7099 => s("夾娃娃機", "minigame"),

        // 8000+ 期間限定活動
        8150..=8199 => s("挑戰賽", "arena"),
        8250..=8269 => s("隊伍建設", "arena"),
        8270..=8299 => s("英雄賽", "arena"),
        8350..=8359 => s("因子研究", "menu"),
        8360..=8369 => s("究極賽事", "race"),
        8100..=8999 => s("活動", "event"),

        _ => s("遊玩中", "icon")
    }
}
