# Tauri Commands 契約（0.5.0）

前端：`window.__TAURI__.core.invoke("command_name", { … })`  
Rust：`snake_case`；JS 參數 **camelCase**。

---

## 翻譯管線

### `one_click_translate`

| 參數 | 型別 | 說明 |
|------|------|------|
| instancePath | string | 遊戲／實例路徑 |
| outputDir | string | 結果根目錄 |
| packName | string | 資源包名 |
| useAi | bool | 是否 AI 補缺 |
| referencePack | string \| null | 參考 zip 路徑 |

回傳 `OneClickResult`（camelCase）：`report`, `packPath`, `workRoot`, `namespaces`, `filesWritten`, `keysTotal`, `aiFilled`, `minemenuMsg`, `playerSummary`。

事件：過程中 `translate-progress`、`translate-log`。

### `supplement_translate`

| 參數 | 型別 |
|------|------|
| outputDir | string |

需已有工作階段 + AI 金鑰。回傳同 `OneClickResult` 形狀。

### `repair_translation_pack`

| 參數 | 型別 |
|------|------|
| outputDir | string |
| useAi | bool |

重建 zip／session；可選 AI。

### `apply_translation_to_game`

| 參數 | 型別 |
|------|------|
| instancePath | string |
| outputDir | string |
| packName | string \| null |

回傳 `ApplyResult`：`backupDir`, `zipCopied`, `questsCopied`, `minemenuCopied`, `playerSummary`, `warnings`。

---

## 工作階段

### `has_session` / `session_status`

| 參數 | 型別 |
|------|------|
| outputDir | string |

`session_status` → `{ ok, path, message }`。

---

## 工具／設定

| Command | 參數 | 回傳 |
|---------|------|------|
| `scan_only` | instancePath | ScanReport |
| `open_path` | path | bool |
| `open_url` | url | bool（僅 http/s） |
| `open_guide_window` | — | 舊第二窗；UI 改用 overlay |
| `create_font_pack` | fontPath, outputDir, packName, packDesc | FontPackResult |
| `save_api_key` | key | string |
| `save_api_settings_cmd` | apiKey, baseUrl | string（金鑰空＝保留） |
| `has_api_key` | — | bool；**一律 true**（AI 一定可用：自備金鑰或代管） |
| `ai_status` | — | `{ ready, usingOwnKey, managedFree, message }` |
| `get_api_settings` | — | `{ baseUrl, hasKey, keyMasked }` |
| `get_default_reference_pack` | — | string \| null |
| `get_ui_prefs` | — | `{ minimizeOnClose }` |
| `set_ui_prefs` | minimizeOnClose | string |
| `quit_app` | — | 結束行程 |
| `cancel_task` | — | string；要求中止進行中的長任務 |
| `check_update` | — | `UpdateCheck`（見下） |
| `download_update` | — | `{ path, launched, message }`；下載並開啟安裝檔 |
| `open_glossary` | — | string（自訂譯名檔路徑）；不存在會先建範本 |
| `suggest_resourcepacks_dir` | instancePath | string（相容舊） |
| `suggest_output_dir` | instancePath | string（建議繁中翻譯輸出） |

### AI 來源（代管 vs 自備金鑰）

`ai_status` 給前端顯示 AI 是誰在付錢：

```json
{ "ready": true, "usingOwnKey": false, "managedFree": true,
  "message": "AI：使用開發者免費提供的翻譯（額度有限，用完可自備金鑰或贊助支持）" }
```

- 有自填金鑰 → 直連使用者自己的上游（`usingOwnKey:true`）。
- 沒填 → 走開發者代管 Worker（`managedFree:true`），exe 內只有非機密的 Worker URL。
- `has_api_key` 回 true 只是讓「勾了 AI 卻沒金鑰」的舊守門不再誤擋，不代表有自填金鑰；
  要判斷自填請用 `ai_status.usingOwnKey` 或 `get_api_settings.hasKey`。

### 檢查更新（`UpdateCheck`）

```json
{ "current": "0.5.0", "latest": "0.5.0", "updateAvailable": false,
  "url": "https://…", "notes": "", "ok": true, "message": "已是最新版（0.5.0）" }
```

- `ok:false` 代表「暫時查不到」（沒網路等），**不是錯誤**，UI 顯示提示即可。
- `download_update` 只在有新版時下載安裝檔並 `open::that` 開啟；**不自我替換 exe**。
- 前端接線：按鈕 id 用 `#btn-check-update`（`app.js` 末端自足區塊會自動接上，
  並在啟動時安靜檢查一次、把狀態寫進 `#update-status`）；也可呼叫 `window.zfCheckUpdate()`。

### 取消語意

`cancel_task` 設定全域旗標，**不會**殺執行緒。掃描迴圈、AI 批次、寫檔各自在下一個
檢查點退出，已寫出的檔案維持完整。被取消的流程回傳 `Err`，訊息固定為：

```
已依你的要求停止；先前已完成的部分仍保留在結果資料夾。
```

前端據此把它顯示成「已停止」而非「失敗」（`app.js` 的 `isCancellation`）。
每次啟動長任務前後端都會先 `reset_cancel()`。

---

## 事件

### `translate-progress`

```json
{ "percent": 0, "message": "…" }
```

### `translate-log`

```json
{ "level": "info|warn|error", "message": "…" }
```

---

## 新增 Command 檢查清單

1. `lib.rs` 寫 `#[tauri::command]`
2. 加入 `generate_handler![…]`
3. `app.js` invoke + 忙碌鎖定按鈕
4. 更新本檔與 CHANGELOG
5. 若影響使用者流程 → 更新說明 overlay / USER-GUIDE
