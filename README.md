# Minecraft 模組包專用翻譯工具

把整合包裡**能翻譯、且影響遊玩**的文字，盡量整理成**台灣用語繁體中文**。

- **不修改** `mods/*.jar`
- **不處理**圖片／貼圖上的字
- 產出資源包 zip + 任務／書本等可覆蓋檔；可一鍵套用（先備份）
- 不宣稱 100% 漢化
- **免安裝任何額外東西**（簡繁轉換內建於執行檔）

目前版本：**1.0.0**（以 `src-tauri/Cargo.toml` 為準）

## 支援範圍（一句話）

**會翻**：Forge／NeoForge／Fabric／Quilt 的語言檔；FTB Quests、Patchouli 書本、OpenLoader、KubeJS、FancyMenu、資料包文字；Origins／Apoli 能力；Better Questing／HQM／Heracles／Modonomicon 任務／書本。支援 Minecraft **1.13～1.21.x 與年份版 26.x**。
**不翻**：圖片上的字、寫死在程式碼／KubeJS 腳本裡的字串、GuideME 的 Markdown 書本、`.zip` 資料包、基岩版。
完整清單與**免責條款** → [`docs/支援範圍與免責聲明.md`](./docs/支援範圍與免責聲明.md)。

## 下載

- **Release**：見本專案 [Releases](https://github.com/KiziRay/minecraft-packlocal/releases) 的最新版（安裝檔 + 免安裝版 + `SHA256SUMS.txt`）。
- 工具內建「檢查更新」會自動抓最新安裝檔並**核對 SHA-256** 後再開啟。
- 下載後請用 `SHA256SUMS.txt` 自行核對雜湊，確認檔案未被竄改（公開原始碼不等於自動保證檔案安全）。

## 這個工具跟「丟給 AI 翻」差在哪

| | 直接丟 AI | 本工具 |
|---|---|---|
| 遊戲會不會壞 | `%s` 被吃掉就跳錯誤 | 佔位符驗證，修不好退回原文 |
| 譯名一致性 | 同一隻怪三種叫法 | 內建官方台灣譯名 + 你的自訂詞 |
| 費用 | 每個整合包重付一次 | 翻譯記憶跨包重用 |
| 簡繁 | 常吐簡中 | 一律過 zh-Hant-TW（台灣用語） |
| 範圍 | 只有你貼進去的 | jar lang、任務、書本、覆寫檔全掃 |

---

## 玩家怎麼用

1. 執行免安裝程式（建置後在 `src-tauri/target/release/`）。
2. 選**遊戲實例**資料夾、**結果根目錄**。
3. （建議）選社群繁中參考 zip；勾 AI 並存金鑰。
4. **開始一鍵翻譯** → 完成後**關遊戲** → **一鍵套用到遊戲**。
5. 遊戲內：語言「繁體中文（台灣）」→ 啟用資源包。

完整說明：程式內「完整使用說明」，或 `docs/USER-GUIDE.md`。

---

## 文件地圖（維修請從這讀）

| 文件 | 給誰 | 內容 |
|------|------|------|
| [AGENTS.md](./AGENTS.md) | AI／維修者 | 硬規則、導航、DoD、陷阱 |
| [ARCHITECTURE.md](./ARCHITECTURE.md) | 開發 | 架構、模組、資料流 |
| [docs/DEVELOPMENT.md](./docs/DEVELOPMENT.md) | 開發 | 環境、建置、除錯、設定路徑 |
| [docs/API-COMMANDS.md](./docs/API-COMMANDS.md) | 前後端 | Tauri command 契約 |
| [docs/EXTENDING.md](./docs/EXTENDING.md) | 開發 | 如何加新文字來源 |
| [docs/USER-GUIDE.md](./docs/USER-GUIDE.md) | 玩家／文案 | 使用說明全文結構 |
| [docs/COMMUNITY.md](./docs/COMMUNITY.md) | 產品 | 社群期望與紅線 |
| [docs/CHANGELOG.md](./docs/CHANGELOG.md) | 所有人 | 版本紀錄 |
| [docs/GUIDE-PLAN.md](./docs/GUIDE-PLAN.md) | 文案 | 說明頁資訊架構 |

---

## 開發者快速開始

```bash
# 需求：Rust stable、Node、Windows WebView2（無其他執行期依賴）

cd modpack-i18n-tool
npm install
npm run dev          # 開發
npm run check        # cargo check（DoD：0 error 0 warning）
npm run test         # cargo test --lib
# npm run build      # 正式建置（產 exe／NSIS）— 需要時再跑
```

設定與快取存放於 `%APPDATA%\modpack-i18n-tool\`：
`secrets.json`（金鑰，勿提交）、`glossary.json`（自訂譯名）、`tm.json`（翻譯記憶）。

---

## 技術棧

Tauri 2 + Rust + 靜態前端（`src/index.html` + `app.js` + `styles.css`，`frontendDist: ../src`）。

---

## 授權與免責

模組／地圖著作權屬原作者。本工具依現況提供，不保證完整或正確。完整支援範圍與免責條款見 [`docs/支援範圍與免責聲明.md`](./docs/支援範圍與免責聲明.md)，使用說明見 [`docs/USER-GUIDE.md`](./docs/USER-GUIDE.md)。

本專案的原始程式碼與 ZeitFrei 自有專案素材採用 [PolyForm Noncommercial 1.0.0](./LICENSE)。玩家可以個人或其他非商業目的使用、修改與分享，但不得直接或間接用於商業產品、付費服務、商業散布或轉授權；需要商業使用請先取得書面同意。第三方程式、資料與素材維持原本授權，請查看 [`NOTICE.md`](./NOTICE.md)。

公開原始碼不代表提供檔案安全或正確性保證。使用下載的執行檔或資源前，請自行確認來源、檔案完整性與使用環境。

技術由 **ZeitFrei** 提供支持。自願贊助：<https://zeitfrei.bobaboba.me>
