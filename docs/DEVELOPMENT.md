# 開發文件（0.4.0）

## 1. 產品目標

一鍵將 Minecraft **整合包實例**中可覆蓋、可遊玩的文字，整理為**台灣用語繁體中文**：

| 做 | 不做 |
|----|------|
| 語言檔 → zh_tw 資源包 zip | 改 mods jar |
| FTB Quests snbt | 圖片／貼圖文字 OCR |
| patchouli／openloader／fancymenu 等覆寫 | 保證 100% 中文化 |
| 內建 zh-Hant-TW 轉換（純 Rust） | 世界閃退修復 |
| 可選 AI 補缺字串 | 用 AI 掃檔／分類 |
| 一鍵套用（先備份） | 強制覆蓋執行中遊戲檔 |

品質路徑：本機掃 + 社群參考包合併 + AI 只補洞，對齊 CTE2 手修「有繁中底再補」思維。

## 2. 環境

| 需求 | 說明 |
|------|------|
| OS | Windows 11（開發預設） |
| Rust | 1.77+（見 Cargo.toml rust-version） |
| Node | 供 `@tauri-apps/cli` |
| WebView2 | Win11 通常內建 |
| Python | **不再需要**（0.4.0 起簡繁轉換內建於執行檔） |

## 3. 建置與執行

```bash
cd modpack-i18n-tool
npm install

# 開發（熱重載前端靜態檔 + Rust）
npm run dev

# DoD 兩條命令
npm run check            # cargo check，必須 0 error 0 warning
npm run test             # cargo test --lib

# 正式包（需要發佈時再跑；日常可不建 exe）
npm run build
# 產物約：
#   src-tauri/target/release/Minecraft 模組包專用翻譯工具.exe
#   NSIS 安裝包（若 bundle 啟用）
```

`tauri.conf.json`：

- `build.frontendDist`: `../src`（直接吃靜態 HTML/JS/CSS）
- `app.withGlobalTauri`: true → `window.__TAURI__.core.invoke`

## 4. 管線階段（一鍵）

| 進度粗區間 | 階段 | AI？ |
|------------|------|------|
| 0–5% | 準備、ensure_result_layout | 否 |
| 5–40% | jar_scan 平行掃 + 合併 locale | 否 |
| 40–43% | 參考包／舊包合併 | 否 |
| 41–88% | fill_missing_with_ai：術語表 → 翻譯記憶 → AI（可關） | 部分 |
| 88–91% | 最終 zh-Hant-TW 轉換 | 否 |
| 91–93% | build_resource_pack | 否 |
| 93–96% | ftbquests | 可 |
| 94–99% | text_overlays | 可 |
| 100% | session + 覆蓋報告 + 錯誤檔 | 否 |

補翻：`supplement_translate` 讀 session + pack，不重掃 mods。  
修復：`repair_translation_pack` 重建 zip／底稿，可選 AI。  
套用：`apply_translation_to_game` 備份後 merge 複製。

## 5. AI 參數（deepseek.rs）

| 常數 | 值 | 說明 |
|------|-----|------|
| BATCH | 140 | 去重後每批唯一句 |
| RETRY_BATCH | 50 | 失敗拆批 |
| PARALLEL | 16 | 同時 HTTP 批次數 |

- Endpoint：`{base}/v1/chat/completions`（任何 OpenAI 相容服務）
- 模型：`secrets.json` 的 `model` 欄位，預設 `deepseek-chat`
- 約束：prompt 明確要求保留 `%s`、`§`、`{0}`、`$(br)`；回 JSON 對應
- **prompt 的約束不算數，回來一定要驗**：`placeholder::guard` 是唯一防線

速率／帳號限流時可調低 PARALLEL。

送進 AI 前會先被這三層擋掉（`resolve_unique`）：

1. 相同原文去重
2. `glossary::exact` 命中官方台灣譯名
3. `tm::get` 命中翻譯記憶（且佔位符仍相容才採用）

## 5b. 佔位符把關（placeholder.rs）

| 類別 | 例子 | 比對方式 | 不符時 |
|------|------|----------|--------|
| positional | `%s` `%d` `%.2f` `%%` | 序列完全相同（順序有意義） | 退回原文 |
| keyed | `%1$s` `{0}` `{player}` `%player%` `$(br)` | 多重集合相同（可重排） | 退回原文 |
| soft | `§a` `&c` | 只記錄 | 放行 |

