//! 任務／書本系統的內嵌文字（非 lang 檔）：
//! Better Questing、HQM（HardcoreQuesting）、Heracles、Modonomicon。
//!
//! 這些系統把玩家可見文字（任務名／說明／書本頁）直接寫進 `config/`、`defaultconfigs/`
//! 或 `data/<ns>/` 的 JSON——`jar_scan`（assets/lang）、`text_overlay`（patchouli/openloader…）
//! 與 FTB Quests（SNBT）都掃不到，於是這類文字整片英文。
//!
//! ⚠️ 安全模型（比 Origins 更保守）：
//! - **只翻「顯示欄位白名單」之下可達的字串**：name／title／subtitle／description／desc／text…。
//!   結構欄位（type／id／icon／item／condition…）的字串永遠不會被送去翻譯——因為只有進入
//!   白名單欄位才會開始擷取，結構層字串根本不進入輸出。
//! - 白名單欄位內再過 `should_translate`（擋資源 id、本地化鍵、URL、序列化 JSON 元件）。
//! - Better Questing 的 NBT 型別後綴（`name:8`／`desc:8`）先去掉再比對欄位名。
//! - 文字元件（`{text,color,clickEvent,extra…}`）只翻 `text`／遞迴 `extra`，其餘（顏色、事件、
//!   translate 鍵）一律跳過，避免把顏色名或 lang 鍵翻掉。
//!
//! 只輸出到 work 目錄（保留相對路徑），不直接改實例；套用時走既有備份流程。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use super::convert::convert_s2tw_batch;
use super::deepseek::translate_plain_strings;

/// 任務資料庫（DefaultQuests.json）可能很大，放寬到 24MB。
const MAX_FILE_BYTES: u64 = 24 * 1024 * 1024;
const MAX_WALK_DEPTH: usize = 16;
const MAX_AI_UNIQUE: usize = 8000;

/// 顯示欄位白名單（去掉 BQ 型別後綴、轉小寫後比對）。只有這些欄位之下的字串會被翻。
const DISPLAY_FIELDS: &[&str] = &[
    "name",
    "title",
    "subtitle",
    "description",
    "desc",
    "text",
    "tooltip",
    "header",
    "message",
    "lore",
    "hover",
    "flavor",
    "flavour",
];

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct QuestsBooksResult {
    pub files_written: usize,
    pub strings_translated: usize,
    pub note: String,
}

