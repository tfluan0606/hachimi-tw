# 繁中服（Komoe PC）相容性筆記

> 調查日期：2026-08-07 ~ 08。所有「繁中服實際」欄位都來自本機實測或 il2cpp dump，不是推論。

## 定位

本 fork **獨立發展，專做繁中 PC 端**。上游 `Hachimi-Hachimi/Hachimi` 已封存（2025-08-01），
後繼者是 `kairusds/Hachimi-Edge`（remote 名 `edge`）。

**Edge 當零件庫用，逐項移植並驗證，不整棵樹接。** 決定依據見〈為什麼不整個接 Edge〉。

---

## 核心工具：il2cpp dumper

`11ad1a6` 加的 dumper 是維護這個 fork 的核心資產。它把「繁中服相容性」從猜謎變成查表。

- 產物：`<遊戲目錄>\hachimi\il2cpp_dump.txt`（約 37 MB）
- 格式：`命名空間.類別::方法(型別 參數名, ...) -> 回傳型別 [static]`

移植任何 Edge 功能前的固定流程：

```bash
grep -F "Gallop.SceneManager::ChangeView" il2cpp_dump.txt
```

對得上就搬，對不上就照 dump 的參數個數改 `get_method_addr(Class, c"Method", <參數數>)`
和對應的 `extern "C" fn` 簽章。

> Edge 沒有這個工具，他們靠貢獻者回報才知道哪裡不對。這是我們的優勢。

---

## 繁中服簽章對照

`get_method_addr` 的第三個參數是**方法的參數個數**（不含 `this`）。對不上就回傳 NULL。

| 方法 | 上游 / Edge 用的 | 繁中服實際 |
|---|---|---|
| `Gallop.SceneManager::ChangeView` | 日服 7 / 非日服 5 | **6**：`nextViewId, viewInfo, callbackOnChangeViewCancel, callbackOnChangeViewAccept, forceChange, isFastDestroy` |
| `Gallop.PartsSingleModeSkillListItem::UpdateItem` | 不符 | **3**：`skillInfo, isPlateEffectEnable, resourceHash` |
| `Gallop.Screen::SetResolution` | 不符 | **4**：`w, h, fullscreen, forceUpdate`（static） |
| `Gallop.Live.Cutt.LiveTimelineKeyCameraLookAtData::GetCharacterWorldPos` | 不符 | **5**：`timelineControl, posFlag, charaParts, charaPos, offset`（static） |

`ChangeView` 值得特別記：**日服 7、Edge 猜非日服 5、繁中服實際是 6**，兩邊都猜錯。
這是我們 `4b3e559`（點擊偏移修正）當初要繞過的根因。

> **已修（2026-08-09）**：改成 6 個參數後 hook 正常裝上，實機 log 有 `new_hook!: ChangeView`
> 且能持續收到畫面切換。**NULL hook 從 4 個降為 3 個**（剩 `UpdateItem`、`InitializeGame`、
> `<ChangeLive>b__41_1`）。`4b3e559` / `76140b8` 的 fallback 仍然保留 —— WM_SIZE 放行改用
> 「第一次 Present」當訊號，跟這個 hook 解耦，兩者不衝突。

### ViewId 數值：繁中服與日服一致（2026-08-09 實測）

il2cpp dump 只含方法簽章、不含 enum 值，所以 `SceneDefine.ViewId` 一度只能靠日服清單推測。
實機走一輪後確認**數值相同**，且橫跨多個相距很遠的區段：

```
1 Splash    2 Title    100/101 Home    1300/1301 SingleMode結算
3000 Story  4000 TeamStadium           6500 ScheduleBook
```

即使如此，`src/il2cpp/hook/umamusume/SceneDefine.rs` 仍刻意用**區段**而非逐一數值對應。
Cygames 是按功能分段配號、新功能往段尾追加，段界比個別數值穩定；台服日後補上新內容時
（例如五週年那批）不必逐項回頭補表。

完整日服清單在 `edge/main:src/il2cpp/hook/umamusume/SceneDefine.rs`，要查名字時去翻。

