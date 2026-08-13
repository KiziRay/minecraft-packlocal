# Tauri Commands 契約（工具 1.0.3；資源包版本另行偵測）

前端：`window.__TAURI__.core.invoke("command_name", { … })`  
Rust：`snake_case`；JS 參數 **camelCase**。

---

# 備份控制補充

`delete_apply_backups_cmd(instancePath, outputDir?)` 會刪除翻譯結果資料夾內、由工具建立的 `翻譯套用備份_*` 目錄，回傳 `DeleteBackupResult`。未傳 `outputDir` 時仍會相容搜尋舊版實例旁的備份；它不會刪除其他名稱的資料夾。

`delete_result_folder_cmd(outputDir)` 會完整刪除指定結果位置下的 `翻譯結果/` 工作資料夾（包含備份），但不會刪除使用者選的上層資料夾；後端會先檢查它是否像工具建立的結果資料夾。

## 翻譯管線

### `one_click_translate`

| 參數 | 型別 | 說明 |
|------|------|------|
| instancePath | string | 遊戲／實例路徑 |
| outputDir | string | 翻譯結果位置；預設是每個實例的自動位置，也可由前端指定玩家選的資料夾。若不是 `翻譯結果` 本身，後端會在裡面建立 `翻譯結果/` |
| packName | string | 相容舊版的參數；後端會忽略並依整合包版本產生名稱 |
| useAi | bool | 是否 AI 補缺 |
| backupBeforeApply | bool | 是否在翻譯完成套用前建立備份；預設前端會勾選 |
| referencePack | string \| null | 參考 zip 路徑 |
| targetVersion | string \| null | **使用者指定的 MC 版本**（如 `1.20.1`、`26.2`）；`null`＝自動偵測 |
| translationMode | string | `append`（預設）、`skip-if-complete`、`force`；Force 忽略共享／本機翻譯記憶，但保留術語表與格式護盾 |
| translationQuality | string | `fast`、`balanced`（預設）、`thorough`；調整 AI 批次大小與提示，不做 API 分流 |
| coverageTier | string | 固定傳 `max`（完整挑戰）；後端忽略其他值 |

回傳 `OneClickResult`（camelCase）：`report`, `packPath`, `workRoot`, `namespaces`, `filesWritten`, `keysTotal`, `aiFilled`, `jarTranslation`, `minemenuMsg`, `playerSummary`。`jarTranslation` 會列出掃描、重建、寫入語言檔與略過錯誤。工作區另可能有 `data/`（JAR 內 Patchouli）、`resourcepacks-extra/`（ZIP 內翻譯副本）與 KubeJS 安全白名單輸出。

事件：過程中 `translate-progress`、`translate-log`。`useAi=false` 時，FTB Quests、文字覆寫、ZIP 文字、Origins、任務／書本與 KubeJS 顯示字串最多 3 路並行；`useAi=true` 維持序列。

### `inspect_translation_helper_cmd`／`prepare_translation_helper_cmd`／`cleanup_translation_helper_cmd`

這三個命令只處理 FTB Quests 的「遊戲內任務文字匯出」補充步驟，不會把輔助模組當成翻譯工具本身，也不會修改任何原始 JAR。

| Command | 參數 | 回傳 |
|---------|------|------|
| `inspect_translation_helper_cmd` | `instancePath`, `outputDir?` | `TranslationHelperStatus`；只讀判斷是否需要、是否相容、是否已有輔助模組 |
| `prepare_translation_helper_cmd` | `instancePath`, `outputDir` | `TranslationHelperStatus`；使用者主動按下後，從 Modrinth 取得相容版本並放入該實例 `mods/` |
| `cleanup_translation_helper_cmd` | `instancePath`, `outputDir` | `TranslationHelperStatus`；只刪除本工具這次下載且仍位於該實例 `mods/` 的 JAR |

`TranslationHelperStatus`（camelCase）包含：`needed`、`supported`、`state`、`helperName`、`minecraftVersion`、`loader`、`command`、`message`、`sourceUrl`、`modPath`、`installedByTool`、`changed`。

