# 變更紀錄

## 0.2.1 — 2026-08-14（台北）— 視窗可縮放＋布局自適應斷點

- **視窗**：`resizable=true`；預設約 1080×760；`minWidth`/`minHeight` 降至 360×400，可明顯縮小而非單一尺寸感。
- **Fluid 布局**：`clamp()`／`minmax`／百分比與 `fr`；內容寬與執行欄寬隨視窗變，大窗仍限制 CTA 不過寬。
- **斷點**：寬（≥1000 雙欄）／中（堆疊＋完整度橫滑）／窄（單欄、抽屜改底部 sheet）；矮窗壓縮說明、步驟改橫向。
- 版本同步：`package.json`／`Cargo.toml`／`tauri.conf.json`／UI／Worker `LATEST_VERSION` → **0.2.1**。

## 0.2.0 — 2026-08-14（台北）— UI 從零重做（視覺大版本）

- **資訊架構重做**：固定 `100dvh` 殼（頂欄／底欄固定，中間工作區捲動）；內容最大寬約 1040px 置中，大螢幕兩側氣氛底，表單不再拉滿全寬。
- **翻譯首屏**：完整度 → 資料夾 → 開始；步驟／完整度／精簡日誌改右側執行面板（高度上限）；「更多選項」改右側抽屜，禁止主欄無限往下長。
- **頂欄精簡**：品牌｜三服務｜⋯ 選單（主題／縮放／更新／說明／版本）。
- **視覺系統**：克制銅橙強調色、Outfit＋Noto Sans TC、統一三服務密度；新氣氛／空狀態資產於 `src/assets/`。
- 版本同步：`package.json`／`Cargo.toml`／`tauri.conf.json`／UI／Worker `LATEST_VERSION` → **0.2.0**。

## 0.1.6 — 2026-08-14（台北）— 精簡日誌＋更多選項延遲顯示

- **右欄日誌精簡**：只顯示最近 6 則狀態／進度／錯誤；完整內容改由「開啟報告」（覆蓋範圍說明／錯誤日誌／工作階段）。
- **更多選項延遲顯示**：未選遊戲資料夾時整塊隱藏；選完後才出現摺疊列（預設仍收合）。
- **進階布局壓短**：結果／版本兩欄、模式／品質並排、AI 驗證與金鑰改巢狀摺疊、間距縮小；展開時左欄內部捲動，不撐高視窗殼。
- 版本同步：`package.json`／`Cargo.toml`／`tauri.conf.json`／UI／Worker `LATEST_VERSION` → **0.1.6**。

## 0.1.5 — 2026-08-14（台北）— 更多選項版面＋小視窗可捲

- **更多選項分組**：結果位置／資源包與版本／翻譯方式／AI／參考翻譯；「刪除結果」改獨立次要列，不再擠在長路徑旁。
- **AI 驗證區壓縮**：Discord／Cloudflare 狀態與按鈕列加高密度，保留完整驗證流程。
- **小視窗可捲**：左主舞台 `overflow-y: auto`，右欄日誌自捲；殼層 flex／grid + `min-height: 0`，約 700×900 仍可捲完欄位與頁尾。
- 收合更多選項時首屏仍對齊 0.1.4 mockup（三卡＋資料夾＋開始＋右欄步驟／完整度／日誌）。
- 版本同步：`package.json`／`Cargo.toml`／`tauri.conf.json`／UI／Worker `LATEST_VERSION` → **0.1.5**。

## 0.1.4 — 2026-08-14（台北）— 依當初 mockup 對齊翻譯首屏

