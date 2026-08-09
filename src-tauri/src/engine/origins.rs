//! Origins / Apoli 能力文字（`data/<ns>/powers|origins|origin_layers/**/*.json`）。
//!
//! 這些不是 lang 檔——`name`／`description` 直接寫字面字串，`jar_scan`（只認 assets/lang）
//! 與 `text_overlay`（patchouli/openloader…）都掃不到，於是能力名稱與說明整片英文。
//!
//! ⚠️ **不能單憑「有 name/description 就翻」**：Apoli 的條件／動作／修飾節點也有 `name`，
//! 但那是**識別字**（例如 `damage_condition.name = "fall"`、attribute modifier 的內部名）。
//! 翻成中文後能力直接失效且不報錯。所以採**路徑感知**：只要祖先鍵是
//! condition／action／modifier／predicate／filter（或 `*_condition` 等後綴），其下的
//! name/description 一律跳過。掃描與寫回共用同一套排除判斷，保證兩邊一致。
//!
//! 只輸出到 work 目錄，不直接改實例。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use walkdir::WalkDir;

use super::convert::convert_s2tw_batch;
use super::deepseek::translate_plain_strings;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_WALK_DEPTH: usize = 16;
const MAX_AI_UNIQUE: usize = 8000;

/// 只翻這兩個欄位（Apoli 玩家可見文字就這兩種）。
const TEXT_FIELDS: &[&str] = &["name", "description"];

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct OriginsResult {
    pub files_written: usize,
    pub strings_translated: usize,
    pub note: String,
}

/// 掃描並翻譯 Origins/Apoli 能力文字，輸出到 output_dir（保留相對路徑）。
pub fn translate_origins<F>(
    minecraft_dir: &Path,
    output_dir: &Path,
    use_ai: bool,
    mut on_progress: F,
) -> Result<OriginsResult, String>
where
    F: FnMut(u8, &str),
{
    on_progress(2, "Origins 能力：掃描 data/powers、origins…");
    let files = collect_origins_files(minecraft_dir);
    if files.is_empty() {
        return Ok(OriginsResult {
            note: "未找到 Origins/Apoli 能力檔（data/powers、origins）。".into(),
            ..Default::default()
        });
    }

    // 讀檔 + 路徑感知擷取可譯字串
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
            &format!("Origins 能力：{} 檔解析失敗已略過（見錯誤日誌）", parse_failures.len()),
        );
    }

    on_progress(
        12,
        &format!("Origins 能力：{} 檔、唯一可譯字串 {}", payloads.len(), unique.len()),
    );

    if unique.is_empty() {
        return Ok(OriginsResult {
            note: format!(
                "掃描 {} 個 Origins/Apoli 檔，無可譯的 name／description（可能已是繁中或都是 id）。",
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
            on_progress(30, &format!("Origins 能力：AI 翻譯 {} 條…", need_ai.len()));
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
        on_progress(50, "Origins 能力：未勾 AI，僅轉換既有中文");
    }

    if map.is_empty() {
        return Ok(OriginsResult {
            note: format!("Origins/Apoli：掃描 {} 檔、唯一字串 {}；無變更。", payloads.len(), unique.len()),
            ..Default::default()
        });
    }

    // 3) 路徑感知寫回（同一套排除判斷，絕不動到 condition/action 的 name）
    on_progress(88, "Origins 能力：寫出檔案…");
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

    on_progress(100, "Origins 能力完成");
    Ok(OriginsResult {
        files_written: written,
        strings_translated: map.len(),
        note: format!(
            "Origins/Apoli：掃描 {} 檔、翻譯表 {} 條、寫出 {} 檔{}。請把輸出目錄對應路徑覆蓋進遊戲。",
            written.max(0),
            map.len(),
            written,
            capped
        ),
    })
}

// ─── 檔案收集 ───────────────────────────────────────────────

fn collect_origins_files(mc: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    // Origins 可能出現的位置：datapacks、kubejs/data、以及實例頂層 data
    let roots = [
        mc.join("datapacks"),
        mc.join("kubejs").join("data"),
        mc.join("data"),
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
            if !is_origins_path(p) {
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

/// 路徑須經過 `data/<ns>/{powers|origins|origin_layers}/`。
fn is_origins_path(path: &Path) -> bool {
    let lower = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let has_data = lower.contains("/data/");
    let has_kind = lower.contains("/powers/")
        || lower.contains("/origins/")
        || lower.contains("/origin_layers/");
    has_data && has_kind
}

// ─── 路徑感知擷取／寫回 ─────────────────────────────────────

/// 祖先鍵屬於這些「機制節點」時，其下的 name/description 是識別字，不可翻。
fn is_excluded_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    matches!(
        k.as_str(),
        "condition"
            | "conditions"
            | "action"
            | "actions"
            | "modifier"
            | "modifiers"
            | "predicate"
            | "predicates"
            | "filter"
            | "filters"
    ) || k.ends_with("_condition")
        || k.ends_with("_conditions")
        || k.ends_with("_action")
        || k.ends_with("_actions")
        || k.ends_with("_modifier")
        || k.ends_with("_modifiers")
}

/// 遍歷 JSON，對「非機制節點下的 name/description 字串」呼叫 `f`。
fn collect_translatable(v: &Value, f: &mut dyn FnMut(&str)) {
    walk(v, false, &mut |s| f(s));
}

fn walk(v: &Value, under_excluded: bool, f: &mut dyn FnMut(&str)) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                if TEXT_FIELDS.contains(&k.as_str()) && !under_excluded {
                    if let Some(s) = child.as_str() {
                        f(s);
                        continue;
                    }
                }
                let child_excluded = under_excluded || is_excluded_key(k);
                walk(child, child_excluded, f);
            }
        }
        Value::Array(a) => {
            for child in a {
                walk(child, under_excluded, f);
            }
        }
        _ => {}
    }
}

