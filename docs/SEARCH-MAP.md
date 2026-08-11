# 搜尋地圖（手翻同路徑・多根・不綁死單一模組）

工具「找文字」的方式對齊手翻習慣：**不假設整包只有一種資料夾結構**。  
不同模組／包作者會把可譯文字塞在不同根目錄；工具用**多根掃描 + 路徑提示 + 內容嗅探 + 顯示欄位白名單**處理。

> 硬不變式：不直接寫入原始 `mods/*.jar`；先在 work 建立完整翻譯副本，再依玩家選項決定是否備份同名檔後替換到實例。  
> **完整度／速度總規格（202608）**：[`LOCALIZE-202608.md`](./LOCALIZE-202608.md)（缺漏、優化、波次）。

---

## 1. 階段對照

| 階段 | 模組 | 找什麼 |
|------|------|--------|
| 本地 lang | `jar_scan` | `assets/<ns>/lang/*`、`.lang` |
| 任務 SNBT | `ftbquests` | FTB Quests 章節／任務 title、description |
| 文字覆寫 | `text_overlay` | patchouli、openloader 資料包顯示字、fancymenu… |
| Origins | `origins` | powers／origins 的 name、description（避開機制 id） |
| 任務／書本 | `quests_books` | Better Questing／HQM／Heracles／Modonomicon 顯示欄位 |

---

## 2. 鬆散語言檔根（`jar_scan`）

只收路徑含 `/lang/` 且副檔名為 `.json` / `.lang` 的檔（locale 形如 `en_us`）。

| 相對 minecraft 根 | 深度 | 說明 |
|-------------------|------|------|
| `mods/` | 2（掃 jar） | jar 內 `assets/*/lang` |
| `resourcepacks/` | 資料夾／zip | 資源包 lang |
| `kubejs/` | 12 | 腳本旁 lang |
| `config/` | 16 | 含 **openloader 巢狀** pack 內 lang |
| `defaultconfigs/` | 12 | 伺服器預設設定旁 lang |
| `datapacks/` | 14 | 世界資料包 lang |
| `global_packs/` | 14 | 全域包（若有） |
| `paxi/` | 12 | Paxi 資料包 |
| `data/` | 10 | 實例頂層 data |

略過：`versions/`、`libraries/`、`cache/`。

---

## 3. 文字覆寫根（`text_overlay`）

| 根 | 收檔規則 |
|----|----------|
| `patchouli_books/` | 全部 `.json`（全字串） |
| `config/openloader/` | 見 §4 |
| `datapacks/`、`global_packs/`、`paxi/datapacks/`、`defaultconfigs/`、`data/`、`kubejs/data/` | 同 §4 |
| `kubejs/` | `/lang/` 下 `.json` |
| `config/fancymenu/`、`defaultconfigs/fancymenu/` | `.txt`／`.json` 可讀文字 |

---

## 4. 資料包 JSON 是否收入（§4 規則）

依序判定：

1. **標準文字路徑** → 收：`lang`、`advancements`、`patchouli`／`patchouli_books`
2. **純機制路徑** → 不收：`recipes`、`loot_tables`、`tags`、`worldgen`、部分 `mmorpg_value_calc` 等
3. **路徑提示** → 收：路徑含跨模組常見顯示片段，例如  
   `spells`、`affix`、`unique`、`perk`、`talent`、`quest`、`dialog`、`powers`、`origins`、`mmorpg_*`、`library_of_exile`…
4. **內容嗅探** → 前 256KB 內出現 `"loc_name"`、`"loc_desc"`、`"effect_tip"`、`"flavor_text"` 等鍵 → 收

### 字串怎麼抽

| 模式 | 何時 | 行為 |
|------|------|------|
| **All** | lang／advancements／patchouli／FancyMenu 引號字 | 可譯過濾後的全部字串 |
| **DisplayFieldsOnly** | 其餘資料包 JSON | **只碰顯示欄位**（見下） |

**顯示欄位（白名單 + `loc_*` 前綴）**  
`loc_name`、`loc_desc`、`effect_tip`、`flavor_text`、`description`、`title`、`tooltip`、`lore`、`text`…  

**刻意不收裸 `name`**：常是 guid／機制 id，翻了會壞遊戲邏輯。

---

## 5. Origins／任務書額外根

與手翻一樣多根：

- Origins：`datapacks`、`kubejs/data`、`data`、`config/openloader`、`global_packs`、`paxi/datapacks`、`defaultconfigs`
- 任務書：上述 + `config`、`hqm`（路徑仍須通過 `betterquesting`／`hqm`／`heracles`／`modonomicon` 等片段）

---

## 6. 和「手翻 CTE2」的關係

CTE2 手翻常走：

- 資源包 lang
- `config/openloader/data/…/mmorpg_spells` 等的 `loc_name`／`effect_tip`
- 任務／設定內嵌字 + 必要時 exact map

工具**不**把 CTE2  exclusive 當唯一路徑，而是把同一套搜尋習慣**泛化**：

- 多根（openloader 只是其中一根）
- 路徑提示不綁死包名（`mmorpg_*` 只是提示之一）
- 顯示欄位白名單對齊「只改玩家看得到的字」

目前工具會在常見下載資料夾、文件、桌面，以及各磁碟的 Downloads／Download／Down／Games 等淺層位置尋找 CTE2／繁中參考資料夾。你提供的
D:\Down\ccc\CTE2\CTE2-繁體中文翻譯-僅翻譯 會被當成可讀取的參考來源；也可以在介面的「參考翻譯（選用）」手動選取其他電腦上的資料夾。
參考內容只會補入缺少的語言鍵，不會覆蓋整合包原本已有的繁中，也不會修改原始 JAR。

仍可能漏的：模組硬編碼在 Java 的字串、圖內文字、未知 schema 且鍵名不在白名單／嗅探列表——需擴充 §4 或加 exact map，而不是改 jar。

---

## 7. 改搜尋時檢查清單

1. 新根是否應進 `jar_scan`（lang）還是 `text_overlay`（內嵌字）？
2. 新 schema 顯示鍵 → 加 `DISPLAY_FIELD_KEYS` 或 `loc_*` 規則；**不要**為了省事改成全字串掃 gameplay JSON。
3. 新「必跳過」機制路徑 → `is_mechanism_only_path`。
4. 更新本檔與 `cargo check`。
