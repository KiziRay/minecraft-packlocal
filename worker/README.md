# worker — AI 代理 + 免安裝版更新端點

Cloudflare Worker，已部署：`https://modpack-i18n.jolin34563.workers.dev`

目前設定以工具 **0.3.0** 為目標。代管 AI 門檻為 **Discord 會員**（Turnstile 已停用）；協定 v3 標頭仍要，舊版會收到 426。

## 端點

| 方法 | 路徑 | 用途 |
|------|------|------|
| GET | `/api/desktop/latest` | 回 `{version, url, notes, sha256}`，桌面版檢查更新用 |
| GET | `/download/<檔名>` | 從 R2 bucket `modpack-i18n` 串流免安裝 EXE（更新下載用） |
| POST | `/api/turnstile/start` | 驗證 Discord 後簽發五分鐘挑戰網址 |
| GET | `/turnstile` | 顯示 Cloudflare Turnstile widget |
| POST | `/api/turnstile/verify` | 呼叫 Siteverify，成功後回傳短效 HMAC 憑證到本機 callback |
| POST | `/v1/chat/completions` | AI 代理：驗證 Discord 後轉發上游 |
| POST | `/tm/lookup` | 社群共享翻譯記憶查詢（精確鍵＋帶語境的跨模組候選） |
| POST | `/tm/contribute` | 貢獻翻譯；存獨立 `TRANSLATIONS` R2 的 `tm/v1/<ns>.json.gz` 與 `tm/v2/global.json.gz` |
| POST | `/glossary/lookup` | 查詢已確認、無衝突的共享術語 |
| POST | `/glossary/contribute` | 貢獻依整合包分類的術語；重複去除、衝突停用 |
| POST | `/api/share/upload` | 登入 Discord 後上傳可安裝翻譯自解檔到獨立 `SHARES` bucket |
| POST | `/api/share/mpu-create` | 大檔 multipart 初始化（需 Discord） |
| PUT | `/api/share/mpu-part` | 上傳分塊 |
| POST | `/api/share/mpu-complete` | 完成 multipart；有 USAGE KV 時回 8 碼短連結 |
| GET/HEAD | `/s/<短碼或長 token>` | 落地頁或下載；24 小時後 404。短碼走 KV，長 token 仍可下載舊檔 |
| GET | `/health` | 健康檢查（`hasKey`、`authGate`；Turnstile 欄位僅相容） |

共享翻譯記憶：依模組分片存獨立 `TRANSLATIONS` R2（`tm/v1/<namespace>.json.gz`），每筆資料包含譯文、語境、整合包分類與衝突標記；另有
`tm/v2/global.json.gz` 提供安全的跨模組候選。精確 keyhash＝`sha256(ns\0key\0正規化原文)[:24]`，
跨模組 skhash＝`sha256(key\0正規化原文\0語境)[:24]`。只存字串、無個資。

**省容量／避免重複儲存的設計**：
- **gzip 壓縮**分片（繁中 JSON 常縮到 1/3 以下；實測重複性高的資料省 ~90%）。
- **keyed map**：同一條只有一個鍵，空白或換行差異會先正規化，避免重複儲存。
- **客戶端只回饋通過格式檢查的條目**，Worker 會依鍵去重並合併整合包來源。
- **只有真的有新條目才寫分片**（`changed` 才 put），沒新增就不動 R2。
- **不同上下文不共用譯文**；同一鍵出現不同譯文時標記 `conflict`，後續不自動套用，交回本機 AI 或人工檢查。
- R2 的分片採合併後寫回；若同時間大量貢獻造成寫入競爭，客戶端會在下一次翻譯重新補送，不能把共享記憶當成唯一備份。

資料區隔離：`DOWNLOADS → modpack-i18n` 只放更新檔；`TRANSLATIONS → modpack-i18n-translations` 只放共享 TM／術語；`SHARES → modpack-i18n-shares` 只放一天分享檔。分享自解檔使用 allowlist，只包含 `resourcepacks`（僅 zip）、指定的 `config` 子資料夾、`patchouli_books`、`kubejs`、`datapacks` 等可安裝內容，不包含 `jar-translated`、session、錯誤日誌、本機路徑或 API 金鑰。