目前只在偵測到 FTB Quests 且版本／載入器有明確相容方案時提供按鈕；不相容、網路失敗或沒有可用版本時，主翻譯流程會跳過。Forge／NeoForge 的支援版本會選 FTB Quest Localizer；Forge 1.20.2／1.20.3 會選 FTB Quests Precision Localizer。使用者原本就放在 `mods/` 的同類模組只會顯示指令，不會被刪除。

翻譯完成後前端會自動呼叫清理命令；若玩家中途離開，也可從任務補充區塊手動清理。清理只認工具自己的狀態檔與實例內 JAR 路徑，不會掃描或刪除其他模組。

### 開不了遊戲：診斷與還原

套用結果面板建議加兩顆按鈕，玩家遇到「加了翻譯後開不起來」時用：

- **讀取最近記錄** → `diagnose_launch_failure(instancePath)` →
  `{ verdict, summary, missing[], translationRelated, source, errorCode, primaryError, evidence[], suspectedMods[], confidence, nextSteps[], gameExitCode, logKind }`。
  `verdict`：`missing_mod`（缺模組）／`runtime`（Java、JVM、記憶體或顯示環境）／`mod_loading`
  （模組載入或版本相容性）／`maybe_our_files`（只有路徑明確指向翻譯輸出才會出現）／`content_missing`
  ／`world_content`（建立世界或世界內容更新）／`content_data`／`unknown`／`no_logs`。`errorCode` 是工具分類代碼，`gameExitCode` 才是從記錄擷取的遊戲退出碼；不能只拿退出碼當根因。
- **還原上次套用** → `restore_last_apply_cmd(instancePath, outputDir?)` →
  `{ backupDir, removed, restored, playerSummary }`。反轉最近一次套用（新增的刪掉、覆蓋的還原）。

設計原則：資源包（語言檔）安全；會影響世界載入的是資料包／任務類。套用時已寫
`套用清單.json` 到備份夾，還原據此精準反轉。

- **貼上錯誤分析** → `diagnose_error_text(text)`：只在本機分析貼上的 crash report、latest.log、debug.log 或錯誤碼；完整記錄會比單一退出碼得到更多結果。`errorCode` 是工具的判斷代碼，不是 Minecraft 的退出碼。

### 版本控制器（前端接線）

- **`detect_mc_version(instancePath) -> string | null`**：偵測整合包的 MC 版本，給 UI 預填版本選單。
- 前端建議加一個 `#target-version` 下拉（選項含 `1.16.5`…`1.21.8`、`1.21.9`、`26.1`、`26.2`、`26.3` 等，
  預設用 `detect_mc_version` 的值），送 `one_click_translate` 時帶 `targetVersion`。
- 後端據此決定 `pack.mcmeta`：**≤1.21.8** 用單一 `pack_format`（與過去一致）；
  **1.21.9＋／年份制 26.x** 用 `min_format`/`max_format` 範圍（同時保留 legacy），
  這樣新版不會被標「不相容」。不傳 `targetVersion` → 自動偵測（26.x 也會自動走範圍制）。
- 版本會存進工作階段，補翻／修復會沿用同一版本重建。

### `supplement_translate`

| 參數 | 型別 |
|------|------|
| outputDir | string |
| useAi | bool |
| backupBeforeApply | bool | 是否在重新套用前建立備份；預設前端會勾選 |

需已有工作階段。`useAi=false` 時仍會套用本機術語表與翻譯記憶，無法離線補上的內容保留原文；回傳同 `OneClickResult` 形狀。

### `repair_translation_pack`

| 參數 | 型別 |
|------|------|
| outputDir | string |
| useAi | bool |
| backupBeforeApply | bool | 是否在重新套用前建立備份；預設前端會勾選 |

重建 zip／session；可選 AI。

### `detect_pack_translation_name`

| 參數 | 型別 |
|------|------|
| instancePath | string |