/// 掃描並翻譯任務／書本系統文字，輸出到 output_dir（保留相對路徑）。
pub fn translate_quests_books<F>(
    minecraft_dir: &Path,
    output_dir: &Path,
    use_ai: bool,
    mut on_progress: F,
) -> Result<QuestsBooksResult, String>
where
    F: FnMut(u8, &str),
{
    on_progress(2, "任務／書本：掃描 Better Questing／HQM／Heracles／Modonomicon…");
    let files = collect_files(minecraft_dir);
    if files.is_empty() {
        return Ok(QuestsBooksResult {
            note: "未找到 Better Questing／HQM／Heracles／Modonomicon 的任務／書本檔。".into(),
            ..Default::default()
        });
    }

    let mut payloads: Vec<(PathBuf, Value)> = Vec::new();
    let mut unique: Vec<String> = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut parse_failures: Vec<String> = Vec::new();

    for path in &files {
        let raw = match fs::read_to_string(path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let v = match super::lenient_json::parse(&raw) {
            Ok(v) => v,
            Err(e) => {
                parse_failures.push(format!("{} — {e}", path.display()));
                continue;
            }
        };
        collect_translatable(&v, &mut |s| {
            if should_translate(s) && seen.insert(s.to_string(), ()).is_none() {
                unique.push(s.to_string());
            }
        });
        payloads.push((path.clone(), v));
    }

    if !parse_failures.is_empty() {
        write_parse_errors(output_dir, &parse_failures);
        on_progress(
            8,
            &format!("任務／書本：{} 檔解析失敗已略過（見錯誤日誌）", parse_failures.len()),
        );
    }

    on_progress(
        12,
        &format!("任務／書本：{} 檔、唯一可譯字串 {}", payloads.len(), unique.len()),
    );

    if unique.is_empty() {
        return Ok(QuestsBooksResult {
            note: format!(
                "掃描 {} 個任務／書本檔，無可譯的顯示文字（可能已是繁中或都是 id）。",
                payloads.len()
            ),
            ..Default::default()
        });
    }

    // 翻譯表：original -> zh
    let mut map: HashMap<String, String> = HashMap::new();

    // 1) 既有中文一律轉台灣正體
    let chinese: Vec<String> = unique.iter().filter(|s| looks_chinese(s)).cloned().collect();
    if !chinese.is_empty() {
        let conv = convert_s2tw_batch(&chinese);
        for (i, orig) in chinese.iter().enumerate() {
            if let Some(c) = conv.get(i) {
                if c != orig && !c.trim().is_empty() {
                    map.insert(orig.clone(), c.clone());
                }
            }
        }
    }

    // 2) AI 補其餘（走既有 glossary→TM→AI→遮罩→guard）
    let mut capped = String::new();
    if use_ai {
        let mut need_ai: Vec<String> = Vec::new();
        let mut skipped = 0usize;
        for s in &unique {
            if map.contains_key(s) || (looks_chinese(s) && !has_latin_letter(s)) {
                continue;
            }
            if need_ai.len() >= MAX_AI_UNIQUE {
                skipped += 1;
                continue;
            }
            need_ai.push(s.clone());
        }
        if skipped > 0 {
            capped = format!(
                "；超過單次上限 {}，本輪未處理 {} 條（再按「再補一些」可續）",
                MAX_AI_UNIQUE, skipped
            );
        }
        if !need_ai.is_empty() {
            on_progress(30, &format!("任務／書本：AI 翻譯 {} 條…", need_ai.len()));
            let translated = translate_plain_strings(&need_ai, |pct, msg| {
                let mapped = 30 + (pct as u16 * 50 / 100) as u8;
                on_progress(mapped.min(80), msg);
            })?;
            for (i, en) in need_ai.iter().enumerate() {
                if let Some(zh) = translated.get(i) {
                    let t = zh.trim();
                    if !t.is_empty() && t != en {
                        map.insert(en.clone(), t.to_string());
                    }
                }
            }
        }
    } else {
        on_progress(50, "任務／書本：未勾 AI，僅轉換既有中文");
    }

    if map.is_empty() {
        return Ok(QuestsBooksResult {
            note: format!(
                "任務／書本：掃描 {} 檔、唯一字串 {}；無變更。",
                payloads.len(),
                unique.len()
            ),
            ..Default::default()
        });
    }

    // 3) 寫回（與擷取共用同一套 walk，結構層字串絕不會被動到）
    on_progress(88, "任務／書本：寫出檔案…");
    let mut written = 0usize;
    for (path, mut v) in payloads {
        let changed = apply_translatable(&mut v, &map);
        if !changed {
            continue;
        }
        let rel = path.strip_prefix(minecraft_dir).unwrap_or(path.as_path());
        let out_path = output_dir.join(rel);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let s = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
        fs::write(&out_path, s + "\n").map_err(|e| format!("{}: {e}", out_path.display()))?;
        written += 1;
    }

    on_progress(100, "任務／書本完成");
    Ok(QuestsBooksResult {
        files_written: written,
        strings_translated: map.len(),
        note: format!(
            "任務／書本（Better Questing／HQM／Heracles／Modonomicon）：翻譯表 {} 條、寫出 {} 檔{}。需套用才進遊戲。",
            map.len(),
            written,
            capped
        ),
    })
}

// ─── 檔案收集 ───────────────────────────────────────────────

fn collect_files(mc: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // 手翻同路徑：任務／書本可能落在不同模組各自的資料夾
    // - config／defaultconfigs：Better Questing、HQM、Heracles
    // - openloader／global_packs：包作者嵌在 datapack 裡的任務書
    // - hqm／data／kubejs／datapacks／paxi：各版本常見落點
    let roots = [
        mc.join("config"),
        mc.join("defaultconfigs"),
        mc.join("hqm"),
        mc.join("data"),
        mc.join("kubejs").join("data"),
        mc.join("datapacks"),
        mc.join("paxi").join("datapacks"),
        mc.join("config").join("openloader"),
        mc.join("global_packs"),
    ];
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for e in WalkDir::new(&root)
            .max_depth(MAX_WALK_DEPTH)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            if p.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase())
                != Some("json".to_string())
            {
                continue;
            }
            if !is_quest_book_path(p) {
                continue;
            }
            if let Ok(meta) = p.metadata() {
                if meta.len() == 0 || meta.len() > MAX_FILE_BYTES {
                    continue;
                }
            }
            if !out.iter().any(|x| x == p) {
                out.push(p.to_path_buf());
            }
        }
    }
    out
}

