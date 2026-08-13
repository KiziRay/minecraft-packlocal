# 架構說明（工具 1.0.2）

## 1. 總覽

```
┌─────────────────────────────────────────────────────────┐
│  UI (WebView2)  src/index.html + app.js + styles.css    │
│  invoke / events: translate-progress, translate-log   │
└───────────────────────────┬─────────────────────────────┘
                            │ Tauri commands
┌───────────────────────────▼─────────────────────────────┐
│  lib.rs  管線編排（one_click / supplement / repair /    │
│          apply）+ emit 進度／日誌                        │
└───────────────────────────┬─────────────────────────────┘
                            │
        ┌───────────────────┼───────────────────┐
        ▼                   ▼                   ▼
   engine/jar_scan     engine/jar_translate engine/jar_docs
   engine/pack_version
   engine/merge_ref    engine/deepseek    engine/diagnose
   engine/convert      engine/pack_out     engine/ftbquests
   engine/text_overlay engine/session      engine/apply_instance
   engine/out_layout   engine/secrets      engine/security
   engine/minemenu     engine/font_pack    engine/glossary
   engine/placeholder  engine/tm           engine/cancel
   engine/updater      engine/share_upload  engine/translation_helper
```

## 1b. 雲端（Cloudflare Worker）

```
桌面 App ──GET  /api/desktop/latest──► Worker（回最新版本）→ 檢查更新
        ├─GET  /api/desktop-auth────► cloud.zeitfrei.uk（Discord OAuth）
        ├─POST /api/turnstile/start─► Worker ──► 瀏覽器 Turnstile ──► Siteverify
        │                                      └─短效 HMAC 憑證→ 127.0.0.1 callback
        └─POST /v1/chat/completions─► Worker ──驗證 session／會員／Turnstile 憑證──► 上游 AI
                                            └─► cloud.zeitfrei.uk/member-tier
```

- Worker 原始碼在 `worker/`，已部署 `modpack-i18n.jolin34563.workers.dev`。
- **DeepSeek、Turnstile 與 HMAC 金鑰只在 Worker secret**，exe 只有非機密的 `MANAGED_BASE_URL`。
- 代管模式沿用 ZeitFrei 桌面登入 callback；Worker 協定 v3 要求有效 session、官方 Discord 會員資格與 Turnstile 短效憑證，任一不符即拒絕。
- Turnstile 原始 token 只使用一次並由 Worker 呼叫 Siteverify；桌面端只收到綁定 Discord user id 的短效憑證，且只保存在記憶體。
- 使用者自填金鑰時客戶端**直連上游、不經 Worker**（見 `secrets::resolve_ai_config`）。

## 2. 一鍵翻譯資料流

```
instance (mods/…)
    │
    ├─ scan_instance ──► zh LangMap + en_only(pending) + ScanReport
    │     （平行掃 jar；多 locale；圖檔略過）
    │
    ├─ merge 參考包 / 遊戲內舊包 ──► 補 zh、減 pending
    │
    ├─ save pending 清單 + session
    │
    ├─ [可選] fill_missing_with_ai(pending) ──► 寫入 zh
    │     去重 → 術語表 → 翻譯記憶 → AI（BATCH=140、PARALLEL=16）
    │     ↓ 每條譯文都過 placeholder::guard
    │     ↓ 通過的寫回翻譯記憶；不通過的退回原文
    │
    ├─ zh-Hant-TW 轉換（整圖再保險，內建純 Rust）
    │
    ├─ rewrite_translated_jars ──► 翻譯結果/jar-translated/*.jar
    │     （完整複製 JAR，只改語言檔；含簽章 JAR 安全略過）
    │
    ├─ build_resource_pack ──► 翻譯結果/resourcepacks/*.zip
    │
    ├─ translate_ftbquests ──► 翻譯結果/config/ftbquests
    │
    ├─ translate_text_overlays ──► patchouli / openloader / …
    │     （use_ai=false 時額外來源最多 3 路並行；use_ai=true 維持序列）
    │
    ├─ jar_docs（只讀 JAR 文件與 class 文字線索）
    ├─ inspect／prepare／cleanup_translation_helper（FTB 任務補充，選用）
    ├─ apply_to_instance（備份後直接套用、啟用資源包，含翻譯 JAR）
    └─ write_coverage_report + 錯誤日誌 + session 更新
```

字體資源包是獨立服務：`font_pack` 先建立 `翻譯結果/resourcepacks/<字體包>/`，若前端勾選套用，`apply_font_pack_to_current_instance` 只複製到目前實例 `resourcepacks`，同名先備份；不修改原始字體檔。

資源包名稱和工具版本分開：`pack_version` 讀取 CurseForge `manifest.json`、Modrinth `modrinth.index.json` 等文件；名稱格式為「模組包翻譯工具+月日+整合包版本」，找不到版本時使用 `R1`。同一工作區可以反覆複查，`TranslateSession.review_pass` 記錄複查次數。

分享檔只取可安裝內容，經 `/api/share/upload` 上傳到獨立的 Cloudflare R2 `SHARES` bucket。更新 EXE 使用 `DOWNLOADS`；共享 TM 與共享術語使用獨立的 `TRANSLATIONS` bucket，三者不共用資料路徑。Worker 每次下載都檢查 24 小時期限，排程只負責清理過期物件。