回 `{ version, packName, source, metadataPath }`。`version` 來自 CurseForge／Modrinth 等整合包文件，找不到時使用 `R1`；不會使用工具版本。

### `inspect_jar_documentation`

| 參數 | 型別 |
|------|------|
| instancePath | string |
| outputDir | string |

只讀掃描 `mods/*.jar` 內的文字文件與 class 可讀字串線索，輸出到工作區 `jar-documentation/`；不執行 JAR，也不把 class 線索直接改回 JAR。翻譯流程另會將語言檔寫入 `jar-translated/` 的完整副本。

### `apply_translation_to_game`（舊版相容命令）

| 參數 | 型別 |
|------|------|
| instancePath | string |
| outputDir | string |
| packName | string \| null |
| backupBeforeApply | bool | 是否在套用前保留被覆蓋檔案；未勾選時不建立工具備份 |

另外回傳的 `ApplyResult` 會包含 `backupCreated` 與 `backupReused`。`backupCreated` 只代表這次執行新建了備份；同一個實例、同一個套用位置且既有備份完整時，會改為 `backupReused: true`，不會建立重複備份資料夾。若未勾選備份，`backupDir` 為空字串，兩個欄位都為 `false`。

回傳 `ApplyResult`：`backupDir`, `backupCreated`, `backupReused`, `zipCopied`, `jarsCopied`, `questsCopied`, `minemenuCopied`, `playerSummary`, `warnings`。`jarsCopied` 是已覆蓋到 `mods` 的翻譯 JAR 數量。

目前 UI 不再顯示獨立的「套用到遊戲」按鈕；一鍵翻譯、再次複查與修復會在流程完成後直接套用。此命令只保留給舊版前端或外部整合使用。

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
| `create_font_pack` | fontPath, outputDir, packName, packDesc, fontOptions?, packFormat?, targetVersion? | FontPackResult |
| `apply_font_pack_to_current_instance` | instancePath, fontPackPath | FontPackApplyResult；複製到目前實例 `resourcepacks`，同名先備份到 `字體套用備份_*` |
| `managed_output_for_instance` | instancePath | 每台電腦每個實例獨立的工作區 |
| `upload_share_package_cmd` | workRoot, name | 先建立只含可安裝內容的暫存 ZIP，再上傳獨立 SHARES R2，回 `{ url, expiresAt }`；需 Discord、官方伺服器會員與 Cloudflare 驗證，連結保留 24 小時 |
| `save_api_key` | key | string |
| `save_api_settings_cmd` | apiKey, provider, baseUrl, model | string（金鑰空＝保留；內建服務商只需 provider＋Key） |
| `set_ai_mode_cmd` | aiMode：`managed`／`custom` | string |
| `has_api_key` | — | bool；只代表是否已儲存自訂 API 金鑰 |
| `ai_status` | — | 見下方；代管模式會讀取 Worker 是否強制 Turnstile，並檢查 Discord 與本機短效憑證；`serviceAvailable` 只反映 Discord，Turnstile health 另有 `turnstileServiceReady`／`turnstileHealthError` |
| `validate_instance_cmd` | `instancePath` | 實例門檻：mods＋模組檔＋config／options／resourcepacks／啟動器特徵 |
| `get_api_settings` | — | `{ provider, model, baseUrl, hasKey, keyMasked, aiMode }`；provider 可為 `deepseek`、`glm`、`openai`、`qwen`、`other` |
| `discord_login` | — | `{ ok, user? , error? }`；開啟既有桌面 OAuth 流程 |
| `cancel_discord_login_cmd` | — | bool |
| `discord_auth_status` | — | `DiscordAuthStatus` |
| `discord_logout` | — | string |
| `turnstile_verify` | — | `{ ok, expiresAt?, error? }`；瀏覽器完成驗證後由本機 callback 接收短效憑證 |
| `cancel_turnstile_verification_cmd` | — | bool |
| `get_default_reference_pack` | — | string \| null |
| `download_cfpa_reference_pack` | `mcVersion`, `destDir?` | `{ path, attribution }`；依 MC 版本嘗試下載 CFPA GitHub Release zip；失敗由前端略過 |
| `get_ui_prefs` | — | `{ minimizeOnClose }` |
| `set_ui_prefs` | minimizeOnClose | string |
| `quit_app` | — | 結束行程 |
| `cancel_task` | — | string；要求中止進行中的長任務 |
| `diagnose_launch_failure` | instancePath | `LaunchDiagnosis`（見下）；讀當機報告判斷缺模組 vs 我們的檔 |
| `restore_last_apply_cmd` | instancePath | `RestoreResult`；一鍵反轉上次套用 |
| `delete_apply_backups_cmd` | instancePath | `DeleteBackupResult`；只刪除本工具建立的翻譯套用備份 |
| `has_apply_backups_cmd` | instancePath, outputDir? | `bool`；只讀檢查是否有可還原的工具備份，供 UI 顯示還原／刪除按鈕 |
| `check_update` | — | `UpdateCheck`（見下） |
| `download_update` | — | `{ path, launched, automatic, message }`；驗證免安裝 EXE 後替換並重開，失敗時保留檔案供手動開啟 |
| `open_glossary` | — | string（自訂譯名檔路徑）；不存在會先建範本 |
| `suggest_resourcepacks_dir` | instancePath | string（相容舊） |
| `suggest_output_dir` | instancePath | string（建議繁中翻譯輸出） |