### 首頁代表馬娘：兩條死路和一條活路

Discord RPC 要顯示首頁角色時查過的，記下來免得再走一次：

| 方法 | 結果 |
|---|---|
| `PartsFavoriteCharacterTop::get_CurrentCharaId` | ❌ 那是「選擇代表馬娘」的**對話框**，只有開著時才有值 |
| `HomeCharacterCreator::CreateAdditionalStandCharacter` | ❌ hook 裝得上但**永遠不觸發**。"Additional" 是字面意思，主角色走 `InitFooterCharacter` coroutine |
| `HomeCharacterCreator::GetFooterCharaInfo()` | ✅ static、0 參數，回傳 `Dictionary<StandPos, CreateInfo>` |

活路的兩個前提：**要在遊戲主執行緒上呼叫**（從背景執行緒用 `Thread::main_thread().schedule`），
而且**進首頁當下讀不到** —— 角色是 `ChangeView` 之後才由 coroutine 擺出來的，要重試個幾秒。

### dress id 和 card id 是兩張表

**沒有可推導的關係**，別想找規律：

```
目白多伯  正常版  card 105901 ← dress 105901   （相同）
          換裝版  card 105902 ← dress 105923   （差 21）
```

遊戲給的是 dress id，站台圖片和 `card_map` 都以 card id 為索引，中間只能查表。
表在 `assets/card_dress_map.json`（來自 `uma-pc-datamine`，原檔照搬，更新＝覆蓋）。

> 陷阱：**不要用 `dress_id_by_rarity` 反轉**。那裡面 2★ 那格是共用制服 `101`，
> 22 張卡都指向它，反轉會變成一對多。只有頂層的 `dress_id` 是該卡專屬的。

### `Gallop.Live.Director` — 唯一語意也不同的

Edge 的 hook 是 `AlterUpdate(this, delta_time, is_update_delta_time)`（2 參數）。
繁中服是**較舊的 API 形狀**：

```
AlterUpdate()                                    ← 0 參數
AlterUpdatePre()                                 ← 0 參數
AlterUpdateTimeline(System.Single& timescale)    ← 1 參數，by-ref
UpdateCharacterDeltaTime(System.Single timescale)
get_LiveCurrentTime() / GetLiveCurrentTime()
get_LiveTotalTime()
```

不能照抄 Edge，要重寫這一層。但對我們反而更好：`timescale` 是 by-ref 傳的，
直接改那個值就能控制播放速度；`get_LiveCurrentTime` / `get_LiveTotalTime`
現成給播放進度滑桿用。

---

## 為什麼不整個接 Edge

2026-08-07 做過完整的 rebase 試驗（`edge/main` v0.27.1 + 我們 7 個 commit），
編譯乾淨、遊戲能開、overlay 能出、選單能開，但：

### 1. 失敗面是我們的 3.5 倍

實機 log 比對同一台機器、同一個遊戲版本：

```
我們現行版本        Edge 版
  4 個 NULL hook     14 個 NULL hook
```

四個共通的（`UpdateItem`、`InitializeGame`、`<ChangeLive>b__41_1`、`ChangeView`）
是從上游封存版就帶著的既有問題。**多出來的 10 個全部來自 Edge 新增的功能。**

（2026-08-09 起 `ChangeView` 已修好，我們這邊剩 3 個。差距只會更大。）

我們 fork 砍得多、表面積小，這不是技術債，是設計。

### 2. 會崩潰

`ConfigEditor::new()` 一次呼叫四類 il2cpp/DB 操作（`get_localized_string` ×5、
`get_champions_resources`、`get_champions_live_max_year`、`umamusume_enum_options` ×3），
其中至少一個在繁中服存取違規。堆疊是 `GameAssembly ×5 ← VERSION ×12`，硬崩潰不是 panic。

而且崩在**我們根本不需要的功能**裡（champions 資源、字型顏色列舉）。

### 3. Edge 最大的開發量對我們無用

278 個 commit 裡最大宗是**翻譯系統**（多來源管理、定期自動更新、atlas 圖片替換、
`ast_ruby`、各國語系）。繁中服本來就是中文。