修復順序：原樣 → 全形轉半形 → 擠掉佔位符中的空白；每步都重新驗證，
第一個通過的採用，全部失敗才退回原文。原文的首尾空白一律補回。

## 6. 設定檔

全部在 `%APPDATA%\modpack-i18n-tool\`：

| 檔案 | 內容 | 誰寫 |
|------|------|------|
| `secrets.json` | API key、base URL、`model`、`minimize_on_close` | `secrets.rs` |
| `glossary.json` | 使用者自訂譯名（覆寫內建術語表） | 使用者；首次執行產生範本 |
| `phrases.json` | 譯後片語修飾規則（選用） | 使用者 |
| `tm.json` | 翻譯記憶，英文→譯文 | `tm.rs`，先寫 `.tmp` 再 rename |

**禁止**把真實 key 寫進 repo 或預設字串。

## 7. 除錯

| 現象 | 查 |
|------|-----|
| UI 無反應 | 是否在 spawn_blocking；看事件是否 emit |
| 譯文被退回原文 | 正常保護行為；看日誌「佔位符檢查」統計與 `placeholder.rs` |
| 資源包顯示不相容 | `detect_minecraft_version` 有沒有讀到實例設定；對照 `VERSION_TO_FORMAT` |
| 停止沒反應 | 檢查點密度；`cancel::check()` 只在階段邊界，長單批要等該批結束 |
| 補翻失敗 | 翻譯工作階段.json 路徑、session_status |
| 套用失敗 | 遊戲是否關閉、路徑權限、備份目錄 |
| 仍英文 | 是否套用、語言台灣繁中、資源包優先級、是否寫死／圖片 |
| API 失敗 | Base URL、金鑰、日誌「翻譯錯誤日誌.txt」 |

日誌：

- UI `#log`
- 事件 `translate-log`
- 檔案 `翻譯結果/翻譯錯誤日誌.txt`

## 8. 前端約定

- 入口：`src/index.html`（勿依賴未建置的 Vite bundle）
- `app.js`：`invoke` 參數用 camelCase（Tauri 2 對應 Rust snake_case）
- 說明：`#guide-overlay`；`src/guide.html` 為舊路徑，勿當主說明
- 外連：`open_url` + `data-url` 按鈕

## 9. 版本號同步

改版本時同時改：

1. `src-tauri/Cargo.toml`
2. `src-tauri/tauri.conf.json`
3. `package.json`
4. `docs/CHANGELOG.md`

## 10. 測試

### 自動（`npm run test`）

56 個單元測試，覆蓋純函式部分：

| 模組 | 重點 |
|------|------|
| `placeholder` | 擷取／相容判定／修復／散文誤判 |
| `glossary` | 精確查表、字界比對、提示上限、無重複鍵 |
| `tm` | 命中統計、佔位符失效的舊記憶要拒用、不存過長字串 |
| `convert` | 簡→繁、台灣詞彙、正體冪等、ASCII 不動 |
| `pack_out` | 版本→pack_format、版本數字排序、啟動器後綴 |
| `deepseek` | 語境判斷、回應解析容錯、進度不越界、不洩漏服務商名 |
| `cancel` | 旗標 reset 語意 |

**新增邏輯必須附測試**，這是 DoD 的一條。網路與檔案系統相關的部分目前靠人工驗證。

### 人工（動到管線時跑）

1. 小型實例：不勾 AI → 應出 zip、有簡中則轉繁。  
2. 勾 AI、小 pending → 日誌見並行批、有寫入、有「翻譯記憶」統計。  
3. 同目錄再跑一次 → 翻譯記憶命中率應明顯上升、AI 呼叫變少。  
4. 補翻同目錄再跑 → pending 下降。  
5. 套用：關遊戲後 zip 出現在 resourcepacks；備份夾存在。  
6. 開遊戲：繁中（台灣）+ 資源包，且資源包**不該**顯示「不相容」。  
7. 翻譯中按「停止」→ 顯示已停止（非失敗），結果資料夾內容完整。  
8. 在 `glossary.json` 改一個譯名 → 重跑後該詞照使用者的寫法。

## 11. 相關文件

- 架構：`../ARCHITECTURE.md`
- 擴充來源：`EXTENDING.md`
- Command 表：`API-COMMANDS.md`
- 維修規則：`../AGENTS.md`
