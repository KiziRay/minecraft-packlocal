//! 模組 JAR 自帶的 zh_tw 語言檔。
//!
//! 翻譯資源包只輸出「工具補的條目」：與模組自帶內容完全相同的條目不再複製一份。
//! 遊戲會把各資源包的語言檔逐條合併，模組自己的 zh_tw 本來就會載入；我們若再抄一份，
//! 模組之後更新翻譯時反而會被這份舊副本蓋掉。

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use super::apply_guard;
use super::apply_knowledge::Knowledge;
use super::apply_record;
use super::jar_scan::LangMap;

/// 讀 `mods/*.jar` 裡的 `assets/<ns>/lang/zh_tw.json`。同一條在不同 JAR 有不同內容時
/// 不列入（遊戲實際用哪份取決於載入順序，保守起見照樣輸出）。讀不到或格式壞掉的略過——
/// 失效方向是「照舊輸出」，不會少翻。
///
/// 工具翻過的 JAR 裡的 zh_tw 有一部分是工具補的，不能拿來排除（原則：無法確定就不排除）：
/// - 新紀錄上、內容是工具版本 → 改讀原檔（新備份區或舊版備份）；沒有原檔就略過這個 JAR。
/// - 舊版清單提過 → 改讀舊版備份的原檔；沒有就略過。
/// - 紀錄讀不出來、被重設或遺失（無法判斷哪些是工具翻過的）→ 完全不排除。
pub fn collect_mod_zh_tw(mc: &Path) -> LangMap {
    // 只讀：翻譯時查的是狀態，不能補回 `.mcpl` 或改紀錄
    let Ok(knowledge) = Knowledge::peek(mc, None) else {
        return LangMap::new();
    };
    if knowledge.uncertain.is_some() {
        return LangMap::new();
    }
    let record = &knowledge.record;
    let backup_dir = apply_record::instance_backup_dir(mc);
    let instance_id = super::mcpl_marker::read_instance(mc).ok().flatten().map(|i| i.id);
    let mut seen: HashMap<String, HashMap<String, Option<String>>> = HashMap::new();
    let Ok(entries) = fs::read_dir(mc.join("mods")) else {
        return LangMap::new();
    };
    let mut jars: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
        })
        .collect();
    jars.sort();
    for jar in jars {
        let rel = apply_record::rel_key(mc, &jar);
        let source = if let Some(entry) = record.files.get(&rel) {
            if record.is_tool_version(&rel, apply_record::file_sha256(&jar).as_deref()) {
                // 原檔要從「屬本包、指回這個檔、指紋相符」的備份讀；對不上就略過這個 JAR
                let original = backup_dir.join(&rel);
                let marker_ok = apply_guard::read_backup_marker(mc, &rel).is_some_and(|m| {
                    m.id == entry.backup_id
                        && Some(m.instance_id.as_str()) == instance_id.as_deref()
                        && apply_record::file_sha256(&original).as_deref() == Some(m.original_sha256.as_str())
                });
                if entry.origin != apply_record::Origin::Known || entry.backup_id.is_empty() || !marker_ok {
                    continue;
                }
                original
            } else {
                jar
            }
        } else if knowledge.legacy_mentions(&rel) {
            match knowledge.legacy_original(&rel) {
                Some((original, _)) => original,
                None => continue,
            }
        } else {
            jar
        };
        for (ns, map) in read_jar_zh_tw(&source) {
            let slot = seen.entry(ns).or_default();
            for (key, value) in map {
                match slot.get(&key) {
                    None => {
                        slot.insert(key, Some(value));
                    }
                    Some(Some(existing)) if *existing != value => {
                        slot.insert(key, None);
                    }
                    _ => {}
                }
            }
        }
    }
    seen.into_iter()
        .map(|(ns, map)| {
            let kept: HashMap<String, String> =
                map.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect();
            (ns, kept)
        })
        .filter(|(_, map)| !map.is_empty())
        .collect()
}