- **翻譯首屏對齊 mockup**：頂欄品牌＋版本＋深色切換；左主區標題／副標、三張完整度大卡（像素風圖示＋估計覆蓋區間＋橘框勾選）、遊戲資料夾路徑、底部大橘鈕「開始翻譯」；右欄步驟追蹤（準備→掃描→翻譯→完成）、整體完整度進度條、「已翻譯 x / y」、日誌與開啟輸出／報告。
- **移除首屏干擾**：拿掉左軌服務 dock、巨大 hero 拼貼、RUN／HITS 工程風 console；字體／診斷改頂欄精簡 tab，進階選項維持摺疊。
- **誠實文案**：覆蓋率區間標為說明性估計，不保證達標。
- 版本同步：`package.json`／`Cargo.toml`／`tauri.conf.json`／UI／Worker `LATEST_VERSION` → **0.1.4**。

## 0.1.3 — 2026-08-14（台北）— 工房殼結構重排＋完整度旗標收斂

- **殼結構重排**：左側服務軌（譯／體／診）取代文件式橫向分頁；完整度三卡為主舞台；路徑＋開始翻譯收進 launch bench；右欄整合為執行台（RUN／HITS）。
- **完整度誠實化**：刪除未接線的 `text_overlay_deep`；Max 档 `write_gap_summary` 真正寫出 `待補缺口摘要.txt`（樣本鍵，非五層盤點）。`#coverage-ack-hard` 綁定 localStorage（不擋開始）。
- **文案**：Max／說明改為「來源同標準＋較仔細品質＋缺口摘要」，不宣稱近 100% 或未做的 TextInventory／GapReport／FormatShield 產品化。
- 版本同步：`package.json`／`Cargo.toml`／`tauri.conf.json`／UI／Worker `LATEST_VERSION` → **0.1.3**。

## 0.1.2 — 2026-08-14（台北）— 深色工房殼與完整度三卡

- **殼與流程**：深色 ZeitFrei workshop 首屏（品牌、完整度三卡、路徑＋主 CTA）；進階選項摺疊；拿掉 Markdown `#`／`>` 文件裝飾。
- **右欄隨服務切換**：翻譯＝進度＋命中；字體＝建包指引；診斷＝分析摘要。
- **CFPA 可選下載**：參考翻譯可一鍵嘗試下載對應 MC 版本 CFPA release zip（失敗可略過、改本機選檔）；不上傳共享 R2。
- **字體預設**：清晰／緊湊／大字＋localStorage 記住設定。
- **skeleton／fade-in**：啟動與分頁切換保留漸進顯示。
- 版本三處同步為 0.1.2。

## 0.1.1 — 2026-08-14（台北）— 重新公開發佈

- **公開樹重置**：以目前本機可發佈工作區重新建立遠端 main 歷史，清除舊 Releases 與 release tags 後重新發佈。
- **授權確認**：根目錄 LICENSE 使用 PolyForm Noncommercial License 1.0.0，並以 NOTICE.md 標明第三方素材與非商業限制。
- **版本同步**：package.json、src-tauri/Cargo.toml、src-tauri/tauri.conf.json 與 UI 顯示同步為 0.1.1。
- **更新通道修正**：Worker 最新版資訊對齊 0.1.1，避免誤提示從 0.1.1 升級到舊產品線 1.0.2。

## 本回合整理（已納入 0.1.1）

