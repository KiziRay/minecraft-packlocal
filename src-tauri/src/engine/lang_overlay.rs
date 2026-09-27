//! B3#7：遊戲資料夾裡散落的語言檔（kubejs/assets、openloader、資料夾型資源包…）。
//!
//! 以前覆寫文字把 `en_us.json` 原地翻成中文再套用回遊戲——英文原檔被中文蓋掉，
//! 其他語系（zh_cn、ja_jp）也被原地改寫。現在：
//! - 英文檔（en_us，沒有才用 en_gb）只讀；在同一個 lang 資料夾產出 `zh_tw.<副檔名>`。
//! - 只寫英文檔有的 key。值的優先序：人工 zh_tw（不是本工具寫的）＞ 簡中轉台灣繁 ＞ AI 譯文 ＞ 英文。
//! - 其他語系檔一律不改寫（覆寫文字收檔時先用 [`is_lang_file`] 分出來交給這裡）。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use super::cjk::looks_chinese;
use super::convert::convert_s2tw;

/// `…/lang/<語系>.json|.lang` 形狀的語言檔。
pub(crate) fn is_lang_file(path: &Path) -> bool {
    locale_of(path).is_some()
}

fn locale_of(path: &Path) -> Option<String> {
    let parent = path.parent()?.file_name()?.to_str()?;
    if !parent.eq_ignore_ascii_case("lang") {
        return None;
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext != "json" && ext != "lang" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let (a, b) = stem.split_once('_')?;
    let ok = (2..=3).contains(&a.len())
        && (2..=4).contains(&b.len())
        && a.chars().all(|c| c.is_ascii_lowercase())
        && b.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    ok.then_some(stem)
}

pub(crate) struct LangJob {
    /// 英文檔（遊戲裡的絕對路徑）
    pub en_path: PathBuf,
    /// 實際讀的英文原檔（遊戲裡的原檔或原檔備份）
    pub read_path: PathBuf,
    pub ext: String,
    /// 英文 key → 原文（依 key 排序，輸出穩定）
    pub en: BTreeMap<String, String>,
    /// 已決定的中文：人工 zh_tw 或簡中轉繁
    pub fixed: HashMap<String, String>,
}

fn read_map(path: &Path) -> Option<HashMap<String, String>> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim_start_matches('\u{FEFF}');
    let is_json = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if is_json {
        return super::lenient_json::parse_object_strings(text).ok();
    }
    Some(
        text.lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.to_string())))
            .collect(),
    )
}

/// 從收到的語言檔建立工作：每個 lang 資料夾一份（以英文檔為準）。
pub(crate) fn collect_jobs(mc: &Path, files: &[PathBuf]) -> Vec<LangJob> {
    let mut dirs: BTreeMap<PathBuf, ()> = BTreeMap::new();
    for f in files {
        if let Some(parent) = f.parent() {
            if is_lang_file(f) {
                dirs.insert(parent.to_path_buf(), ());
            }
        }
    }
    let mut jobs = Vec::new();
    let index = super::tool_products::ToolIndex::for_game(mc);
    for dir in dirs.keys() {
        let Some(en_path) = ["en_us.json", "en_us.lang", "en_gb.json", "en_gb.lang"]
            .iter()
            .map(|n| dir.join(n))
            .find(|p| p.is_file())
        else {
            continue;
        };
        // 審查 F2：英文來源必須是原檔（工具版本讀原檔備份；沒有就不翻並列出）
        let read_path = match index.read_source(&en_path) {
            super::tool_products::ReadSource::Use(p) => p,
            super::tool_products::ReadSource::NeedsOriginal => {
                super::output_guard::record_needs_original(&super::apply_record::rel_key(mc, &en_path));
                continue;
            }
        };
        let Some(en) = read_map(&read_path) else { continue };
        let ext = en_path.extension().and_then(|e| e.to_str()).unwrap_or("json").to_ascii_lowercase();
        let mut fixed = HashMap::new();
        // 簡中先放，人工繁中後放（蓋過簡中）
        for (name, human) in [("zh_cn", false), ("zh_tw", true)] {
            for candidate_ext in ["json", "lang"] {
                let path = dir.join(format!("{name}.{candidate_ext}"));
                if !path.is_file() {
                    continue;
                }
                // 人工 zh_tw 必須確認是原檔；簡中也必須讀原檔（舊版工具曾原地改寫 zh_cn）
                let read = if human {
                    if !index.is_original(&path) {
                        continue;
                    }
                    path.clone()
                } else {
                    match index.read_source(&path) {
                        super::tool_products::ReadSource::Use(p) => p,
                        super::tool_products::ReadSource::NeedsOriginal => continue,
                    }
                };
                for (k, v) in read_map(&read).unwrap_or_default() {
                    if en.contains_key(&k) && looks_chinese(&v) {
                        fixed.insert(k, if human { v } else { convert_s2tw(&v) });
                    }
                }
            }
        }
        jobs.push(LangJob { en_path, read_path, ext, en: en.into_iter().collect(), fixed });
    }
    jobs
}

