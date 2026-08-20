# 模組包最快且完整本地化 — 開發規格 202608

> **版本代號**：`202608`（2026-08）  
> **產品目標**：在**不改原始 `mods/*.jar`** 前提下，讓任意 Java 整合包**盡快**、**盡完整**變成可玩的**台灣繁體（zh_tw）**體驗。  
> **地位**：本檔是 2026-08 起「完整度／速度」的開發真相源；實作與驗收以本檔優先序為準。  
> **相關**：搜尋路徑見 `SEARCH-MAP.md`；管線總覽見 `DEVELOPMENT.md`；社群期望見 `COMMUNITY.md`；擴充範本見 `EXTENDING.md`。

---

## 0. 一句話與兩條硬約束

| 項目 | 內容 |
|------|------|
| 一句話 | **本機多根搜尋 → 社群／既有譯文優先 → 格式護盾下的 AI 分批補洞 → 一鍵套用（可備份）** |
| 硬約束 A | **永不直接改寫玩家 `mods/*.jar` 本體**；lang 走資源包／`jar-translated` 副本 |
| 硬約束 B | **不承諾 100% 螢幕無英文**；圖內字、Java 硬編碼、未知 schema 明示為未支援並可擴充 |

「完整」= 可遊玩文字來源覆蓋率高 + 不弄壞遊戲。  
「最快」= 少 AI 量、高命中快取、I/O 與並行可調、可中斷可續跑。

---

## 1. 成功定義（DoD · 202608）

### 1.1 完整度（Coverage）

| 等級 | 定義 | 202608 目標 |
|------|------|-------------|
| L0 物品／介面 lang | jar + 資源包 + 鬆散 `/lang/` | **必達**（已有） |
| L1 任務主線 | FTB SNBT + Better Questing／HQM／Heracles／Modonomicon | **必達**（已有） |
| L2 書本／手冊 | Patchouli、部分 openloader 書頁 | **必達**（已有，持續加深） |
| L3 資料包顯示字 | openloader／datapacks 的 `loc_*`／`effect_tip`／`flavor_text` 等 | **必達**（202608 多根 + 顯示欄位，見 SEARCH-MAP） |
| L4 能力／種族 | Origins／Apoli `name`／`description`（路徑感知） | **必達**（已有） |
| L5 選單／覆寫 | FancyMenu、MineMenu 編碼 | **應達** |
| L6 腳本硬字串 | KubeJS 顯示 API 的 `.js`／`.ts` 字面 | **已做（嚴格白名單；CraftTweaker 仍不自動改寫）** |
| L7 zip 內資料包 | 未解壓的 datapack／resourcepack zip 內部 | **已做（安全上限；原 ZIP 只讀）** |
| L8 圖內字／硬編碼 | 貼圖文字、Java 字串 | **不做**（文件標明） |

**完整度 KPI（人工可驗）**

1. 標準 FTB + lang 包：套用後主介面／任務書無大片英文（允許專有名詞）。  
2. CTE 類（openloader + Mine and Slash 系）：技能／詞綴 `loc_name` 有進 work 與套用路徑。  
3. 覆蓋報告列出「掃到／寫出／未支援」三欄，玩家看得懂。

### 1.2 速度（Speed）

| 指標 | 目標 | 手段方向 |
|------|------|----------|
| 零 AI 路徑 | 大包數分鐘級（SSD） | 平行掃 jar、只讀文字資源、OpenCC 批次 |
| 有 AI 路徑 | **少呼叫**：TM／術語／社群／既有 zh 優先 | `resolve_unique` 三層擋；Append 模式 |
| 重跑同一包 | AI 趨近 0 | 持久 TM、session 補翻、檔級 skip |
| 可中斷 | 停止後狀態可續 | cancel 點 + session pending |
| 感知不卡死 | UI 心跳／每批 % | 既有 progress 事件，禁止長段無 emit |

**速度 KPI**

1. 第二次同設定重跑：AI 批次數顯著下降（TM 命中）。  
2. 關閉 AI：仍產出可用繁中底（簡→台 + 既有 zh）。  
3. 掃描階段不呼叫任何翻譯 API。

