# 模組包最快且完整本地化 — 開發規格 202608

> **版本代號**：`202608`（2026-08）  
> **產品目標**：在**不改原始 `mods/*.jar`** 前提下，讓任意 Java 整合包**盡快**、**盡完整**變成可玩的**台灣繁體（zh_tw）**體驗。  
> **地位**：本檔是 2026-08 起「完整度／速度」的開發真相源；實作與驗收以本檔優先序為準。  
> **相關**：搜尋路徑見 `SEARCH-MAP.md`；管線總覽見 `DEVELOPMENT.md`；社群期望見 `COMMUNITY.md`；擴充範本見 `EXTENDING.md`。

---

## 0. 一句話與兩條硬約束

| 項目 | 內容 |
|------|------|
| 一句話 | **本機多根搜尋 → 社群／既有譯文優先 → 格式護盾下的 AI 只補洞 → 一鍵套用（可備份）** |
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
| L6 腳本硬字串 | KubeJS `.js` 字面、CraftTweaker | **列缺漏**（P1） |
| L7 zip 內資料包 | 未解壓的 datapack zip 內部 | **列缺漏**（P1） |
| L8 圖內字／硬編碼 | 貼圖文字、Java 字串 | **不做**（文件標明） |

**完整度 KPI（人工可驗）**

1. 標準 FTB + lang 包：套用後主介面／任務書無大片英文（允許專有名詞）。  
2. CTE 類（openloader + Mine and Slash 系）：技能／詞綴 `loc_name` 有進 work 與套用路徑。  
3. 覆蓋報告列出「掃到／寫出／未支援」三欄，玩家看得懂。

### 1.2 速度（Speed）

| 指標 | 目標 | 手段方向 |
|------|------|----------|
| 零 AI 路徑 | 大包數分鐘級（SSD） | 平行掃 jar、只讀 lang、OpenCC 批次 |
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
| 格式護盾進階（AE 宏、item tag 遮罩表） | **不足** | `placeholder` 已有 %s 等 | 見 §4.1 |
| 雙層快取（機翻／AI 分檔） | **不足** | 僅 `tm.json` | 見 §4.2 |
| Append／Skip90／Force 三模式 | **不足** | 補翻近似 Append | 見 §4.3 |
| 介面 vs 劇情分品質引擎 | **不足** | 單一 AI 路徑 | 見 §4.4 |
| FTB → lang 匯出橋接 | **不足** | 無 | 見 §3 P1 |
| KubeJS 腳本字面字串 | **缺** | — | §3 P1 |
| zip datapack 內文 | **缺** | 只掃鬆散 | §3 P1 |
| GuideME Markdown | **缺** | — | §3 P2 |
| 共享 TM 社群端 | 部分 | `shared_tm` + Worker | 視服務是否上線 |
| Provider 鏈（多後備） | 部分 | 單一 base/model | 見 §4.5 |
| 掃描／翻譯進度分離的「預估剩餘」 | **不足** | 有 % 無 ETA | §5 |

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
| M1.1 | **KubeJS／腳本硬字串** | 腳本 UI／提示英文 | 新模組：僅字面字串 + 極嚴白名單路徑；預設關或「進階」 | 測試 fixture 1 檔 |
| M1.2 | **zip 內 datapack／RP** | 打包資料包未翻 | 安全解壓上限 + 與鬆散相同規則 | 測小 zip |
| M1.3 | **FTB lang 匯出相容** | 與 FTB Quest Localizer 生態互通 | 可選：snbt→kubejs lang 或讀既有 FTBLang | 文件說明 + 一條整合測 |
| M1.4 | **Heracles／Modonomicon 邊角路徑** | 任務漏 | 擴 `is_quest_book_path` 片段 | grep 新包路徑 |
| M1.5 | **Patchouli 在 jar 內書** | 僅掃實例覆寫時漏 | jar 掃時可選抽出 patchouli 頁（或 RP 合併） | 文件標支援範圍 |
| M1.6 | **覆蓋報告不夠「完整」** | 玩家不知漏哪 | report：依來源計數 + 未支援原因 | 報告檔欄位齊 |
| M1.7 | **格式自癒寫回 TM** | 壞譯文污染記憶 | 寫 TM 前必須 guard；壞的不入庫 | 測試拒寫 |

### P2 — 完整度長尾／體驗

| ID | 缺漏 | 建議 |
|----|------|------|
| M2.1 | GuideME／Markdown 手冊 | 新 B 類來源或標未支援 |
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