而且 C 類壞掉的 hook 大多正好是翻譯相關（文字換行、行距、技能名）——
繁中服對不上的地方，剛好集中在不需要的功能上。

---

## 已知死路

### 自動更新不能靠 installer

Edge 的 updater（`src/core/updater.rs`）跟舊版**機制完全相同**：

```rust
"install --install-dir \"{}\" --target \"{}\" --sleep 1000 --prompt-for-game-exit --launch-game -- {}"
```

下載 `hachimi_installer.exe`，把目前載入的 DLL 路徑當 `--target` 傳進去。
而 `kairusds/Installer` 的 `Target::VALUES` 只有 `UnityPlayer.dll` / `cri_mana_vpx.dll`。

**我們用 `version.dll` 代理安裝，兩邊的 installer 都不認得**，會跳
`Failed to determine target type`。（2026-08-07 有使用者實際回報這個錯誤，
原因是舊版 `REPO_PATH` 指著已封存的上游 repo，觸發更新提示後按了「是」。）

所以自動更新必須**自己寫換檔邏輯**：

1. 下載新 DLL 到 `hachimi\update\version.dll.new`
2. `MoveFileW` 把現役 `version.dll` 改名成 `.old`
   （Windows 允許改名已載入的 DLL，不允許刪除 —— 關鍵技巧，不需 helper exe 或管理員權限）
3. `MoveFileW` 把 `.new` 移到 `version.dll`
4. 通知「更新完成，請重開遊戲」
5. 下次啟動清掉 `.old`；第 3 步失敗就把 `.old` 移回去回滾

同磁碟機內移動。值得從 Edge 抄的只有 **blake3 校驗**（release 附 `blake3.json`，
下載後比對雜湊，避免半截檔案覆蓋掉能用的版本）。Codeberg 鏡像可跳過 ——
它只鏡像「檢查更新」，實際下載仍走 GitHub。

### 繁中服的視窗 hook 跑在 render thread —— 別在那裡呼叫 SendMessage 類的 API

承上：因為 `FindWindowW` 查不到，`wnd_hook::ensure_installed` **一定**是從 render_hook 的
首次 Present 被呼叫的，也就是**在 render thread 上**。此時遊戲主執行緒正等著 render thread
交件。所以在那個函式（以及任何從 GUI／Present 呼叫的程式碼）裡，只要碰到會 SendMessage
的 Win32 API，就是死鎖：

| API | 內部行為 |
|---|---|
| `GetWindowTextW` / `GetWindowTextLengthW` | `WM_GETTEXT` / `WM_GETTEXTLENGTH` |
| `SetWindowTextW` | `WM_SETTEXT` |
| `SetWindowPos`（含 always-on-top） | `WM_WINDOWPOSCHANGING` |

2026-09-01 實際踩到：`3b8c21a` 把 `GetWindowTextW`/`SetWindowTextW` 直接寫在
`ensure_installed` 尾巴，遊戲一開就沒有回應，log 停在 `Adding CBT hook` 之後、無任何錯誤。
日服／國際服 `FindWindowW` 找得到視窗，`ensure_installed` 在載入早期就跑完，所以上游和
Edge 都不會踩到——**又一個「他們沒事不代表我們沒事」的例子**。

解法：post 一個 `WM_APP+n` 給視窗，實際動作在我們自己的 wndproc 裡做，那本來就跑在擁有
視窗的執行緒上。`PostMessageW` 不等回應，從哪條執行緒呼叫都安全。
（選單裡的切換點若已包在 `Thread::main_thread().schedule` 裡則本來就安全。）

### 視窗標題查找在繁中服是失效的

```
[INFO] wnd_hook: Game window not found by title yet; will install on first Present
[INFO] wnd_hook: Subclassing game window          ← fallback 才成功
```

`FindWindowW(class="UnityWndClass", title=...)` 查不到。我們用 `komoeumamusume`，
Edge 用 `賽馬娘Pretty Derby`，**可能兩個都不對**。

