//! KubeJS 顯示字串的安全進階支援。
//!
//! 不解析也不改寫任意腳本邏輯，只處理明確的顯示 API 呼叫（B3#3：清單見 script_scan，依位置替換；
//! CraftTweaker 的 scripts/*.zs 比照處理）。
//! 這讓腳本裡的 UI 提示可以翻譯，同時避免把變數、事件、ID、條件式當成文案。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::cancel;
use super::convert::convert_s2tw_batch;
use super::deepseek::fill_missing_with_ai_with_scope;
use super::jar_scan::LangMap;
use super::script_scan::{find_literals, replace_literals, ScriptKind};
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
    /// B3#3：不上傳共享庫的原文（伺服器腳本、.tell 類）
    pub private_sources: Vec<String>,
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
    // 審查 F1：本輪產出清單（先清掉腳本上一輪的條目）
    super::text_sources::begin(output_dir, "scripts");
    // 審查 F-a：本輪完整跑完才 commit；中途取消或出錯時上一輪的清單保持有效
    let round: Result<_, String> = (|| {
    let index = super::tool_products::ToolIndex::for_game(minecraft_dir);
    let files = collect_script_files(minecraft_dir);
    let mut report = ScriptLiteralReport {
        files_scanned: files.len(),
        ..Default::default()
    };
    if files.is_empty() {
        report.note = "KubeJS 顯示字串：沒有符合安全白名單的腳本".into();
        return Ok(report);
    }

    // B3#3：依位置找出顯示 API 的參數（不再整檔字串取代）
    let mut payloads = Vec::new();
    let mut unique = Vec::new();
    let mut seen = HashMap::new();
    let mut private = std::collections::BTreeSet::new();
    for path in &files {
        cancel::check()?;
        // 審查 F2：讀原檔（遊戲裡是工具改過的腳本就讀原檔備份；沒有就不翻並列出）
        let Some((read, raw)) = super::tool_products::read_game_text(&index, minecraft_dir, path) else {
            continue;
        };
        let kind = if is_zs(path) { ScriptKind::CraftTweaker } else { ScriptKind::KubeJs };
        let literals = find_literals(&raw, kind, is_server_script(minecraft_dir, path));
        for lit in &literals {
            if lit.private {
                private.insert(lit.text.clone());
            }
            if !seen.contains_key(&lit.text) {
                seen.insert(lit.text.clone(), unique.len());
                unique.push(lit.text.clone());
            }
        }
        payloads.push((path.clone(), read, raw, literals));
    }
    report.strings_found = unique.len();
    // 送 AI 前先登記「不上傳」：翻譯途中的自動上傳也會在共用入口被擋
    super::share_policy::mark_private(private.iter());
    report.private_sources = private.into_iter().collect();
    if unique.is_empty() {
        report.note = format!("KubeJS 顯示字串：掃描 {} 個腳本，沒有可翻譯字串", files.len());
        return Ok(report);
    }

    let kube_ns = super::shared_identity::pack_namespace(scope);
    let mut pending = LangMap::new();
    for (index, text) in unique.iter().enumerate() {
        pending
            .entry(kube_ns.clone())
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
    if let Some(entries) = translated.get(&kube_ns) {
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
    super::output_guard::guard_map("KubeJS 腳本", &mut map);
    let shareable: HashMap<String, String> = map
        .iter()
        .filter(|(k, _)| !report.private_sources.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if !shareable.is_empty() {
        let _ = super::shared_tm::contribute_plain_pairs(&shareable, &HashMap::new(), "overlay", scope);
    }

    for (path, read, raw, literals) in payloads {
        cancel::check()?;
        let output = replace_literals(&raw, &literals, |lit| map.get(&lit.text).cloned());
        if output == raw {
            continue;
        }
        let relative = path
            .strip_prefix(minecraft_dir)
            .map_err(|e| e.to_string())?;
        let target = output_dir.join(relative);
        let output = super::output_guard::finish_file(&path.to_string_lossy(), raw.as_bytes(), output.into_bytes());
        if output == raw.as_bytes() {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&target, output).map_err(|e| e.to_string())?;
        super::text_sources::record(output_dir, &target, minecraft_dir, &path, &read, "scripts");
        // 審查 F6：含私有字串的腳本產物，分享包不帶
        if literals.iter().any(|lit| lit.private) {
            super::text_sources::mark_private_output(output_dir, &target);
        }
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
    })();
    if round.is_ok() {
        super::text_sources::commit(output_dir, "scripts", minecraft_dir);
    }
    round
}

fn is_zs(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("zs"))
}

fn is_server_script(mc: &Path, path: &Path) -> bool {
    path.strip_prefix(mc.join("kubejs").join("server_scripts")).is_ok()
}

fn collect_script_files(mc: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // B3#3：CraftTweaker 的 scripts/*.zs 比照處理
    let roots: [(PathBuf, &[&str]); 4] = [
        (mc.join("kubejs").join("client_scripts"), &["js", "ts"]),
        (mc.join("kubejs").join("server_scripts"), &["js", "ts"]),
        (mc.join("kubejs").join("startup_scripts"), &["js", "ts"]),
        (mc.join("scripts"), &["zs"]),
    ];
    for (root, exts) in roots {
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
                    .map(|s| exts.contains(&s.to_ascii_lowercase().as_str()))
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
