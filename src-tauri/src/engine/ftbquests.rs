//! FTB Quests 任務／劇情文字（config/ftbquests/**/*.snbt）
//! 只翻顯示欄（title／description／subtitle）；**绝不**翻 type／shape／auto 等結構 id。

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::convert::convert_s2tw_batch;
use super::deepseek::translate_plain_strings_with_scope;
use super::mech_tokens::{is_ascii_enum_token, is_poisoned_mech_translation, is_resource_path_token};
use super::placeholder::{self, GuardStats};
use super::translation_scope::TranslationScope;

/// 可翻譯的顯示欄位（鍵名小寫比對）。
const DISPLAY_KEYS: &[&str] = &["title", "description", "subtitle"];

/// 結構欄：值是 ResourceLocation／enum，翻成中文會讓遊戲崩潰。
const STRUCT_KEYS: &[&str] = &[
    "type",
    "shape",
    "auto",
    "id",
    "icon",
    "item",
    "command",
    "dependency",
    "dependencies",
    "default_quest_shape",
    "filename",
    "path",
    "tag",
    "entity",
    "dimension",
    "biome",
    "stat",
    "advancement",
    "structure",
    "table_id",
    "loot_table",
];

/// 逐項統計。目前彙整成 `note` 給玩家看，其餘欄位保留供排查回報用。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct QuestTranslateResult {
    pub files_seen: usize,
    pub strings_found: usize,
    pub strings_unique: usize,
    pub strings_translated: usize,
    pub files_written: usize,
    pub output_dir: String,
    pub note: String,
}

