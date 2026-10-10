<img align="left" width="80" height="80" src="assets/icon.png">

# Hachimi TW

繁體中文 | [English below](#english)

UM:PD **繁中服（Komoe 代理，PC 端）**專用的遊戲增強 mod。

> 本專案是 [Hachimi](https://github.com/Hachimi-Hachimi/Hachimi) 的分支（fork），核心架構與大部分底層程式碼來自上游 Hachimi；
> 技能詳細數據移植自其後繼專案 [Hachimi-Edge](https://github.com/kairusds/Hachimi-Edge)，之後獨立發展、不再與上游同步。授權沿用 **GNU GPLv3**。

---

## ⚠️ 使用前須知

- 使用 mod 本質上違反遊戲 TOS，**使用風險與封號責任自負**。
- 請勿在公開場合大肆宣傳或連結本專案；提到遊戲時請用「UM:PD」或代稱。
- 個人維護的專案，不保證穩定，也不提供安裝協助。

## 功能

**繁中服相容**
- 以 `version.dll` 代理注入，直接解析繁中服 `GameAssembly.dll` 的標準 il2cpp 匯出，不依賴綁定特定版本的位址表
- 繁中服的遊戲視窗、區域判定、各項 hook 簽章都照台服實際簽章調整

**遊戲資訊**
- **技能詳細數據**：技能說明改為顯示實際的發動條件與效果數值（讀取遊戲資料庫），並整理成易讀的格式：
  跑法／距離等限制標在開頭、時間條件以顏色標示、加強版只寫差異等。設定編輯器「顯示技能詳細數據」開啟（預設關）
- **因子卡片**：收集練成馬娘的因子資料並在遊戲內產生卡片圖
- **比賽擷取**：自動存下練習賽、自訂配對賽、群英聯賽的結果封包，逐幀比賽資料已解碼成 JSON（存於 `hachimi/race_capture/`）

**畫面與操作**
- FPS 上限、垂直同步、解析度縮放、畫質設定
- Live 播放速度與進度顯示
- UI 動畫加速、劇情文字速度、劇情選項自動選擇延遲
- 隱藏遊戲自訂游標、視窗置頂、自訂視窗標題
- 遊戲內設定選單：GUI 縮放、選單熱鍵自訂、Windows 輸入法支援

**其他**
- **Discord Rich Presence**：顯示目前所在的遊戲畫面與首頁代表馬娘（預設關）
- **自動更新**：從本專案的 GitHub release 下載新版並以 blake3 校驗；可選擇自動下載或只通知

## 安裝

1. 從 [Releases](https://github.com/tfluan0606/hachimi-tw/releases) 下載 `version.dll`。
2. **完全關閉遊戲與啟動器**。
3. 把 `version.dll` 放進遊戲本體目錄（與 `komoeumamusume.exe` 同一層）。
4. 啟動遊戲，按 **→（右方向鍵）** 開啟選單。

移除：刪掉遊戲目錄下的 `version.dll` 即可，遊戲原始檔案不受影響。

設定檔在遊戲目錄的 `hachimi/config.json`，大部分選項可在遊戲內選單的設定編輯器調整。

## 自動更新

- 遊戲啟動時會檢查本專案最新的 release，有新版時通知。
- 設定編輯器勾選「自動更新」後，會在背景下載並校驗，完成後提示重開遊戲。
- 從**更早的版本**升級時，舊版的更新器可能指向別的地方，需要手動換一次 `version.dll`，之後就會自動更新。

## 建置

需要 Rust 與 MSVC toolchain（`x86_64-pc-windows-msvc`）。

```powershell
git clone https://github.com/tfluan0606/hachimi-tw.git
cd hachimi-tw
cargo build --release
```

產出 `target/release/hachimi.dll`，改名為 `version.dll` 使用。

發版：`.\tools\release.ps1 <版號>`（改版號、測試、打 tag、推送），GitHub Actions 會自動建置並發布 release。

## 特別感謝

- [Hachimi](https://github.com/Hachimi-Hachimi/Hachimi)（LeadRDRK 及貢獻者）：本專案的基礎
- [Hachimi-Edge](https://github.com/kairusds/Hachimi-Edge)：技能詳細數據等功能的移植來源
- [GameTora](https://gametora.com/umamusume/skills)、[UmaTL hachimi-sd](https://github.com/UmaTL/hachimi-sd)：技能資料格式的參考（經由 Hachimi-Edge）
- 上游 Hachimi 致謝的各專案：Trainers' Legend G、umamusume-localify、Carotenify、umamusu-translate、frida-il2cpp-bridge

## 授權

[GNU GPLv3](LICENSE)

© 2024-2025 LeadRDRK and contributors
© 2026 Micky (tfluan0606)

---

## English

**Hachimi TW** is a game enhancement mod for the **UM:PD Traditional Chinese client (Komoe, PC)**.
It is a fork of [Hachimi](https://github.com/Hachimi-Hachimi/Hachimi), with the skill data descriptions ported from [Hachimi-Edge](https://github.com/kairusds/Hachimi-Edge); it is now developed independently and does not track either upstream. Licensed under **GNU GPLv3**, same as upstream.

Highlights: TW client compatibility (version.dll proxy injection, standard il2cpp export resolution), in-game skill data descriptions,
factor cards, race packet capture, Discord Rich Presence, graphics/playback options and a self-hosted auto-updater.

Install: download `version.dll` from [Releases](https://github.com/tfluan0606/hachimi-tw/releases), close the game and launcher,
put it next to `komoeumamusume.exe`, then press → in game to open the menu.

Use at your own risk — using mods violates the game's TOS. Please don't publicly advertise or link this project.
