# 變更紀錄

格式：版本 — 日期（台北）— 摘要。

## 1.0.2 — 2026-08-10（台北）

- **直接覆蓋安裝、不建資料夾**：選到有效遊戲實例時，翻譯中繼檔預設放工具自管的暫存區（`%APPDATA%\modpack-i18n-tool\work`，`managed_output_base` 命令），不再在你的資料夾另建「翻譯結果」；「套用到遊戲」直接覆蓋安裝（先備份、可一鍵還原）。想要看得到的輸出仍可按「使用啟動器建議的位置」。
- **打包成分享檔（新增 `engine/share_pack.rs`）**：勾「也建立可分享的打包檔」或按「打包成分享檔」，把整包翻譯結果壓成單一 zip 供手動分享（`create_share_package` 命令，附 3 個單元測試）。**上傳雲端拿分享短連結為後續版本**（自建在本專案 Worker）。
- **Turnstile 改為選用**：保留 GPT 的 Turnstile（協定驗證、Discord 會員閘）程式，但**只有服務端設好 `TURNSTILE_*` 金鑰才強制**；未設定時不擋，代管 AI 維持「Discord 登入即可」，避免沒金鑰時把免費翻譯整個弄壞（`authorizeManagedAi` 條件化、client proof 選用、`ai_status` 不再要求 Turnstile）。
- **支持開發按鈕**：移到主畫面操作區的醒目位置，並可重複出現（操作區＋頁尾）。
- **詳細使用說明**：新增 `docs/詳細使用說明.md`（實例是什麼、各啟動器資料夾在哪、該選哪個、如何安裝與分享）。
- 版本 1.0.2；`cargo test` 125 綠、`cargo check` 0 warning。

## 1.0.1 — 2026-08-10（台北）

- **桌面 UI 修復**：改善深色／淺色模式對比、背景模糊與主視窗滾動；全面放大主介面、狀態欄、日誌與使用說明文字，並針對高解析度螢幕再提高一級。新增 Ctrl＋↑／↓與 Ctrl＋滾輪調整 100%～150% 介面比例；開啟「關閉視窗時縮到背景」時，Alt+F4／關閉視窗會先提示再縮到背景；恢復 Discord 與支持開發連結。
- **AI 來源與 Discord 驗證**：AI 可明確選擇「開發者提供的 API」或「自訂 API」。開發者 API 沿用 ZeitFrei 桌面 Discord 登入，必須加入官方伺服器；Worker 同時驗證新版協定、session 與會員資格，舊版不能繞過。自訂 API 不受 Discord 限制。
- **執行檔更名**：產品名改為「Minecraft 模組整合包翻譯工具」；發佈檔改為 `minecraftpacklocal-1.0.1-setup.exe`／`-portable.exe`（GitHub 資產用 ASCII 名，頁面以中文標示安裝版／免安裝版）。
- **Worker 協定 v2**：代管 AI 需 `X-Zeitfrei-AI-Protocol: 2` ＋ Discord session ＋ 官方伺服器會員；舊版用戶端收到 426 需更新。安裝檔更新到 1.0.1（R2 + `/api/desktop/latest` 已同步、雜湊驗過）。
- **可靠自動更新**：比照 ZeitFrei Tool 的防連點與脫離父行程做法，改由官方 NSIS 安裝器自動更新並重開；強制驗證官方下載路徑、SHA-256、檔案大小與 PE 標頭，自動流程失敗時保留可見安裝程式與瀏覽器下載退路。

## 1.0.0 — 2026-08-10（台北）— 公開發佈

- **任務／書本系統翻譯（新增，`engine/quests_books.rs`）**：Better Questing、HQM、Heracles、Modonomicon
  的顯示文字。採「顯示欄位白名單」路徑感知——只翻 name／title／subtitle／description／text… 可達的字串，
  結構欄位（`id`／`type`／`icon`…）與序列化元件不動；BQ 的 `name:8`／`desc:8` 型別後綴會先剝除。串入一鍵與補翻流程。
