//! 遊戲資料夾驗證：選資料夾（瀏覽、手動輸入、上次）與開始翻譯共用。
//! 玩家看得到的字（reason、hints）一律用詞表（src/copy/terms.js）的說法，B5d 集中在這裡改。

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
            reason: "遊戲資料夾可以翻譯。".into(),
            missing: Vec::new(),
            hints,
        }
    }
}

/// 找不到 mods 時的原因句（與 jar_scan::resolve_minecraft_dir 同一句）。
pub const NO_MODS_REASON: &str = "這裡找不到 mods，可能選到上一層或下一層。";

impl InstanceValidation {
    /// 逾時或連不到這個資料夾本身（網路磁碟斷線、外接硬碟拔掉）。
    pub fn unreachable(network: bool) -> Self {
        Self::fail(
            if network { "連不到這個網路磁碟上的資料夾，可能是連線不穩。" } else { "連不到這個資料夾（外接硬碟沒接上？）。" },
            vec!["路徑".into()],
            vec!["接上後按「重新檢查」。".into()],
        )
    }
}

/// 驗證路徑是否為可翻譯的遊戲資料夾（對齊掃描與套用所需結構）。
pub fn validate_instance_path(instance_or_mc: &Path) -> InstanceValidation {
    if instance_or_mc.as_os_str().is_empty() {
        return InstanceValidation::fail(
            "尚未選擇遊戲資料夾。",
            vec!["路徑".into()],
            vec!["請選模組整合包的遊戲資料夾（裡面有 mods）。".into()],
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
            "選到的不是資料夾，請重新選擇。",
            vec!["資料夾".into()],
            vec!["請選模組整合包的遊戲資料夾，不是單一檔案。".into()],
        );
    }

    let mc = match resolve_minecraft_dir(instance_or_mc) {
        Ok(mc) => mc,
        Err(e) => {
            return InstanceValidation::fail(
                e,
                vec!["mods".into()],
                vec!["請選模組整合包的遊戲資料夾（裡面有 mods，或 minecraft、.minecraft 裡有 mods）。".into()],
            );
        }
    };

    let mods = mc.join("mods");
    if !mods.is_dir() {
        return InstanceValidation::fail(
            NO_MODS_REASON,
            vec!["mods".into()],
            vec!["請選模組整合包的遊戲資料夾（裡面有 mods）。".into()],
        );
    }

    let mut missing = Vec::new();
    let mut hints = Vec::new();

    let jar_count = count_mod_archives(&mods);
    if jar_count == 0 {
        missing.push("模組檔".into());
        hints.push("mods 裡沒有模組檔，無法翻譯。".into());
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
        missing.push("遊戲資料夾特徵".into());
        hints.push(
            "沒找到 config、options.txt、resourcepacks、saves 或啟動器的設定檔（instance.cfg、mmc-pack.json、minecraftinstance.json 等）。"
                .into(),
        );
    }

    if !missing.is_empty() {
        return InstanceValidation::fail(
            "這個資料夾不像模組整合包的遊戲資料夾，請重新選擇。",
            missing,
            hints,
        );
    }

    if !has_config {
        hints.push("沒找到 config（仍可翻譯；任務與設定檔裡的文字可能較少）。".into());
    }
    if !has_resourcepacks {
        hints.push("沒找到 resourcepacks（套用時會建立）。".into());
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