/// 用翻譯表就地替換；回傳是否有變更。排除判斷與擷取完全一致。
fn apply_translatable(v: &mut Value, map: &HashMap<String, String>) -> bool {
    apply_walk(v, false, map)
}

fn apply_walk(v: &mut Value, under_excluded: bool, map: &HashMap<String, String>) -> bool {
    let mut changed = false;
    match v {
        Value::Object(m) => {
            for (k, child) in m.iter_mut() {
                let is_text_field = TEXT_FIELDS.contains(&k.as_str());
                if is_text_field && !under_excluded {
                    if let Value::String(s) = child {
                        if let Some(t) = map.get(s.as_str()) {
                            if t != s {
                                *s = t.clone();
                                changed = true;
                            }
                        }
                        continue;
                    }
                }
                let child_excluded = under_excluded || is_excluded_key(k);
                if apply_walk(child, child_excluded, map) {
                    changed = true;
                }
            }
        }
        Value::Array(a) => {
            for child in a.iter_mut() {
                if apply_walk(child, under_excluded, map) {
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
    // 純資源 id / 本地化鍵：minecraft:xxx、origins.power.foo.name
    if t.contains(':')
        && !t.contains(' ')
        && t.is_ascii()
        && t.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '/' | '.' | '-')
        })
    {
        return false;
    }
    // 看起來像本地化鍵：全小寫 + 至少一個點，無空白（origin.foo.name）
    if !t.contains(' ')
        && t.contains('.')
        && t.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
        })
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
    body.push_str("\n======== Origins/Apoli 能力檔解析失敗 ========\n");
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
        collect_translatable(v, &mut |s| out.push(s.to_string()));
        out
    }

    #[test]
    fn extracts_power_name_and_description() {
        let v = json!({
            "type": "apoli:attribute",
            "name": "Iron Lungs",
            "description": "Hold your breath longer."
        });
        let got = collect(&v);
        assert!(got.contains(&"Iron Lungs".to_string()));
        assert!(got.contains(&"Hold your breath longer.".to_string()));
        // type 是 id，不可被抓
        assert!(!got.contains(&"apoli:attribute".to_string()));
    }

    #[test]
    fn does_not_extract_name_inside_condition() {
        // damage_condition.name = "fall" 是識別字，翻了能力就壞
        let v = json!({
            "type": "apoli:action_on_hit",
            "name": "Fall Damage Power",
            "condition": { "type": "apoli:damage", "name": "fall" }
        });
        let got = collect(&v);
        assert!(got.contains(&"Fall Damage Power".to_string()));
        assert!(!got.contains(&"fall".to_string()), "條件內的 name 不可被抓");
    }

    #[test]
    fn excludes_action_modifier_predicate_filter() {
        let v = json!({
            "name": "Real Power",
            "action": { "name": "act_id" },
            "bientity_action": { "name": "bi_id" },
            "modifier": { "name": "mod_id" },
            "predicate": { "name": "pred_id" },
            "item_condition": { "name": "cond_id" }
        });
        let got = collect(&v);
        assert_eq!(got, vec!["Real Power".to_string()]);
    }

    #[test]
    fn nested_powers_translate_but_their_conditions_dont() {
        let v = json!({
            "powers": {
                "a": { "name": "Alpha", "condition": { "name": "id_a" } },
                "b": { "name": "Beta" }
            }
        });
        let mut got = collect(&v);
        got.sort();
        assert_eq!(got, vec!["Alpha".to_string(), "Beta".to_string()]);
    }

    #[test]
    fn apply_only_touches_clean_nodes() {
        let mut v = json!({
            "name": "Speed",
            "condition": { "name": "Speed" }
        });
        let mut map = HashMap::new();
        map.insert("Speed".to_string(), "速度".to_string());
        let changed = apply_translatable(&mut v, &map);
        assert!(changed);
        // 能力名翻了，條件內同字串的 id 不動
        assert_eq!(v["name"], "速度");
        assert_eq!(v["condition"]["name"], "Speed");
    }

    #[test]
    fn should_translate_filters_ids_and_keys() {
        assert!(should_translate("Iron Lungs"));
        assert!(should_translate("Water Breathing"));
        assert!(!should_translate("minecraft:water"));
        assert!(!should_translate("origins.power.foo.name"));
        assert!(!should_translate("a"));
    }

    #[test]
    fn origins_path_recognition() {
        assert!(is_origins_path(Path::new(
            "inst/datapacks/pack/data/origins/powers/fly.json"
        )));
        assert!(is_origins_path(Path::new(
            "inst/kubejs/data/origins/origins/elf.json"
        )));
        assert!(!is_origins_path(Path::new("inst/data/foo/recipes/x.json")));
        assert!(!is_origins_path(Path::new(
            "inst/assets/minecraft/lang/en_us.json"
        )));
    }
}
