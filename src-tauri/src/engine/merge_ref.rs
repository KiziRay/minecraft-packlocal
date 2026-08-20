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
    use super::translation_quality::is_usable_zh;
    let mut n = 0usize;
    for (ns, map) in reference {
        let slot = base.entry(ns.clone()).or_default();
        for (k, v) in map {
            let t = v.trim();
            if t.is_empty() {
                continue;
            }
            if !is_usable_zh("", v) {
                continue;
            }
            if !slot.contains_key(k) {
                slot.insert(k.clone(), v.clone());
                n += 1;
            } else if let Some(cur) = slot.get(k) {
                if !is_usable_zh("", cur) {
                    slot.insert(k.clone(), v.clone());
                    n += 1;
                }
            }
        }
    }
    n
}

/// 從 pending_en 去掉 base 已有「有效中文」的 key
pub fn subtract_covered(pending: &mut LangMap, base: &LangMap) {
    use super::translation_quality::is_usable_zh;
    pending.retain(|ns, map| {
        map.retain(|k, en| {
            match base.get(ns).and_then(|m| m.get(k)) {
                Some(zh) => !is_usable_zh(en, zh),
                None => true,
            }
        });
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
        let slot = zh.entry(ns).or_default();
        if name == "zh_tw.json" {
            slot.extend(map);
        } else {
            for (key, value) in map {
                slot.entry(key).or_insert(value);
            }
        }
    }
    let keys: usize = zh.values().map(|m| m.len()).sum();
    if keys == 0 {
        return Err("參考包裡找不到 zh_tw.json 或 zh_cn.json。".into());
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
        if !lower.ends_with("zh_tw.json") && !lower.ends_with("zh_cn.json") {
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
        let slot = zh.entry(ns).or_default();
        if lower.ends_with("zh_tw.json") {
            slot.extend(map);
        } else {
            for (key, value) in map {
                slot.entry(key).or_insert(value);
            }
        }
    }
    let keys: usize = zh.values().map(|m| m.len()).sum();
    if keys == 0 {
        return Err("參考 zip 裡找不到 zh_tw.json 或 zh_cn.json。".into());
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

/// 在常見資料夾與各磁碟的淺層路徑尋找 CTE2／繁中參考包。
///
/// 不寫死任何使用者名稱或工作區路徑。對不在常見資料夾的參考包，
/// 前端仍提供手動選取資料夾，避免為了自動搜尋掃描整顆磁碟。
pub fn discover_default_reference() -> Option<PathBuf> {
    let mut roots = Vec::new();
    for root in [
        dirs::download_dir(),
        dirs::document_dir(),
        dirs::desktop_dir(),
        std::env::current_dir().ok(),
    ]
    .into_iter()
    .flatten()
    {
        roots.push(root);
    }

    // 支援像 D:\\Down\\ccc\\CTE2 這種自訂磁碟路徑，但只看常見的第一層資料夾。
    for drive in b'A'..=b'Z' {
        let letter = drive as char;
        for name in ["Downloads", "Download", "Down", "Games", "Mods", "Projects"] {
            roots.push(PathBuf::from(format!("{}:\\{}", letter, name)));
        }
    }

    let mut candidates = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .max_depth(5)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let lower = name.to_ascii_lowercase();
            let looks_like_reference = lower.contains("cte2")
                || lower.contains("cfpa")
                || lower.contains("zh_tw")
                || lower.contains("zh_cn")
                || lower.contains("resourcepack")
                || lower.contains("resource-pack")
                || lower.contains("chinese")
                || name.contains("繁體")
                || name.contains("繁中")
                || name.contains("漢化")
                || name.contains("汉化")
                || name.contains("翻譯")
                || name.contains("翻译")
                || name.contains("僅翻譯");
            if !looks_like_reference {
                continue;
            }
            if path.is_file()
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("zip") || s.eq_ignore_ascii_case("jar"))
                    .unwrap_or(false)
            {
                candidates.push(path.to_path_buf());
            } else if path.is_dir() && has_reference_lang(path) {
                candidates.push(path.to_path_buf());
            }
        }
    }

    candidates.sort_by_key(|path| {
        (
            !path
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase().contains("cte2"))
                .unwrap_or(false),
            std::cmp::Reverse(path.metadata().map(|m| m.len()).unwrap_or(0)),
        )
    });
    candidates.into_iter().next()
}

fn has_reference_lang(root: &Path) -> bool {
    WalkDir::new(root)
        .max_depth(8)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_str()
                    .map(|name| {
                        name.eq_ignore_ascii_case("zh_tw.json")
                            || name.eq_ignore_ascii_case("zh_cn.json")
                    })
                    .unwrap_or(false)
        })
}

/// 依 MC 版本候選標籤（精確 → 大版本），供 CFPA release 比對。
pub fn cfpa_version_candidates(mc_version: &str) -> Vec<String> {
    let v = mc_version.trim();
    if v.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    out.push(v.to_string());
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() >= 2 {
        let major_minor = format!("{}.{}", parts[0], parts[1]);
        if major_minor != v {
            out.push(major_minor);
        }
    }
    out
}

