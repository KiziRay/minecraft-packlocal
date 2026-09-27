//! B3#6：Origins／Apoli 的 `name`／`description` 也可以是文字元件（JSON 物件或陣列），
//! 例如 `{"text": "Fire Immunity", "color": "gold"}`、`[{"text": "Line 1"}, {"text": "Line 2"}]`。
//! 只翻 `text`（含 `extra` 裡的）；`translate`（語言鍵，已由語言檔翻）、`keybind`、`score`、
//! `selector`、`nbt` 一律不動。

use std::collections::HashMap;

use serde_json::Value;

const SKIP_KEYS: &[&str] = &["translate", "keybind", "score", "selector", "nbt", "font", "color", "insertion", "clickEvent", "hoverEvent"];

/// 是不是文字元件（而不是一般結構）。
pub fn is_component(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.contains_key("text") || m.contains_key("translate") || m.contains_key("extra"),
        Value::Array(a) => !a.is_empty() && a.iter().all(|x| x.is_string() || is_component(x)),
        _ => false,
    }
}

pub fn collect(v: &Value, f: &mut dyn FnMut(&str)) {
    match v {
        Value::String(s) => f(s),
        Value::Object(m) => {
            for (k, child) in m {
                if SKIP_KEYS.contains(&k.as_str()) {
                    continue;
                }
                if k == "text" {
                    if let Some(s) = child.as_str() {
                        f(s);
                    }
                } else if k == "extra" {
                    collect(child, f);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect(x, f)),
        _ => {}
    }
}

pub fn apply(v: &mut Value, map: &HashMap<String, String>) -> bool {
    match v {
        Value::String(s) => match map.get(s.as_str()) {
            Some(t) if t != s => {
                *s = t.clone();
                true
            }
            _ => false,
        },
        Value::Object(m) => {
            let mut changed = false;
            for (k, child) in m.iter_mut() {
                if k == "text" && child.is_string() || k == "extra" {
                    changed |= apply(child, map);
                }
            }
            changed
        }
        Value::Array(a) => {
            let mut changed = false;
            for x in a.iter_mut() {
                changed |= apply(x, map);
            }
            changed
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tool_products::test_support::temp_game;
    use std::io::Write;

    #[test]
    fn b3_origins_text_component_names_are_collected_and_applied() {
        let mut v: Value = serde_json::from_str(
            r#"{"name":{"text":"Fire Immunity","color":"gold"},"description":[{"text":"Line one"},{"translate":"x.y"}]}"#,
        )
        .unwrap();
        let mut got = Vec::new();
        for field in ["name", "description"] {
            assert!(is_component(&v[field]));
            collect(&v[field], &mut |s| got.push(s.to_string()));
        }
        assert_eq!(got, vec!["Fire Immunity", "Line one"]);
        let map = HashMap::from([("Fire Immunity".to_string(), "火焰免疫".to_string())]);
        assert!(apply(&mut v["name"], &map));
        assert_eq!(v["name"]["text"], "火焰免疫");
        assert_eq!(v["name"]["color"], "gold");
        assert_eq!(v["description"][1]["translate"], "x.y");
    }

    #[test]
    fn b3_origins_in_datapack_zip_are_translated() {
        let mc = temp_game("originszip");
        let work = temp_game("originszip-work");
        std::fs::create_dir_all(mc.join("datapacks")).unwrap();
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(mc.join("datapacks/o.zip")).unwrap());
            zip.start_file("data/ns/powers/p.json", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(r#"{"name":{"text":"火焰免疫简体"},"condition":{"name":"fall"}}"#.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        crate::engine::archive_overlay::translate_archive_overlays(&mc, &work, false, None, |_, _| {}).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(work.join("datapacks/o.zip")).expect("要重建 zip")).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("data/ns/powers/p.json").unwrap(), &mut text).unwrap();
        assert!(text.contains("火焰免疫簡體"), "{text}");
        assert!(text.contains("\"fall\""), "條件節點的 name 不動：{text}");
        let _ = std::fs::remove_dir_all(&mc);
        let _ = std::fs::remove_dir_all(&work);
    }
}