### 4.2 雙快取與自癒 — **應優化**

| 層 | 用途 | 現況 | 目標 |
|----|------|------|------|
| TM 機翻級 | Google／快模型 | 單一 `tm.json` | 可選分檔 `tm_mt.json`／`tm_ai.json` 或 namespace 前綴 |
| 自癒 | 修復 `% s`、括號 | 部分在 guard 修復鏈 | 讀 TM 時跑 heal，壞條目標記不採用 |
| 詞典 | 強制譯名 | `glossary.json` | 保持；文件強調「先 glossary 再 AI」 |

### 4.3 翻譯模式：Append／Skip／Force — **應優化**

| 模式 | 行為 | 對應玩家情境 |
|------|------|----------------|
| **Append**（預設） | 只翻缺 key／缺字串；已有 zh 不動 | 模組包小更新 |
| **Skip-if-complete** | 某 ns 或某來源 ≥ 閾值（如 90%）整段跳過 | 大包已大半漢化 |
| **Force** | 忽略既有機翻重翻（仍保留 glossary） | 品質升級 |

實作落點：session + `fill_missing_with_ai` 入口參數；UI 三選一。

### 4.4 分品質引擎（快／好）— **應優化**

| 軌道 | 來源 | 引擎建議 | 產出 |
|------|------|----------|------|
| 快 | lang 介面字、短 tooltip | 本機 TM + 可選快速模型／甚至 MT | 單一 RP 底 |
| 好 | FTB 描述、Patchouli、flavor | 高品質 AI、較小 batch | 可疊第二 RP 或同包後寫 |

MineAI 建議「兩包疊加」；我們可在**單一 work** 內用來源標籤選模型，避免玩家搞兩個 zip（預設單包，進階可拆）。

### 4.5 Provider 鏈與本地 LLM — **應優化**

| 項目 | 目標 |
|------|------|
| `PROVIDER_CHAIN` | 多 endpoint 失敗切換（對齊 Habier） |
| Ollama／本機 | 無網可翻劇情（可選） |
| 代管 Worker | 維持額度；不與自訂鏈衝突 |
| 不洩漏廠商名 | 既有 `sanitize_provider_name` 保持 |

### 4.6 來源目錄發現 — **已做一輪，可再優化**

已落地：多根 + 路徑提示 + 內容嗅探 + 顯示欄位（`SEARCH-MAP.md`）。  
再優化：

- 掃描結果快取（檔 mtime + size → 跳過未變檔）  
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
| AI 唯一字串上限 | 各模組 `MAX_AI_UNIQUE` |

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
| S8 | 進度 ETA（依批耗時） | 體感 | P2 |
| S9 | 磁碟在 Y:/Z: 時提示改本機 work | 避網路碟抖動 | P2 |
| S10 | jar 掃描 worker 數可設定 | 弱機不炸 | P2 |

---

## 6. 完整度優化清單（Coverage backlog）

| ID | 項目 | 優先 | 對應缺漏 |
|----|------|------|----------|
| C1 | 維持並擴充 SEARCH-MAP 多根／白名單 | P0 | M0.1–0.2 |
| C2 | format_shield 全管線 | P0 | M0.3、§4.1 |
| C3 | KubeJS 字面（進階開關） | P1 | M1.1 |
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
| **W0 基線** | 文件落地（本檔 + DEVELOPMENT 索引）；SEARCH-MAP 與實碼一致 | 文件互鏈；cargo check 綠 |
| **W1 護盾** | format_shield + TM 寫入守衛 + 測試 | 壞 `%s`／`<item:>` 不進 TM |
| **W2 模式** | Append／Force（Skip 可跟）UI + session | 重跑 Append 不重翻已有 |
| **W3 加速** | 掃描快取 + AI 動態 batch + 來源並行 | 二次掃描明顯加快 |
| **W4 來源** | zip datapack 或 KubeJS 進階（擇一先做需求高者） | 支援範圍文件更新 |
| **W5 報告** | 覆蓋報告依來源 + 未支援列表 | 玩家可指出漏翻類型 |
| **W6 品質分軌** | 介面快／劇情好（可選模型） | 文件與 UI 說明 |

狀態欄（維護時改）：

| 波次 | 狀態 | 日期 |
|------|------|------|
| W0 | **文件已開** | 2026-08-12 |
| W1–W6 | 未開始 | — |

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