/// 路徑須經過已知任務／書本系統的資料夾，避免掃到無關 JSON。
fn is_quest_book_path(path: &Path) -> bool {
    let lower = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    lower.contains("/betterquesting/")
        || lower.contains("/hqm/")
        || lower.contains("/hardcorequesting/")
        || lower.contains("/heracles/")
        || lower.contains("/modonomicon/")
}

// ─── 顯示欄位擷取／寫回 ─────────────────────────────────────

/// 去掉 Better Questing 的 NBT 型別後綴（`name:8` → `name`）。
fn base_key(k: &str) -> &str {
    if let Some(idx) = k.rfind(':') {
        let (pre, post) = k.split_at(idx);
        let digits = &post[1..];
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return pre;
        }
    }
    k
}

fn is_display_field(base_lower: &str) -> bool {
    DISPLAY_FIELDS.contains(&base_lower)
}

fn collect_translatable(v: &Value, f: &mut dyn FnMut(&str)) {
    struct_walk(v, &mut |s| f(s));
}

/// 結構層：只有進入白名單顯示欄位才開始擷取；其餘一律繼續往下找顯示欄位。
fn struct_walk(v: &Value, f: &mut dyn FnMut(&str)) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                let b = base_key(k).to_ascii_lowercase();
                if is_display_field(&b) {
                    display_walk(child, f);
                } else {
                    struct_walk(child, f);
                }
            }
        }
        Value::Array(a) => {
            for child in a {
                struct_walk(child, f);
            }
        }
        _ => {}
    }
}

/// 顯示欄位內：翻字串、遞迴陣列；物件只認文字元件的 `text`／`extra`，其餘（顏色、事件、
/// translate 鍵…）跳過。
fn display_walk(v: &Value, f: &mut dyn FnMut(&str)) {
    match v {
        Value::String(s) => {
            f(s);
        }
        Value::Array(a) => {
            for child in a {
                display_walk(child, f);
            }
        }
        Value::Object(m) => {
            for (k, child) in m {
                let b = base_key(k).to_ascii_lowercase();
                if b == "text" || b == "extra" {
                    display_walk(child, f);
                }
                // 其餘 component 屬性（color/clickEvent/hoverEvent/translate/font…）跳過
            }
        }
        _ => {}
    }
}

fn apply_translatable(v: &mut Value, map: &HashMap<String, String>) -> bool {
    struct_apply(v, map)
}