**原則**：
1. 本地全部整理完才 AI；AI 只收字串。
2. **AI 的輸出一律不信任**——佔位符驗證是硬關卡，不是建議。
3. 任何上限被觸發都要寫進 note，不得靜默截斷。

### 補譯三層（`deepseek::resolve_unique`）

| 層 | 來源 | 成本 | 何時命中 |
|----|------|------|----------|
| 1 | `glossary` 內建／使用者術語表 | 0 | 整條字串就是已知術語 |
| 2 | `tm` 翻譯記憶 | 0 | 這句話以前翻過（含別的整合包） |
| 3 | AI | 有 | 前兩層都沒有 |

第 2 層取用前會再驗一次佔位符——記憶是舊資料，原文可能已經改了。

## 3. 核心型別

| 型別 | 定義處 | 意義 |
|------|--------|------|
| `LangMap` | `jar_scan.rs` | `namespace → (lang_key → 字串)` |
| `ScanReport` | 同上 | 掃檔統計與 errors |
| `TranslateSession` | `session.rs` | 補翻用：pending、pack 路徑、instance |
| `ResultLayout` | `out_layout.rs` | work_root / resourcepacks / config |
| `BuildOptions` | `pack_out.rs` | 包名、描述、output、pack_format |
| `CoverageStats` | `out_layout.rs` | 覆蓋報告數字 |
| `JarTranslationReport` | `jar_translate.rs` | JAR 掃描／重建／語言檔統計與錯誤 |
| `ApplyResult` | `apply_instance.rs` | 套用摘要、翻譯 JAR 數量與備份路徑 |
| `Placeholders` | `placeholder.rs` | positional／keyed／soft 三類佔位符 |
| `GuardStats` | `placeholder.rs` | 檢查／修復／退回計數 |
| `Glossary` | `glossary.rs` | 術語表（內建 + 使用者覆寫） |
| `Tm` | `tm.rs` | 本機翻譯記憶 |
| `TranslationScope` | `translation_scope.rs` | 整合包名稱分類與穩定識別 |
| `AiFillReport` | `deepseek.rs` | 三層命中數與退回數 |
| `TranslationHelperStatus` | `translation_helper.rs` | FTB 任務補充的相容性、安裝與清理狀態 |

## 4. 前端事件

| 事件 | payload | 用途 |
|------|---------|------|
| `translate-progress` | `{ percent, message }` | 進度條與步驟 |
| `translate-log` | `{ level, message }` | info/warn/error 日誌 |

重工作在 `spawn_blocking`，避免 UI 假死。

## 5. 設定存放

| 項目 | 位置 |
|------|------|
| API 金鑰、Base URL、模型、AI 來源、縮小偏好 | `%APPDATA%\modpack-i18n-tool\secrets.json` |
| Discord 桌面登入 session | `%APPDATA%\modpack-i18n-tool\discord-session.json` |
| Turnstile 短效憑證 | 僅目前行程記憶體；不寫入磁碟 |
| 使用者自訂譯名 | `%APPDATA%\modpack-i18n-tool\glossary.json` |
| 翻譯記憶 | `%APPDATA%\modpack-i18n-tool\tm.json` |
| 前端 | 無本地 storage 硬性依賴；偏好走後端 |

翻譯記憶刻意放在 APPDATA 而非結果目錄：它的價值來自**跨整合包**重用。

## 6. 與 ZeitFrei 生態

- 開發者代管 AI 沿用 `cloud.zeitfrei.uk` 的 Discord 桌面登入與會員端點，並由 Cloudflare Turnstile 保護共用額度；不接點數系統。
- 自訂 API 完全獨立，不需要 ZeitFrei 帳號或 Discord 會員資格。
- 推廣連結與珍奶贊助為**外開瀏覽器**（`open_url`）。
- 技術選擇對齊 ZeitFrei-Tool（Tauri 2 靜態前端），翻譯資料與工具設定仍由本專案管理。

## 7. 目錄職責

```
modpack-i18n-tool/
  src/                 前端（tauri frontendDist）
  src-tauri/
    src/lib.rs         command + 管線
    src/engine/        純邏輯模組（含選用的 translation_helper）
    tauri.conf.json    產品名、版本、bundle
  docs/                開發與產品文件
  AGENTS.md            維修硬規則
  README.md            入口
```

## 8. 已知債務

- `lib.rs` 管線偏長，可日後拆 `pipeline.rs`。
- `en_only` 命名仍沿用，語意已是「待譯非繁中來源」。
- ZIP datapack／resourcepack 已有安全重建流程；特殊結構、超限檔案與不在顯示欄白名單的內容仍會跳過並記錄。
- KubeJS **腳本內硬字串**未完整 AST 抽取（僅 lang 路徑 + overlay 規則）。
- Better Questing、HQM、Heracles、Modonomicon 已有顯示欄位處理；Paxi 與特殊任務 schema 仍可能需要人工補充。
- FTB 任務遊戲內匯出是選用橋接；只在相容版本顯示輔助模組，不相容時不阻擋主翻譯流程。
- 翻譯記憶以英文原文為鍵，不含語境；同一句英文在不同語境會共用譯文。
  語境只影響送給 AI 的 prompt。這是為了命中率的刻意取捨。
- 取消只在階段邊界與批次邊界生效；單一 AI 批次（最多 140 句）需等它結束。
- 覆寫文字單次 AI 上限 8000 條，超過要再按一次「再補一些」（會回報剩餘數）。
