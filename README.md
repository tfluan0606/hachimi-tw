# hachimi-tw

繁體中文 | [English below](#english)

> **這是 [Hachimi](https://github.com/Hachimi-Hachimi/Hachimi) 的非官方分支（fork）**，目的是讓它能在 **UM:PD 繁中服（Komoe 代理，PC 端）** 上運作。所有核心功能、設計與絕大部分程式碼皆來自上游 Hachimi 專案，本分支只做繁中服所需的相容性調整。
>
> Forked from **Hachimi-Hachimi/Hachimi** · Licensed under **GNU GPLv3**（與上游相同）。

---

## ⚠️ 使用前須知

- 本專案本質上違反遊戲 TOS，**使用風險自負，封號責任自負**。請勿在公開場合大肆宣傳、連結本 repo 或原版 Hachimi（沿用上游立場，避免搜尋引擎收錄；提到遊戲時請用「UM:PD」或代稱）。
- 這是為個人研究/自用而做的相容性移植，**不提供支援、不保證穩定、不接受「幫我裝」類請求**。
- 上游完整功能文件請見 Hachimi 官方站；本 README 只說明「繁中服跟上游有什麼不同」。

## 繁中服做了哪些調整（相對上游）

上游 Hachimi 針對日服/國際服，直接套到繁中服會閃退或功能失效。本分支的改動集中在 `src/windows/`：

| 檔案 | 調整內容 |
|------|----------|
| `proxy/version.rs`（新增） | 以 **version.dll 劫持**做早期注入：轉發 17 個 version.dll 匯出到 System32 真檔，DllMain 先裝轉發再進 Hachimi 初始化。 |
| `proxy/mod.rs`、`proxy/exports.def` | 掛上 version.dll proxy 模組與匯出表。 |
| `main.rs`、`hook.rs` | DllMain 先呼叫 `proxy::version::init()`；早期載入路徑改走 version.dll proxy，等 `cri_ware_unity.dll` 載入時觸發 il2cpp hook。 |
| `symbols_impl.rs` | **關鍵修正**：繁中服 `GameAssembly.dll` 用「標準」`il2cpp_*` 匯出名（未混淆），改為直接 `GetProcAddress` 解析。移除上游那套從 `UnityPlayer.dll` 寫死 RVA 還原混淆名的機制——該 RVA 綁定特定 build，在繁中服會越界 panic（`bounds check failed`）。 |
| `game_impl.rs` | 新增 `Region::Taiwan`，對應 `komoeumamusume.exe`。 |
| `wnd_hook.rs` | 繁中服遊戲視窗為 `class=UnityWndClass`、`title=komoeumamusume`（啟動器是另一個 class `CGameLauncherWnd`，不會誤抓）。 |
| `log_impl.rs` | 改用檔案 logger（`hachimi_tw_<exe>.log`），依 exe 名分檔，避免啟動器與遊戲本體共寫。 |

## 建置

需要 Rust + MSVC toolchain（`x86_64-pc-windows-msvc`）。

```powershell
git clone --recursive https://github.com/tfluan0606/hachimi-tw.git
cd hachimi-tw
cargo build --release
```

產出 `target/release/hachimi.dll`。

## 部署

1. 完全關閉遊戲與啟動器。
2. 把 `hachimi.dll` 複製到遊戲本體目錄，**改名為 `version.dll`**。
3. （可選）在遊戲目錄下建 `hachimi/config.json` 設定 `target_fps`、`disable_gui` 等。
4. 啟動遊戲；預設按 **→（右方向鍵）** 呼出設定選單。

> 移除：刪掉 `version.dll` 即可，遊戲原始檔案不受影響（真 version.dll 由 System32 提供）。

---

## English

**Unofficial fork of [Hachimi](https://github.com/Hachimi-Hachimi/Hachimi)** that makes it run on the **UM:PD Traditional-Chinese client (Komoe, PC)**. All core functionality and the vast majority of the code come from the upstream Hachimi project; this fork only adds the compatibility changes needed for the TW client. Licensed under **GNU GPLv3**, same as upstream.

Please refer to the [upstream repository](https://github.com/Hachimi-Hachimi/Hachimi) for full feature documentation. See the table above for what differs on the TW client (version.dll-hijack injection, standard il2cpp export resolution, TW region/window detection, per-exe file logging).

Use at your own risk — this violates the game's TOS. Do not publicly advertise or link this repo.

## Credits

Built entirely on top of **[Hachimi](https://github.com/Hachimi-Hachimi/Hachimi)** by the Hachimi authors. All upstream special-thanks still apply (Trainers' Legend G, umamusume-localify(-android), Carotenify, umamusu-translate, frida-il2cpp-bridge).

## License

[GNU GPLv3](LICENSE) — inherited from upstream Hachimi. Any distribution of this fork must remain GPLv3 and retain the original copyright notices.