- **文件補全**：新增 `docs/支援範圍與免責聲明.md`（哪些能翻、哪些不能翻＋免責條款）；README／USER-GUIDE
  補支援矩陣與共享翻譯記憶隱私說明；「覆蓋範圍說明.txt」更新支援清單與未支援項（GuideME Markdown、
  `.zip` 資料包、KubeJS 腳本硬字串、基岩版）。
- **公開發佈**：GitHub `KiziRay/minecraft-packlocal`（PolyForm Noncommercial 1.0.0）；Release 附安裝版／免安裝版與 `SHA256SUMS.txt`。
- 以下項目隨本版一併發佈：
- **社群共享翻譯記憶**（`engine/shared_tm.rs` + Worker `/tm/lookup`、`/tm/contribute`）：
  **隱藏、預設開、零設定**。翻譯時先查社群共享庫，命中就免送 AI；AI 新產出的譯文匿名回饋。
  - **以 `(模組, lang key, 原文雜湊)` 為單位**——跨整合包安全：任何含同模組同版本的包都能重用，
    上下文由 lang key 保證、版本由原文雜湊保證；共享來的一樣過佔位符守衛才採用。
  - 只送字串（原文＋譯文＋雜湊），**無任何個資、路徑、身分**；存開發者 R2（依模組分片）。
  - 服務未就緒／沒網路都**靜默略過**，不擋翻譯。
- **開不了遊戲的診斷與還原**（`engine/diagnose.rs`、`apply_instance.rs`）：
  - `diagnose_launch_failure`：讀當機報告／`latest.log`，判斷是**缺模組（點名缺什麼、與翻譯無關）**
    還是可能我們的檔，直接告訴玩家該補什麼，不再誤怪翻譯。
  - `restore_last_apply_cmd`：**一鍵還原**上次套用（套用時寫 `套用清單.json` 記錄新增/覆蓋，
    還原據此精準反轉）。套用後摘要也加了「開不起來怎麼辦」指引。
- **版本控制器**：`one_click_translate` 新增 `targetVersion` 參數、新增 `detect_mc_version` command。
  使用者可指定整合包的 MC 版本（或自動偵測），**不再靠猜**。
- **26.x／年份制相容**：`pack.mcmeta` 依版本自動選格式——**≤1.21.8** 用單一 `pack_format`
  （零回歸）；**1.21.9＋／26.x** 用 `min_format`/`max_format` 範圍（＋保留 legacy），
  新版不再把資源包標「不相容」。pack_format 對照表下探到 1.13。
- 前端接線契約見 `docs/API-COMMANDS.md`（UI 的版本下拉由前端加）。
- **未來規劃：獨立單一模組翻譯**（尚未實作）：讓玩家直接選擇單一 `.jar`／`.zip` 模組，
  不需要建立整合包實例，獨立掃描語言檔並輸出只包含該模組 namespace 的翻譯資源包；
  目前仍請使用「翻譯整合包」流程處理單一模組。

## 0.5.1 — 2026-08-10

依「其他工具踩過的雷」做的一輪稽核修復。逐項對照見 `docs/AUDIT-2026-08.md`。

### 覆蓋範圍
- **Origins/Apoli 能力文字**（`engine/origins.rs`）：新增掃描 `data/<ns>/powers|origins|
  origin_layers` 的 `name`／`description`。**路徑感知**——祖先是 condition／action／
  modifier／predicate／filter 時一律跳過，避免把 `damage_condition.name="fall"` 這種
  識別字翻成中文害能力失效。掃描與寫回共用同一套排除判斷。（本工具原本完全沒掃 data/。）

### 穩健性
- **寬鬆 JSON 解析**（`engine/lenient_json.rs`）：容忍 `//`／`/* */` 註解與尾逗號
  （Gson 讀得動、serde 嚴格會拒收）。套進 jar 語言檔與覆寫檔解析——原本這類檔會**整檔靜默消失**。
