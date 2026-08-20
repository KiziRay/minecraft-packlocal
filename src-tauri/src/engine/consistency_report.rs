//! 同 scope 英文原文對應多種譯文的輕量不一致報告（只讀、不改譯文）。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use super::out_layout::ResultLayout;

const MAX_FINDINGS: usize = 50;

/// 掃描語言表：相同英文（normalize）→ ≥2 種 zh → 寫入報告。
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
    let mut entries: Vec<_> = by_en.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (en, variants) in entries {
        if variants.len() < 2 {
            continue;
        }
        let mut zh_list: Vec<_> = variants.keys().cloned().collect();
        zh_list.sort();
        lines.push(format!(
            "「{en}」→ {}（例：{}）",
            zh_list.join(" / "),
            variants
                .values()
                .flatten()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
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
同一英文原文在不同 key 出現 ≥2 種繁中譯文時列出，供人工校對。\n\n{}\n",
        MAX_FINDINGS,
        lines.join("\n")
    );
    fs::write(&path, body).ok()?;
    Some(path)
}

fn normalize_en(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
        let _ = fs::remove_dir_all(&root);
        let _ = PathBuf::from(".");
    }
}
