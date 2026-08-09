# worker — AI 代理 + 桌面版更新端點

Cloudflare Worker，已部署：`https://modpack-i18n.jolin34563.workers.dev`

## 端點

| 方法 | 路徑 | 用途 |
|------|------|------|
| GET | `/api/desktop/latest` | 回 `{version, url, notes, sha256}`，桌面版檢查更新用 |
| GET | `/download/<檔名>` | 從 R2 bucket `modpack-i18n` 串流安裝檔（更新下載用） |
| POST | `/v1/chat/completions` | AI 代理：驗證 Discord 資格，注入伺服器端 DeepSeek 金鑰後轉發上游 |
| POST | `/tm/lookup` | 社群共享翻譯記憶查詢（`{items:[{ns,kh}]}` → `{hits:{kh:zh}}`） |
| POST | `/tm/contribute` | 貢獻翻譯（`{items:[{ns,kh,zh}]}`）；存 R2 `tm/v1/<ns>.json` |
| GET | `/health` | 健康檢查（`hasKey` 表代管金鑰是否設好） |

共享翻譯記憶：依模組分片存 R2（`tm/v1/<namespace>.json.gz = {keyhash: zh}`），keyhash＝
`sha256(ns\0key\0src)[:24]`。只存字串、無個資。

**省容量／避免重複儲存的設計**：
- **gzip 壓縮**分片（繁中 JSON 常縮到 1/3 以下；實測重複性高的資料省 ~90%）。
- **keyed map**：同一條只有一個鍵，天生去重、不會重複儲存。
- **客戶端只回饋「本次新由 AI 產出」的條目**（共享庫命中、術語表、本機記憶都不重送）。
- **只有真的有新條目才寫分片**（`changed` 才 put），沒新增就不動 R2。
- 讀改寫為 last-write-wins（偶發遺漏下次翻譯自動補回）。有 KV 權限可再升級成 KV。

`wrangler.toml` 目前指向 `1.0.0` 安裝檔；實際線上版本以 `/api/desktop/latest` 回應為準。

## 代管 AI 授權

開發者代管 API 只提供給已登入 Discord、且仍在 ZeitFrei 官方伺服器的玩家。桌面端送出以下標頭：

- `X-Zeitfrei-AI-Protocol: 2`
- `X-Zeitfrei-Client-Version: <桌面版版本>`
- `X-Zeitfrei-Session: <桌面登入 session>`

Worker 會先向 `cloud.zeitfrei.uk/api/check-upload` 驗證 session，再以該 session 對應的 Discord user id 查詢 `member-tier`。缺少新版協定、登入過期或不在官方伺服器都會拒絕，因此只修改舊版 UI 無法繞過限制。

桌面登入沿用既有的 `/api/desktop-auth` 與 `127.0.0.1:19420..19430/callback`，不需要新增 Discord Developer Portal callback，也不需要修改現有機器人。完成程式更新後仍需手動重新部署本 Worker，線上限制才會生效。

## ⚠️ 上線前唯一必做：設定金鑰

代管 AI 需要一把 DeepSeek 金鑰放在 Worker 的 **secret**（不會進版控、不會進 exe）。
在 `worker/` 目錄執行，貼上你的金鑰：

```bash
cd worker
npx wrangler secret put DEEPSEEK_KEY
```

沒設之前，`/v1/chat/completions` 會回 `503 server_not_ready`，客戶端顯示
「免費翻譯暫時無法使用，可自行填金鑰或稍後再試」。設好後即刻生效，不必重新 deploy。

## 改版發佈（實際流程）

```bash
# 1. 在專案根建置
npm run build
#    產物：src-tauri/target/release/bundle/nsis/模組包翻譯工具_<版本>_x64-setup.exe

# 2. 算 sha256
certutil -hashfile "src-tauri/target/release/bundle/nsis/模組包翻譯工具_0.5.1_x64-setup.exe" SHA256

# 3. 上傳到 R2（一定要 --remote，否則只進本地模擬）
cd worker
npx wrangler r2 object put "modpack-i18n/modpack-i18n-0.5.1-setup.exe" \
  --file "../src-tauri/target/release/bundle/nsis/模組包翻譯工具_0.5.1_x64-setup.exe" \
  --content-type "application/octet-stream" --remote
```

4. 改 `wrangler.toml` 三個值後 `npx wrangler deploy`：

```toml
[vars]
LATEST_VERSION   = "0.5.1"
DOWNLOAD_URL     = "https://modpack-i18n.jolin34563.workers.dev/download/modpack-i18n-0.5.1-setup.exe"
INSTALLER_SHA256 = "<步驟 2 的雜湊>"
```

客戶端「檢查更新」就會抓到 0.5.1、下載安裝檔（自動驗 sha256）並開啟。

## 選用：每日免費額度上限（KV）

保護共用金鑰不被少數人刷爆。需要 Workers KV 權限的 API token：

```bash
npx wrangler kv namespace create USAGE
# 把回傳 id 填進 wrangler.toml 的 [[kv_namespaces]]（取消註解），再 deploy
```

`DAILY_TOKEN_BUDGET` 控制所有人的每日總量，`PER_USER_DAILY_TOKEN_BUDGET` 控制單一 Discord 帳號的每日用量。沒有 KV 也能運作，但不會在 Worker 層記帳；此時以 DeepSeek 帳號餘額為最終上限。

## 安全性

- 真正的 DeepSeek 金鑰只存在 Worker secret，客戶端不持有、不傳送。
- Worker 逐次驗證登入 session 與官方 Discord 會員資格，驗證服務異常時採拒絕存取。
- 舊版未帶 `MANAGED_AI_PROTOCOL` 指定版本時回 `426 client_upgrade_required`。
- Worker 鎖定模型（`UPSTREAM_MODEL`），避免被拿去打別的昂貴模型。
- 只轉發 `messages`／`temperature`，不透傳任意欄位。
