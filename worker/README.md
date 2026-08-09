# worker — AI 代理 + 桌面版更新端點

Cloudflare Worker，已部署：`https://modpack-i18n.jolin34563.workers.dev`

目前 Turnstile／協定 v3 程式碼屬於「未發布」變更。不要只部署 Worker：必須先把支援 v3 的桌面版改成新版本並準備好更新檔，否則已發布的 1.0.1 會收到 426，卻沒有較新版本可更新。

## 端點

| 方法 | 路徑 | 用途 |
|------|------|------|
| GET | `/api/desktop/latest` | 回 `{version, url, notes, sha256}`，桌面版檢查更新用 |
| GET | `/download/<檔名>` | 從 R2 bucket `modpack-i18n` 串流安裝檔（更新下載用） |
| POST | `/api/turnstile/start` | 驗證 Discord 後簽發五分鐘挑戰網址 |
| GET | `/turnstile` | 顯示 Cloudflare Turnstile widget |
| POST | `/api/turnstile/verify` | 呼叫 Siteverify，成功後回傳短效 HMAC 憑證到本機 callback |
| POST | `/v1/chat/completions` | AI 代理：驗證 Discord 與 Turnstile 憑證後轉發上游 |
| POST | `/tm/lookup` | 社群共享翻譯記憶查詢（`{items:[{ns,kh}]}` → `{hits:{kh:zh}}`） |
| POST | `/tm/contribute` | 貢獻翻譯（`{items:[{ns,kh,zh}]}`）；存 R2 `tm/v1/<ns>.json` |
| GET | `/health` | 健康檢查（`hasKey`、`turnstileReady`） |

共享翻譯記憶：依模組分片存 R2（`tm/v1/<namespace>.json.gz = {keyhash: zh}`），keyhash＝
`sha256(ns\0key\0src)[:24]`。只存字串、無個資。

**省容量／避免重複儲存的設計**：
- **gzip 壓縮**分片（繁中 JSON 常縮到 1/3 以下；實測重複性高的資料省 ~90%）。
- **keyed map**：同一條只有一個鍵，天生去重、不會重複儲存。
- **客戶端只回饋「本次新由 AI 產出」的條目**（共享庫命中、術語表、本機記憶都不重送）。
- **只有真的有新條目才寫分片**（`changed` 才 put），沒新增就不動 R2。
- 讀改寫為 last-write-wins（偶發遺漏下次翻譯自動補回）。有 KV 權限可再升級成 KV。

`wrangler.toml` 目前指向 `1.0.1` 安裝檔；實際線上版本以 `/api/desktop/latest` 回應為準。桌面端只接受本 Worker 的 `/download/*.exe`，並強制驗證 SHA-256 後才會啟動安裝。

## 代管 AI 授權

開發者代管 API 只提供給已登入 Discord、仍在 ZeitFrei 官方伺服器，且完成 Cloudflare Turnstile 的玩家。桌面端送出以下標頭：

- `X-Zeitfrei-AI-Protocol: 3`
- `X-Zeitfrei-Client-Version: <桌面版版本>`
- `X-Zeitfrei-Session: <桌面登入 session>`
- `X-Zeitfrei-Turnstile: <短效 HMAC 憑證>`

Worker 會先向 `cloud.zeitfrei.uk/api/check-upload` 驗證 session，再查 Discord `member-tier`。通過後桌面端才能申請 Turnstile 挑戰；原始 Turnstile token 由 Worker 呼叫 Siteverify，成功後簽發綁定 Discord user id、兩小時有效的 HMAC 憑證。缺少新版協定、登入資格或憑證都會拒絕，因此只修改舊版 UI 無法繞過限制。

桌面登入沿用既有的 `/api/desktop-auth` 與 `127.0.0.1:19420..19430/callback`；Turnstile 使用 `127.0.0.1:19431..19440/turnstile-callback`，不需要修改 Discord Developer Portal 或機器人。完成程式更新後仍需手動重新部署本 Worker，線上限制才會生效。

## 上線前必要設定

代管 AI 需要三個 Worker secrets（不會進版控、不會進 exe）：

```bash
cd worker
npx wrangler secret put DEEPSEEK_KEY
npx wrangler secret put TURNSTILE_SECRET_KEY
npx wrangler secret put TURNSTILE_PROOF_SECRET
```

- `TURNSTILE_SECRET_KEY`：Cloudflare widget 的 Site Secret。
- `TURNSTILE_PROOF_SECRET`：至少 32 字元的獨立隨機值，用來簽挑戰狀態與短效憑證，不可與 Site Secret 共用。
- 公開的 Site Key 放在 `wrangler.toml`；widget hostname 必須限制為 `modpack-i18n.jolin34563.workers.dev`。

任何 Secret 缺少時採拒絕存取。Secret 更新會直接套用；程式碼與 `[vars]` 變更仍需執行 `npx wrangler deploy`。

## 改版發佈（實際流程）

```bash
# 1. 在專案根建置
npm run build
#    產物：src-tauri/target/release/bundle/nsis/模組包翻譯工具_<版本>_x64-setup.exe

# 2. 算 sha256
certutil -hashfile "src-tauri/target/release/bundle/nsis/模組包翻譯工具_1.0.1_x64-setup.exe" SHA256

# 3. 上傳到 R2（一定要 --remote，否則只進本地模擬）
cd worker
npx wrangler r2 object put "modpack-i18n/minecraftpacklocal-1.0.1-setup.exe" \
  --file "../src-tauri/target/release/bundle/nsis/模組包翻譯工具_1.0.1_x64-setup.exe" \
  --content-type "application/octet-stream" --remote
```

4. 改 `wrangler.toml` 三個值後 `npx wrangler deploy`：

```toml
[vars]
LATEST_VERSION   = "1.0.1"
DOWNLOAD_URL     = "https://modpack-i18n.jolin34563.workers.dev/download/minecraftpacklocal-1.0.1-setup.exe"
INSTALLER_SHA256 = "<步驟 2 的雜湊>"
```

舊版客戶端按下「檢查更新」後，就會抓到 1.0.1、驗證安裝檔並自動安裝與重開；自動下載失敗時仍可改用瀏覽器下載。

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
