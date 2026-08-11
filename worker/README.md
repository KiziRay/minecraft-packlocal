# worker — AI 代理 + 免安裝版更新端點

Cloudflare Worker，已部署：`https://modpack-i18n.jolin34563.workers.dev`

目前設定以工具 1.0.2 為目標。Turnstile／協定 v3 必須和支援它的桌面版一起使用；舊版會收到 426，不會繞過 Discord 會員與安全驗證。

## 端點

| 方法 | 路徑 | 用途 |
|------|------|------|
| GET | `/api/desktop/latest` | 回 `{version, url, notes, sha256}`，桌面版檢查更新用 |
| GET | `/download/<檔名>` | 從 R2 bucket `modpack-i18n` 串流免安裝 EXE（更新下載用） |
| POST | `/api/turnstile/start` | 驗證 Discord 後簽發五分鐘挑戰網址 |
| GET | `/turnstile` | 顯示 Cloudflare Turnstile widget |
| POST | `/api/turnstile/verify` | 呼叫 Siteverify，成功後回傳短效 HMAC 憑證到本機 callback |
| POST | `/v1/chat/completions` | AI 代理：驗證 Discord 與 Turnstile 憑證後轉發上游 |
| POST | `/tm/lookup` | 社群共享翻譯記憶查詢（精確鍵＋帶語境的跨模組候選） |
| POST | `/tm/contribute` | 貢獻翻譯（模組、語言鍵、原文雜湊、語境、譯文）；存 R2 `tm/v1/<ns>.json.gz` 與 `tm/v2/global.json.gz` |
| POST | `/api/share/upload` | 登入並通過安全驗證後，上傳可安裝翻譯 ZIP 到獨立 `SHARES` bucket |
| GET/HEAD | `/s/<token>` | 下載 24 小時有效的分享檔；過期立即回 404 |
| GET | `/health` | 健康檢查（`hasKey`、`turnstileReady`） |

共享翻譯記憶：依模組分片存 R2（`tm/v1/<namespace>.json.gz`），每筆資料包含譯文、語境與衝突標記；另有
`tm/v2/global.json.gz` 提供安全的跨模組候選。精確 keyhash＝`sha256(ns\0key\0正規化原文)[:24]`，
跨模組 skhash＝`sha256(key\0正規化原文\0語境)[:24]`。只存字串、無個資。

**省容量／避免重複儲存的設計**：
- **gzip 壓縮**分片（繁中 JSON 常縮到 1/3 以下；實測重複性高的資料省 ~90%）。
- **keyed map**：同一條只有一個鍵，空白或換行差異會先正規化，避免重複儲存。
- **客戶端只回饋「本次新由 AI 產出」的條目**（共享庫命中、術語表、本機記憶都不重送）。
- **只有真的有新條目才寫分片**（`changed` 才 put），沒新增就不動 R2。
- **不同上下文不共用譯文**；同一鍵出現不同譯文時標記 `conflict`，後續不自動套用，交回本機 AI 或人工檢查。
- R2 的分片採合併後寫回；若同時間大量貢獻造成寫入競爭，客戶端會在下一次翻譯重新補送，不能把共享記憶當成唯一備份。

分享檔隔離：`DOWNLOADS → modpack-i18n` 只放安裝檔與翻譯記憶；`SHARES → modpack-i18n-shares` 只放一天分享檔。分享 ZIP 使用 allowlist，只包含 `resourcepacks`、`jar-translated`、指定的 `config` 子資料夾、`patchouli_books`、`kubejs`、`datapacks` 等可安裝內容，不包含 session、錯誤日誌、本機路徑或 API 金鑰。

`wrangler.toml` 目前以工具 1.0.2 為本機建置目標；是否部署由維護者另行決定。桌面端只接受本 Worker 的 `/download/*.exe`，並強制驗證 SHA-256 後才會啟動免安裝更新。

## 翻譯記憶與分享頁補充

共享翻譯記憶會先用「模組、語言鍵、正規化原文」做精確比對；跨模組候選還要符合語言鍵、正規化原文與語境。空白或換行不同的相同文字可以重用，不同譯文會標記 conflict 並停止自動套用，避免上下文衝突。只存匿名文字，不存本機路徑、Discord 身分或整合包檔案。

分享連結的 GET 請求先回傳可嵌入的介紹頁，只有 download=1 或 /download 才回傳 ZIP。介紹頁提供 cloud.zeitfrei.uk 遊戲下載中心與 cloud.zeitfrei.uk/zeitfreitool 工具箱連結，使用 Open Graph 標籤供論壇、聊天平台與其他第三方平台預覽；連結 24 小時後失效。