---

## 2. 現況能力矩陣（對照 202608 目標）

| 能力 | 狀態 | 模組／位置 | 備註 |
|------|------|------------|------|
| jar 內 lang 平行掃描 | 已有 | `jar_scan` | 不改原 jar |
| 鬆散 lang 多根 | 已有 | kubejs／config／defaultconfigs／datapacks／global_packs／paxi／data | 深度加深含 openloader 巢狀 |
| 資源包輸出 + pack_format | 已有 | `pack_out` | 依實例版本 |
| OpenCC 等價 s2twp | 已有 | `convert`（純 Rust） | 不依賴本機 Python |
| 術語表／TM／佔位符守衛 | 已有 | `glossary`／`tm`／`placeholder` | 退回原文優於壞格式 |
| AI 批次補洞 | 已有 | `deepseek` | 代管額度／自訂 key |
| FTB Quests SNBT | 已有 | `ftbquests` | |
| Patchouli／FancyMenu／openloader 顯示字 | 已有 | `text_overlay` | 顯示欄位白名單 + 嗅探 |
| Origins | 已有 | `origins` | 排除機制節點 name |
| BQ／HQM／Heracles／Modonomicon | 已有 | `quests_books` | |
| 一鍵套用＋備份 | 已有 | `apply_instance` | |
| 診斷缺模組閃退 | 已有 | `diagnose` | 與翻譯無關的 Structory 等 |
| 格式護盾進階（AE 宏、item tag 遮罩表） | 已有 | `placeholder` + mask／unmask／guard | `$(...)`、`<item:…>`、`#mod:tag`、Markdown link |
| 雙層快取（機翻／AI 分檔） | **不足** | 僅 `tm.json` | 見 §4.2 |
| Append／Skip90／Force 三模式 | 已有 | `translation_mode` + session + UI | Force 忽略共享／本機 TM，仍保留 glossary／guard |
| 介面 vs 劇情分品質引擎 | 已有 | `translation_quality` + prompt／batch | fast／balanced／thorough；API 分流不做 |
| FTB → lang 匯出橋接 | 部分 | FTB SNBT 直接覆寫；保留原格式 | 不把任務資料硬轉成 gameplay lang |
| KubeJS 腳本字面字串 | 已有 | `script_literals` | 只改 `Text.of`／`Component.literal`／`text.literal` |
| zip datapack 內文 | 已有 | `archive_overlay` | 安全解壓→共用掃描→重建 ZIP |
| GuideME Markdown | 已有 | `text_overlay` Markdown | 只處理可讀行，連結／格式先遮罩 |
| 共享 TM 社群端 | 部分 | `shared_tm` + Worker | 視服務是否上線 |
| Provider 鏈（多後備） | 部分 | 單一 base/model | 見 §4.5 |
| 掃描／翻譯進度分離的「預估剩餘」 | 已有第一版 | 來源階段進度＋批次進度 | UI 依真實百分比與已用時間估算，早期／等待回應時不亂顯示 |

---

## 3. 缺漏清單（Missing · 依優先）

### P0 — 完整度／安全（必須維持或補洞）

| ID | 缺漏 | 影響 | 建議方向 | 驗收 |
|----|------|------|----------|------|
| M0.1 | 未知顯示鍵未進白名單 | 技能／物品仍英文 | 擴 `DISPLAY_FIELD_KEYS`／`loc_*`；更新 SEARCH-MAP | 抽樣 openloader 有 `loc_*` 檔寫進 work |
| M0.2 | 誤翻 gameplay id | 閃退或邏輯壞 | **禁止** gameplay JSON 全字串掃；維持 DisplayFieldsOnly | 單元測試：id 欄不進 unique |
| M0.3 | 佔位符被 AI 弄壞 | 崩潰／空白 UI | 強化 `placeholder::guard` + 遮罩表（§4.1） | 既有 + 新測：`%s`、`$(br)`、`<item:…>` |
| M0.4 | 套用弄丟可還原 | 玩家無法回復 | apply 必可備份／restore | 人工：還原後檔在 |