fn struct_apply(v: &mut Value, map: &HashMap<String, String>) -> bool {
    let mut changed = false;
    match v {
        Value::Object(m) => {
            for (k, child) in m.iter_mut() {
                let b = base_key(k).to_ascii_lowercase();
                if is_display_field(&b) {
                    if display_apply(child, map) {
                        changed = true;
                    }
                } else if struct_apply(child, map) {
                    changed = true;
                }
            }
        }
        Value::Array(a) => {
            for child in a.iter_mut() {
                if struct_apply(child, map) {
                    changed = true;
                }
            }
        }
        _ => {}
    }
    changed
}

fn display_apply(v: &mut Value, map: &HashMap<String, String>) -> bool {
    let mut changed = false;
    match v {
        Value::String(s) => {
            if let Some(t) = map.get(s.as_str()) {
                if t != s {
                    *s = t.clone();
                    changed = true;
                }
            }
        }
        Value::Array(a) => {
            for child in a.iter_mut() {
                if display_apply(child, map) {
                    changed = true;
                }
            }
        }
        Value::Object(m) => {
            for (k, child) in m.iter_mut() {
                let b = base_key(k).to_ascii_lowercase();
                if (b == "text" || b == "extra") && display_apply(child, map) {
                    changed = true;
                }
            }
        }
        _ => {}
    }
    changed
}

// ─── 字串過濾 ───────────────────────────────────────────────

fn should_translate(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 2 {
        return false;
    }
    // 序列化的文字元件字串（NBT display Name 之類）：整段當不透明翻會壞掉，跳過。
    if (t.starts_with('{') || t.starts_with('[')) && (t.contains("\"text\"") || t.contains("\"translate\"")) {
        return false;
    }
    // 純資源 id / URL：minecraft:xxx、https://…
    if t.contains(':')
        && !t.contains(' ')
        && t.is_ascii()
        && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '/' | '.' | '-'))
    {
        return false;
    }
    // 看起來像本地化鍵：全小寫 + 至少一個點、無空白（quest.foo.title）
    if !t.contains(' ')
        && t.contains('.')
        && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return false;
    }
    let has_alpha = t.chars().any(|c| c.is_ascii_alphabetic());
    has_alpha || looks_chinese(t)
}