overlay 能出來完全是靠 `76140b8` 的 fallback —— render_hook 首次 Present 時
用 swapchain 的 `OutputWindow` 補裝。**那個 commit 沒有過時，是關鍵的。**

> 陷阱：`ensure_installed(hwnd)` 必須用呼叫端傳進來的 hwnd，
> 不能在函式內再做一次 `FindWindowW` 覆蓋掉（rebase 時踩過這個坑）。

---

## Edge 功能分類（繁中服可用性）

### A — 零風險，完全不碰 il2cpp

- ~~**Discord Rich Presence**~~ — 已做，且不再是 A 類（加了場景顯示後會用到 `ChangeView` hook）
- ~~開選單熱鍵可在 UI 修改~~ — 已做（`3b8c21a`）
- ~~GUI 縮放~~ — 已做（`3b8c21a`）
- ~~自訂視窗標題~~ — 已做（`3b8c21a`）
- Windows IME 支援（GUI 裡能打中文）
- Config Editor 搜尋列 —— **要先有 IME**。選項名稱現在都是中文，沒有 IME 就打不進搜尋框
- updater 的 blake3 校驗 —— 單獨搬沒用，得等自寫換檔邏輯（見〈已知死路〉）
- 內建 webview

> `enable_file_logging` **不需要搬** —— 我們已經有自製版（`hachimi_tw_<exe>.log`）。

### B — 碰 il2cpp 但未報 NULL，需實測

- Freeform window（解除視窗長寬比鎖定，可任意拖拉）
- MSAA / 陰影解析度 / 非等向性過濾（直接改 Unity QualitySettings）
- `live_vocals_swap`（演唱會換人唱，實作在 `LiveUtil.rs`）
- SMTC（遊戲音樂出現在 Windows 媒體浮層。實測有在跑，能抓到 `HomeViewController`）
- 工作列進度（接到遊戲資源下載/連線中狀態，依賴 `Connecting`、
  `DownloadErrorProcessor`、`BackgroundDownloadProgressUI` 三個 hook）

> 「沒報 NULL」只代表 hook 裝上了，不代表功能正確。

### C — 繁中服壞掉，但簽章已查到，可修

| 功能 | 壞在哪 | 值得修？ |
|---|---|---|
| **Free Camera**（Live/賽事自由視角，三種模式：自由移動/第一人稱/跟隨角色） | `GetCharacterWorldPos`、`Director::AlterUpdate` | ✅ |
| **Live 循環播放 + 進度滑桿** | `Director::AlterUpdate` | ✅ |
| ~~**解析度縮放**~~ | — | ✅ **已經做好了**，見下 |
| 返回鍵處理 | `BackKeyInputManager` | ✅ Free Camera 的配套，攔截返回鍵避免誤觸退出 |
| 劇情文字換行 / 行距 | `LineHeadWrapCommon`、`SetLineSpacing` | ❌ 翻譯衍生 |
| 自訂技能說明視窗 | `UpdateItem`、`SetSkillNameText` | 之後再議 |
| 賽事分析事件列表 | class 不存在 | ⛔ 日服五週年功能，台服約還要一年 |
| 劇情素質變化特效 | class 不存在 | ❌ 素材替換系統 |

> **解析度縮放不必再做**（2026-08-14 查證）。我們走的路和 Edge 不同：Edge 攔
> `Screen::SetResolution`，我們是 hook `Gallop.Screen::get_Width` / `get_Height` 回傳縮放後的值
> （`windows::utils::get_scaling_res`），config 的 `resolution_scaling` 與 GUI 下拉都已經在。
> log 裡 Screen 沒有任何 NULL。上面那格原本是照抄 Edge 的問題清單，對我們不成立。

### D — 對繁中服無意義

整套翻譯系統、Android/Zygisk、國際服與 Steam 日服專屬修正。

**不要嘗試剝離翻譯系統**：`Localize::Get`、`StoryTimelineData`、`TextCommon` 這些 hook
本身就是翻譯機制，`tl_repo::Updater` 被 `Hachimi` struct 直接持有。
現行做法（拿掉 UI 入口 + `disable_translations: true`，引擎留著但不動作）已經是對的，
完全剝離換來的只有二進位小一點。

