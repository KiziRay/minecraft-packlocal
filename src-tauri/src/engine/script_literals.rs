//! KubeJS 顯示字串的安全進階支援。
//!
//! 不解析也不改寫任意 JavaScript 邏輯，只處理明確的顯示 API 呼叫：
//! `Text.of("...")`、`Component.literal("...")`、`text.literal("...")`。
//! 這讓腳本裡的 UI 提示可以翻譯，同時避免把變數、事件、ID、條件式當成文案。

use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::cancel;
use super::convert::convert_s2tw_batch;
use super::deepseek::fill_missing_with_ai_with_scope;
use super::jar_scan::LangMap;
use super::translation_scope::TranslationScope;

const MAX_SCRIPT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SCRIPT_FILES: usize = 5_000;

#[derive(Debug, Clone, Default)]
pub struct ScriptLiteralReport {
    pub files_scanned: usize,
    pub files_written: usize,
    pub strings_found: usize,
    pub strings_translated: usize,
    pub note: String,
}

pub fn translate_kubejs_literals<F>(
    minecraft_dir: &Path,
    output_dir: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<ScriptLiteralReport, String>
where
    F: FnMut(u8, &str),
{
    let files = collect_script_files(minecraft_dir);
    let mut report = ScriptLiteralReport {
        files_scanned: files.len(),
        ..Default::default()
    };
    if files.is_empty() {
        report.note = "KubeJS 顯示字串：沒有符合安全白名單的腳本".into();
        return Ok(report);
    }

    let call_re = Regex::new(
        r#"(?i)(?:Text\.of|Component\.literal|text\.literal)\s*\(\s*("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')\s*\)"#,
    )
    .map_err(|e| e.to_string())?;
    let mut payloads = Vec::new();
    let mut unique = Vec::new();
    let mut seen = HashMap::new();
    for path in &files {
        cancel::check()?;
        let raw = fs::read_to_string(path).map_err(|e| format!("{}：{e}", path.display()))?;
        let literals = call_re
            .captures_iter(&raw)
            .filter_map(|capture| {
                let token = capture.get(1)?.as_str();
                decode_literal(token).map(|text| (token.to_string(), text))
            })
            .collect::<Vec<_>>();
        for (_, text) in &literals {
            if should_translate(text) && !seen.contains_key(text) {
                seen.insert(text.clone(), unique.len());
                unique.push(text.clone());
            }
        }
        payloads.push((path.clone(), raw, literals));
    }
    report.strings_found = unique.len();
    if unique.is_empty() {
        report.note = format!("KubeJS 顯示字串：掃描 {} 個腳本，沒有可翻譯字串", files.len());
        return Ok(report);
    }

    let mut pending = LangMap::new();
    for (index, text) in unique.iter().enumerate() {
        pending
            .entry("__kubejs_script_literals".into())
            .or_default()
            .insert(index.to_string(), text.clone());
    }
    let mut translated = LangMap::new();
    on_progress(20, &format!("KubeJS 顯示字串：準備翻譯 {} 條…", unique.len()));
    let ai_report = fill_missing_with_ai_with_scope(&mut translated, &pending, use_ai, scope, |pct, msg| {
        on_progress(20 + pct.saturating_mul(55) / 100, msg);
    })?;
    // fill_missing 只會新增非中文／可翻譯內容；中文與 glossary 命中也照樣可用。
    let mut map = HashMap::new();
    if let Some(entries) = translated.get("__kubejs_script_literals") {
        for (index, source) in unique.iter().enumerate() {
            if let Some(value) = entries.get(&index.to_string()) {
                if value.trim() != source.trim() && !value.trim().is_empty() {
                    map.insert(source.clone(), value.clone());
                }
            }
        }
    }
    // AI／既有記憶輸出最後仍統一台灣繁體；不會改變腳本語法。
    if !map.is_empty() {
        let keys = map.keys().cloned().collect::<Vec<_>>();
        let values = keys.iter().map(|key| map[key].clone()).collect::<Vec<_>>();
        for (index, value) in convert_s2tw_batch(&values).into_iter().enumerate() {
            if let Some(key) = keys.get(index) {
                map.insert(key.clone(), value);
            }
        }
    }

    for (path, raw, literals) in payloads {
        cancel::check()?;
        let mut output = raw.clone();
        let mut replacements = Vec::new();
        for (token, source) in literals {
            if let Some(value) = map.get(&source) {
                replacements.push((token.clone(), encode_like(&token, value)));
            }
        }
        replacements.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        for (from, to) in replacements {
            output = output.replace(&from, &to);
        }
        if output == raw {
            continue;
        }
        let relative = path
            .strip_prefix(minecraft_dir)
            .map_err(|e| e.to_string())?;
        let target = output_dir.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(target, output).map_err(|e| e.to_string())?;
        report.files_written += 1;
    }
    report.strings_translated = map.len();
    let method_note = if use_ai {
        ai_report.note()
    } else {
        "僅使用本機術語表、翻譯記憶與台灣繁體轉換".into()
    };
    report.note = format!(
        "KubeJS 顯示字串：掃描 {} 個腳本、找到 {} 條、寫出 {} 個檔案、翻譯 {} 條；{}",
        report.files_scanned,
        report.strings_found,
        report.files_written,
        report.strings_translated,
        method_note
    );
    on_progress(100, "KubeJS 顯示字串完成");
    Ok(report)
}

fn collect_script_files(mc: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root_name in ["client_scripts", "server_scripts", "startup_scripts"] {
        let root = mc.join("kubejs").join(root_name);
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root)
            .max_depth(24)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file()
                || !path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| matches!(s.to_ascii_lowercase().as_str(), "js" | "ts"))
                    .unwrap_or(false)
            {
                continue;
            }
            if fs::metadata(path)
                .map(|m| m.len() <= MAX_SCRIPT_BYTES)
                .unwrap_or(false)
            {
                out.push(path.to_path_buf());
            }
            if out.len() >= MAX_SCRIPT_FILES {
                return out;
            }
        }
    }
    out
}

fn should_translate(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && t.chars().any(|c| c.is_alphabetic())
        && !t.contains("minecraft:")
        && !t.contains("#forge:")
}

fn decode_literal(token: &str) -> Option<String> {
    if token.starts_with('"') {
        serde_json::from_str(token).ok()
    } else {
        let body = token.strip_prefix('\'')?.strip_suffix('\'')?;
        Some(
            body.replace("\\'", "'")
                .replace("\\\\", "\\")
                .replace("\\n", "\n")
                .replace("\\t", "\t"),
        )
    }
}

fn encode_like(token: &str, value: &str) -> String {
    if token.starts_with('"') {
        serde_json::to_string(value).unwrap_or_else(|_| token.to_string())
    } else {
        format!(
            "'{}'",
            value
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_display_calls_are_candidates() {
        assert!(should_translate("Hello world"));
        assert!(!should_translate("minecraft:stone"));
        assert_eq!(decode_literal(r#""hello\nworld""#).as_deref(), Some("hello\nworld"));
        assert_eq!(encode_like("'x'", "你好"), "'你好'");
    }
}