/// 還需要翻的英文（已有人工／簡中的不送）。
pub(crate) fn candidates(jobs: &[LangJob]) -> Vec<String> {
    let mut out = Vec::new();
    for job in jobs {
        for (k, v) in &job.en {
            if !job.fixed.contains_key(k) && should_translate(v) {
                out.push(v.clone());
            }
        }
    }
    out
}

fn should_translate(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().any(|c| c.is_alphabetic()) && !super::mech_tokens::skip_before_ai(t)
}

/// 寫出 zh_tw（相對遊戲資料夾的位置放進 `output_dir`）。`map` 必須已過 output guard。
pub(crate) fn write_outputs(mc: &Path, jobs: &[LangJob], map: &HashMap<String, String>, output_dir: &Path) -> Result<usize, String> {
    let mut written = 0usize;
    for job in jobs {
        let rel_dir = job.en_path.parent().and_then(|p| p.strip_prefix(mc).ok()).map(Path::to_path_buf).unwrap_or_default();
        let file_label = format!("{}/zh_tw.{}", rel_dir.to_string_lossy().replace('\\', "/"), job.ext);
        let mut entries: Vec<(String, String)> = Vec::new();
        let mut any = false;
        for (k, en) in &job.en {
            let value = if let Some(fixed) = job.fixed.get(k) {
                super::output_guard::check_human(&file_label, k, en, fixed)
            } else if let Some(zh) = map.get(en) {
                zh.clone()
            } else {
                en.clone()
            };
            any |= value != *en;
            entries.push((k.clone(), value));
        }
        if !any {
            continue;
        }
        let bytes = if job.ext == "json" {
            let obj: serde_json::Map<String, serde_json::Value> =
                entries.into_iter().map(|(k, v)| (k, serde_json::Value::String(v))).collect();
            serde_json::to_string_pretty(&serde_json::Value::Object(obj)).map_err(|e| e.to_string())? + "\n"
        } else {
            entries.iter().map(|(k, v)| format!("{k}={v}\n")).collect::<String>()
        };
        let original = if job.ext == "json" { "{}".as_bytes() } else { "".as_bytes() };
        let bytes = super::output_guard::finish_file(&file_label, original, bytes.into_bytes());
        if bytes == original {
            continue;
        }
        let out_path = output_dir.join(&rel_dir).join(format!("zh_tw.{}", job.ext));
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&out_path, bytes).map_err(|e| format!("{}: {e}", out_path.display()))?;
        super::text_sources::record(output_dir, &out_path, mc, &job.en_path, &job.read_path, "overlay");
        written += 1;
    }
    Ok(written)
}

/// B3#7 來源階層：`upper`（較上層：資源包 ＞ 鬆散語言檔 ＞ 模組）的非空值蓋過 `into`。
/// 同一層內仍由各自的合併規則決定（jar_scan 的「較長者勝」）。
pub(crate) fn overlay_raw(into: &mut super::jar_scan::RawLang, upper: super::jar_scan::RawLang) {
    for (ns, locales) in upper {
        let ns_entry = into.entry(ns).or_default();
        for (locale, map) in locales {
            let slot = ns_entry.entry(locale).or_default();
            for (k, v) in map {
                if !v.trim().is_empty() {
                    slot.insert(k, v);
                }
            }
        }
    }
}

/// 工具翻過、又找不到原檔的 JAR（審查 F5）：工具只寫 zh_tw，所以只過濾 zh_tw——
/// 留下改寫 JAR 時記下的「模組自帶 key」，其餘（工具補的機翻）丟掉；zh_cn 等作者語系照留。
pub(crate) fn keep_native_zh_tw_only(
    raw: &mut super::jar_scan::RawLang,
    keep: &HashMap<String, std::collections::HashSet<String>>,
) {
    for (ns, locales) in raw.iter_mut() {
        if let Some(tw) = locales.get_mut("zh_tw") {
            let allowed = keep.get(ns);
            tw.retain(|k, _| allowed.is_some_and(|a| a.contains(k)));
        }
        locales.retain(|locale, map| locale != "zh_tw" || !map.is_empty());
    }
}

#[cfg(test)]
#[path = "lang_overlay_tests.rs"]
mod tests;
