//! 整合包／實例目錄解析與 mods／關鍵檔掃描（佐證，不取代例外鏈）。

use std::fs;
use std::path::{Path, PathBuf};

use super::super::jar_scan::resolve_minecraft_dir;

#[derive(Debug, Clone, Default)]
pub(super) struct FilesystemEvidence {
    pub pack_root: PathBuf,
    pub mod_jars: Vec<String>,
    pub has_rlmixins: bool,
    pub has_srparasites: bool,
    pub jar_summary: String,
}

impl FilesystemEvidence {
    pub(super) fn jar_contains(&self, needle: &str) -> bool {
        let n = needle.to_ascii_lowercase();
        self.mod_jars
            .iter()
            .any(|name| name.to_ascii_lowercase().contains(&n))
    }
}

/// 單行且像本機已存在目錄時，視為 pack_dir 輸入。
pub(super) fn looks_like_existing_pack_path(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
        return false;
    }
    if trimmed.len() < 2 || trimmed.len() > 512 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("exception")
        || lower.contains("caused by")
        || lower.contains("description:")
        || lower.contains("---- minecraft crash")
        || lower.contains("[main/")
        || lower.contains("process exited")
    {
        return false;
    }
    let pathy = trimmed.contains('/')
        || trimmed.contains('\\')
        || (trimmed.len() >= 2 && trimmed.as_bytes().get(1) == Some(&b':'));
    if !pathy {
        return false;
    }
    let path = Path::new(trimmed);
    path.is_dir()
}

/// 解析遊戲資料根並掃描 mods 檔名（僅 basename，不傾倒完整清單給玩家）。
pub(super) fn scan_pack_filesystem(instance_or_mc: &Path) -> Result<FilesystemEvidence, String> {
    if !instance_or_mc.exists() {
        return Err("找不到這個資料夾，請重新選擇。".into());
    }
    if !instance_or_mc.is_dir() {
        return Err("選取的不是資料夾。".into());
    }
    let pack_root = resolve_minecraft_dir(instance_or_mc).unwrap_or_else(|_| instance_or_mc.to_path_buf());
    let mods_dir = pack_root.join("mods");
    if !mods_dir.is_dir() {
        return Err(
            "找不到 mods 資料夾。請選啟動器實例根（含 mods，或 minecraft/mods、.minecraft/mods）。"
                .into(),
        );
    }

    let mut mod_jars = Vec::new();
    if let Ok(entries) = fs::read_dir(&mods_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "jar" || ext == "zip" {
                if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                    mod_jars.push(name.to_string());
                }
            }
        }
    }
    mod_jars.sort();

    let has_rlmixins = mod_jars.iter().any(|name| {
        let lower = name.to_ascii_lowercase();
        lower.contains("rlmixins") || lower.contains("rl-mixins")
    });
    let has_srparasites = mod_jars.iter().any(|name| {
        let lower = name.to_ascii_lowercase();
        lower.contains("srparasites")
            || lower.contains("srp-")
            || lower.contains("scapeandrunparasites")
            || lower.contains("scape_and_run")
    });

    let jar_summary = if mod_jars.is_empty() {
        "mods 內沒有 .jar／.zip。".into()
    } else {
        format!("mods 掃描到 {} 個模組檔。", mod_jars.len())
    };

    Ok(FilesystemEvidence {
        pack_root,
        mod_jars,
        has_rlmixins,
        has_srparasites,
        jar_summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("packlocal-diagnose-{label}-{nanos}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn rejects_multiline_crash_as_path() {
        assert!(!looks_like_existing_pack_path(
            "Caused by: java.lang.NoClassDefFoundError\nat foo.Bar"
        ));
    }

    #[test]
    fn detects_existing_pack_path() {
        let dir = temp_dir("path");
        assert!(looks_like_existing_pack_path(dir.to_str().unwrap()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scans_rlmixins_without_srp() {
        let root = temp_dir("mods");
        let mods = root.join("mods");
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("RLMixins-1.0.jar"), b"x").unwrap();
        fs::write(mods.join("other-mod.jar"), b"x").unwrap();
        let evidence = scan_pack_filesystem(&root).expect("scan");
        assert!(evidence.has_rlmixins);
        assert!(!evidence.has_srparasites);
        let _ = fs::remove_dir_all(&root);
    }
}