/// 將遊戲裡的 ftbquests 翻譯後輸出到 out/config/ftbquests（不直接改遊戲，避免弄壞）。
/// 二次補翻：若工作目錄已有 snbt，優先讀工作副本；已是繁中的顯示字串不送 AI。
pub fn translate_ftbquests<F>(
    minecraft_dir: &Path,
    output_dir: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<QuestTranslateResult, String>
where
    F: FnMut(u8, &str),
{
    let mc_src = minecraft_dir.join("config").join("ftbquests");
    if !mc_src.is_dir() {
        return Ok(QuestTranslateResult {
            files_seen: 0,
            strings_found: 0,
            strings_unique: 0,
            strings_translated: 0,
            files_written: 0,
            output_dir: String::new(),
            note: "此整合包沒有 config/ftbquests，略過任務翻譯。".into(),
        });
    }

    let work_src = output_dir.join("config").join("ftbquests");
    let prefer_work = work_src.is_dir();

    on_progress(5, "任務：掃描 FTB Quests（.snbt）…");
    let mut files: Vec<PathBuf> = Vec::new();
    for e in WalkDir::new(&mc_src).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_file()
            && p.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("snbt"))
                .unwrap_or(false)
        {
            files.push(p.to_path_buf());
        }
    }

    let mut file_texts: Vec<(PathBuf, String)> = Vec::new();
    let mut all_strings: Vec<String> = Vec::new();
    let mut string_set: HashMap<String, ()> = HashMap::new();
    let mut strings_found = 0usize;
    let mut from_work = 0usize;

    for path in &files {
        let rel = path.strip_prefix(&mc_src).unwrap_or(path.as_path());
        let read_path = if prefer_work {
            let candidate = work_src.join(rel);
            if candidate.is_file() {
                from_work += 1;
                candidate
            } else {
                path.clone()
            }
        } else {
            path.clone()
        };
        let text = fs::read_to_string(&read_path).map_err(|e| format!("{}: {e}", read_path.display()))?;
        let extracted = extract_display_strings(&text);
        strings_found += extracted.len();
        for s in extracted {
            if string_set.insert(s.clone(), ()).is_none() {
                all_strings.push(s);
            }
        }
        // 以實際讀到的內容為 rewrite 底稿（工作副本優先）
        file_texts.push((path.clone(), text));
    }

    on_progress(
        20,
        &format!(
            "任務：找到 {} 個 snbt、約 {} 條顯示字串（唯一 {}）{}",
            files.len(),
            strings_found,
            all_strings.len(),
            if from_work > 0 {
                format!("；沿用工作目錄 {from_work} 檔")
            } else {
                String::new()
            }
        ),
    );

    if all_strings.is_empty() {
        return Ok(QuestTranslateResult {
            files_seen: files.len(),
            strings_found: 0,
            strings_unique: 0,
            strings_translated: 0,
            files_written: 0,
            output_dir: String::new(),
            note: "任務檔裡沒有可翻譯的顯示文字（可能已是中文或格式特殊）。".into(),
        });
    }

    let mut map: HashMap<String, String> = HashMap::new();
    let mut guard = GuardStats::default();
    let mut skipped_zh = 0usize;

    // 既有中文：只做台灣繁轉換，不送 AI
    let chinese: Vec<String> = all_strings
        .iter()
        .filter(|s| looks_chinese(s))
        .cloned()
        .collect();
    if !chinese.is_empty() {
        let converted = convert_s2tw_batch(&chinese);
        for (i, s) in chinese.iter().enumerate() {
            if let Some(c) = converted.get(i) {
                if c != s {
                    if let Some(safe) = placeholder::guard(s, c, &mut guard) {
                        map.insert(s.clone(), safe);
                    }
                } else if !has_latin_letter(s) {
                    skipped_zh += 1;
                }
            }
        }
    }

    if use_ai {
        let need_ai: Vec<String> = strings_needing_ai(&all_strings, &map);
        skipped_zh += all_strings
            .iter()
            .filter(|s| looks_chinese(s) && !has_latin_letter(s) && !map.contains_key(*s))
            .count();
        if need_ai.is_empty() {
            on_progress(
                50,
                &format!(
                    "任務：顯示字串已是繁中或無需 AI（略過 {} 條），不重跑全量",
                    skipped_zh
                ),
            );
        } else {
            on_progress(
                30,
                &format!(
                    "任務：AI 只補仍為英文的顯示字串 {} 條（已略過繁中 {}）…",
                    need_ai.len(),
                    skipped_zh
                ),
            );
            let app_prog = &mut on_progress;
            let translated = translate_plain_strings_with_scope(&need_ai, scope, |pct, msg| {
                let mapped = 30 + (pct as u16 * 50 / 100) as u8;
                app_prog(mapped.min(80), msg);
            })?;
            for (i, en) in need_ai.iter().enumerate() {
                if let Some(zh) = translated.get(i) {
                    let t = zh.trim();
                    if !t.is_empty() {
                        if let Some(safe) = placeholder::guard(en, t, &mut guard) {
                            if is_safe_display_translation(en, &safe) {
                                map.insert(en.clone(), safe);
                            }
                        }
                    }
                }
            }
        }
    } else {
        on_progress(40, "任務：未勾 AI，僅對既有中文做台灣繁轉換…");
    }

    if !map.is_empty() {
        on_progress(82, "任務：簡體→台灣繁體…");
        let keys: Vec<String> = map.keys().cloned().collect();
        let vals: Vec<String> = keys.iter().map(|k| map.get(k).unwrap().clone()).collect();
        let conv = convert_s2tw_batch(&vals);
        for (i, k) in keys.iter().enumerate() {
            if let Some(v) = conv.get(i) {
                map.insert(k.clone(), v.clone());
            }
        }
    }

    if !map.is_empty() {
        let _ = super::shared_tm::contribute_plain_pairs(&map, &HashMap::new(), "overlay", scope);
    }

    let dest_root = output_dir.join("config").join("ftbquests");
    on_progress(88, "任務：寫出翻譯後的 quest 檔…");
    let mut written = 0usize;
    let mut applied = 0usize;

    for (src_path, text) in &file_texts {
        let rel = src_path.strip_prefix(&mc_src).unwrap_or(src_path.as_path());
        let (new_text, n) = rewrite_display_strings(text, &map);
        applied += n;
        if new_text != *text {
            let out_path = dest_root.join(rel);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&out_path, new_text.as_bytes()).map_err(|e| e.to_string())?;
            written += 1;
        } else if prefer_work && !dest_root.join(rel).is_file() {
            // 工作目錄有內容但未變更時仍確保輸出存在（方便套用）
            let out_path = dest_root.join(rel);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&out_path, text.as_bytes()).map_err(|e| e.to_string())?;
            written += 1;
        }
    }

    let readme = output_dir.join("【任務翻譯】請複製到遊戲.txt");
    let _ = fs::write(
        &readme,
        format!(
            "【FTB Quests 任務／劇情翻譯】\n\
1. 翻譯完成前請關閉遊戲。\n\
2. 工具完成時會依你的備份選項，直接把 config\\ftbquests 套用到正確的遊戲資料夾。\n\
3. 只翻譯標題／說明等顯示文字；任務類型（type）等識別字不會改，避免遊戲崩潰。\n\
4. 開遊戲檢查任務書；若仍有英文，回工具按「再補一些」。\n\
\n\
統計：掃描 {} 檔、唯一字串 {}、套用約 {} 處、寫出 {} 檔。\n\
輸出：{}\n",
            files.len(),
            all_strings.len(),
            applied,
            written,
            dest_root.display()
        ),
    );

    on_progress(100, "任務翻譯完成");
    let gap_note = if skipped_zh > 0 {
        format!("；已略過約 {skipped_zh} 條繁中顯示字串不重送 AI")
    } else {
        String::new()
    };
    Ok(QuestTranslateResult {
        files_seen: files.len(),
        strings_found,
        strings_unique: all_strings.len(),
        strings_translated: map.len(),
        files_written: written,
        output_dir: dest_root.display().to_string(),
        note: format!(
            "任務／劇情已處理：變更 {} 條 → 寫出 {} 個 snbt（結構欄 type／shape 等未送翻）{gap_note}。",
            map.len(),
            written
        ),
    })
}