- **開發規格 LOCALIZE-202608**：新增 `docs/LOCALIZE-202608.md`（目標最快且盡完整本地化；缺漏／全球開源可優化技術／速度與完整度 backlog／W0–W6 波次）。`DEVELOPMENT.md`、`COMMUNITY.md`、`EXTENDING.md`、`SEARCH-MAP.md` 已互鏈。
- UI 改成依流程狀態顯示：未選實例時收起輸出與 AI，翻譯中收起複查／分享，完成後才開放分享。
- 新增 CTE2／繁中參考資料自動搜尋與手動選取，支援不同電腦與不同磁碟路徑。
- 分享檔上傳前增加兩項人工確認；R2 分享頁先顯示可嵌入介紹頁，實際 ZIP 需要明確下載，並保留 cloud.zeitfrei.uk 遊戲下載與工具箱連結。
- 共享翻譯記憶加入正規化原文、語境與衝突標記；完全相同的內容可重用，衝突內容不自動覆蓋。
- AI 改為所有翻譯流程的選用功能；未勾選時仍會使用參考包、術語表與翻譯記憶，無法離線補上的內容保留原文。翻譯進行中只鎖定翻譯選項，仍可調整「關閉視窗時縮到背景」。
- 結果位置改名為「翻譯結果位置」：可維持自動位置，也可在翻譯前指定資料夾；勾選但留白會回到自動位置。移除多餘的「實際套用位置」區塊。
- 套用備份改放在 `翻譯結果/` 內，新增「刪除結果」可完整清理結果與備份；未啟用 AI 時，輸出的覆蓋說明不再寫入 AI 相關文字。
- 自訂 API 選取後直接顯示設定欄位；資源包名稱固定使用目前偵測到的整合包名稱與資源包版本，翻譯中不可修改。
- **錯誤分析改為證據判讀**：讀取 crash report 開頭與結尾，並合併最新 crash report、latest.log、debug.log 與 hs_err_pid 記錄。
- **增加錯誤分類**：補上 Java／JVM／記憶體／顯示環境、Mixin／模組載入、資料檔、註冊表與退出碼判讀；未知時不再直接把翻譯列為可能原因。
- **分析結果更完整**：顯示證據強度、最接近錯誤、可疑模組、遊戲退出碼、證據來源與可執行的下一步。
- **LOCALIZE-202608 完整度實作**：新增 `<item:…>`／`#mod:tag` 等格式護盾與 TM 寫入守衛；新增 Append／Skip-if-complete／Force、fast／balanced／thorough 品質選擇。
- **擴充翻譯來源**：支援 ZIP datapack／resourcepack 文字安全重建、KubeJS `Text.of`／`Component.literal`／`text.literal` 顯示字串、JAR 內 `data/*/patchouli_books`、GuideME／自訂 Markdown 可讀行；所有來源仍不改原始 JAR／ZIP。
- **完整翻譯優先**：覆寫、Origins、任務／書本與 KubeJS 來源超過 8,000 條時會自動分批，不再把後半段留到下一次；設定／手冊／ZIP／JAR 顯示型 `.properties` 也會嘗試翻譯。
- **JAR 顯示文字複查**：新增 `engine/jar_display.rs`，會把 JAR 內可辨識的 JSON／Markdown／properties 玩家文字接入翻譯流程，建立 `jar-translated` 副本；class 只留線索，不改程式碼。
- **JAR 路徑與重名修正**：同名 JAR 會依完整路徑分開暫存，並保留先前語言檔翻譯副本，避免顯示文字複查把前一步結果覆蓋回原文。
- **硬碟占用整理**：放寬文字掃描檔案與深度上限；翻譯中斷時清理 ZIP／JAR 暫存，掃描快取會移除不存在路徑並限制約 8 MiB。一般掃描不需要 GPU。
- **覆蓋報告加強**：`覆蓋範圍說明.txt` 新增本次來源明細與略過／錯誤原因；輸出區分 `resourcepacks-extra`、`data` 與工作檔，方便判斷是哪一類文字沒有翻到。

格式：版本 — 日期（台北）— 摘要。

## 未發布修正