fn is_allowed_cfpa_download_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://github.com/")
        || lower.starts_with("https://objects.githubusercontent.com/")
        || lower.starts_with("https://release-assets.githubusercontent.com/"))
        && !lower.contains("..")
}

/// 嘗試從 CFPA GitHub Releases 下載對應 MC 大版本的簡中資源包 zip。
/// 失敗回 Err（呼叫端應略過、不擋主流程）；成功回本機 zip 路徑。
pub fn try_download_cfpa_pack(mc_version: &str, dest_dir: &Path) -> Result<PathBuf, String> {
    let candidates = cfpa_version_candidates(mc_version);
    if candidates.is_empty() {
        return Err("尚未指定 Minecraft 版本，無法下載 CFPA。".into());
    }
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("無法建立下載目錄：{e}"))?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .user_agent("modpack-i18n-tool/0.1.2 (+https://github.com/KiziRay/minecraft-packlocal)")
        .build()
        .map_err(|e| format!("建立下載客戶端失敗：{e}"))?;

    let api = "https://api.github.com/repos/CFPAOrg/Minecraft-Mod-Language-Package/releases?per_page=40";
    let releases: serde_json::Value = client
        .get(api)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("連線 CFPA Releases 失敗（可改本機選 zip）：{e}"))?
        .error_for_status()
        .map_err(|e| format!("讀取 CFPA Releases 失敗：{e}"))?
        .json()
        .map_err(|e| format!("解析 CFPA Releases 失敗：{e}"))?;

    let list = releases
        .as_array()
        .ok_or_else(|| "CFPA Releases 回傳格式異常。".to_string())?;

    for cand in &candidates {
        let cand_l = cand.to_ascii_lowercase();
        for rel in list {
            let tag = rel
                .get("tag_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let name = rel
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !(tag.contains(&cand_l) || name.contains(&cand_l)) {
                continue;
            }
            let assets = rel.get("assets").and_then(|v| v.as_array());
            let Some(assets) = assets else { continue };
            for asset in assets {
                let asset_name = asset
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let lower_name = asset_name.to_ascii_lowercase();
                if !lower_name.ends_with(".zip") {
                    continue;
                }
                let url = asset
                    .get("browser_download_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !is_allowed_cfpa_download_url(url) {
                    continue;
                }
                let safe_name = format!(
                    "cfpa-{}-{}.zip",
                    cand.replace('.', "_"),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                );
                let dest = dest_dir.join(safe_name);
                let bytes = client
                    .get(url)
                    .timeout(std::time::Duration::from_secs(180))
                    .send()
                    .map_err(|e| format!("下載 CFPA zip 失敗：{e}"))?
                    .error_for_status()
                    .map_err(|e| format!("下載 CFPA zip HTTP 錯誤：{e}"))?
                    .bytes()
                    .map_err(|e| format!("讀取 CFPA zip 失敗：{e}"))?;
                if bytes.len() < 64 || bytes.len() > 900 * 1024 * 1024 {
                    return Err("CFPA zip 大小異常，已略過。".into());
                }
                if !(bytes.starts_with(b"PK")) {
                    return Err("下載內容不是有效 zip。".into());
                }
                std::fs::write(&dest, &bytes).map_err(|e| format!("寫入 CFPA zip 失敗：{e}"))?;
                return Ok(dest);
            }
        }
    }
    Err(format!(
        "找不到對應 {} 的 CFPA Release zip（可改本機選檔）。",
        candidates.join(" / ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_cn_reference_fills_missing_without_overwriting_zh_tw() {
        let root = std::env::temp_dir().join(format!("merge_ref_cfpa_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let lang = root.join("assets/example/lang");
        std::fs::create_dir_all(&lang).unwrap();
        std::fs::write(
            lang.join("zh_cn.json"),
            r#"{"item.example.a":"简中 A","item.example.b":"简中 B"}"#,
        )
        .unwrap();
        std::fs::write(lang.join("zh_tw.json"), r#"{"item.example.a":"繁中 A"}"#).unwrap();

        let (reference, files) = load_reference_zh_tw(&root).unwrap();
        assert_eq!(files, 2);
        let namespace = reference.get("example").unwrap();
        assert_eq!(namespace.get("item.example.a").unwrap(), "繁中 A");
        assert_eq!(namespace.get("item.example.b").unwrap(), "简中 B");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfpa_version_candidates_prefer_exact_then_major_minor() {
        assert_eq!(
            cfpa_version_candidates("1.20.1"),
            vec!["1.20.1".to_string(), "1.20".to_string()]
        );
        assert!(cfpa_version_candidates("").is_empty());
    }

    #[test]
    fn cfpa_download_url_allowlist() {
        assert!(is_allowed_cfpa_download_url(
            "https://github.com/CFPAOrg/Minecraft-Mod-Language-Package/releases/download/1.20.1/x.zip"
        ));
        assert!(!is_allowed_cfpa_download_url("http://evil.example/x.zip"));
        assert!(!is_allowed_cfpa_download_url("https://evil.example/x.zip"));
    }
}