/// 仍需送 AI 的顯示字串：未在 map、且非「純繁中」。
pub fn strings_needing_ai(all: &[String], already: &HashMap<String, String>) -> Vec<String> {
    all.iter()
        .filter(|s| !already.contains_key(*s))
        .filter(|s| !(looks_chinese(s) && !has_latin_letter(s)))
        .cloned()
        .collect()
}

/// 從 SNBT 擷取僅屬顯示欄的字串。
pub fn extract_display_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_description_list = false;
    let mut bracket_depth = 0i32;

    let re_kv = Regex::new(r#"(?:^|[\{,])\s*([A-Za-z0-9_]+)\s*:\s*"((?:\\.|[^"\\])*)""#).expect("kv");
    let re_bare = Regex::new(r#"^\s*"((?:\\.|[^"\\])*)""#).expect("bare");
    let re_key_only = Regex::new(r#"^\s*([A-Za-z0-9_]+)\s*:\s*(\[)?\s*$"#).expect("key");
    let re_inline_list =
        Regex::new(r#"^\s*([A-Za-z0-9_]+)\s*:\s*\[(.*)\]\s*,?\s*$"#).expect("inline_list");
    let re_quoted = Regex::new(r#""((?:\\.|[^"\\])*)""#).expect("quoted");

    for line in text.lines() {
        let trimmed = line.trim();
        if in_description_list {
            bracket_depth += trimmed.chars().filter(|&c| c == '[').count() as i32;
            bracket_depth -= trimmed.chars().filter(|&c| c == ']').count() as i32;
            if let Some(cap) = re_bare.captures(trimmed) {
                let s = unescape_snbt(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
                push_display_candidates(&s, &mut out);
            }
            if bracket_depth <= 0 {
                in_description_list = false;
                bracket_depth = 0;
            }
            continue;
        }

        let mut matched_kv = false;
        for cap in re_kv.captures_iter(line) {
            matched_kv = true;
            let key = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_ascii_lowercase();
            let raw = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let s = unescape_snbt(raw);
            if is_display_key(&key) {
                push_display_candidates(&s, &mut out);
            }
        }
        if matched_kv {
            continue;
        }

        if let Some(cap) = re_inline_list.captures(line) {
            let key = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_ascii_lowercase();
            if is_display_key(&key) {
                let inner = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                for q in re_quoted.captures_iter(inner) {
                    let s = unescape_snbt(q.get(1).map(|m| m.as_str()).unwrap_or(""));
                    push_display_candidates(&s, &mut out);
                }
            }
            continue;
        }

        if let Some(cap) = re_key_only.captures(line) {
            let key = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_ascii_lowercase();
            let opens_list = cap.get(2).is_some() || trimmed.ends_with('[');
            if is_display_key(&key) && (opens_list || trimmed.contains('[')) {
                in_description_list = true;
                bracket_depth = trimmed.chars().filter(|&c| c == '[').count() as i32
                    - trimmed.chars().filter(|&c| c == ']').count() as i32;
                if bracket_depth <= 0 {
                    in_description_list = false;
                    bracket_depth = 0;
                }
            }
        }
    }
    out
}

/// 只改寫顯示欄上的字串，避免把 title 譯文誤套到 type。
pub fn rewrite_display_strings(text: &str, map: &HashMap<String, String>) -> (String, usize) {
    if map.is_empty() {
        return (text.to_string(), 0);
    }
    let mut applied = 0usize;
    let mut out_lines: Vec<String> = Vec::new();
    let mut in_description_list = false;
    let mut bracket_depth = 0i32;

    let re_kv = Regex::new(
        r#"(^|[\{,])(\s*)([A-Za-z0-9_]+)(\s*:\s*")((?:\\.|[^"\\])*)(")"#,
    )
    .expect("kv");
    let re_bare = Regex::new(r#"^(\s*")((?:\\.|[^"\\])*)(".*)$"#).expect("bare");
    let re_key_only = Regex::new(r#"^\s*([A-Za-z0-9_]+)\s*:\s*(\[)?\s*$"#).expect("key");
    let re_inline_list =
        Regex::new(r#"^(\s*)([A-Za-z0-9_]+)(\s*:\s*\[)(.*)(\]\s*,?\s*)$"#).expect("inline_list");
    let re_quoted = Regex::new(r#""((?:\\.|[^"\\])*)""#).expect("quoted");

    for line in text.lines() {
        let trimmed = line.trim();
        if in_description_list {
            bracket_depth += trimmed.chars().filter(|&c| c == '[').count() as i32;
            bracket_depth -= trimmed.chars().filter(|&c| c == ']').count() as i32;
            if let Some(cap) = re_bare.captures(line) {
                let prefix = cap.get(1).map(|m| m.as_str()).unwrap_or("\"");
                let raw = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                let suffix = cap.get(3).map(|m| m.as_str()).unwrap_or("\"");
                let s = unescape_snbt(raw);
                if let Some((rewritten, n)) = rewrite_json_component_string(&s, map) {
                    out_lines.push(format!("{prefix}{}{suffix}", escape_snbt(&rewritten)));
                    applied += n;
                } else if let Some(zh) = map.get(&s) {
                    out_lines.push(format!("{prefix}{}{suffix}", escape_snbt(zh)));
                    applied += 1;
                } else {
                    out_lines.push(line.to_string());
                }
            } else {
                out_lines.push(line.to_string());
            }
            if bracket_depth <= 0 {
                in_description_list = false;
                bracket_depth = 0;
            }
            continue;
        }

        let mut kv_hits = 0usize;
        let mut kv_out = String::new();
        let mut kv_last = 0usize;
        let mut kv_changed = false;
        for cap in re_kv.captures_iter(line) {
            kv_hits += 1;
            let full = cap.get(0).expect("full");
            let lead = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let indent = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let key = cap.get(3).map(|m| m.as_str()).unwrap_or("");
            let mid = cap.get(4).map(|m| m.as_str()).unwrap_or(": \"");
            let raw = cap.get(5).map(|m| m.as_str()).unwrap_or("");
            let quote = cap.get(6).map(|m| m.as_str()).unwrap_or("\"");
            kv_out.push_str(&line[kv_last..full.start()]);
            kv_out.push_str(lead);
            let key_l = key.to_ascii_lowercase();
            let s = unescape_snbt(raw);
            if is_display_key(&key_l) {
                if let Some((rewritten, n)) = rewrite_json_component_string(&s, map) {
                    kv_out.push_str(&format!("{indent}{key}{mid}{}{quote}", escape_snbt(&rewritten)));
                    applied += n;
                    kv_changed = true;
                    kv_last = full.end();
                    continue;
                }
                if let Some(zh) = map.get(&s) {
                    kv_out.push_str(&format!("{indent}{key}{mid}{}{quote}", escape_snbt(zh)));
                    applied += 1;
                    kv_changed = true;
                    kv_last = full.end();
                    continue;
                }
            }
            kv_out.push_str(&line[full.start() + lead.len()..full.end()]);
            kv_last = full.end();
        }
        if kv_hits > 0 {
            kv_out.push_str(&line[kv_last..]);
            out_lines.push(if kv_changed { kv_out } else { line.to_string() });
            continue;
        }

        if let Some(cap) = re_inline_list.captures(line) {
            let indent = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let key = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let mid = cap.get(3).map(|m| m.as_str()).unwrap_or(": [");
            let inner = cap.get(4).map(|m| m.as_str()).unwrap_or("");
            let suffix = cap.get(5).map(|m| m.as_str()).unwrap_or("]");
            let key_l = key.to_ascii_lowercase();
            if is_display_key(&key_l) {
                let mut new_inner = String::new();
                let mut last = 0usize;
                let mut changed = false;
                for q in re_quoted.captures_iter(inner) {
                    let full = q.get(0).expect("full");
                    let raw = q.get(1).map(|m| m.as_str()).unwrap_or("");
                    let s = unescape_snbt(raw);
                    new_inner.push_str(&inner[last..full.start()]);
                    if let Some((rewritten, n)) = rewrite_json_component_string(&s, map) {
                        new_inner.push('"');
                        new_inner.push_str(&escape_snbt(&rewritten));
                        new_inner.push('"');
                        applied += n;
                        changed = true;
                    } else if let Some(zh) = map.get(&s) {
                        new_inner.push('"');
                        new_inner.push_str(&escape_snbt(zh));
                        new_inner.push('"');
                        applied += 1;
                        changed = true;
                    } else {
                        new_inner.push_str(full.as_str());
                    }
                    last = full.end();
                }
                new_inner.push_str(&inner[last..]);
                if changed {
                    out_lines.push(format!("{indent}{key}{mid}{new_inner}{suffix}"));
                    continue;
                }
            }
            out_lines.push(line.to_string());
            continue;
        }

        if let Some(cap) = re_key_only.captures(line) {
            let key = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_ascii_lowercase();
            if is_display_key(&key) && trimmed.contains('[') {
                in_description_list = true;
                bracket_depth = trimmed.chars().filter(|&c| c == '[').count() as i32
                    - trimmed.chars().filter(|&c| c == ']').count() as i32;
                if bracket_depth <= 0 {
                    in_description_list = false;
                    bracket_depth = 0;
                }
            }
        }
        out_lines.push(line.to_string());
    }

    // 保留原本是否以換行結尾
    let mut joined = out_lines.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    (joined, applied)
}

fn is_display_key(key: &str) -> bool {
    DISPLAY_KEYS.iter().any(|k| *k == key)
}

fn push_display_candidates(s: &str, out: &mut Vec<String>) {
    if let Some(texts) = json_component_texts(s) {
        out.extend(texts);
        return;
    }
    if should_translate_quest_string(s) {
        out.push(s.to_string());
    }
}

fn json_component_texts(s: &str) -> Option<Vec<String>> {
    let t = s.trim();
    if !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    let v: Value = serde_json::from_str(t).ok()?;
    if !looks_like_text_component(&v) {
        return None;
    }
    let mut out = Vec::new();
    collect_component_text_values(&v, &mut out);
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn looks_like_text_component(v: &Value) -> bool {
    match v {
        Value::Object(m) => {
            m.contains_key("text") || m.contains_key("translate") || m.contains_key("extra")
        }
        Value::Array(a) => a.iter().any(looks_like_text_component),
        _ => false,
    }
}

fn collect_component_text_values(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(text)) = m.get("text") {
                if should_translate_quest_string(text) {
                    out.push(text.clone());
                }
            }
            for (k, child) in m {
                if matches!(k.as_str(), "text" | "translate" | "keybind" | "nbt" | "selector") {
                    continue;
                }
                collect_component_text_values(child, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_component_text_values(x, out);
            }
        }
        _ => {}
    }
}

fn rewrite_json_component_string(
    s: &str,
    map: &HashMap<String, String>,
) -> Option<(String, usize)> {
    let t = s.trim();
    if !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    let mut v: Value = serde_json::from_str(t).ok()?;
    if !looks_like_text_component(&v) {
        return None;
    }
    let mut n = 0usize;
    apply_component_text_values(&mut v, map, &mut n);
    if n == 0 {
        return None;
    }
    Some((v.to_string(), n))
}

fn apply_component_text_values(
    v: &mut Value,
    map: &HashMap<String, String>,
    n: &mut usize,
) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(text)) = m.get_mut("text") {
                if let Some(zh) = map.get(text.as_str()) {
                    if zh != text {
                        *text = zh.clone();
                        *n += 1;
                    }
                }
            }
            let keys: Vec<String> = m.keys().cloned().collect();
            for k in keys {
                if matches!(k.as_str(), "text" | "translate" | "keybind" | "nbt" | "selector") {
                    continue;
                }
                if let Some(child) = m.get_mut(&k) {
                    apply_component_text_values(child, map, n);
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                apply_component_text_values(x, map, n);
            }
        }
        _ => {}
    }
}

#[allow(dead_code)]
fn is_struct_key(key: &str) -> bool {
    STRUCT_KEYS.iter().any(|k| *k == key)
}

fn should_translate_quest_string(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 2 {
        return false;
    }
    if t.chars().all(|c| c.is_ascii_hexdigit()) && t.len() >= 8 {
        return false;
    }
    if t.contains(':') && !t.contains(' ') && t.is_ascii() {
        return false;
    }
    if t.starts_with("http://") || t.starts_with("https://") {
        return false;
    }
    if looks_mostly_code(t) {
        return false;
    }
    // 全小寫 snake_case 單 token（checkmark／item／command）不當顯示文翻
    if is_ascii_enum_token(t) || is_resource_path_token(t) {
        return false;
    }
    let has_alpha = t.chars().any(|c| c.is_ascii_alphabetic());
    let has_cjk = looks_chinese(t);
    has_alpha || has_cjk
}

fn is_safe_display_translation(src: &str, zh: &str) -> bool {
    !is_poisoned_mech_translation(src, zh)
        && !(is_ascii_enum_token(src) && looks_chinese(zh))
}

fn looks_chinese(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn has_latin_letter(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
}

fn looks_mostly_code(s: &str) -> bool {
    if s.starts_with('{') && s.contains('}') {
        return true;
    }
    if s.contains("\"id\":") || s.contains("Count:") {
        return true;
    }
    false
}

fn unescape_snbt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\"') => out.push('\"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape_snbt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
	id: "ABC"
	title: "Welcome to Craft to Exile 2!"
	subtitle: "Contents"
	description: [
		"Click the checkmark to finish each page."
		""
		"&eTeam options.&r"
	]
	shape: "circle"
	tasks: [{
		id: "77FDE21B75D712EC"
		title: "Click when done reading"
		type: "checkmark"
	}]
	rewards: [{
		id: "4E506FC73E0B1EAC"
		item: "minecraft:bread"
		title: "Bread"
		type: "item"
	}]
}
"#;

    #[test]
    fn extract_skips_struct_enums_keeps_titles() {
        let got = extract_display_strings(SAMPLE);
        assert!(got.iter().any(|s| s.contains("Welcome")));
        assert!(got.iter().any(|s| s.contains("Click the checkmark")));
        assert!(got.iter().any(|s| s == "Bread"));
        assert!(!got.iter().any(|s| s == "checkmark"));
        assert!(!got.iter().any(|s| s == "item"));
        assert!(!got.iter().any(|s| s == "circle"));
        assert!(!got.iter().any(|s| s == "minecraft:bread"));
    }

    #[test]
    fn rewrite_does_not_touch_type_field() {
        let mut map = HashMap::new();
        map.insert(
            "Click when done reading".into(),
            "閱讀後點擊".into(),
        );
        // 惡意：若誤把 checkmark 放進 map 也不應寫進 type
        map.insert("checkmark".into(), "勾選標記".into());
        let (out, n) = rewrite_display_strings(SAMPLE, &map);
        assert!(n >= 1);
        assert!(out.contains("type: \"checkmark\""));
        assert!(!out.contains("type: \"勾選標記\""));
        assert!(out.contains("title: \"閱讀後點擊\""));
    }

    #[test]
    fn enum_tokens_not_translatable() {
        assert!(!should_translate_quest_string("checkmark"));
        assert!(!should_translate_quest_string("item"));
        assert!(!should_translate_quest_string("command"));
        assert!(!should_translate_quest_string("root.txt"));
        assert!(should_translate_quest_string("Click when done"));
    }

    #[test]
    fn json_component_titles_extract_and_rewrite_text_only() {
        let sample = r#"{
	title: "{\"text\":\"Campaign\",\"color\":\"green\"}"
	id: "abc"
	shape: "circle"
}
"#;
        let got = extract_display_strings(sample);
        assert!(got.iter().any(|s| s == "Campaign"), "{got:?}");
        assert!(!got.iter().any(|s| s.contains("color")), "{got:?}");

        let mut map = HashMap::new();
        map.insert("Campaign".into(), "戰役".into());
        let (out, n) = rewrite_display_strings(sample, &map);
        assert!(n >= 1);
        assert!(out.contains("戰役"), "{out}");
        assert!(out.contains("color"), "{out}");
        assert!(out.contains("shape: \"circle\""), "{out}");

        let translate_only = r#"title: "{\"translate\":\"ftbquests.chapter.foo\"}""#;
        let got2 = extract_display_strings(translate_only);
        assert!(
            !got2.iter().any(|s| s.contains("ftbquests.chapter")),
            "{got2:?}"
        );

        let inline = r##"{ id: "3B0CDB8395E59365", title: "{\"text\":\"Campaign\",\"color\":\"green\"}" }"##;
        let got3 = extract_display_strings(inline);
        assert!(got3.iter().any(|s| s == "Campaign"), "{got3:?}");
        let mut map2 = HashMap::new();
        map2.insert("Campaign".into(), "戰役".into());
        let (out2, n2) = rewrite_display_strings(inline, &map2);
        assert!(n2 >= 1);
        assert!(out2.contains("戰役"), "{out2}");
        assert!(out2.contains("green"), "{out2}");
        assert!(out2.contains("3B0CDB8395E59365"), "{out2}");
    }

    #[test]
    fn inline_description_array_extract_and_rewrite() {
        let sample = r#"{
	title: "&f&l如何：&r&a章節"
	description: ["To open the &2Quest Chapters&r screen, close this quest and move your cursor to the left."]
	shape: "circle"
}
"#;
        let got = extract_display_strings(sample);
        assert!(
            got.iter().any(|s| s.contains("Quest Chapters") && s.contains("&2")),
            "{got:?}"
        );
        assert!(got.iter().any(|s| s.contains("如何")), "{got:?}");

        let mut map = HashMap::new();
        map.insert(
            "To open the &2Quest Chapters&r screen, close this quest and move your cursor to the left."
                .into(),
            "關閉此任務並將游標移到左側，即可開啟&2任務章節&r畫面。".into(),
        );
        let (out, n) = rewrite_display_strings(sample, &map);
        assert!(n >= 1);
        assert!(out.contains("任務章節"), "{out}");
        assert!(out.contains("&2"), "{out}");
        assert!(out.contains("shape: \"circle\""), "{out}");
        assert!(!out.contains("To open the"), "{out}");
    }

    #[test]
    fn ai_gap_skips_pure_chinese_keeps_english() {
        let all = vec![
            "Welcome to the pack!".into(),
            "歡迎來到整合包！".into(),
            "Click the checkmark".into(),
        ];
        let map = HashMap::new();
        let need = strings_needing_ai(&all, &map);
        assert!(need.iter().any(|s| s.contains("Welcome")));
        assert!(need.iter().any(|s| s.contains("Click")));
        assert!(!need.iter().any(|s| s.contains("歡迎")));
    }
}
