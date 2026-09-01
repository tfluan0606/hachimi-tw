# 接手須知（放下一陣子之後回來先看這頁）

> 最後更新：2026-08-29。上一次實際寫 code 是 2026-08-15，之後專案暫停。

## 三十秒版本

- 這是 **UM:PD 繁中服（Komoe）PC 端**專用的 Hachimi fork，獨立發展。
- 基底是上游 `Hachimi-Hachimi/Hachimi` 的最後一個 commit（`5688b71`，2025-08-01 封存）。
  上游沒有任何我們沒跟到的東西。
- 活躍後繼者 `kairusds/Hachimi-Edge`（remote `edge`）**只當零件庫**，逐項移植並實機驗證，
  不整棵樹接。原因見 `tw-compat-notes.md` 的〈為什麼不整個接 Edge〉——那是實測結論，別重新辯論。
- 改動全部在 `main` 上，已推 origin。

## 分支長怎樣（2026-08-29 整理過）

```
main                        唯一主線。所有已完成的功能都在這，已推 origin。
wip/tier2-training-helper   2026-07-14 停在半路的育成輔助。裡面的 SkillLearningList.rs
                            是「顯示技能實際發動條件」那個延後項目的起點，動工前先回頭看。
experiment/edge-full-rebase 2026-08-07 的「整棵樹接 Edge」試驗。結論是不接（14 個 NULL
                            hook、設定編輯器硬崩）。留著當證據，不要往上開發，不要合併。
```

原本的 `discord-rpc` 和 `factor-card` 兩條分支已經刪掉——內容早就全在 `main` 裡了，
留著只會讓人以為還有沒併回來的東西。

## 「我遊戲裡跑的是哪一版」

repo 的 code 和你實際裝的 DLL **不是同一個東西**，這點最容易搞混。已經打上 tag：

| tag | commit | 對應成品 |
|---|---|---|
| `dist-20260722` | `bfe8996` | `dist/ASKR-因子卡片-Hachimi-TW-20260722.zip` |
| `dist-20260729` | `76140b8` | `dist/ASKR-Hachimi-TW-完整版-20260729.zip`、`dist/pkg/version.dll` |

**注意：`dist/` 裡最新的那包不等於遊戲裡裝的那個。** 兩者是分開的——`dist/` 是打包給人用的
成品，遊戲裡的則常常是隨手 build 上去的本機版本。判斷實際裝了哪一版，最快的方法是看
`<遊戲目錄>\hachimi\config.json` 有哪些鍵：config 會依當時的 `Config` struct 補齊欄位，
所以有 `enable_discord_rpc` 就代表至少含 `13c46c4`，有 `gui_scale` 就代表含 `3b8c21a`，依此類推。

2026-09-01 起遊戲裡裝的是 `f7362ce`（＝當時的 `main`），舊的那顆備份在同目錄的
`version.dll.bak-20260901`（8/12 build，只到 Discord RPC 那批）。

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
| API 擷取開關（config `api_capture` + 選單「API 擷取」） | ✅ `f7362ce` |
| **Free Camera 階段 3：鍵盤自由視角** | 🔶 **停在這**。`8719f80` 只加了 Transform/Camera 綁定，還沒有 `free_camera.rs` |
| 階段 4 返回鍵、階段 5 手把 | ❌ 未動 |

## 回來的第一步

1. `git pull`，確認在 `main`。
2. 讀 `tw-compat-notes.md`（技術筆記本體：繁中服簽章對照、Edge 功能 A/B/C/D 分類、已知死路）。
   **移植任何 Edge 功能前，先 grep il2cpp dump 查實際簽章**，不要照抄 Edge——這個 fork 的教訓是
   每次猜結構都錯。
3. 要繼續 Free Camera 的話，事前調查已經做完寫在 `tw-compat-notes.md` 的〈未來工作包〉：
   簽章沒有一個對不上，但 Edge 的 `free_camera.rs` 有 2209 行，**不要整檔搬**，照那裡的五個階段切。
   兩個地雷：先不要碰 gamepad；`types.rs` 的 `Quaternion_t` 欄位順序是 w,x,y,z 而 Unity 是 x,y,z,w。
