# 開發文件

> 本文件是目前程式的技術總覽。新增功能前先讀本文件、[`ARCHITECTURE.md`](../ARCHITECTURE.md)、[`EXTENDING.md`](./EXTENDING.md) 與 [`API-COMMANDS.md`](./API-COMMANDS.md)。給其他 AI 的快速交接版見 [`AI-HANDOFF.md`](./AI-HANDOFF.md)。
>
> 完整度與速度的長期規劃仍以 [`LOCALIZE-202608.md`](./LOCALIZE-202608.md) 為準；它是 backlog，不代表每一項都已完成。

## 1. 專案狀態

| 項目 | 目前內容 |
|---|---|
| 產品 | Minecraft Java 整合包 → 台灣用語繁體中文翻譯工具 |
| 桌面殼 | Tauri 2 + Rust + WebView2 |
| 前端 | `src/index.html`、`src/app.js`、`src/styles.css`，無 Vite bundle |
| 雲端 | `worker/`：AI 代理、更新、分享與共享翻譯資料服務 |
| 目前版本 | `src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`package.json` 目前均為 `1.0.2`；改版時必須重新讀取三處 |
| 原始 JAR | 只讀；輸出翻譯副本，不直接改 `mods/*.jar` |
| AI | 選用；關閉 AI 仍可做掃描、簡繁轉換、術語表、翻譯記憶與輸出 |
| 平台 | Windows 11 為主要開發與測試環境 |

## 2. 產品邊界

### 會處理

- 模組 JAR、資源包與鬆散資料夾中的 `assets/*/lang/*` 語言檔。
- FTB Quests、Patchouli、OpenLoader、FancyMenu、KubeJS 顯示 API、Origins／Apoli、任務／書本顯示欄位。
- JAR 內可辨識的語言檔、Patchouli 與顯示型 JSON／Markdown／properties；產出翻譯副本。
- ZIP datapack／resourcepack 的安全重建副本。
- 簡體中文轉台灣繁體、內建與使用者術語表、本機／共享翻譯記憶、可選 AI 補缺。
- 翻譯完成後直接套用、可選備份、還原、錯誤分析與 24 小時分享檔。

### 明確不處理

- 圖片或貼圖上的文字 OCR。
- Java `.class` 內寫死的文字改寫。
- 任意 KubeJS／CraftTweaker 腳本邏輯；只處理明確的顯示 API 白名單。
- gameplay id、路徑、機制節點、物品識別字與不明 schema。
- 基岩版、世界缺模組、版本衝突、存檔損壞等遊戲本身問題；工具只協助分析。
- 不宣稱 100% 翻譯或保證每個整合包都能啟動。

## 3. 目錄與責任

| 位置 | 責任 |
|---|---|
| `src/index.html` | 主畫面、說明 overlay、表單與狀態區塊 |
| `src/app.js` | UI 狀態、Tauri `invoke`、進度／日誌、翻譯與輔助流程 |
| `src/styles.css` | 深色／淺色主題、響應式版面、文字大小與背景 |
| `src-tauri/src/lib.rs` | Tauri commands、長流程編排、套用與事件 |
| `src-tauri/src/engine/jar_scan.rs` | JAR／鬆散語言檔搜尋、locale 合併、掃描報告 |
| `src-tauri/src/engine/deepseek.rs` | 去重、術語表、TM、AI 批次與格式驗證前的翻譯調度 |
| `src-tauri/src/engine/convert.rs` | 內建簡繁轉換與台灣用語處理 |
| `src-tauri/src/engine/placeholder.rs` | `%s`、`{0}`、`§`、`$(...)`、item／tag／Markdown 等格式護盾 |
| `src-tauri/src/engine/glossary.rs`、`tm.rs` | 本機術語表與翻譯記憶 |
| `src-tauri/src/engine/shared_tm.rs`、`shared_glossary.rs` | 共享翻譯記憶與共享術語資料接入 |
| `src-tauri/src/engine/text_overlay.rs`、`quests_books.rs`、`origins.rs` | 顯示欄位白名單來源 |
| `src-tauri/src/engine/jar_translate.rs`、`jar_display.rs`、`jar_patchouli.rs` | JAR 翻譯副本與文件／書本複查 |
| `src-tauri/src/engine/translation_helper.rs` | FTB 任務匯出輔助模組的偵測、準備與清理 |
| `src-tauri/src/engine/apply_instance.rs` | 備份、覆蓋、還原與套用摘要 |
| `src-tauri/src/engine/out_layout.rs`、`session.rs` | 結果目錄、報告與可續跑工作階段 |
| `src-tauri/src/engine/diagnose.rs` | Crash report／latest.log 的證據分類，不把退出碼當根因 |
| `worker/src/` | Cloudflare Worker；不要把 Worker secret 放入桌面程式 |

## 4. 一鍵翻譯資料流

```text
選實例或 minecraft 資料夾
  → 找到實際 minecraft 根目錄
  → 選擇自動或自訂「翻譯結果位置」
  → inspect_translation_helper（只在需要時顯示任務補充）
  → 本機掃描 JAR、資源包、鬆散資料與文件
  → 合併繁中、參考包、共享 TM、術語表與本機 TM
  → 對仍缺少的玩家文字執行內建轉換／可選 AI
  → 每條譯文通過 placeholder guard
  → 寫出 resourcepacks、config、data、jar-translated 等結果
  → 建立工作階段、覆蓋報告與錯誤日誌
  → 選項允許時備份，再直接套用到遊戲目錄
  → 完成後清理工具自己下載的 FTB 輔助模組
```

`supplement_translate` 用於同一工作階段的再次複查；`repair_translation_pack` 用於重建結果。現行 UI 不顯示獨立的「套用到遊戲」按鈕，三種流程完成後都會直接套用。`apply_translation_to_game` 只保留舊版相容用途。

## 5. 路徑與輸出規則

- 使用者可選實例根目錄，或直接選有 `mods/` 的 Minecraft 目錄；`resolve_minecraft_dir` 負責判斷。
- 自動輸出位置由 `managed_output_for_instance` 產生；自訂位置通常會建立 `翻譯結果/`。
- 結果根目錄可能包含：`resourcepacks/`、`config/`、`data/`、`patchouli_books/`、`jar-translated/`、`resourcepacks-extra/`、`翻譯工作階段.json`、`覆蓋範圍說明.txt`、`翻譯錯誤日誌.txt`。
- 備份只放在結果資料夾，由玩家勾選是否建立；再次套用會檢查既有備份並避免重複建立。
- 本機設定與快取在 `%APPDATA%\modpack-i18n-tool\`：`secrets.json`、`glossary.json`、`tm.json`、`scan-cache.json`。
- 不要在程式碼寫死使用者的磁碟代號、啟動器名稱或固定家目錄；路徑必須由偵測、選取或環境變數取得。

## 6. AI 與共享資料

翻譯成本由低到高依序為：內建台灣術語表 → 本機 TM → 共享 TM／共享術語 → AI。AI 只接收待翻譯字串與必要語境，掃描、分類、去重與輸出在本機完成。

- `managed` 代管模式：透過 Worker；需依目前服務設定完成 Discord／Turnstile 驗證。
- `custom` 自訂模式：使用者選服務商預設與自己的 Key；內建服務商不需手填 Base URL，其他 OpenAI 相容服務才自行輸入。
- DeepSeek 金鑰、Turnstile secret、HMAC secret 只放 Worker secret；不得進 EXE、預設值或 Git。
- 共享資料必須和更新檔、分享檔分開；相同內容去重，衝突內容不可自動覆蓋。
- 任何 AI 譯文、TM 譯文與共享譯文在寫回前都要通過格式護盾；失敗就保留原文。

## 7. FTB 任務補充流程

這是選用的遊戲內匯出橋接，不是翻譯引擎必要依賴。

| Minecraft／Loader | 使用的輔助模組 | 指令 |
|---|---|---|
| 1.18.2、1.19.2、1.20.1、1.20.4 Forge／NeoForge | FTB Quest Localizer | `/ftblang export en_us` |
| 1.20.2、1.20.3 Forge | FTB Quests Precision Localizer | `/ftblang en_us ftbquests normal` |

流程：偵測 FTB Quests → 顯示相容方案 → 使用者按準備 → 從 Modrinth 取得版本並放到該實例 `mods/` → 使用者啟動遊戲執行指令 → 回工具重新翻譯 → 掃描 `FTBLang`／`exported` → 完成後清理工具自己下載的 JAR。玩家原本安裝的同類模組不會被刪除。不相容、沒有網路或找不到版本時，主翻譯直接跳過。

相關 commands 與 `TranslationHelperStatus` 以 [`API-COMMANDS.md`](./API-COMMANDS.md) 為準。

## 8. Cloudflare 與資料隔離

| 用途 | Worker／R2 邊界 |
|---|---|
| 代管 AI | Worker 驗證後呼叫上游；secret 不進桌面端 |
| 更新 | `DOWNLOADS`；依目前 updater 契約驗證官方 URL、SHA-256 與 PE 檔頭 |
| 一日分享 | `SHARES`；只放可安裝內容，連結有效 24 小時 |
| 共享 TM／術語 | `TRANSLATIONS`；不與更新或分享混用 |

新增 Cloudflare 資料時，必須使用獨立 binding／路徑／物件前綴，不可把資料寫進既有更新或分享 bucket。

## 9. 開發與驗證

```bash
npm install
npm run check
npm run test
node --check src/app.js
git diff --check
```

Worker 另行驗證：

```bash
npm run check:worker
npm run test:worker
```

人工測試至少包括：不開 AI 的小型實例、AI 小批次、同包重跑 TM 命中、FTB 輔助模組準備／重新掃描／清理、直接套用與還原、錯誤分析、分享前二次確認、24 小時分享期限與不同磁碟路徑。

## 10. 新增功能規則

1. 先把邏輯放到新的 `engine/*.rs` 模組，再由 `lib.rs` 編排；不要繼續把所有邏輯塞進 `lib.rs`。
2. 新文字來源必須同時有 scan、translate、write、apply 或明確說明為只讀線索。
3. 不新增無法測試的全域狀態；純函式必須附單元測試，網路／檔案流程要有 timeout、錯誤日誌與可跳過行為。
4. 新增前端可見流程時，同步 `src/index.html`、`src/app.js`、`docs/USER-GUIDE.md`、`docs/API-COMMANDS.md` 與 `docs/CHANGELOG.md`。
5. 改動版本時同步 `Cargo.toml`、`tauri.conf.json`、`package.json` 與變更紀錄；沒有發佈要求不要自行建置或部署。

## 11. 已知限制與待整理事項

- `lib.rs` 仍是主要流程編排檔，後續可另案拆出 pipeline，不要在小功能中順手大拆。
- JAR `.class`、圖片、執行期動態字串仍只能列為線索或未支援。
- AI 品質受原文上下文、術語表與第三方服務影響；不能把「完成」寫成 100%。
- `LOCALIZE-202608.md` 中的 Provider 多後備分流仍不在目前範圍。
- 若文件與程式版本不一致，以 `Cargo.toml`、`tauri.conf.json`、`package.json` 與實際測試結果為準，並在變更紀錄補充原因。

## 12. 文件閱讀順序

| 讀者 | 順序 |
|---|---|
| 新開發者 | `AGENTS.md` → 本文件 → `ARCHITECTURE.md` → `EXTENDING.md` → `API-COMMANDS.md` |
| 其他 AI | `AGENTS.md` → [`AI-HANDOFF.md`](./AI-HANDOFF.md) → 本文件 → 相關程式檔 → DoD 命令 |
| UI／文案 | `USER-GUIDE.md` → `src/index.html` → `src/app.js` → `styles.css` |
| 翻譯來源開發 | `LOCALIZE-202608.md` → `SEARCH-MAP.md` → `EXTENDING.md` → 對應 engine 模組 |