- **離線完整度加速**：未勾 AI 時，FTB Quests、文字覆寫、ZIP 文字、Origins、任務／書本與 KubeJS 顯示字串會最多 3 路並行整理；勾 AI 時維持序列，避免多來源同時打翻譯服務。各來源錯誤會進 UI 與 `翻譯錯誤日誌.txt`，不會 panic 中斷。
- **字體包可直接套用**：字體資源包建立後可勾選套用到目前整合包 `resourcepacks`；若已有同名資源包會先建立 `字體套用備份_*`。
- **右欄命中儀表**：進度欄新增 glossary／TM／共享庫／AI／略過／待補摘要，資料來自進度、日誌與完成摘要；完整細節仍寫入 `覆蓋範圍說明.txt`。
- **參考翻譯加強**：手動參考包支援資料夾與 zip；自動搜尋納入 CFPA、zh_cn／zh_tw、漢化／翻譯等常見命名。參考包只填缺，`zh_cn` 會作為弱來源並在合併後轉台灣用語，不上傳到共享 R2。
- **0.1.1 三服務優化**：完整度授權（先翻能玩的／標準／盡量完整）、分享 Turnstile 改讀 `ai_status`、字體包 pack_format／拒 TTC／副檔名修正、診斷舊 crash 時間窗與翻譯證據提前、還原 `mc_dir` 校驗與失敗可見、覆蓋報告命中統計、深色工房 UI（完整度三卡、進階摺疊）。
- **分享 Turnstile**：分享上傳改讀 `ai_status.turnstileVerified`，不再呼叫未註冊的 `turnstile_status` command。
- **AGENTS 對齊**：版本敘述改以三處版本檔為準；雲端導航含分享／共享 TM；更新器敘述對齊免安裝 EXE 驗證後替換。
- **開發文件整理**：重寫 `docs/DEVELOPMENT.md` 作為目前技術總覽，新增 `docs/AI-HANDOFF.md` 給其他 AI 使用，補上 FTB 任務輔助模組、Cloudflare 資料隔離、路徑規則、UI 狀態、驗證命令與已知限制；README 文件地圖與架構圖同步更新。
- **FTB 任務補充改成選用流程**：只有偵測到 FTB Quests 且版本／載入器相容時才顯示；可一鍵準備相容的任務匯出模組、照畫面指令匯出後重新翻譯。工具自己下載的輔助模組會在翻譯完成後自動清理，使用者原本安裝的模組不會被刪除；不相容或下載失敗會直接跳過，不阻擋主流程。
- **自訂 API 服務商預設**：自訂 API 改為服務商預設接入；目前內建 DeepSeek、智譜 GLM、OpenAI 與通義千問，使用者只需選服務商並填 API Key，不必輸入 Base URL。Key 在介面固定以 `#` 顯示；未內建的 OpenAI 相容服務仍可手動填 Base URL／模型，並修正 GLM 的 `/chat/completions` 端點路徑。
- **修復代管 AI 驗證狀態不同步**：工具現在會讀取 Worker `/health` 判斷 Turnstile 是否強制；已啟用時，AI 狀態會要求本機短效憑證，翻譯引擎也不再用空白憑證送出請求，避免翻譯到一半反覆收到 HTTP 428。
- **重複翻譯不重複備份**：再次套用同一個整合包時，工具會檢查實例路徑、套用目標與既有備份清單；備份完整就沿用原備份，只有發現新的尚未備份目標時才建立新的備份。日誌與 `ApplyResult` 會分別顯示「沿用既有備份」或「新建備份」。

## 1.0.2 — 2026-08-10（台北）

- **本機 UI 精簡**：路徑資訊改成更緊湊的雙欄排列，減少重複說明；高解析度螢幕提高文字與控制項尺寸。
- **免安裝更新**：停用 NSIS 安裝包，只建置可攜式 EXE。更新器會驗證 SHA-256，等待目前工具關閉後替換同一路徑並重新開啟。