### P1 — 完整度明顯缺口（202608 主攻）

| ID | 缺漏 | 影響 | 建議方向 | 驗收 |
|----|------|------|----------|------|
| M1.1 | **KubeJS／腳本硬字串** | 腳本 UI／提示英文 | 已做：僅 `Text.of`／`Component.literal`／`text.literal`，不解析任意邏輯 | `script_literals` 單測 |
| M1.2 | **zip 內 datapack／RP** | 打包資料包未翻 | 已做：安全解壓上限 + 與鬆散相同規則 + 重建副本 | `archive_overlay` 單測 |
| M1.3 | **FTB lang 匯出相容** | 與 FTB Quest Localizer 生態互通 | 已做：保留 FTB SNBT 原格式直接覆寫，避免誤轉成 gameplay lang | FTB 流程整合 |
| M1.4 | **Heracles／Modonomicon 邊角路徑** | 任務漏 | 已擴充路徑片段並在補翻／修復流程重跑 | `quests_books` 路徑單測 |
| M1.5 | **Patchouli 在 jar 內書** | 僅掃實例覆寫時漏 | 已做：抽出 `data/*/patchouli_books` 成 work/data 覆寫，原 JAR 不改 | `jar_patchouli` 單測 |
| M1.6 | **覆蓋報告不夠「完整」** | 玩家不知漏哪 | **1.0.5**：三欄＝台灣可玩／港繁 hint／仍待譯；zh_hk 不算 covered | 報告檔欄位齊 |
| M1.7 | **格式自癒寫回 TM** | 壞譯文污染記憶 | 已做：`Tm::insert` 與 AI 回寫雙重 guard | 測試拒寫 |

### P2 — 完整度長尾／體驗

| ID | 缺漏 | 建議 |
|----|------|------|
| M2.1 | GuideME／Markdown 手冊 | 已做：白名單根目錄與 Markdown 可讀行 |
| M2.2 | MineColonies／自訂 GUI 字串 | 路徑提示表擴充 |
| M2.3 | 基岩版 | 不做，文件寫明 |
| M2.4 | 圖內 OCR | 不做 |
| M2.5 | 即時滑過翻譯（遊戲內模組） | 非本工具範圍；可文件連結第三方 |
| M2.6 | CFPA 自動拉取 | 可選「合併線上簡中包再 s2twp」（授權／網路要玩家同意） |

### P3 — 非翻譯但影響「裝完能玩」

| ID | 項目 | 說明 |
|----|------|------|
| M3.1 | 缺模組閃退 | `diagnose` 已涵蓋；與本地化分開提示 |
| M3.2 | 翻譯後資料包語法壞 | 只改顯示欄；機制路徑黑名單維持 |

---

## 4. 可優化技術（來自全球開源對照 · 202608）

對照專案（只作技術參考，**不**複製授權不清的程式）：

