# 架構說明（0.4.0）

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
   engine/jar_scan     engine/merge_ref    engine/deepseek
   engine/convert      engine/pack_out     engine/ftbquests
   engine/text_overlay engine/session      engine/apply_instance
   engine/out_layout   engine/secrets      engine/security
   engine/minemenu     engine/font_pack    engine/glossary
   engine/placeholder  engine/tm           engine/cancel
   engine/updater
```

## 1b. 雲端（Cloudflare Worker）

```
桌面 App ──GET  /api/desktop/latest──► Worker（回最新版本）→ 檢查更新
        └─POST /v1/chat/completions─► Worker（注入 DeepSeek 金鑰）──► 上游 AI
```

- Worker 原始碼在 `worker/`，已部署 `modpack-i18n.jolin34563.workers.dev`。
- **金鑰只在 Worker secret**，exe 只有非機密的 `MANAGED_BASE_URL`。
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
    ├─ build_resource_pack ──► 翻譯結果/resourcepacks/*.zip
    │
    ├─ translate_ftbquests ──► 翻譯結果/config/ftbquests
    │
    ├─ translate_text_overlays ──► patchouli / openloader / …
    │
    └─ write_coverage_report + 錯誤日誌 + session 更新
```

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
| `ApplyResult` | `apply_instance.rs` | 套用摘要與備份路徑 |
| `Placeholders` | `placeholder.rs` | positional／keyed／soft 三類佔位符 |
| `GuardStats` | `placeholder.rs` | 檢查／修復／退回計數 |
| `Glossary` | `glossary.rs` | 術語表（內建 + 使用者覆寫） |
| `Tm` | `tm.rs` | 翻譯記憶 |
| `AiFillReport` | `deepseek.rs` | 三層命中數與退回數 |

## 4. 前端事件

| 事件 | payload | 用途 |
|------|---------|------|
| `translate-progress` | `{ percent, message }` | 進度條與步驟 |
| `translate-log` | `{ level, message }` | info/warn/error 日誌 |

重工作在 `spawn_blocking`，避免 UI 假死。

## 5. 設定存放

| 項目 | 位置 |
|------|------|
| API 金鑰、Base URL、模型、縮小偏好 | `%APPDATA%\modpack-i18n-tool\secrets.json` |
| 使用者自訂譯名 | `%APPDATA%\modpack-i18n-tool\glossary.json` |
| 翻譯記憶 | `%APPDATA%\modpack-i18n-tool\tm.json` |
| 前端 | 無本地 storage 硬性依賴；偏好走後端 |

翻譯記憶刻意放在 APPDATA 而非結果目錄：它的價值來自**跨整合包**重用。

## 6. 與 ZeitFrei 生態

- **不**接 cloud.zeitfrei.uk 登入、Worker、R2、點數。
- 推廣連結與珍奶贊助為**外開瀏覽器**（`open_url`）。
- 技術選擇對齊 ZeitFrei-Tool（Tauri 2 靜態前端），業務獨立。

## 7. 目錄職責

```
modpack-i18n-tool/
  src/                 前端（tauri frontendDist）
  src-tauri/
    src/lib.rs         command + 管線
    src/engine/        純邏輯模組
    tauri.conf.json    產品名、版本、bundle
  docs/                開發與產品文件
  AGENTS.md            維修硬規則
  README.md            入口
```

## 8. 已知債務

- `lib.rs` 管線偏長，可日後拆 `pipeline.rs`。
- `en_only` 命名仍沿用，語意已是「待譯非繁中來源」。
- `datapacks` 的 zip 本體未展開掃（只掃目錄內檔）。
- KubeJS **腳本內硬字串**未完整 AST 抽取（僅 lang 路徑 + overlay 規則）。
- 尚未支援的任務／書本系統：Better Questing、HQM、Modonomicon、Paxi
  （FTB Quests 與 Patchouli 已支援）。
- 翻譯記憶以英文原文為鍵，不含語境；同一句英文在不同語境會共用譯文。
  語境只影響送給 AI 的 prompt。這是為了命中率的刻意取捨。
- 取消只在階段邊界與批次邊界生效；單一 AI 批次（最多 140 句）需等它結束。
- 覆寫文字單次 AI 上限 8000 條，超過要再按一次「再補一些」（會回報剩餘數）。