- **資源包版本獨立處理**：資源包名稱改為 `模組包翻譯工具+月日+整合包版本`，版本來自 CurseForge／Modrinth 等 metadata；找不到時使用 `R1`，不再把工具版本當成資源包版本。每個實例有獨立工作區，可反覆複查。
- **JAR 文件複查**：新增 `engine/jar_docs.rs`，只讀抽取 JAR 內可讀文字文件與 class 文字線索；不執行、不修改 JAR。這能提高搜尋範圍，但寫死在程式、圖片與執行時產生的文字仍可能無法翻譯。
- **JAR 語言檔翻譯副本**：翻譯流程會掃描模組 JAR 的 `assets/<模組>/lang/*.json`／`.lang`，建立 `jar-translated/` 完整副本；套用前備份同名 `mods` 檔再替換。含簽章 JAR 會略過並寫入錯誤日誌，避免產生失效簽章。
- **翻譯完成直接套用**：後端在建立資源包後先備份，再直接覆蓋正確 Minecraft 資料夾，並更新 `options.txt` 啟用資源包；不再需要額外按「套用到遊戲」。
- **錯誤分析獨立頁**：可貼上錯誤碼、crash report 或讀取實例最近記錄，加入 `errorCode`、主要錯誤與線索；會特別辨識 `unbound value`，避免把模組註冊問題誤判為翻譯問題。
- **分享檔隔離**：分享 ZIP 改用可安裝檔 allowlist，透過 `SHARES → modpack-i18n-shares` 獨立 R2 bucket 上傳，短連結 24 小時後失效；不與安裝檔／翻譯記憶資料混用。
- **新手 UI／文件**：移除輸出選擇與手動套用的多餘步驟，新增錯誤分析頁、資源包版本提示、JAR 複查說明與 24 小時分享說明。
- **手翻同路徑・多根搜尋**：`jar_scan` 鬆散 lang 擴到 defaultconfigs／datapacks／global_packs／paxi／data，config 加深以涵蓋 openloader 巢狀；`text_overlay` 多根掃資料包，路徑提示＋內容嗅探收入 `loc_name`／`effect_tip` 等，gameplay JSON 只翻顯示欄位白名單（不碰 id）；Origins／任務書同源加 openloader／global_packs。規格見 `docs/SEARCH-MAP.md`。
- **直接覆蓋安裝、不建資料夾**：選到有效遊戲實例時，翻譯中繼檔預設放工具自管的暫存區（`%APPDATA%\modpack-i18n-tool\work`，`managed_output_base` 命令），不再在你的資料夾另建「翻譯結果」；「套用到遊戲」直接覆蓋安裝（先備份、可一鍵還原）。想要看得到的輸出仍可按「使用啟動器建議的位置」。
- **打包成分享檔（新增 `engine/share_pack.rs`）**：勾「也建立可分享的打包檔」或按「打包成分享檔」，把整包翻譯結果壓成單一 zip 供手動分享（`create_share_package` 命令，附 3 個單元測試）。**上傳雲端拿分享短連結為後續版本**（自建在本專案 Worker）。
- **Turnstile 依服務端設定決定**：只有 Worker 設好 `TURNSTILE_*` 金鑰並啟用強制模式時才要求安全驗證；未啟用時保留「Discord 登入即可」的相容行為。
- **支持開發按鈕**：移到主畫面操作區的醒目位置，並可重複出現（操作區＋頁尾）。
- **詳細使用說明**：新增 `docs/詳細使用說明.md`（實例是什麼、各啟動器資料夾在哪、該選哪個、如何安裝與分享）。
- 版本 0.1.1；發佈前驗證以本回合命令輸出為準。

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
  **隱藏、預設開、零設定**。翻譯時先查社群共享庫，命中就免送 AI；通過格式檢查的譯文匿名回饋。
  - 新增共享術語表（`engine/shared_glossary.rs` + Worker `/glossary/lookup`、`/glossary/contribute`），依整合包名稱分類；相同譯文去重，不同譯文標記衝突。
  - TM 與術語資料改放獨立 Cloudflare R2 `TRANSLATIONS`，不與更新檔或一日分享檔混用。
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
# 本回合整理（尚未發布版本）

- 備份由玩家決定：翻譯、複查、修復與手動套用都會讀取「套用前建立備份」；未勾選時不建立工具備份。錯誤分析頁新增「刪除全部備份」，只刪除本工具建立的備份目錄。