`wrangler.toml` 目前以工具 1.0.2 為本機建置目標；是否部署由維護者另行決定。桌面端只接受本 Worker 的 `/download/*.exe`，並強制驗證 SHA-256 後才會啟動免安裝更新。

## 翻譯記憶與分享頁補充

共享翻譯記憶會先用「模組、語言鍵、正規化原文」做精確比對；跨模組候選還要符合語言鍵、正規化原文與語境。空白或換行不同的相同文字可以重用，不同譯文會標記 conflict 並停止自動套用，避免上下文衝突。共享術語必須有至少兩個不同整合包分類確認後才會自動套用；只存匿名文字，不存本機路徑、Discord 身分或整合包檔案。

分享連結的 GET 請求先回傳可嵌入的介紹頁（OG 標題「繁體中文模組包翻譯工具」、副標「讓模組包翻譯不再困難」），只有 download=1 或 /download 才回傳自解 exe（檔名「模組包繁中翻譯自解檔.exe」）。介紹頁提供 cloud.zeitfrei.uk 遊戲下載中心與解壓密碼、選 Minecraft 目錄說明；連結 24 小時後失效。有 USAGE KV 時公開路徑為 `/s/<8碼>`；沒有 KV 時仍用長 token，與現況相容。

## 代管 AI 授權

開發者代管 API 只提供給已登入 Discord、仍在 ZeitFrei 官方伺服器的玩家。桌面端送出以下標頭：

- `X-Zeitfrei-AI-Protocol: 3`
- `X-Zeitfrei-Client-Version: <桌面版版本>`
- `X-Zeitfrei-Session: <桌面登入 session>`

Worker 會先向 `cloud.zeitfrei.uk/api/check-upload` 驗證 session，再查 Discord `member-tier`。缺少協定標頭或登入資格會拒絕。Turnstile 路由仍保留於程式碼，但預設不再強制。

桌面登入沿用既有的 `/api/desktop-auth` 與 `127.0.0.1:19420..19430/callback`。完成程式更新後仍需手動重新部署本 Worker，線上限制才會生效。

## 上線前必要設定

分享檔使用獨立的 R2 bucket，第一次設定只需建立一次：

```powershell
Set-Location -LiteralPath '<專案根目錄>\worker'
npx wrangler r2 bucket create modpack-i18n-shares
```

不要把 `SHARES` 改成 `modpack-i18n`，也不要把分享 ZIP 放進 `DOWNLOADS`。以下 Wrangler 指令都要在 `worker` 資料夾執行；如果目前提示字元是 `C:\Windows\System32>`，先執行 `Set-Location`，不要直接在 System32 執行。部署前可用 `npx wrangler deploy --dry-run` 檢查設定；Worker 每小時清理過期分享檔，但下載路由會先檢查期限，因此 R2 lifecycle 延遲不會讓過期連結繼續可用。

代管 AI 需要三個 Worker secrets（不會進版控、不會進 exe）：

```powershell
Set-Location -LiteralPath '<專案根目錄>\worker'
npx wrangler secret put DEEPSEEK_KEY
# 以下僅在你仍要啟用 Turnstile 時才需要：
# npx wrangler secret put TURNSTILE_SECRET_KEY
# npx wrangler secret put TURNSTILE_PROOF_SECRET
```

- 0.3.0 起預設 `TURNSTILE_ENFORCED=0`，代管閘門不依賴 Turnstile Secret。
- 若日後重新強制 Turnstile：公開的 Site Key 放在 `wrangler.toml`；widget hostname 必須限制為 `modpack-i18n.jolin34563.workers.dev`。

`DEEPSEEK_KEY` 缺少時代管 AI 不可用。Secret 更新會直接套用；程式碼與 `[vars]` 變更仍需執行 `npx wrangler deploy`。

## 改版發佈（實際流程）