---

## 未來工作包

**已完成**

1. ~~Discord Rich Presence~~ — 2026-08-09 完成，比原本規劃的 A 類大：
   除了「正在玩」還會顯示目前畫面（首頁／育成／競技場…），資料來自修好的 `ChangeView` hook。
   自己的 Application ID `1535642752665260093`，預設**關閉**（Edge 是預設開啟）。
   實作要點見 `src/windows/discord.rs` 的檔頭：hook 跑在遊戲主執行緒，只寫 atomic，
   named pipe 通訊全在 worker；Discord 的 `SET_ACTIVITY` 約 15 秒節流，期間變化合併成最後一筆。

**已排定**

2. Free Camera + 返回鍵 — 原本這包還有「解析度縮放」，查證後發現**早就做好了**（見 C 類）。

   ### 動工前先看這裡（2026-08-14 事前調查）

   簽章都查過了，**沒有一個對不上**：

   | 需要的 | dump 裡的實際簽章 |
   |---|---|
   | `Gallop.Live.Cutt.LiveTimelineKeyCameraLookAtData::GetCharacterWorldPos` | 5 參數 static，如筆記所載 |
   | `Gallop.BackKeyInputManager::ExecuteBackKeyAction()` | 0 參數 |
   | `Gallop.BackKeyInputManager::set_OverrideAction(System.Action)` | 存在 |

   > `set_OverrideAction` 值得注意：**這是官方留的覆寫點，比 hook `Update` 乾淨**。
   > 代價是要從 Rust 造一個 C# `Action` delegate（`symbols::create_delegate` 已經有）。
   > 先試這條，不要一開始就去攔 `Update`。

   **真正的成本不在簽章，在移植面積：**

   ```
   edge/main:src/windows/free_camera.rs                2209 行
   edge/main:src/il2cpp/hook/Unity_InputSystem/Gamepad.rs 172 行   ← 我們沒有這個模組
   edge/main:src/il2cpp/hook/umamusume/ModelController.rs  19 行   ← 我們沒有
   ```

   再加上 `Director` 那層本來就得重寫（繁中服是舊 API 形狀，見上面的簽章對照）。

   **所以不要整檔搬。** 建議切成可各自驗證的階段，每階段都要能單獨開遊戲確認：

   1. `Director::AlterUpdateTimeline` 的 by-ref `timescale` —— 先做播放速度，最小、最好驗
   2. `get_LiveCurrentTime` / `get_LiveTotalTime` → 進度滑桿
   3. 鍵盤操作的自由視角（**先不要碰 gamepad**，那是另外 172 行且與核心無關）
   4. `BackKeyInputManager` 攔返回鍵
   5. 手把支援 —— 真的需要再說

   ※ 這個 fork 的教訓：每次「猜結構」都錯，每次都是實機 log 抓出來的。
   2209 行一次進來沒辦法二分定位問題。

**延後**

- 顯示實際技能發動條件（取代原本文字說明）。資料現成：`SkillData::get_Condition() -> System.String`，
  `master.mdb` 在 `<遊戲目錄>\komoeumamusume_Data\Persistent\master\master.mdb`（34 MB，
  SQLCipher 加密，我們已 hook `sqlite3_open_v2` / `sqlite3_key`）。
  工作量在**翻譯條件式**（`distance_type==2&running_style==1&phase_random==2`
  → 「中距離・逃・中盤」），需要一張對照表，模式同 `crates/factor-card/assets/*.json`。
  另可考慮做成自己的 overlay 視窗，就不必碰 `UpdateItem` hook。
  ※ `tier2-training-helper` 分支的 WIP 有 `SkillLearningList.rs`，動工前先回頭看。
- 賽事分析事件列表 — 等台服上五週年內容。

---

## 相關 remote

```
origin    https://github.com/tfluan0606/hachimi-tw.git      我們的
edge      https://github.com/kairusds/Hachimi-Edge.git      零件庫（活躍後繼者）
upstream  https://github.com/Hachimi-Hachimi/Hachimi.git    已封存，僅供追溯
```
