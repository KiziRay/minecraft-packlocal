//! 本機合併參考翻譯包（zip／資料夾）— 不呼叫 AI。
//! 用途：以先前完整 CTE2 繁中包為底，再疊本包掃到的模組語言。

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::ZipArchive;

use super::jar_scan::LangMap;
use super::security::{is_safe_zip_entry_name, MAX_ZIP_ENTRY_BYTES};

/// 從參考資源包讀入全部 zh_tw（本機）
pub fn load_reference_zh_tw(path: &Path) -> Result<(LangMap, usize), String> {
    if path.is_dir() {
        return load_ref_dir(path);
    }
    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "zip" || ext == "jar" {
            return load_ref_zip(path);
        }
    }
    Err(format!(
        "參考包路徑無效（需資料夾或 .zip）：{}",
        path.display()
    ))
}

/// 把 reference 填進 base：base 沒有的 key 才補（本機，不 AI）
/// 回傳補了幾條
pub fn merge_fill_missing(base: &mut LangMap, reference: &LangMap) -> usize {
    let mut n = 0usize;
    for (ns, map) in reference {
        let slot = base.entry(ns.clone()).or_default();
        for (k, v) in map {
            let t = v.trim();
            if t.is_empty() {
                continue;
            }
            if !slot.contains_key(k) {
                slot.insert(k.clone(), v.clone());
                n += 1;
            }
        }
    }
    n
}

/// 從 pending_en 去掉 base 已有的 key
pub fn subtract_covered(pending: &mut LangMap, base: &LangMap) {
    pending.retain(|ns, map| {
        map.retain(|k, _| base.get(ns).and_then(|m| m.get(k)).is_none());
        !map.is_empty()
    });
}

fn load_ref_dir(root: &Path) -> Result<(LangMap, usize), String> {
    let mut zh: LangMap = HashMap::new();
    let mut files = 0usize;
    for e in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name != "zh_tw.json" && name != "zh_cn.json" {
            continue;
        }
        // prefer only zh_tw for reference pack; also allow zh_cn as weaker fill later
        if name != "zh_tw.json" {
            continue;
        }
        let Some(ns) = ns_from_lang_path(p, root) else {
            continue;
        };
        let text = std::fs::read_to_string(p).map_err(|e| e.to_string())?;
        let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&text) else {
            continue;
        };
        if map.is_empty() {
            continue;
        }
        files += 1;
        zh.entry(ns).or_default().extend(map);
    }
    let keys: usize = zh.values().map(|m| m.len()).sum();
    if keys == 0 {
        return Err("參考包裡找不到 zh_tw.json。".into());
    }
    Ok((zh, files))
}

fn load_ref_zip(path: &Path) -> Result<(LangMap, usize), String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut zh: LangMap = HashMap::new();
    let mut files = 0usize;
    for i in 0..zip.len() {
        let mut file = match zip.by_index(i) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let name_raw = file.name().to_string();
        if !is_safe_zip_entry_name(&name_raw) {
            continue;
        }
        let name = name_raw.replace('\\', "/");
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with("zh_tw.json") {
            continue;
        }
        if !lower.contains("/lang/") {
            continue;
        }
        if file.size() > MAX_ZIP_ENTRY_BYTES {
            continue;
        }
        let parts: Vec<&str> = name.split('/').collect();
        let Some(li) = parts.iter().position(|p| *p == "lang") else {
            continue;
        };
        if li == 0 {
            continue;
        }
        let ns = parts[li - 1].to_string();
        if ns.is_empty() || ns == ".." || ns.len() > 64 {
            continue;
        }
        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).is_err() {
            continue;
        }
        let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&String::from_utf8_lossy(&buf))
        else {
            continue;
        };
        if map.is_empty() {
            continue;
        }
        files += 1;
        zh.entry(ns).or_default().extend(map);
    }
    let keys: usize = zh.values().map(|m| m.len()).sum();
    if keys == 0 {
        return Err("參考 zip 裡找不到 zh_tw.json。".into());
    }
    Ok((zh, files))
}

fn ns_from_lang_path(path: &Path, _root: &Path) -> Option<String> {
    // .../assets/<ns>/lang/zh_tw.json
    let parent = path.parent()?; // lang
    let ns_dir = parent.parent()?; // ns
    let ns = ns_dir.file_name()?.to_str()?.to_string();
    if ns.is_empty() || ns.len() > 64 {
        return None;
    }
    Some(ns)
}

/// 常見預設參考包路徑（本機搜尋，不 AI）
pub fn discover_default_reference() -> Option<PathBuf> {
    let candidates = [
        r"C:\Users\jolin\Downloads\zeitfreigame\CTE2TW\CTE2 TW (2.0.3).zip",
        r"C:\Users\jolin\Downloads\zeitfreigame\CTE2\CTE2-繁體中文翻譯包.zip",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    // 目錄內第一個大 zip
    let dir = PathBuf::from(r"C:\Users\jolin\Downloads\zeitfreigame\CTE2TW");
    if dir.is_dir() {
        let mut zips: Vec<_> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("zip"))
                    .unwrap_or(false)
            })
            .collect();
        zips.sort_by_key(|p| std::cmp::Reverse(p.metadata().map(|m| m.len()).unwrap_or(0)));
        if let Some(p) = zips.into_iter().next() {
            return Some(p);
        }
    }
    None
}
