# 如何擴充新的文字來源

目標：新來源可掃、可譯、可寫到「翻譯結果」、需要時可「套用」進遊戲；**不改原始 jar、不碰圖片**。模組語言檔會寫入 `jar-translated/` 的翻譯副本。

優先加哪些來源、完整度／速度 backlog：見 **`LOCALIZE-202608.md`**（§3 缺漏、§6 完整度、§7 波次）。路徑規則：`SEARCH-MAP.md`。

## 1. 選擴充類型

| 類型 | 適合 | 現有範例 |
|------|------|----------|
| A. 語言檔進資源包 | `assets/*/lang/*.json` | `jar_scan` + `pack_out` |
| B. 設定／任務覆寫檔 | snbt、自訂 JSON 樹 | `ftbquests`、`text_overlay` |
| C. 編碼／小修 | 不改語意只修編碼 | `minemenu` |

優先加在 **B** 的獨立模組，避免再塞爆 `jar_scan`。

## 2. 建議步驟（B 類覆寫）

### 2.1 新檔 `engine/my_source.rs`

```rust
// 示意
pub struct MyResult { pub files_written: usize, pub note: String }

pub fn translate_my_source<F>(
    minecraft_dir: &Path,
    output_dir: &Path,  // 通常是 work_root = 翻譯結果
    use_ai: bool,
    mut on_progress: F,
) -> Result<MyResult, String>
where
    F: FnMut(u8, &str),
{
    // 1. 收集檔案（略過 .png/.jpg/.ogg… 與過大檔）
    // 2. 抽可譯字串 → unique 列表
    // 3. convert_s2tw_batch（中文）
    // 4. 其餘非繁：use_ai 時 translate_plain_strings
    // 5. 寫回 output_dir 下「相對 minecraft 的路徑」
    // 6. note 說明寫了什麼
}
```

共用：

- `convert::convert_s2tw_batch`（內建 zh-Hant-TW，不需外部程式）
- `deepseek::translate_plain_strings`（已並行；**內含**去重 → 術語表 → 翻譯記憶 → AI，
  且回來的每條都過佔位符把關，被退回的那條會是空字串）
- `cancel::check()?` — 長迴圈每輪或每階段呼叫一次，讓「停止」按鈕有效
- 字串過濾：可參考 `text_overlay` / `ftbquests` 的 should_translate

> `translate_plain_strings` 回傳空字串＝該條沒有可用譯文（未命中或被佔位符把關退回），
> 呼叫端要保留原文，不可以寫入空字串。

### 2.2 掛進 `engine/mod.rs`

```rust
mod my_source;
pub use my_source::{translate_my_source, MyResult};
```

### 2.3 串 `lib.rs` 一鍵流程

在 `run_one_click` 的 ftbquests／overlay 之後或之內呼叫：

- 進度區間預留 1–3%（emit_progress）
- note 併入 `player_summary` 與 `CoverageStats`（或擴充 Coverage 欄位）
- 錯誤：`emit_error` + `error_lines` + `append_error_file`

補翻 `run_supplement`：若來源依賴 AI，有金鑰時可重跑。

### 2.4 套用 `apply_instance.rs`

若產出在 work 下某目錄且需進遊戲：

1. `has_xxx = dir_has_files(work.join(…))`
2. 依玩家備份選項把 `mc` 對應路徑保存到 `翻譯結果/翻譯套用備份_*`
3. `merge_copy_dir` 到實例
4. summary 列出名稱

**禁止**複製進 `mods/`。

### 2.5 文件與 UI

- `docs/CHANGELOG.md`
- `ARCHITECTURE.md` 資料流一行
- 說明 overlay「會翻譯什麼」+ `USER-GUIDE.md`
- `AGENTS.md` 導航表一行

## 3. A 類（更多 lang 路徑）

改 `jar_scan.rs`：

- 新增 walk 根目錄或深度
- locale 仍走 `is_locale_code`；pending 合併邏輯勿只認 en_us
- 圖檔 `is_image_ext` 保持略過
- 平行掃注意 merge 執行緒安全（現有 chunk+join 模式）

## 4. 字串過濾原則

應譯：

- 含字母或 CJK、長度合理、給玩家看的句子

不譯：

- 純 id（`modid:item`）、路徑、副檔名媒體、過短符號、色碼 only

`deepseek::looks_untranslatable` 已擋掉全小寫資源 id 與網址；新來源若有自己的
過濾器，記得保持一致，不要把 `minecraft:stone` 送出去。

## 5. 效能

- 唯一字串去重再送 AI
- 大目錄設 depth／2MB 上限（見 text_overlay）
- AI 內層依 provider 限制分批，來源超過 8,000 個候選字串時再由流程自動續批；預算仍由 provider budget／限制控制
- **上限被觸發時必須回報剩餘數**（AGENTS.md 硬不變式 12），不可靜默截斷

## 6. 驗收

- [ ] 無 AI 時：既有中文路徑不崩
- [ ] 有 AI：有寫入、日誌有批進度
- [ ] 譯文有過佔位符把關（用 `translate_plain_strings` 就自動有）
- [ ] 長迴圈有 `cancel::check()`，按停止會停
- [ ] 輸出只在 `翻譯結果/`
- [ ] 套用有備份、可還原
- [ ] `npm run check` 0 error 0 warning
- [ ] `npm run test` 全綠，且新邏輯有附測試

## 7. 不建議做的擴充

- 解包改 jar 內 class／assets 後重打包
- 圖片 OCR
- 把整包 config toml 無差別全翻（易弄壞數值／id）
- 背景強制關 Minecraft 行程（可提示，勿預設 kill）