fn looks_chinese(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn has_latin_letter(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
}

fn write_parse_errors(output_dir: &Path, failures: &[String]) {
    if failures.is_empty() {
        return;
    }
    let p = output_dir.join("翻譯錯誤日誌.txt");
    let mut body = fs::read_to_string(&p).unwrap_or_else(|_| {
        "【模組包繁中翻譯 — 錯誤／警告日誌】\n有問題時請把本檔內容一併提供。\n".into()
    });
    body.push_str("\n======== 任務／書本（BQ/HQM/Heracles/Modonomicon）解析失敗 ========\n");
    for f in failures {
        body.push_str("• ");
        body.push_str(f);
        body.push('\n');
    }
    let _ = fs::write(p, body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collect(v: &Value) -> Vec<String> {
        let mut out = Vec::new();
        collect_translatable(v, &mut |s| {
            if should_translate(s) {
                out.push(s.to_string());
            }
        });
        out.sort();
        out
    }

    #[test]
    fn better_questing_name_desc_with_nbt_suffix() {
        // BQ 用 name:8 / desc:8；型別後綴要先去掉才認得出是顯示欄位。
        let v = json!({
            "questDatabase:9": [
                {
                    "questID:3": 0,
                    "properties:10": {
                        "betterquesting:10": {
                            "name:8": "Welcome to the Pack",
                            "desc:8": "Collect wood to begin.",
                            "icon:10": { "id:8": "minecraft:book" }
                        }
                    },
                    "tasks:9": [ { "taskID:8": "bq_standard:retrieval" } ]
                }
            ]
        });
        let got = collect(&v);
        assert!(got.contains(&"Welcome to the Pack".to_string()));
        assert!(got.contains(&"Collect wood to begin.".to_string()));
        // id 與 taskID 是識別字，不可被抓
        assert!(!got.contains(&"minecraft:book".to_string()));
        assert!(!got.contains(&"bq_standard:retrieval".to_string()));
    }

    #[test]
    fn heracles_title_subtitle_description_and_text_components() {
        let v = json!({
            "type": "heracles:item",
            "item": "minecraft:diamond",
            "title": "Find Diamonds",
            "subtitle": { "text": "Dig deep", "color": "aqua" },
            "description": ["Line one", { "text": "Line two", "color": "red" }],
            "icon": { "id": "minecraft:diamond" }
        });
        let got = collect(&v);
        assert!(got.contains(&"Find Diamonds".to_string()));
        assert!(got.contains(&"Dig deep".to_string()));
        assert!(got.contains(&"Line one".to_string()));
        assert!(got.contains(&"Line two".to_string()));
        // 結構欄位不可被抓
        assert!(!got.contains(&"heracles:item".to_string()));
        assert!(!got.contains(&"minecraft:diamond".to_string()));
        // component 的顏色不可被抓
        assert!(!got.contains(&"aqua".to_string()));
        assert!(!got.contains(&"red".to_string()));
    }

    #[test]
    fn modonomicon_page_text_and_titles() {
        let v = json!({
            "pages": [
                { "type": "modonomicon:text", "title": "Chapter 1", "text": "Long body text here." }
            ],
            "name": "Guide Book",
            "tooltip": "Open me"
        });
        let got = collect(&v);
        assert!(got.contains(&"Chapter 1".to_string()));
        assert!(got.contains(&"Long body text here.".to_string()));
        assert!(got.contains(&"Guide Book".to_string()));
        assert!(!got.contains(&"modonomicon:text".to_string()));
    }

    #[test]
    fn skips_translate_key_and_lang_keys() {
        // 文字元件用 translate（lang 鍵）時，不該把 lang 鍵翻掉
        let v = json!({ "title": { "translate": "quest.mypack.start" } });
        let got = collect(&v);
        assert!(got.is_empty(), "translate 值是 lang 鍵，不可被抓");
    }

    #[test]
    fn apply_replaces_only_display_strings() {
        let mut v = json!({
            "type": "heracles:item",
            "item": "minecraft:stone",
            "title": "Collect Stone",
            "icon": { "id": "minecraft:stone" }
        });
        let mut map = HashMap::new();
        map.insert("Collect Stone".to_string(), "收集石頭".to_string());
        // 就算 item 的值恰好等於某翻譯鍵也不會被動（結構層不進 map 替換）
        map.insert("minecraft:stone".to_string(), "不該用到".to_string());
        let changed = apply_translatable(&mut v, &map);
        assert!(changed);
        assert_eq!(v["title"], "收集石頭");
        assert_eq!(v["item"], "minecraft:stone");
        assert_eq!(v["icon"]["id"], "minecraft:stone");
    }

    #[test]
    fn path_recognition() {
        assert!(is_quest_book_path(Path::new(
            "inst/config/betterquesting/DefaultQuests.json"
        )));
        assert!(is_quest_book_path(Path::new(
            "inst/config/heracles/quests/start/intro.json"
        )));
        assert!(is_quest_book_path(Path::new(
            "inst/data/mypack/modonomicon/books/guide/book.json"
        )));
        assert!(is_quest_book_path(Path::new("inst/hqm/quests.json")));
        assert!(!is_quest_book_path(Path::new(
            "inst/config/somemod/settings.json"
        )));
        assert!(!is_quest_book_path(Path::new(
            "inst/assets/minecraft/lang/en_us.json"
        )));
    }

    #[test]
    fn serialized_component_string_is_skipped() {
        // NBT display Name 常是序列化 JSON 字串，整段翻會壞
        assert!(!should_translate("{\"text\":\"Cool Sword\"}"));
        assert!(should_translate("Cool Sword"));
        assert!(!should_translate("minecraft:diamond"));
        assert!(!should_translate("quest.foo.title"));
    }
}
