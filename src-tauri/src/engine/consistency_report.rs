//! 同 scope 英文原文對應多種譯文的輕量不一致報告（只讀、不改譯文）。
//! 另寫 JSON 建議檔，供選用併入使用者術語表（opt-in）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::out_layout::ResultLayout;

const MAX_FINDINGS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencySuggestion {
    pub en: String,
    pub variants: Vec<String>,
    /// 預設建議：出現次數最多的譯文（並列時取字典序較前者）。
    pub preferred: String,
    pub sample_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencySuggestionsFile {
    pub version: u32,
    pub note: String,
    pub suggestions: Vec<ConsistencySuggestion>,
}

/// 掃描語言表：相同英文（normalize）→ ≥2 種 zh → 寫入報告與建議 JSON。
pub fn write_consistency_hints(
    layout: &ResultLayout,
    en_by_ns: &HashMap<String, HashMap<String, String>>,
    zh_by_ns: &HashMap<String, HashMap<String, String>>,
) -> Option<PathBuf> {
    let mut by_en: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for (ns, en_map) in en_by_ns {
        let Some(zh_map) = zh_by_ns.get(ns) else {
            continue;
        };
        for (key, en) in en_map {
            let Some(zh) = zh_map.get(key) else {
                continue;
            };
            let norm = normalize_en(en);
            if norm.is_empty() || zh.trim().is_empty() {
                continue;
            }
            by_en
                .entry(norm)
                .or_default()
                .entry(zh.trim().to_string())
                .or_default()
                .push(format!("{ns}:{key}"));
        }
    }

    let mut lines: Vec<String> = Vec::new();
    let mut suggestions: Vec<ConsistencySuggestion> = Vec::new();
    let mut entries: Vec<_> = by_en.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (en, variants) in entries {
        if variants.len() < 2 {
            continue;
        }
        let mut ranked: Vec<(String, usize)> = variants
            .iter()
            .map(|(zh, keys)| (zh.clone(), keys.len()))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let preferred = ranked
            .first()
            .map(|(zh, _)| zh.clone())
            .unwrap_or_default();
        let mut zh_list: Vec<_> = variants.keys().cloned().collect();
        zh_list.sort();
        let sample_keys: Vec<String> = variants
            .values()
            .flatten()
            .take(3)
            .cloned()
            .collect();
        lines.push(format!(
            "「{en}」→ {}（例：{}）",
            zh_list.join(" / "),
            sample_keys.join(", ")
        ));
        suggestions.push(ConsistencySuggestion {
            en,
            variants: zh_list,
            preferred,
            sample_keys,
        });
        if lines.len() >= MAX_FINDINGS {
            break;
        }
    }
    if lines.is_empty() {
        return None;
    }

    let path = layout.work_root.join("用詞不一致提示.txt");
    let body = format!(
        "用詞不一致（僅提示，未改譯文；最多 {} 條）\n\
同一英文原文在不同 key 出現 ≥2 種繁中譯文時列出，供人工校對。\n\
進階：同目錄「用詞不一致建議.json」可選用併入術語表（不會自動寫入）。\n\n{}\n",
        MAX_FINDINGS,
        lines.join("\n")
    );
    fs::write(&path, body).ok()?;

    let json_path = layout.work_root.join("用詞不一致建議.json");
    let payload = ConsistencySuggestionsFile {
        version: 1,
        note: "opt-in：呼叫 merge_consistency_suggestions 才會寫入使用者術語表；preferred 為出現次數最多的譯文。".into(),
        suggestions,
    };
    if let Ok(raw) = serde_json::to_string_pretty(&payload) {
        let _ = fs::write(&json_path, raw + "\n");
    }
    Some(path)
}

/// 將建議 JSON 的 preferred 併入指定術語表（不覆蓋既有鍵，除非 `overwrite`）。
pub fn merge_consistency_suggestions_into(
    suggestions_path: &Path,
    glossary_path: &Path,
    overwrite: bool,
) -> Result<(usize, PathBuf), String> {
    let raw = fs::read_to_string(suggestions_path)
        .map_err(|e| format!("讀取建議檔失敗：{e}"))?;
    let file: ConsistencySuggestionsFile = serde_json::from_str(&raw)
        .map_err(|e| format!("建議檔 JSON 無效：{e}"))?;
    if file.suggestions.is_empty() {
        return Err("建議檔沒有可併入項目".into());
    }

    if let Some(parent) = glossary_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("無法建立術語表目錄：{e}"))?;
    }
    let mut map: HashMap<String, serde_json::Value> = if glossary_path.is_file() {
        let existing = fs::read_to_string(glossary_path).unwrap_or_else(|_| "{}".into());
        serde_json::from_str(&existing).unwrap_or_default()
    } else {
        HashMap::new()
    };

    let mut added = 0usize;
    for item in file.suggestions {
        let en = item.en.trim();
        let zh = item.preferred.trim();
        if en.is_empty() || zh.is_empty() {
            continue;
        }
        if !overwrite {
            if let Some(v) = map.get(en) {
                if v.as_str().map(|s| !s.trim().is_empty()).unwrap_or(true) {
                    continue;
                }
            }
        }
        map.insert(en.to_string(), serde_json::Value::String(zh.to_string()));
        added += 1;
    }

    let pretty = serde_json::to_string_pretty(&map).map_err(|e| format!("序列化術語表失敗：{e}"))?;
    fs::write(glossary_path, pretty + "\n").map_err(|e| format!("寫入術語表失敗：{e}"))?;
    Ok((added, glossary_path.to_path_buf()))
}