fn read_jar_zh_tw(jar: &Path) -> Vec<(String, HashMap<String, String>)> {
    let mut out = Vec::new();
    let Ok(file) = fs::File::open(jar) else {
        return out;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return out;
    };
    let names: Vec<String> = archive.file_names().map(|n| n.to_string()).collect();
    for name in names {
        let parts: Vec<&str> = name.split('/').collect();
        if parts.len() != 4 || parts[0] != "assets" || parts[2] != "lang" || parts[3] != "zh_tw.json" {
            continue;
        }
        let Ok(mut entry) = archive.by_name(&name) else {
            continue;
        };
        let mut text = String::new();
        if entry.read_to_string(&mut text).is_err() {
            continue;
        }
        let text = text.trim_start_matches('\u{feff}');
        if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(text) {
            out.push((parts[1].to_string(), map));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_jar(path: &Path, files: &[(&str, &str)]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, body) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn tool_translated_jar_is_read_from_the_original_backup() {
        use crate::engine::apply_record::{self, ApplyRecord};
        let root = std::env::temp_dir().join(format!("mcpl-native-tool-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        fs::create_dir_all(mc.join("mods")).unwrap();
        // 先有原檔（只有模組自帶的那條），經前置條件建立有效備份（附標記）
        write_jar(&mc.join("mods/x.jar"), &[("assets/x/lang/zh_tw.json", r#"{"item.own":"模組自帶"}"#)]);
        let ctx = crate::engine::apply_guard::Ctx::begin(&mc).unwrap();
        let backup_id =
            crate::engine::apply_guard::require_original_backup(&ctx, &mc.join("mods/x.jar"), "mods/x.jar", "gx").unwrap();
        // 遊戲裡的 JAR 換成工具翻過的版本（帶工具補的 zh_tw）
        write_jar(&mc.join("mods/x.jar"), &[("assets/x/lang/zh_tw.json", r#"{"item.x":"工具補的","item.own":"模組自帶"}"#)]);
        // 另一個工具翻過、但沒有原檔備份的 JAR：不能拿來排除
        write_jar(&mc.join("mods/y.jar"), &[("assets/y/lang/zh_tw.json", r#"{"item.y":"工具補的"}"#)]);
        let mut record = ApplyRecord::default();
        for jar in ["x.jar", "y.jar"] {
            let rel = format!("mods/{jar}");
            let sha = apply_record::file_sha256(&mc.join("mods").join(jar)).unwrap();
            record.set_entry(&rel, apply_record::FileKind::Overwritten, apply_record::Origin::Known, None, sha);
            if jar == "x.jar" {
                record.files.get_mut(&rel).unwrap().backup_id = backup_id.clone();
            }
        }
        apply_record::save(&mc, &mut record).unwrap();

        let map = collect_mod_zh_tw(&mc);
        assert_eq!(map.get("x").and_then(|m| m.get("item.own")).map(String::as_str), Some("模組自帶"));
        assert!(map.get("x").map(|m| !m.contains_key("item.x")).unwrap_or(true), "工具補的條目下次仍要輸出");
        assert!(!map.contains_key("y"), "讀不到原檔就不排除");
        let _ = fs::remove_dir_all(apply_record::record_dir(&mc));
        let _ = fs::remove_dir_all(apply_record::backup_container(&mc));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_listed_jar_without_original_is_not_used_for_exclusion() {
        let root = std::env::temp_dir().join(format!("mcpl-native-legacy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        fs::create_dir_all(mc.join("mods")).unwrap();
        write_jar(&mc.join("mods/z.jar"), &[("assets/z/lang/zh_tw.json", r#"{"item.z":"舊版工具補的"}"#)]);
        let legacy = root.join("翻譯套用備份_20250101_1");
        fs::create_dir_all(&legacy).unwrap();
        let manifest = serde_json::json!({
            "mc_dir": mc.display().to_string(),
            "added": [],
            "overwritten": ["mods/z.jar"]
        });
        fs::write(legacy.join("套用清單.json"), manifest.to_string()).unwrap();
        let map = collect_mod_zh_tw(&mc);
        assert!(!map.contains_key("z"), "舊版翻過、找不到原檔的 JAR 不能拿來排除");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nothing_is_excluded_when_the_record_was_reset() {
        use crate::engine::apply_record;
        let root = std::env::temp_dir().join(format!("mcpl-native-reset-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        fs::create_dir_all(mc.join("mods")).unwrap();
        write_jar(&mc.join("mods/w.jar"), &[("assets/w/lang/zh_tw.json", r#"{"item.w":"可能是工具補的"}"#)]);
        let dir = apply_record::record_dir(&mc);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{}.broken-1", apply_record::RECORD_FILE)), "壞掉").unwrap();
        assert!(collect_mod_zh_tw(&mc).is_empty(), "紀錄被重設過就無法判斷，寧可全部照樣輸出");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reads_mod_bundled_zh_tw_and_drops_conflicts() {
        let root = std::env::temp_dir().join(format!("mcpl-native-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("mods")).unwrap();
        write_jar(
            &root.join("mods/a.jar"),
            &[("assets/a/lang/zh_tw.json", r#"{"item.a":"甲","shared":"一"}"#), ("assets/a/lang/en_us.json", "{}")],
        );
        write_jar(&root.join("mods/b.jar"), &[("assets/a/lang/zh_tw.json", r#"{"shared":"二"}"#)]);
        let map = collect_mod_zh_tw(&root);
        assert_eq!(map["a"].get("item.a").map(String::as_str), Some("甲"));
        assert!(!map["a"].contains_key("shared"), "兩個 JAR 內容不同的條目不算模組自帶");
        let _ = fs::remove_dir_all(root);
    }
}
