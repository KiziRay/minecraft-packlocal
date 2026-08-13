//! 遊戲／整合包實例資料夾驗證：選路徑與一鍵翻譯入口共用。

use serde::Serialize;
use std::fs;
use std::path::Path;

use super::jar_scan::resolve_minecraft_dir;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceValidation {
    pub ok: bool,
    pub mc_dir: String,
    pub reason: String,
    pub missing: Vec<String>,
    pub hints: Vec<String>,
}

impl InstanceValidation {
    fn fail(reason: impl Into<String>, missing: Vec<String>, hints: Vec<String>) -> Self {
        Self {
            ok: false,
            mc_dir: String::new(),
            reason: reason.into(),
            missing,
            hints,
        }
    }

    fn pass(mc: &Path, hints: Vec<String>) -> Self {
        Self {
            ok: true,
            mc_dir: mc.display().to_string(),
            reason: "已確認為可翻譯的遊戲／整合包實例。".into(),
            missing: Vec::new(),
            hints,
        }
    }
}

/// 驗證路徑是否為可翻譯的 Minecraft 實例（對齊 scan／套用所需結構）。
pub fn validate_instance_path(instance_or_mc: &Path) -> InstanceValidation {
    if instance_or_mc.as_os_str().is_empty() {
        return InstanceValidation::fail(
            "尚未選擇遊戲資料夾。",
            vec!["路徑".into()],
            vec!["請選取包含 mods 的實例資料夾。".into()],
        );
    }
    if !instance_or_mc.exists() {
        return InstanceValidation::fail(
            "找不到這個資料夾，請重新選擇。",
            vec!["路徑".into()],
            vec!["確認路徑是否存在（可含空白字元）。".into()],
        );
    }
    if !instance_or_mc.is_dir() {
        return InstanceValidation::fail(
            "選取的不是資料夾。",
            vec!["資料夾".into()],
            vec!["請選取實例根目錄，而非單一檔案。".into()],
        );
    }

    let mc = match resolve_minecraft_dir(instance_or_mc) {
        Ok(mc) => mc,
        Err(e) => {
            return InstanceValidation::fail(
                e,
                vec!["mods".into()],
                vec![
                    "請選啟動器裡的實例資料夾（內有 mods，或 minecraft/mods、.minecraft/mods）。"
                        .into(),
                ],
            );
        }
    };

    let mods = mc.join("mods");
    if !mods.is_dir() {
        return InstanceValidation::fail(
            "找不到 mods 資料夾。",
            vec!["mods".into()],
            vec!["整合包翻譯需要 mods；請確認選對實例。".into()],
        );
    }

    let mut missing = Vec::new();
    let mut hints = Vec::new();

    let jar_count = count_mod_archives(&mods);
    if jar_count == 0 {
        missing.push("mods/*.jar".into());
        hints.push("mods 內沒有模組檔（.jar／.zip），無法進行整合包翻譯。".into());
    }

    let has_config = mc.join("config").is_dir();
    let has_options = mc.join("options.txt").is_file();
    let has_resourcepacks = mc.join("resourcepacks").is_dir();
    let has_saves = mc.join("saves").is_dir();
    let root = instance_or_mc;
    let has_launcher_meta = root.join("instance.cfg").is_file()
        || root.join("mmc-pack.json").is_file()
        || root.join("minecraftinstance.json").is_file()
        || root.join("manifest.json").is_file()
        || mc.join("instance.cfg").is_file()
        || mc.join("minecraftinstance.json").is_file();

    let support_signals = [
        has_config,
        has_options,
        has_resourcepacks,
        has_saves,
        has_launcher_meta,
    ]
    .iter()
    .filter(|&&v| v)
    .count();

    if support_signals == 0 {
        missing.push("實例特徵".into());
        hints.push(
            "未找到 config、options.txt、resourcepacks、saves 或啟動器實例檔（instance.cfg／mmc-pack.json／minecraftinstance.json 等）。"
                .into(),
        );
    }

    if !missing.is_empty() {
        return InstanceValidation::fail(
            "此資料夾不像可翻譯的 Minecraft 整合包實例。",
            missing,
            hints,
        );
    }

    if !has_config {
        hints.push("未找到 config（仍可翻譯；任務／覆寫來源可能較少）。".into());
    }
    if !has_resourcepacks {
        hints.push("未找到 resourcepacks（套用時會建立）。".into());
    }

    InstanceValidation::pass(&mc, hints)
}

fn count_mod_archives(mods: &Path) -> usize {
    let Ok(entries) = fs::read_dir(mods) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let path = entry.path();
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        let lower = ext.to_ascii_lowercase();
                        lower == "jar" || lower == "zip"
                    })
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "modpack-i18n-instance-validate-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn rejects_missing_mods() {
        let root = temp_dir("no-mods");
        let result = validate_instance_path(&root);
        assert!(!result.ok);
        assert!(result.missing.iter().any(|m| m.contains("mods")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_empty_mods_without_signals() {
        let root = temp_dir("empty-mods");
        fs::create_dir_all(root.join("mods")).unwrap();
        let result = validate_instance_path(&root);
        assert!(!result.ok);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_typical_instance() {
        let root = temp_dir("ok-instance");
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(root.join("mods/example.jar"), b"pk").unwrap();
        fs::write(root.join("options.txt"), "lang:en_us\n").unwrap();
        let result = validate_instance_path(&root);
        assert!(result.ok, "{result:?}");
        assert!(result.mc_dir.contains("ok-instance"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accepts_nested_minecraft_mods() {
        let root = temp_dir("nested-mc");
        let mc = root.join("minecraft");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(mc.join("config")).unwrap();
        fs::write(mc.join("mods/mod.jar"), b"pk").unwrap();
        fs::write(root.join("mmc-pack.json"), "{}").unwrap();
        let result = validate_instance_path(&root);
        assert!(result.ok, "{result:?}");
        let _ = fs::remove_dir_all(root);
    }
}
