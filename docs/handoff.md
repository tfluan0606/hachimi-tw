# 接手須知（放下一陣子之後回來先看這頁）

> 最後更新：2026-10-10。2026-08-29 暫停後，10-04 恢復開發，10-10 發了 **v1.1.0**。

## 三十秒版本

- 這是 **UM:PD 繁中服（Komoe）PC 端**專用的 Hachimi fork，獨立發展。
- 基底是上游 `Hachimi-Hachimi/Hachimi` 的最後一個 commit（`5688b71`，2025-08-01 封存）。
  上游沒有任何我們沒跟到的東西。
- 活躍後繼者 `kairusds/Hachimi-Edge`（remote `edge`）**只當零件庫**，逐項移植並實機驗證，
  不整棵樹接。原因見 `tw-compat-notes.md` 的〈為什麼不整個接 Edge〉——那是實測結論，別重新辯論。
- 改動全部在 `main` 上，已推 origin。發版走 `tools/release.ps1` → GitHub Actions 自動開 release，
  玩家端由自動更新器接手（流程見 `tw-compat-notes.md`〈發版流程〉與 `release-notes/`）。

## 分支長怎樣（2026-10-10 整理過）

```
main                        唯一主線。所有已完成的功能都在這，已推 origin。
local/datamine              只在本機、不要推：全量 API 擷取＋request 側擷取（2026-10-10 起 main 已移除，不公開）。
```

已刪掉的分支留了封存 tag（`git tag --list "archive/*"`），要撈回來從 tag 開分支即可：

```
archive/edge-full-rebase      2026-08-07 的「整棵樹接 Edge」試驗。結論是不接（14 個 NULL hook、
                              設定編輯器硬崩）。之後 egui 等套件是自己逐項升級的（7079393）。
archive/tier2-training-helper 2026-07-14 擱置的育成輔助。讀技能頁的部分已被一鍵學習取代、
                              發動條件已由 skill_data_desc 做掉；還有參考價值的是活體讀牌組
                              （SingleModeDeck.rs）和 il2cpp 全類別 dump 工具（src/il2cpp/dump.rs）。
archive/ui-speed-button-diag  （只在本機，沒推）診斷「UI 加速下育成結束評價對話框偶爾沒出『下一步』
                              按鈕」的 log。問題還沒解，使用者有解法構想，下次遇到再處理。
```

`discord-rpc`、`factor-card`、`feat/practice-race-capture` 也都刪了——內容早就全在 `main` 裡。

## 「我遊戲裡跑的是哪一版」

- **發給別人的**：GitHub release（v1.1.0 起），tag = DLL 內建版號。7 月的 `dist-*` tag／`dist/` 打包已不再使用。
- **遊戲裡實際裝的**：常常是隨手本機編譯上去的版本，不等於任何 release。看 log 第一行
  `Hachimi TW vX.Y.Z-<commit>[-dirty]` 就知道是哪個 commit、有沒有未 commit 的改動。

## 進度

| 項目 | 狀態 |
|---|---|
| ChangeView hook 修成 6 參數（NULL hook 4→3） | ✅ `a228c07` |
| Discord Rich Presence（含場景、首頁代表馬娘） | ✅ `13c46c4` `3e3da40` |
| GUI 縮放、自訂視窗標題、選單熱鍵改鍵 | ✅ `3b8c21a` |
| Windows IME | ✅ `7b8c385` |
| 解析度縮放 | ✅ 本來就有（走 `get_Width`/`get_Height`，不是 Edge 的 `SetResolution`） |
| Free Camera 階段 1–2：播放速度、進度滑桿 | ✅ `960657e` |
| 因子卡片（遊戲內 + 網站端 `render-card`） | ✅ |
| API 擷取開關（config `api_capture` + 選單「API 擷取」） | 🗑️ 2026-10-10 從 main 移除，只留本機 `local/datamine` |
| 一鍵學習（技能頁原生按鈕、主要／次要清單、設定檔、搜尋加入） | ✅ `51052f4` `fd2d7b6` |
| egui 0.27→0.35、egui-directx11 0.13、windows 0.62；GUI 開著遊戲吃不到輸入 | ✅ `7079393` |
| UI 重整：選單精簡、設定即時生效＋自動存檔、獨立設定視窗 | ✅ `1942e86`（設計與踩雷見記憶／commit 訊息） |
| 更新流程：選單上檢查更新、詢問視窗附更新內容、下載 timeout | ✅ `05e9cdd` |
| C 執行階段靜態連結（不需 VC++ 套件） | ✅ `e63d937` |
| Free Camera 階段 3：鍵盤自由視角 | 🔶 8 月停在這。`8719f80` 只加了 Transform/Camera 綁定，還沒有 `free_camera.rs` |
| 階段 4 返回鍵、階段 5 手把 | ❌ 未動 |
| UI 加速下育成結束評價框偶爾沒「下一步」 | ❌ 未解，使用者有解法構想；診斷 log 在本機 tag `archive/ui-speed-button-diag` |

## 回來的第一步

1. `git pull`，確認在 `main`。
2. 讀 `tw-compat-notes.md`（技術筆記本體：繁中服簽章對照、Edge 功能 A/B/C/D 分類、已知死路）。
   **移植任何 Edge 功能前，先 grep il2cpp dump 查實際簽章**，不要照抄 Edge——這個 fork 的教訓是
   每次猜結構都錯。
3. 要繼續 Free Camera 的話，事前調查已經做完寫在 `tw-compat-notes.md` 的〈未來工作包〉：
   簽章沒有一個對不上，但 Edge 的 `free_camera.rs` 有 2209 行，**不要整檔搬**，照那裡的五個階段切。
   兩個地雷：先不要碰 gamepad；`types.rs` 的 `Quaternion_t` 欄位順序是 w,x,y,z 而 Unity 是 x,y,z,w。