```powershell
Set-Location -LiteralPath '<專案根目錄>'
# 1. 在專案根建置
npm run build
#    產物：src-tauri/target/release/Minecraft 模組整合包翻譯工具.exe

# 2. 算 sha256
certutil -hashfile "src-tauri/target/release/Minecraft 模組整合包翻譯工具.exe" SHA256

# 3. 切到 worker 後上傳到 R2（一定要 --remote，否則只進本地模擬）
Set-Location -LiteralPath '<專案根目錄>\worker'
npx wrangler r2 object put "modpack-i18n/minecraft-packlocal-v0.3.0-windows-x64.exe" `
  --file "../.upload/minecraft-packlocal-v0.3.0-windows-x64.exe" `
  --content-type "application/octet-stream" --remote
```

4. 改 `wrangler.toml` 三個值後，確認目前位於 `worker` 資料夾，再執行 `npx wrangler deploy`：

```toml
[vars]
LATEST_VERSION   = "0.3.0"
DOWNLOAD_URL     = "https://modpack-i18n.jolin34563.workers.dev/download/minecraft-packlocal-v0.3.0-windows-x64.exe"
UPDATE_SHA256    = "<certutil 算出的 sha256>"
```

客戶端按下「檢查更新」後，會抓到免安裝 EXE、驗證 SHA-256，完成替換後自動重開；自動下載失敗時仍可改用瀏覽器下載。

## 選用：每日免費額度上限（KV）

保護共用金鑰不被少數人刷爆。需要 Workers KV 權限的 API token：

```bash
npx wrangler kv namespace create USAGE
# 把回傳 id 填進 wrangler.toml 的 [[kv_namespaces]]（取消註解），再 deploy
```

`WEEKLY_SHARED_TOKEN_BUDGET` 控制所有人的**每週**共享總量（目前 1000 萬，UTC 週一 00:00 重置），`PER_USER_DAILY_TOKEN_BUDGET` 控制單一 Discord 帳號的**每日**總額度（目前 50 萬）。到巴哈姆特貼文按 GP 並領取加成後，個人今日總額度為 100 萬（`GP_REWARD_BONUS` +50 萬，寫入 `gp_reward:{userId}`，**不是**減少已使用量）。共享 KV key 為 `usage:shared:YYYY-Www`（TTL 7 天）；個人仍為 `usage:user:YYYY-MM-DD:{userId}`。沒有 KV 也能運作，但不會在 Worker 層記帳；此時以 DeepSeek 帳號餘額為最終上限。

Discord webhook：`DISCORD_TOOL_UPDATE_WEBHOOK`（版本更新）、`DISCORD_FEEDBACK_WEBHOOK`（匿名回饋）、`DISCORD_REPORT_WEBHOOK`（診斷回報）、`DISCORD_JOIN_WEBHOOK`（會員驗證成功公告，需 secret + USAGE KV 防刷）。

分享次數同樣綁 USAGE KV：`SHARE_DAILY_LIMIT`（預設 3）與 `SHARE_ACTIVE_LIMIT`（預設 2，同時未過期檔）。沒有 KV 時不擋上傳、不發短碼。超限回 429（客戶端顯示「今天的分享次數已達上限」）。未完成 multipart 超過 `SHARE_MPU_STALE_SECONDS`（預設 1 小時）會在 hourly cleanup 中止。短碼 KV 鍵 `share:id:<8碼>` TTL 24 小時，cleanup 會同時刪 R2 物件與短碼。

## 安全性

- 真正的 DeepSeek 金鑰只存在 Worker secret，客戶端不持有、不傳送。
- Turnstile Site Secret 與 HMAC Secret 只存在 Worker secret；Siteverify 驗證 action 與 hostname。
- 原始 Turnstile token 單次使用；桌面端只保存綁定 Discord user id 的短效憑證，且不寫入磁碟。
- Worker 逐次驗證登入 session、官方 Discord 會員資格與短效憑證，任一服務異常時採拒絕存取。
- 舊版未帶 `MANAGED_AI_PROTOCOL` 指定版本時回 `426 client_upgrade_required`。
- Worker 鎖定模型（`UPSTREAM_MODEL=deepseek-v4-flash`，忽略客戶端 `model`），並**強制** `thinking.disabled`。
- CORS 不再使用 `Access-Control-Allow-Origin: *`；僅反射 allowlist Origin，或允許無 Origin（桌面 WebView）。
- 共享 TM／glossary 貢獻需 Discord 登入 + 會員驗證，並有每日 KV 限速。