## 代管 AI 授權

開發者代管 API 只提供給已登入 Discord、仍在 ZeitFrei 官方伺服器，且完成 Cloudflare Turnstile 的玩家。桌面端送出以下標頭：

- `X-Zeitfrei-AI-Protocol: 3`
- `X-Zeitfrei-Client-Version: <桌面版版本>`
- `X-Zeitfrei-Session: <桌面登入 session>`
- `X-Zeitfrei-Turnstile: <短效 HMAC 憑證>`

Worker 會先向 `cloud.zeitfrei.uk/api/check-upload` 驗證 session，再查 Discord `member-tier`。通過後桌面端才能申請 Turnstile 挑戰；原始 Turnstile token 由 Worker 呼叫 Siteverify，成功後簽發綁定 Discord user id、兩小時有效的 HMAC 憑證。缺少新版協定、登入資格或憑證都會拒絕，因此只修改舊版 UI 無法繞過限制。

桌面登入沿用既有的 `/api/desktop-auth` 與 `127.0.0.1:19420..19430/callback`；Turnstile 使用 `127.0.0.1:19431..19440/turnstile-callback`，不需要修改 Discord Developer Portal 或機器人。完成程式更新後仍需手動重新部署本 Worker，線上限制才會生效。

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
npx wrangler secret put TURNSTILE_SECRET_KEY
npx wrangler secret put TURNSTILE_PROOF_SECRET
```

- `TURNSTILE_SECRET_KEY`：Cloudflare widget 的 Site Secret。
- `TURNSTILE_PROOF_SECRET`：至少 32 字元的獨立隨機值，用來簽挑戰狀態與短效憑證，不可與 Site Secret 共用。
- 公開的 Site Key 放在 `wrangler.toml`；widget hostname 必須限制為 `modpack-i18n.jolin34563.workers.dev`。

任何 Secret 缺少時採拒絕存取。Secret 更新會直接套用；程式碼與 `[vars]` 變更仍需執行 `npx wrangler deploy`。若只要補 Site Secret，正確指令是：

```powershell
Set-Location -LiteralPath 'C:\Users\jolin\Downloads\zeitfreigame\modpack-i18n-tool\worker'
npx wrangler secret put TURNSTILE_SECRET_KEY
```

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
npx wrangler r2 object put "modpack-i18n/minecraftpacklocal-1.0.2-portable.exe" `
--file "../src-tauri/target/release/Minecraft 模組整合包翻譯工具.exe" `
  --content-type "application/octet-stream" --remote
```

4. 改 `wrangler.toml` 三個值後，確認目前位於 `worker` 資料夾，再執行 `npx wrangler deploy`：

```toml
[vars]
LATEST_VERSION   = "1.0.2"
DOWNLOAD_URL     = "https://modpack-i18n.jolin34563.workers.dev/download/minecraftpacklocal-1.0.2-portable.exe"
UPDATE_SHA256    = "145489079e07fdbb0064e93e80af190ae445af0581128c3d209fc6ccd1d37c63"
```

客戶端按下「檢查更新」後，會抓到免安裝 EXE、驗證 SHA-256，完成替換後自動重開；自動下載失敗時仍可改用瀏覽器下載。

## 選用：每日免費額度上限（KV）

保護共用金鑰不被少數人刷爆。需要 Workers KV 權限的 API token：

```bash
npx wrangler kv namespace create USAGE
# 把回傳 id 填進 wrangler.toml 的 [[kv_namespaces]]（取消註解），再 deploy
```

`DAILY_TOKEN_BUDGET` 控制所有人的每日總量，`PER_USER_DAILY_TOKEN_BUDGET` 控制單一 Discord 帳號的每日用量。沒有 KV 也能運作，但不會在 Worker 層記帳；此時以 DeepSeek 帳號餘額為最終上限。

## 安全性

- 真正的 DeepSeek 金鑰只存在 Worker secret，客戶端不持有、不傳送。
- Turnstile Site Secret 與 HMAC Secret 只存在 Worker secret；Siteverify 驗證 action 與 hostname。
- 原始 Turnstile token 單次使用；桌面端只保存綁定 Discord user id 的短效憑證，且不寫入磁碟。
- Worker 逐次驗證登入 session、官方 Discord 會員資格與短效憑證，任一服務異常時採拒絕存取。
- 舊版未帶 `MANAGED_AI_PROTOCOL` 指定版本時回 `426 client_upgrade_required`。
- Worker 鎖定模型（`UPSTREAM_MODEL`），避免被拿去打別的昂貴模型。
- 只轉發 `messages`／`temperature`，不透傳任意欄位。
