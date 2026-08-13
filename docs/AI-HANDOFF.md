# AI 開發交接文件

這份文件給 Claude、Gemini、Grok 等 AI 閱讀。它不是玩家使用說明；要了解完整架構請接著讀 [`DEVELOPMENT.md`](./DEVELOPMENT.md)。

## 1. 開始前必讀

1. 專案根目錄 `AGENTS.md`：優先級最高的維修規則。
2. [`DEVELOPMENT.md`](./DEVELOPMENT.md)：目前實作、資料流與產品邊界。
3. [`ARCHITECTURE.md`](../ARCHITECTURE.md)：架構與資料流細節。
4. [`EXTENDING.md`](./EXTENDING.md)：新增翻譯來源的方式。
5. [`API-COMMANDS.md`](./API-COMMANDS.md)：Tauri command 契約。
6. 只再讀與當前任務有關的 `src-tauri/src/engine/*`、`src-tauri/src/lib.rs`、`src/app.js`、`src/index.html` 或 `worker/`。

不要把 `README.md`、舊版 changelog 或其他 AI 的摘要當成比程式碼更高的真相來源。版本、command 名稱、資料結構要先用搜尋或讀檔確認。

## 2. 目前產品定義

- 輸入：Minecraft Java 整合包實例，或直接選有 `mods/` 的 Minecraft 目錄。
- 輸出：台灣用語繁體翻譯資源包、設定／任務覆寫、JAR 翻譯副本，完成後可直接套用。
- 原始 `mods/*.jar` 只讀；不把翻譯寫回原始 JAR。
- AI 是選用功能；不開 AI 也必須完成本機掃描、簡繁轉換、術語表、TM 與輸出。
- 不保證 100%，不處理圖片 OCR、Java class 改寫與任意腳本邏輯。
- 目前版本必須重新讀取 `src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`package.json`；整理時三處都是 `0.1.2`。

## 3. 不可破壞的規則

- 不修改 `D:/ZstdTauri/ZeitFrei-Tool-2.1.43` 或 `Y:/Azatosz`。
- 不把 DeepSeek、Turnstile、HMAC 或使用者 API key 寫進程式、預設值、測試 fixture 或 Git。
- 不用 AI 找檔、分類或判斷 gameplay 結構；AI 只翻譯字串。
- 不翻資源 id、路徑、機制節點、條件、action、modifier、predicate、filter。
- 寫回任何譯文前必須通過 `placeholder::guard`；不符合就保留原文。
- 超過批次或檔案上限必須寫日誌，不可靜默漏掉後半段。
- 長任務要有 timeout、取消檢查、錯誤記錄；不能讓 UI 無限等待。
- 不強制關閉 Minecraft，不直接覆蓋正在使用的檔案；套用前提示玩家關閉遊戲。
- 新增的 Cloudflare 資料必須使用獨立 binding、bucket 或前綴，不能混用更新檔、分享檔與共享翻譯資料。

## 4. 關鍵流程與檔案

| 要改的事情 | 先看 |
|---|---|
| 一鍵、複查、修復、套用 | `src-tauri/src/lib.rs`、`engine/apply_instance.rs` |
| 語言檔與鬆散路徑搜尋 | `engine/jar_scan.rs`、`docs/SEARCH-MAP.md` |
| JAR 翻譯副本 | `engine/jar_translate.rs`、`jar_display.rs`、`jar_patchouli.rs` |
| FTB／書本／覆寫 | `engine/ftbquests.rs`、`quests_books.rs`、`text_overlay.rs` |
| Origins／Apoli | `engine/origins.rs`；掃描和寫回要共用排除規則 |
| AI／術語／TM／格式 | `deepseek.rs`、`glossary.rs`、`tm.rs`、`placeholder.rs` |
| 結果／工作階段 | `out_layout.rs`、`session.rs` |
| FTB 任務輔助模組 | `translation_helper.rs`、`jar_scan.rs`、`src/app.js` |
| 錯誤分析 | `diagnose.rs` 及其支援模組 |
| AI、更新、R2 | `worker/src/`、`engine/secrets.rs`、`engine/updater.rs` |
| UI | `src/index.html`、`src/app.js`、`src/styles.css` |

## 5. FTB 輔助模組目前行為

這是選用的補充流程，不是主翻譯依賴：

- 1.18.2、1.19.2、1.20.1、1.20.4 Forge／NeoForge：FTB Quest Localizer，指令 `/ftblang export en_us`。
- 1.20.2、1.20.3 Forge：FTB Quests Precision Localizer，指令 `/ftblang en_us ftbquests normal`。
- 由 `inspect_translation_helper_cmd` 判斷是否顯示。
- `prepare_translation_helper_cmd` 從 Modrinth 查相容版本，下載到該實例 `mods/`。
- 使用者回遊戲執行指令後，前端用 `onRun()` 重新掃描。
- `cleanup_translation_helper_cmd` 只刪除工具自己下載、狀態檔記錄、且仍位於該實例 `mods/` 的 JAR。
- 使用者原本安裝的同類模組不刪除；不相容、無網路、無版本時主流程直接跳過。
- 匯出資料夾搜尋包括 Minecraft 根目錄、實例根目錄的 `FTBLang`、`ftblang`、`exported`。

修改這個流程時，優先保持「只顯示需要的按鈕、只刪工具自己的檔案、失敗可跳過」三件事。

## 6. 修改流程

1. 先判斷是回答問題、診斷，還是實際修改；沒有修改要求不要順手改碼。
2. 寫出 3–7 行目標、白名單與 DoD；先讀相關檔案再編輯。
3. 新邏輯優先放新 engine 模組，再由 `lib.rs` 串接；不要無理由重構大型檔案。
4. 前端可見功能要同步 UI、API 文件、玩家說明與 changelog。
5. 新增純函式就加單元測試；網路／檔案流程至少要有錯誤、timeout、跳過或清理路徑。
6. 編輯後重新讀關鍵段落，再跑驗證命令。
7. 回報時分開寫「已驗證」與「尚未人工驗證」，不要用「應該可以」代替測試結果。

## 7. 必跑驗證

```bash
npm run check
npm run test
node --check src/app.js
git diff --check
```

若有改 Worker，再跑：

```bash
npm run check:worker
npm run test:worker
```

除非使用者明確要求，不要執行 `npm run build`、Worker deploy、GitHub push 或 Release 發布。

## 8. 文件與實作不一致時

- 目前程式版本以三個版本檔為準；整理時確認為 `0.1.2`。根目錄 `AGENTS.md` 已對齊版本敘述與免安裝更新行為，勿再照抄舊的 0.5.1／「禁止自我替換」概述。
- `LOCALIZE-202608.md` 是規劃與 backlog，不是完成清單；功能是否存在要回到實際 engine、前端接線與測試確認。

## 9. 常見誤判

- `Process exited with code: -1` 只是退出結果，不是真正根因；需要 crash report、latest.log 或 debug.log。
- ModernUI、AllTheLeaks、Explicit GC 的最後幾行不代表一定是翻譯造成的崩潰；先找 `Caused by`、Mixin、缺模組、註冊表與世界資料錯誤。
- 翻譯後閃退時，先用 `diagnose_launch_failure` 分類，再使用還原功能排除翻譯輸出；不要直接把所有閃退歸咎於 AI。
- JAR 文件掃描能增加線索，但不能安全改寫 `.class`。
- 匯出任務資料不是把任務 SNBT 改成 gameplay lang；保持來源格式，避免破壞任務結構。
- `AI 完成` 不等於 `所有遊戲文字完成`；報告要列出來源、寫入、跳過與未支援項目。

## 10. 交付格式

每次交付至少回報：

- 修改摘要：每個檔案一句話。
- 實際檔案位置。
- UI、資料路徑或 API 是否改變。
- 測試命令與實際結果。
- 尚未人工驗證或仍存在的風險。
