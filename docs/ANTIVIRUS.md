# 降低防毒誤判（給維護者）

本工具是正常的翻譯服務工具，但**未簽章的新 exe** 幾乎一定會被 Windows SmartScreen
與部分防毒以「未知發行者」為由攔下，甚至誤刪。以下是我們已做的、以及只有你能做的。

## 已在程式／打包端做的（0.5.0）

| 措施 | 為什麼降低誤判 | 位置 |
|------|----------------|------|
| **免安裝更新替換** | 更新器先驗證檔案，再由背景排程等待舊程式結束後替換；失敗時保留下載檔供手動開啟 | `engine/updater.rs` |
| **更新 EXE sha256 驗證** | 半途損毀／被掉包會被擋，也讓行為可預期 | `engine/updater.rs` |
| **不生隱藏 powershell／主控台** | 隱藏子程序是常見惡意行為特徵。簡繁轉換已改內建純 Rust，不再外呼 python | `engine/convert.rs` |
| **免安裝 EXE** | 不寫入 Program Files、不要求系統管理員提權；使用者可直接把工具放在有權限的資料夾 | `tauri.conf.json` |
| **完整版本資訊／發行者／版權** | 有 publisher、copyright、描述的檔案比「空白中繼資料」可信 | `tauri.conf.json` |
| **release profile：strip + lto** | 體積小、無除錯符號，減少啟發式雜訊 | `Cargo.toml` |
| **AI 金鑰不進 exe** | 內嵌憑證／金鑰會被部分引擎當可疑字串；改用 Worker 代理 | `engine/secrets.rs` |

## 只有你能做的（照效果排序）

1. **程式碼簽章（最有效）**：用 OV 或 EV 憑證簽 exe 與安裝檔。EV 憑證可立即建立
   SmartScreen 信譽；OV 需累積下載量。簽章後絕大多數誤判消失。
   - Tauri 設定：`bundle.windows.certificateThumbprint` + `signCommand`，或用
     `signtool sign /fd sha256 /tr <timestamp> /td sha256 <檔案>`。
2. **向微軟回報誤判**：<https://www.microsoft.com/wdsi/filesubmission>（選「Software developer」，
   附下載連結說明是正常工具）。通常 1–3 天內把雜湊加入白名單。
3. **各家防毒回報窗口**（被哪家刪就回報哪家）：
   - Bitdefender：<https://www.bitdefender.com/consumer/support/answer/29358/>
   - Avast/AVG：`falsepositive` 表單
   - Kaspersky：`https://opentip.kaspersky.com`
4. **穩定的下載來源**：固定用 `cloud.zeitfrei.uk` 之類自有網域長期提供，累積網域信譽；
   不要每次換連結。
5. **不要用 UPX 之類的加殼／壓縮器**：加殼是惡意軟體最常見的規避手法，一加殼誤判率飆升。
   （本專案未使用，維持現狀即可。）

## 更新版本時要做

改版發佈流程（配合 `worker/`）：

1. `npm run build` 產生 `src-tauri/target/release/Minecraft 模組整合包翻譯工具.exe`（免安裝版）。
2. 簽章（見上）。
3. 算 sha256：`certutil -hashfile <免安裝 EXE> SHA256`。
4. 將 EXE 上傳到 Worker 使用的 R2 `DOWNLOADS` bucket，檔名使用 `*-portable.exe`。
5. 更新 Worker 的版本資訊（見 `worker/wrangler.toml` 的 `LATEST_VERSION`、`DOWNLOAD_URL`，
   可加 `RELEASE_NOTES`、`sha256`）：改完 `cd worker && npx wrangler deploy`。
6. 客戶端「檢查更新」就會抓到新版並提示下載。

> `/api/desktop/latest` 目前回 `{version, url, notes, sha256}`；
> 在 Worker 回應加上 `sha256` 欄位即可，客戶端會自動比對。