/// 將建議 JSON 的 preferred 併入使用者術語表（不覆蓋既有鍵，除非 `overwrite`）。
pub fn merge_consistency_suggestions(
    suggestions_path: &Path,
    overwrite: bool,
) -> Result<(usize, PathBuf), String> {
    merge_consistency_suggestions_into(
        suggestions_path,
        &super::glossary::user_glossary_path(),
        overwrite,
    )
}

/// 結果工作根下的建議檔路徑。
pub fn consistency_suggestions_path(work_root: &Path) -> PathBuf {
    work_root.join("用詞不一致建議.json")
}

/// 若建議檔存在，回傳路徑與建議條數。
pub fn consistency_suggestions_status(work_root: &Path) -> Option<(PathBuf, usize)> {
    let path = consistency_suggestions_path(work_root);
    if !path.is_file() {
        return None;
    }
    let raw = fs::read_to_string(&path).ok()?;
    let file: ConsistencySuggestionsFile = serde_json::from_str(&raw).ok()?;
    Some((path, file.suggestions.len()))
}

fn normalize_en(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_when_same_en_has_two_zh() {
        let root = std::env::temp_dir().join(format!(
            "mcpl-consistency-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let layout = ResultLayout {
            user_base: root.clone(),
            work_root: root.clone(),
            resourcepacks: root.join("resourcepacks"),
            config: root.join("config"),
            minemenu: root.join("minemenu"),
        };
        let mut en = HashMap::new();
        let mut zh = HashMap::new();
        en.insert(
            "a".into(),
            [("k1".into(), "Creeper".into()), ("k2".into(), "creeper".into())]
                .into_iter()
                .collect(),
        );
        zh.insert(
            "a".into(),
            [("k1".into(), "苦力怕".into()), ("k2".into(), "爬行者".into())]
                .into_iter()
                .collect(),
        );
        let note = write_consistency_hints(&layout, &en, &zh).expect("report");
        let text = fs::read_to_string(&note).unwrap();
        assert!(text.contains("苦力怕"));
        assert!(text.contains("爬行者"));
        let json_path = root.join("用詞不一致建議.json");
        assert!(json_path.is_file());
        let payload: ConsistencySuggestionsFile =
            serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(payload.suggestions.len(), 1);
        assert!(
            payload.suggestions[0].variants.contains(&"苦力怕".into())
                && payload.suggestions[0].variants.contains(&"爬行者".into())
        );
        assert!(!payload.suggestions[0].preferred.is_empty());
        let glossary = root.join("user-glossary-test.json");
        let (merged, out) =
            merge_consistency_suggestions_into(&json_path, &glossary, false).expect("merge");
        assert!(merged >= 1);
        assert_eq!(out, glossary);
        let glossary_map: HashMap<String, serde_json::Value> =
            serde_json::from_str(&fs::read_to_string(&glossary).unwrap()).unwrap();
        assert!(glossary_map.contains_key("creeper"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cte2_work_layout_smoke_if_present() {
        let Some(appdata) = std::env::var_os("APPDATA") else {
            return;
        };
        let packs = PathBuf::from(appdata).join("modpack-i18n-tool").join("work").join("packs");
        if !packs.is_dir() {
            return;
        }
        let Ok(entries) = fs::read_dir(&packs) else {
            return;
        };
        let cte = entries.filter_map(|e| e.ok()).find(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.contains("exile") || name.contains("cte2")
        });
        let Some(cte) = cte else {
            return;
        };
        let work = cte.path().join("翻譯結果");
        if !work.is_dir() {
            return;
        }
        let markers = [
            work.join("翻譯工作階段.json"),
            work.join("待補缺口摘要.txt"),
            work.join("覆蓋範圍說明.txt"),
            work.join("用詞不一致提示.txt"),
            work.join("執行日誌.txt"),
        ];
        assert!(
            markers.iter().any(|p| p.is_file()),
            "CTE2 翻譯結果應至少有工作階段／缺口／覆蓋／用詞提示／日誌之一：{}",
            work.display()
        );
        let _ = consistency_suggestions_path(&work);
    }
}
