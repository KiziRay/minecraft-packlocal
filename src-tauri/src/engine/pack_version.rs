//! Detects the version of the modpack/resource pack being translated.
//!
//! This is deliberately separate from the application version.  A translated
//! resource pack must keep following the modpack version, even when the tool
//! itself receives a patch release.

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

use super::jar_scan::resolve_minecraft_dir;
use super::security::sanitize_folder_name;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackVersionInfo {
    pub version: String,
    pub pack_name: String,
    /// 整合包本身的名稱；與輸出的翻譯資源包名稱分開。
    pub modpack_name: String,
    pub source: String,
    pub metadata_path: Option<String>,
}

pub fn detect_pack_version(instance_or_minecraft: &Path) -> PackVersionInfo {
    let mc = resolve_minecraft_dir(instance_or_minecraft)
        .unwrap_or_else(|_| instance_or_minecraft.to_path_buf());
    // 使用者可能選實例根目錄，也可能直接選 minecraft；分類名稱要取
    // 實際選取的實例名稱，不能在暫存／共用父目錄名稱上分類。
    let selected_name = instance_or_minecraft
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let selected_direct_minecraft = selected_name.eq_ignore_ascii_case("minecraft")
        || selected_name.eq_ignore_ascii_case(".minecraft");
    let root = if selected_direct_minecraft {
        mc.parent().unwrap_or(&mc)
    } else {
        instance_or_minecraft
    };

    let candidates = [
        (root.join("manifest.json"), "CurseForge manifest"),
        (root.join("modrinth.index.json"), "Modrinth index"),
        (root.join("profile.json"), "pack profile"),
        (root.join("pack.json"), "pack metadata"),
        (root.join("instance.json"), "instance metadata"),
        (root.join("mmc-pack.json"), "Prism instance metadata"),
        (mc.join("manifest.json"), "Minecraft manifest"),
        (mc.join("modrinth.index.json"), "Minecraft Modrinth index"),
    ];

    for (path, source) in candidates {
        if let Some(version) = read_version(&path) {
            return PackVersionInfo {
                version: safe_version(&version),
                pack_name: String::new(),
                modpack_name: read_pack_name(&path).unwrap_or_else(|| fallback_pack_name(root)),
                source: source.to_string(),
                metadata_path: Some(path.display().to_string()),
            };
        }
    }

    PackVersionInfo {
        version: "R1".to_string(),
        pack_name: String::new(),
        modpack_name: fallback_pack_name(root),
        source: "首次翻譯（找不到整合包版本檔）".to_string(),
        metadata_path: None,
    }
}

pub fn build_pack_name(instance_or_minecraft: &Path) -> (String, PackVersionInfo) {
    let mut info = detect_pack_version(instance_or_minecraft);
    let (month, day) = today_month_day();
    let raw = format!("模組包翻譯工具+{month:02}{day:02}+{}", info.version);
    let safe = sanitize_folder_name(&raw).unwrap_or(raw);
    info.pack_name = safe.clone();
    (safe, info)
}

fn read_version(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    for key in ["versionId", "version_id", "version", "release", "packVersion"] {
        if let Some(value) = json.get(key).and_then(Value::as_str) {
            if is_useful_version(value) {
                return Some(value.to_string());
            }
        }
    }
    for parent_key in ["pack", "metadata", "project"] {
        if let Some(parent) = json.get(parent_key) {
            for key in ["versionId", "version_id", "version", "release"] {
                if let Some(value) = parent.get(key).and_then(Value::as_str) {
                    if is_useful_version(value) {
                        return Some(value.to_string());
                    }
                }
            }
        }
    }
    None
}

fn read_pack_name(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    for key in ["name", "displayName", "display_name", "title"] {
        if let Some(value) = json.get(key).and_then(Value::as_str) {
            if is_useful_name(value) {
                return Some(value.trim().to_string());
            }
        }
    }
    for parent_key in ["pack", "metadata", "project"] {
        if let Some(parent) = json.get(parent_key) {
            for key in ["name", "displayName", "display_name", "title"] {
                if let Some(value) = parent.get(key).and_then(Value::as_str) {
                    if is_useful_name(value) {
                        return Some(value.trim().to_string());
                    }
                }
            }
        }
    }
    None
}

fn fallback_pack_name(root: &Path) -> String {
    root.file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("未命名整合包")
        .chars()
        .take(120)
        .collect()
}

fn is_useful_name(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.len() <= 160 && !value.contains(['\r', '\n', '\0'])
}

fn is_useful_version(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 80
        && !value.eq_ignore_ascii_case("latest")
        && !value.eq_ignore_ascii_case("unknown")
}

fn safe_version(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('_');
    if cleaned.is_empty() {
        "R1".to_string()
    } else {
        cleaned.chars().take(48).collect()
    }
}

fn today_month_day() -> (u32, u32) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + 8 * 60 * 60;
    let days = (seconds / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let _year = if month <= 2 { y + 1 } else { y };
    (month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pack_version_{name}_{}", std::process::id()))
    }

    #[test]
    fn reads_curseforge_version_without_using_tool_version() {
        let root = temp_root("curseforge");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("minecraft")).unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{"name":"Example","version":"2.4.1","minecraft":{"version":"1.20.1"}}"#,
        )
        .unwrap();
        let info = detect_pack_version(&root);
        assert_eq!(info.version, "2.4.1");
        assert_eq!(info.modpack_name, "Example");
        assert!(!info.version.contains("1.0.2"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn falls_back_to_review_one_when_pack_version_is_missing() {
        let root = temp_root("fallback");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("minecraft")).unwrap();
        let info = detect_pack_version(&root);
        assert_eq!(info.version, "R1");
        assert_eq!(info.modpack_name, root.file_name().unwrap().to_string_lossy());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pack_name_contains_date_and_detected_pack_version() {
        let root = temp_root("name");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("minecraft")).unwrap();
        fs::write(root.join("modrinth.index.json"), r#"{"versionId":"release-7"}"#).unwrap();
        let (name, _) = build_pack_name(&root);
        assert!(name.starts_with("模組包翻譯工具+"));
        assert!(name.ends_with("+release-7"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn direct_minecraft_selection_uses_instance_name_for_fallback() {
        let root = temp_root("direct-minecraft");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("minecraft").join("mods")).unwrap();
        let info = detect_pack_version(&root.join("minecraft"));
        assert_eq!(info.modpack_name, root.file_name().unwrap().to_string_lossy());
        let _ = fs::remove_dir_all(&root);
    }
}
