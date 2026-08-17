# Minecraft 模組包專用翻譯工具（公開原始碼）

把整合包裡**能翻譯、且影響遊玩**的文字，盡量整理成**台灣用語繁體中文**。

目前版本：**1.0.0**。正式版發佈檔名 `MCPL-1.0.0.exe`。

## 這個倉庫公開什麼／不公開什麼

| 公開 | 不公開 |
|------|--------|
| 前端 UI（`src/`） | 掃描／一鍵翻譯引擎核心（jar 掃描、AI 批譯、overlay／任務等） |
| 可獨立審查的安全 Rust：`updater`／`placeholder`／`security`／`hashutil` | 完整可建置產品後端 |
| Cloudflare Worker 路由（證明代管金鑰／webhook **不在 exe**） | `DEEPSEEK_KEY`、`DISCORD_REPORT_WEBHOOK` 等 **secret 值** |
| LICENSE（PolyForm Noncommercial 1.0.0）、玩家說明與 CHANGELOG | 內部營運筆記／完整私有樹 |

完整可執行檔請用工具內「檢查更新」，或由官方 Worker 下載：

- `GET https://modpack-i18n.jolin34563.workers.dev/api/desktop/latest`
- `GET https://modpack-i18n.jolin34563.workers.dev/download/MCPL-1.0.0.exe`

下載後請核對回傳的 SHA-256。公開原始碼**不等於**保證任意第三方建置產物安全。

## 產品邊界（不變）

- **不改**原始 `mods/*.jar` 內容（只讀）
- **不處理**圖片／貼圖上的字
- 寫入前必過佔位符驗證（`%s`／`{0}` 等）；修不好退回原文
- 不宣稱 100% 漢化
- 代管 AI 金鑰只在 Cloudflare Worker secret，永不進 exe／git

## 授權

本倉庫原始碼與原專案材料採 **PolyForm Noncommercial 1.0.0**（見 [`LICENSE`](./LICENSE)）。第三方元件見 [`NOTICE.md`](./NOTICE.md)。商業使用需另洽著作權人。

## 玩家怎麼用

1. 下載官方免安裝 `MCPL-*.exe`（WebView2 必要；分享功能另需 NanaZip）。
2. 選遊戲實例 → 一鍵翻譯 → 套用（建議先關遊戲；可還原上次套用）。
3. 遊戲語言選繁體中文（台灣），啟用資源包。

完整說明：[`docs/USER-GUIDE.md`](./docs/USER-GUIDE.md)。支援範圍與免責：[`docs/支援範圍與免責聲明.md`](./docs/支援範圍與免責聲明.md)。

## 開發者：公開部分怎麼檢查

```bash
npm install
npm run check:worker
npm run test:worker
# 公開 Rust 模組（updater／placeholder／security）可於私有完整樹用 npm run test 驗證
# 本公開樹的 src-tauri 僅含審查用模組 stub，無法產出完整翻譯 exe
```

Worker 部署與 secret 設定說明見 [`worker/README.md`](./worker/README.md)（只寫 secret **名稱**，不寫值）。

## 下載與更新

- 正式更新通道走 Cloudflare Worker／R2（檔名 `MCPL-{version}.exe`），客戶端強制驗證 SHA-256 與 PE 標頭後才替換。
- GitHub Releases 若有資產僅作輔助；**自動更新以 Worker 為準**。