- **解析失敗不再靜默**：救不回的檔記進「翻譯錯誤日誌.txt」，使用者分得清是檔案壞了還是漏翻。
- **空間不足拒絕執行**（`engine/disk.rs`）：一鍵／補翻／修復／字體前先查目標磁碟可用空間
  （`GetDiskFreeSpaceExW`，無額外相依），不足就明確中止，不再寫到一半失敗留半成品。

### 品質與服務
- **換行守衛**：原文單行卻被 AI 加了 `\n` → 收斂成單行；原文多行卻被壓成一行（常伴子句消失）
  → 退回原文。
- **代管模式降並行**：共用金鑰的並行批次由 16 降到 4，避免多人同時把 DeepSeek 打到限流（429）。

### 打包與逆向防護
- **免安裝版 exe**：`mainBinaryName` 改為 `Minecraft 模組包翻譯工具`，產出可直接執行的
  獨立 exe（`src-tauri/target/release/Minecraft 模組包翻譯工具.exe`），不需安裝。
- **前端最小化**：打包時把 `app.js` 以 terser 壓縮＋混淆、剔除未引用的 `main.ts`，
  抬高抄襲門檻（不動 GPT 的 `src/`，改由暫存目錄建置）。原生 Rust 側 `strip`+`lto` 已開。
- **刻意不加殼**（UPX/Themida）：那會大幅提高防毒誤判，與「降低誤刪」衝突。細節見
  `docs/HARDENING.md`。
- **美術素材壓縮**：`src/assets` 26→9MB，安裝檔 31→13.5MB。

### 說明
- 稽核對照與「我們為何不受某些問題影響」寫在 `docs/AUDIT-2026-08.md`。
- 硬體：本工具用雲端 AI + 純 Rust + WebView2，**無 GPU／本地模型需求**，Win10/11、
  AMD／NVIDIA／Intel 皆可（無硬體限制）。
- 隱私：送 AI 的只有字串與通用語境類別（非 lang key／檔案路徑），HTTPS，金鑰在 Worker 端。

### UI/UX 同步
- 版本選擇器會在選取遊戲實例後呼叫 `detect_mc_version`，也允許玩家手動指定版本，並將 `targetVersion` 傳入翻譯流程。
- AI 狀態改由 `ai_status` 顯示「開發者代管」或「自備金鑰」，沒有自備金鑰時不再被舊 UI 文案阻擋。
- 主畫面加入檢查更新入口、較清楚的空白狀態、停止任務說明與輸出檢查提示。
- 使用說明 overlay 與快速指南同步更新 Minecraft 版本、AI 來源、停止與更新流程。

## 0.5.0 — 2026-08-10

引入 Koudesuk/Modpack_Translator（MIT）的技術、免金鑰代管 AI、雲端更新與防毒友善打包。
**「一鍵翻譯」流程不變**，只是更準、更省、更容易上手。

### 翻譯品質與良率
- **佔位符遮罩（mask/unmask）**：送 AI 前把 `%s`／`%1$s`／`{key}`／`§c`／`$(…)`／MDX 標籤／
  markdown 連結／`\n` 換成 `{0} {1}` 簡單索引，收回再還原。模型看不到脆弱 token，
  弄壞率大降；還原後仍過既有 guard。技術移植自該專案 `preprocessor.py`（見 `NOTICE.md`）。
- **併入 1,945 條官方繁中術語表**（`src-tauri/assets/minecraft_glossary_zh_tw.json`，MIT）：
  與現有精選、使用者詞典三層合併（使用者 > 精選 > 官方大表）。命中的整條字串免送 AI，
  用詞也更一致。

### 免金鑰、零設定的 AI（隱蔽式）
- **預設就能翻譯**：不再需要玩家先弄 API 金鑰。沒有自填金鑰時，AI 走**開發者代管的
  Cloudflare Worker**（`worker/`），金鑰以伺服器端 secret 保管、**不進 exe**。