| 專案 | 可抄的想法 |
|------|------------|
| [Habier/minecraft-modpack-translator](https://github.com/Habier/minecraft-modpack-translator) | 來源表清晰、provider 鏈、Ollama 後備、啟動器多根發現、workspace／export 分離 |
| [MineAI-Modpack-Translator](https://github.com/Thedrezik/MineAI-Modpack-Translator) | Titanium Shield 遮罩、cache 自癒、dictionary、Append／Skip／Force、介面 vs 劇情分引擎 |
| [Y-RyuZU/MinecraftModsLocalizer](https://github.com/Y-RyuZU/MinecraftModsLocalizer) | 同 Tauri 架構、FTB 多目錄、中斷、批次 chunk |
| [alex-serbet/Minecraft-Mods-Localizer](https://github.com/alex-serbet/Minecraft-Mods-Localizer) | 多 provider GUI 體驗 |
| CFPA / I18nUpdateMod | 既有譯文資產優先於 AI；分發是資源包不是改 jar |
| GTNH-Translations | 整合包級 monorepo 流程（內容工程，非一鍵工具） |
| FTB Quest Localizer（模組） | snbt ↔ lang 橋接 |

### 4.1 格式護盾（Titanium Shield 級）— **應優化**

**現況**：`placeholder.rs` 管 `%s`／`{0}`／`$(br)` 等，回譯必 guard。  
**缺口**：未系統遮罩／還原：

- Patchouli／AE 風格：`$(#hex)`、`$(br)`、`$(l:…)`
- Item／tag：`<item:mod:id>`、`#mod:tag`
- Markdown／連結：`](http…)`、書本連結
- 顏色碼全形／半形混用

**目標設計**

1. 送 AI 前：`mask(s) -> (masked, tokens[])`  
2. AI 後：`unmask` → `placeholder::guard` → 失敗則退回原文或僅 OpenCC  
3. 單元測試 fixture 來自真實 AE／Patchouli 句  

**檔案**：擴 `placeholder.rs` 或新 `engine/format_shield.rs`；`deepseek`／`text_overlay` 共用。

### 4.2 雙快取與自癒 — **已部分優化**

| 層 | 用途 | 現況 | 目標 |
|----|------|------|------|
| TM 機翻級 | Google／快模型 | 單一 `tm.json` | 有上下文提示時以原文＋上下文隔離；舊無上下文條目仍相容 |
| 自癒 | 修復 `% s`、括號 | `guard` + `Tm::insert` 雙重守門 | 壞條目不採用、不寫回，避免污染後續整合包 |
| 詞典 | 強制譯名 | `glossary.json` | 保持；文件強調「先 glossary 再 AI」 |

### 4.3 翻譯模式：Append／Skip／Force — **已完成**

| 模式 | 行為 | 對應玩家情境 |
|------|------|----------------|
| **Append**（預設） | 只翻缺 key／缺字串；已有 zh 不動 | 模組包小更新 |
| **Skip-if-complete** | 某 ns 或某來源 ≥ 閾值（如 90%）整段跳過 | 大包已大半漢化 |
| **Force** | 忽略既有機翻重翻（仍保留 glossary） | 品質升級 |

實作落點：`translation_mode`、session、`fill_missing_with_mode` 入口參數；UI 三選一。Force 只對目前待補字串生效，不會重翻已有繁中 key。

### 4.4 分品質引擎（快／好）— **已完成第一版**

| 軌道 | 來源 | 引擎建議 | 產出 |
|------|------|----------|------|
| 快 | lang 介面字、短 tooltip | 本機 TM + 可選快速模型／甚至 MT | 單一 RP 底 |
| 好 | FTB 描述、Patchouli、flavor | 高品質 AI、較小 batch | 可疊第二 RP 或同包後寫 |

本工具在單一 work 內以 `translation_quality` 調整 AI 批次大小與提示：fast 180、balanced 140、thorough 70。API 多後備分流不在本次範圍內，三種品質仍使用使用者選定的同一個 API。

### 4.5 Provider 鏈與本地 LLM — **應優化**

| 項目 | 目標 |
|------|------|
| `PROVIDER_CHAIN` | 多 endpoint 失敗切換（對齊 Habier） |
| Ollama／本機 | 無網可翻劇情（可選） |
| 代管 Worker | 維持額度；不與自訂鏈衝突 |
| 不洩漏廠商名 | 既有 `sanitize_provider_name` 保持 |

### 4.6 來源目錄發現 — **已做一輪，可再優化**

已落地：多根 + 路徑提示 + 內容嗅探 + 顯示欄位（`SEARCH-MAP.md`）。  
已落地第一版：鬆散語言檔使用檔案大小＋修改時間快取；第二次掃描可跳過未變的語言檔。
資源包資料夾與 JAR／ZIP 內部仍會重新驗證，避免壓縮內容或外部檔案變更造成舊結果誤用。

保留後續：

- 嗅探並行化、機制路徑黑名單可持續加  
- 可選「僅掃描變更」給補翻

### 4.7 匯出／工作區分離 — **應優化**

Habier：`workspace/`（快取）vs `export/`（給玩家）。  
我們：`翻譯結果`／managed work 已有；應在文件與 UI 分清：

- **給玩家**：RP zip、可套用覆寫、覆蓋報告、分享包  
- **內部**：session、TM、錯誤日誌、中間 raw  

避免玩家把內部快取當漢化包傳出去。

### 4.8 安全解壓與資源上限 — **維持並寫死**

| 規則 | 值（可調但需記錄） |
|------|-------------------|
| 單檔讀取上限 | 見各模組 `MAX_FILE_BYTES` |
| zip 條目安全名 | `security::is_safe_zip_entry_name` |
| 步行深度 | SEARCH-MAP 表 |
| AI 唯一字串上限 | API 內層依品質分批；覆寫／Origins／任務來源外層每 8,000 條一批，會自動續處理 |

新增 zip datapack 支援時**必須**沿用安全解壓，禁止 zip-slip。

---

## 5. 速度優化清單（Speed backlog）

| ID | 項目 | 預期收益 | 優先 |
|----|------|----------|------|
| S1 | 掃描 mtime 快取 | 重跑秒開 | P1 |
| S2 | TM／glossary 命中率儀表 | 玩家知為何變快 | P2 |
| S3 | AI 動態 batch（短句併、長句拆） | 降延遲與失敗 | P1 |
| S4 | 來源級並行（ftb／overlay／origins 可限並行） | 縮 wall time | P1 |
| S5 | 關閉 AI 時跳過一切網路 | 已大致如此；回歸測 | P0 |
| S6 | 共享 TM 先查再 AI | 降成本 | P1（服務在線時） |
| S7 | Skip-if-complete | 大包更新極快 | P1 |
| S8 | 進度 ETA（依目前真實百分比） | 體感 | 已完成第一版 |
| S9 | 磁碟在 Y:/Z: 或 UNC 時提示風險 | 避網路碟抖動 | 已完成提示 |
| S10 | jar 掃描 worker 數可設定 | 弱機不炸 | 已完成；可用 `MODPACK_I18N_JAR_WORKERS=1..16` |

---

## 6. 完整度優化清單（Coverage backlog）

| ID | 項目 | 優先 | 對應缺漏 |
|----|------|------|----------|
| C1 | 維持並擴充 SEARCH-MAP 多根／白名單 | P0 | M0.1–0.2 |
| C2 | format_shield 全管線 | P0 | M0.3、§4.1 |
| C3 | KubeJS 字面（安全白名單） | P1 | M1.1 |
| C4 | zip datapack | P1 | M1.2 |
| C5 | FTB lang 橋 | P1 | M1.3 |
| C6 | 覆蓋報告 3.0 | P1 | M1.6 |
| C7 | jar 內 Patchouli | P1 | M1.5 |
| C8 | CFPA 可選合併 + s2twp | P2 | M2.6 |
| C9 | 分品質引擎 | P1 | §4.4 |
| C10 | Append／Force UI | P1 | §4.3 |

---

## 7. 建議實作順序（202608 迭代）

> 原則：**先不破壞 → 再加快既有 → 再擴來源**。每一項合併前：`npm run check` + `npm run test`，並更新本檔狀態欄。

| 波次 | 內容 | 完成判準 |
|------|------|----------|
| **W0 基線** | 文件落地（本檔 + DEVELOPMENT 索引）；SEARCH-MAP 與實碼一致 | 已完成 |
| **W1 護盾** | format_shield + TM 寫入守衛 + 測試 | 已完成 |
| **W2 模式** | Append／Force／Skip90 UI + session | 已完成 |
| **W3 加速** | AI 動態 batch + JAR 並行；鬆散 lang 掃描快取 | 已完成第一版 |
| **W4 來源** | ZIP datapack／RP、KubeJS 顯示字串、JAR Patchouli、Markdown | 已完成第一版 |
| **W5 報告** | 覆蓋報告依來源 + 未支援列表 | 已完成 |
| **W6 品質分軌** | 介面快／劇情好（同一 API，不做分流） | 已完成第一版 |

狀態欄（維護時改）：

| 波次 | 狀態 | 日期 |
|------|------|------|
| W0–W2 | **已完成** | 2026-08-12 |
| W3 | **已完成第一版：鬆散 lang 掃描快取；壓縮來源仍做安全重掃** | 2026-08-12 |
| W4–W6 | **已完成第一版** | 2026-08-12 |

---

## 8. 管線目標形狀（邏輯，非必須一次改碼）

```
[實例路徑]
  → resolve_minecraft_dir
  → 多根掃描（lang / snbt / overlay / origins / quests）  … 無 AI，可快取
  → 合併既有 zh_tw／zh_cn／參考包／共享 TM／glossary
  → OpenCC s2twp
  → 缺洞：mask → AI/MT（依來源品質檔）→ unmask → guard → TM
  → 產出：RP zip + 覆寫樹 + jar-translated + 報告
  → 可選 apply（備份）
```

與「手翻」對齊：同一套搜尋地圖（SEARCH-MAP），不同模組不同資料夾，**不**假設單一樹。

---

## 9. 驗收腳本（人工 · 完整度＋速度）

### 9.1 速度

1. 關 AI，掃中型包：記時；日誌無任何 chat/completions。  
2. 開 AI 跑完一次；同設定再跑：AI 批／token 下降。  
3. 翻譯中按停止：可停且 work 不腐壞。

### 9.2 完整度

1. 僅 lang 包：RP 裝上後物品名繁中。  
2. FTB 包：任務 title／description 進 work 且套用後可見。  
3. openloader 含 `loc_name` 的 JSON：work 內對應欄為繁中或已送翻佇列。  
4. 故意壞譯文 `% s`：不得寫入 TM、不得進最終 RP。  
5. 缺 Structory 類模組：診斷指向缺 jar，不誣賴翻譯。

### 9.3 回歸命令

```bash
cd modpack-i18n-tool
npm run check
npm run test
```

---

## W7 — 覆蓋判斷與 zh_hk（產品 1.0.5）

| 規則 | 說明 |
|------|------|
| `zh_tw` | 原生台繁；計入台灣可玩；**不再**整包 s2twp |
| `zh_cn` | 合併時 s2twp → `CnConverted`；計入可玩 |
| `zh_hk` | 僅補缺 + s2twp → `HkHint`；**不**計 covered／skip |
| skip-if-complete | 只算 playable 來源 |
| 覆蓋報告 | 台灣可玩／港繁提示／仍待譯 三欄 |
| 共享代管額度 | **每週** 1000 萬；個人仍每日 |

驗收：僅 zh_hk 的 ns 不得觸發 skip；報告有 hk_hint 說明。

---

## 10. 明確不做（防範圍膨脹）

1. 改玩家原始 `mods/*.jar` 當預設。  
2. OCR 圖片字。  
3. 保證零英文／官方級譯文。  
4. 用 AI 做檔案分類或全碟掃描。  
5. 基岩版、服務端專用協議漢化。  
6. 替玩家修「缺模組／世界gen unbound」類整合包損壞（只診斷）。

---

## 11. 文件與版本維護

| 動作 | 檔案 |
|------|------|
| 改搜尋根／白名單 | 先改碼 → 同步 `SEARCH-MAP.md` → 本檔 §2 狀態 |
| 完成波次 | 本檔 §7 狀態欄 + `CHANGELOG.md` |
| 支援範圍變更 | `支援範圍與免責聲明.md` |
| 對玩家一句話 | `USER-GUIDE.md`／快速說明可鏈「完整度與速度目標見開發文件 LOCALIZE-202608」 |

**版本字串**：規格代號 `202608`；產品 semver 仍走 Cargo／tauri／package（見 DEVELOPMENT §9）。

---

## 12. 對開發者的優先提示

若只能做一件事讓「又快又完整」立刻變好：

1. **完整**：守住顯示欄位白名單 + 擴真正漏的 `loc_*` 來源（已做多根，持續加鍵／路徑）。  
2. **快**：TM／glossary／Append + 掃描快取，讓第二次跑幾乎不燒 AI。  
3. **穩**：format_shield 與 guard 寫 TM 前必過。

下一個 session 開工請先讀：本檔 §7 波次表 + `SEARCH-MAP.md` + `DEVELOPMENT.md` §4 管線。