### AI 來源（開發者代管 vs 自訂 API）

使用者必須明確選擇來源。代管模式的狀態範例：

```json
{ "ready": true, "aiMode": "managed", "usingOwnKey": false,
  "managedFree": true, "loggedIn": true, "inGuild": true,
  "serviceAvailable": true, "turnstileRequired": true, "turnstileVerified": true,
  "turnstileExpiresAt": 1786320000, "displayName": "玩家名稱",
  "inviteUrl": "https://discord.gg/zeitfrei", "message": "Cloudflare 安全驗證已完成。" }
```

- `managed`：需要 Discord 登入且仍在官方伺服器；`turnstileRequired: true` 時還要完成 Cloudflare Turnstile。桌面端會先讀取 Worker `/health` 的設定，避免把服務端未啟用的驗證誤當成必要步驟。Worker 協定 v3 缺少 session、會員資格或必要的短效憑證時直接拒絕。
- `custom`：使用者選擇服務商預設並使用自己的 Key 直接連上游，不需要 Discord 驗證；內建服務商不需要填 Base URL，其他 OpenAI 相容服務才需要自行填寫。
- Discord 登入沿用 `https://cloud.zeitfrei.uk/api/desktop-auth` 與本機 `127.0.0.1:19420..19430/callback`，不需要新增 Discord Developer Portal callback。
- Turnstile 使用 `127.0.0.1:19431..19440/turnstile-callback`；原始 token 由 Worker 呼叫 Siteverify，桌面端只在記憶體保存綁定 Discord user id 的短效 HMAC 憑證。
- `discord-login-url` event payload 為 `{ "url": "https://…" }`，供前端顯示瀏覽器未自動開啟時的備用網址。
- `turnstile-url` event 同樣回 `{ "url": "https://…" }`，供前端重新開啟驗證頁。

### 檢查更新（`UpdateCheck`）

```json
{ "current": "1.0.2", "latest": "1.0.2", "updateAvailable": false,
  "url": "https://…", "notes": "", "ok": true, "message": "已是最新版（1.0.2）" }
```

- `ok:false` 代表「暫時查不到」（沒網路等），**不是錯誤**，UI 顯示提示即可。
   - `download_update` 防止重複執行，只接受官方 Worker `/download/*-portable.exe`；必須通過 SHA-256、100 KB～256 MB 與 `MZ` PE 標頭檢查。
- Windows 會下載並驗證官方免安裝 EXE，再由脫離 Tauri Job 的背景排程等待目前工具關閉，替換同一路徑後重新開啟；若無法自動替換，則保留下載檔供使用者手動開啟。
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