- **額度用完→贊助提示**：代管額度或餘額用盡時回 402/429，客戶端顯示既有的贊助引導。
- **自備金鑰仍可**：進階設定填自己的金鑰就直連上游，不佔用開發者額度。
- 安全性：金鑰不寫進程式（符合硬規則 #7 與安全底線）；exe 內只有非機密的 Worker URL。

### 檢查更新
- 新增 `check_update`／`download_update` command 與「檢查更新」接線（UI 由前端接 `#btn-check-update`）。
- 比對 Worker `/api/desktop/latest` 的版本；有新版→**下載官方安裝檔並開啟**，使用者手動點一次完成。
- **刻意不自我替換 exe**（那是防毒誤判的頭號來源）；安裝檔可選 sha256 驗證。

### 降低防毒誤判
- NSIS 改 `currentUser` 免提權安裝；補上 copyright／描述等版本資訊。
- 不再產生隱藏 powershell／主控台（簡繁轉換早已內建純 Rust）。
- 詳見 `docs/ANTIVIRUS.md`（含程式碼簽章與微軟誤判回報步驟）。

### 基礎建設
- 部署 Cloudflare Worker `modpack-i18n`（AI 代理 + 更新端點）。
- 版本升至 0.5.0；單元測試 56 → 71。

## 0.4.0 — 2026-08-10

翻譯**正確性**與**成本**的一次大改，玩家端最大的改變是不必再裝 Python。

### 正確性（會影響遊戲能不能正常跑）
- **佔位符保護** `engine/placeholder.rs`：譯文寫入前驗證 `%s`／`%d`／`%1$s`／`{0}`／
  `$(br)`／`%player%` 的數量與順序。可修的自動修（全形 `％`、被插入的空白、被吃掉的首尾空白），
  修不好一律**退回英文原文**。避免遊戲丟 `MissingFormatArgumentException`。
  - 位置參數順序被調換也會擋（`%s %d` → `%d %s` 會讓 Java 丟型別例外）
  - 「50% chance」這種散文不再被誤判成佔位符
- **資源 id 不再送 AI**：`minecraft:stone_sword` 這類全小寫帶冒號的字串直接略過，
  避免 JEI／配方對不上。
- **pack_format 真的偵測了**：讀 CurseForge／Modrinth／Prism／MultiMC／ATLauncher／
  官方啟動器的實例設定判斷 Minecraft 版本，對照到 1.16–1.21.9 的 pack_format。
  舊版寫死 15，在 1.21.x 會被遊戲標成「不相容」。補翻與修復也不再把它重設成 15。
- 錯誤日誌時間戳從 `unix=1754…` 改成看得懂的 `2026-08-10 12:34:56 UTC`。

### 品質
- **內建 Minecraft 台灣譯名術語表** `engine/glossary.rs`：苦力怕、終界使者、獄髓、
  絲綢之觸等 200+ 條官方譯名。整條字串命中就直接用，不花 AI；
  批次內出現的術語會附在 prompt 裡約束 AI 用詞，避免同一個怪在不同模組有三種譯名。
- **使用者自訂譯名**：`%APPDATA%\modpack-i18n-tool\glossary.json`，
  首次執行自動產生範本，UI 加「改譯名」按鈕。（舊版 `load_phrase_dict` 的自訂路徑
  參數從來沒被傳過，等於功能是死的。）
- **AI 看得到語境**：由 lang key 推出「物品名／介面文字／提示說明」等提示，
  物品名不會再被翻成一整句解釋。

### 成本與速度
- **翻譯記憶** `engine/tm.rs`：`%APPDATA%\modpack-i18n-tool\tm.json`。
  英文→已驗證譯文跨整合包重用，第二個包常有大量命中，直接省下對應的 API 呼叫。
- **移除 Python／OpenCC 依賴**：改用內建 `zhconv`（純 Rust，zh-Hant-TW，含台灣詞彙轉換）。
  - 玩家不必 `pip install`，也不會再遇到「OpenCC 不可用 → 交出簡中」
  - 幾十萬條字串不再需要數百次子程序呼叫，也不再閃黑窗

### 使用體驗
- **停止按鈕**：長任務可中止，在下一個檢查點乾淨收尾，已完成的檔案保留。
  取消不再被當成「失敗」顯示紅字。
- 覆寫文字超過單次上限時會明講剩幾條，不再靜默截斷。
- AI 設定新增 `model` 欄位，可接任何 OpenAI 相容端點。

### 工程
- 新增 56 個單元測試（`npm run test`），驗證等級由 B 升為 **A**。
- `cargo check` 零 warning；DoD 改為命令判定。

## 0.3.9 — 2026-08-09

### 功能
- **多來源文字**：`text_overlay`（patchouli、openloader、kubejs lang、datapacks 目錄、fancymenu）
- **多語系 pending**：缺繁中時 en_us → en_gb → 其他非 zh_tw/zh_hk 皆可當來源
- **一鍵套用**擴充：patchouli／openloader／kubejs／fancymenu／datapacks（先備份）
- **覆蓋範圍說明.txt**、完整說明頁（overlay 分組目錄）
- **請我喝珍奶**外連：`https://zeitfrei.bobaboba.me`
- 產品定位文案：可遊玩文字→台灣繁（除圖片）、不改 jar

### 效能
- jar 掃描 8–16 執行緒平行
- AI：`PARALLEL=16`、`BATCH=140`
- `translate_plain_strings` 改並行（任務／覆寫加速）

### 文件
- 新增／重寫：`AGENTS.md`、`ARCHITECTURE.md`、`docs/DEVELOPMENT.md`、`API-COMMANDS.md`、`EXTENDING.md`、`USER-GUIDE.md`、`README.md`

## 0.3.8 — 2026-08-09

- 社群策略落地：`apply_to_instance`、覆蓋報告、說明紅線
- pack_format 偵測（預設 15）
- 結果目錄 `翻譯結果/` 佈局

## 0.3.7 及更早

- 一鍵掃 lang + OpenCC + DeepSeek 補缺
- 參考包合併、補翻 session、修復 zip
- FTB Quests snbt、MineMenu 編碼
- 字體包、關閉縮小、錯誤日誌
- 主視窗說明 overlay（取代易全白的第二 WebView）

---

發版時：改 Cargo.toml／tauri.conf.json／package.json 版本號，並在本檔加一節。
## 0.4.0 UI/UX 重製｜2026-08-10

- 重做主畫面為「翻譯工作台」：將準備、翻譯進度與輸出結果分成清楚的兩欄工作區。
- 更新繁中玩家文案、操作層級、行動版排版、鍵盤 focus 與 reduced-motion 支援。
- 改用石墨黑、銅橙與草木綠的單一視覺系統，移除原本偏紫色的模板化樣式；未新增或生成任何美術素材。
- 說明頁改為主視窗內的深色 overlay，保留「不改動 jar、套用前備份、圖片文字不處理」等重要提醒。

## 0.4.0 文件型 UI 與字體服務｜2026-08-10

- 視覺改為 Linear 工具列、Notion 文件區塊、Markdown 標題與 code 標記的低裝飾工作區。
- 字體資源包改為獨立服務，新增獨立輸出位置與字體大小、字重感、位移、oversample 設定。
- 更新主視窗使用說明、`docs/USER-GUIDE.md`、AI 隱私提醒與完整免責說明。

## 0.4.0 Icon 與主題｜2026-08-10

- 採用使用者提供的 Grok image model 原圖作為專屬工具 Icon；程式僅將原圖打包成 Tauri 所需的桌面與安裝包尺寸。
- 主視窗預設深色模式，可切換淺色模式；選擇會保存在本機，下次啟動沿用。

## 0.4.0 完整美術資產接入｜2026-08-10

- 接入使用者提供的 9 張原始 raster 資產：Icon、深／淺色工作區背景、翻譯服務、字體服務、進度狀態、輸出結果與說明頁背景。
- 前端服務頁、狀態欄、輸出提示與使用說明 overlay 均改用實際圖片資產；未以 CSS、Canvas 或程式繪圖取代。
