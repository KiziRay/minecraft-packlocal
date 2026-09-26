//! JAR 內 Origins／Apoli 能力檔的非破壞式翻譯。
//!
//! `origins.rs` 只掃鬆散的資料包根（`datapacks`、`config/openloader`、`data`…），
//! **模組把 powers 打包在自己的 JAR 裡時完全掃不到**。Origins 系整合包
//! （Craft to Exile 2 之類）幾乎都是後者，能力名稱與說明會整片留在英文。
//!
//! 做法沿用 `jar_patchouli.rs` 已經驗證過的三段式：
//! 抽出到暫存區 → 翻譯 → 嵌回 `jar-translated` 副本；原始 JAR 不動。
//!
//! **關鍵**：翻譯那一步直接呼叫 `origins::translate_origins()`，而不是丟給通用的
//! 文字覆寫掃描器。因為 Origins 的 `name`／`description` 在條件節點底下是
//! **識別字**（例如 `damage_condition.name = "fall"`），翻成中文會讓能力靜默失效。
//! `origins.rs` 已經有 `*_condition`／`*_action`／`*_modifier`／`predicate`／`filter`
//! 的排除規則，重用它才不會重蹈覆轍。

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::ZipArchive;

use super::cancel;
use super::jar_display::{is_signature_path, jar_key, rebuild_jar};
use super::jar_scan::resolve_minecraft_dir;
use super::security::{check_jar_size, is_safe_zip_entry_name, MAX_ZIP_ENTRY_BYTES};
use super::translation_scope::TranslationScope;

#[derive(Debug, Clone, Default)]
pub struct JarOriginsReport {
    pub jars_scanned: usize,
    pub files_found: usize,
    pub strings_translated: usize,
    pub files_written: usize,
    pub skipped: Vec<String>,
    pub note: String,
}

struct ExtractedJar {
    jar_key: String,
    source_jar: PathBuf,
    entries: usize,
}

pub fn translate_jar_origins<F>(
    instance_or_mc: &Path,
    work_root: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<JarOriginsReport, String>
where
    F: FnMut(u8, &str),
{
    let mc = resolve_minecraft_dir(instance_or_mc)?;
    let mods = mc.join("mods");
    let jars = list_jars(&mods);
    let mut report = JarOriginsReport {
        jars_scanned: jars.len(),
        ..Default::default()
    };
    if jars.is_empty() {
        report.note = "JAR 內 Origins：沒有找到模組 JAR".into();
        return Ok(report);
    }

    let stage_root = work_root.join(".jar-origins-stage");
    let translated_root = work_root.join(".jar-origins-translated");
    let _ = fs::remove_dir_all(&stage_root);
    let _ = fs::remove_dir_all(&translated_root);
    fs::create_dir_all(&stage_root).map_err(|e| e.to_string())?;
    let _cleanup = TempDirGuard(vec![stage_root.clone(), translated_root.clone()]);

    let mut extracted: Vec<ExtractedJar> = Vec::new();
    for (index, jar) in jars.iter().enumerate() {
        cancel::check()?;
        on_progress(
            1 + ((index * 35) / jars.len().max(1)) as u8,
            &format!("JAR 能力檔：模組 {}/{}", index + 1, jars.len()),
        );
        match extract_origins(jar, &stage_root) {
            Ok(Some(item)) => {
                report.files_found += item.entries;
                extracted.push(item);
            }
            Ok(None) => {}
            Err(error) => report.skipped.push(format!("{}：{error}", jar.display())),
        }
    }
    if extracted.is_empty() {
        report.note = "JAR 內 Origins：沒有找到 data/*/powers|origins 能力檔".into();
        return Ok(report);
    }

    // 每個 JAR 的抽出內容各自是一個「假的 minecraft 目錄」，直接交給既有的
    // origins 翻譯流程處理——排除規則（condition／action／modifier）由它負責。
    for (index, item) in extracted.iter().enumerate() {
        cancel::check()?;
        let stage = stage_root.join("jars").join(&item.jar_key);
        let out = translated_root.join("jars").join(&item.jar_key);
        fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let sub = super::origins::translate_origins(&stage, &out, use_ai, scope, |pct, msg| {
            on_progress(
                38 + (((index * 100 + pct as usize) * 45) / (extracted.len() * 100).max(1)) as u8,
                msg,
            );
        })?;
        report.strings_translated += sub.strings_translated;
    }

    for (index, item) in extracted.iter().enumerate() {
        cancel::check()?;
        // origins 的輸出落在 out/data/...，剛好就是 JAR 內的相對路徑
        let translated = translated_root.join("jars").join(&item.jar_key);
        if !translated.is_dir() || dir_is_empty(&translated) {
            continue;
        }
        let relative = item
            .source_jar
            .strip_prefix(&mods)
            .map_err(|e| format!("JAR 不在 mods 根目錄：{e}"))?;
        let output = work_root.join("jar-translated").join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // 這個 JAR 可能已經被 Patchouli／顯示文字流程重建過，要接著疊加而不是覆蓋
        let base = if output.is_file() {
            output.as_path()
        } else {
            item.source_jar.as_path()
        };
        let stage = stage_root.join("jars").join(&item.jar_key);
        match rebuild_jar(base, &output, &translated, &stage) {
            Ok(()) => {
                report.files_written += count_files(&translated);
                if let Err(e) = super::jar_sources::record_source(work_root, relative, &item.source_jar) {
                    report.skipped.push(format!("{}：{e}", item.source_jar.display()));
                }
            }
            Err(error) => report
                .skipped
                .push(format!("{}：{error}", item.source_jar.display())),
        }
        on_progress(
            85 + ((index * 12) / extracted.len().max(1)) as u8,
            &format!("JAR 能力檔：重建模組 {}/{}", index + 1, extracted.len()),
        );
    }

    report.note = format!(
        "JAR 內 Origins：掃描 {} 個 JAR、找到 {} 個能力檔，翻譯 {} 條，寫入 jar-translated 副本（不改原 jar）{}",
        report.jars_scanned,
        report.files_found,
        report.strings_translated,
        if report.skipped.is_empty() {
            String::new()
        } else {
            format!("；{} 個 JAR 略過", report.skipped.len())
        }
    );
    on_progress(100, "JAR 內 Origins 完成");
    Ok(report)
}

struct TempDirGuard(Vec<PathBuf>);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
}

fn list_jars(mods: &Path) -> Vec<PathBuf> {
    if !mods.is_dir() {
        return Vec::new();
    }
    WalkDir::new(mods)
        .min_depth(1)
        .max_depth(2)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("jar"))
                    .unwrap_or(false)
        })
        .collect()
}

fn extract_origins(source_jar: &Path, stage_root: &Path) -> Result<Option<ExtractedJar>, String> {
    check_jar_size(source_jar)?;
    let file = File::open(source_jar).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("JAR 不是有效 ZIP：{e}"))?;
    let mut has_signature = false;
    let mut entries = 0usize;
    let key = jar_key(source_jar);
    let stage = stage_root.join("jars").join(&key);
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

    for index in 0..archive.len() {
        cancel::check()?;
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if !is_safe_zip_entry_name(&name) {
            return Err(format!("包含不安全的 ZIP 路徑：{name}"));
        }
        if is_signature_path(&name) {
            has_signature = true;
        }
        if !is_origins_entry(&name) || entry.size() > MAX_ZIP_ENTRY_BYTES {
            continue;
        }
        let target = stage.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.contains(&0) {
            continue;
        }
        fs::write(target, bytes).map_err(|e| e.to_string())?;
        entries += 1;
    }
    drop(archive);
    if has_signature {
        let _ = fs::remove_dir_all(&stage);
        return Err("JAR 含有簽章，不能重建後維持有效簽章；已保留原檔".into());
    }
    if entries == 0 {
        let _ = fs::remove_dir_all(&stage);
        return Ok(None);
    }
    Ok(Some(ExtractedJar {
        jar_key: key,
        source_jar: source_jar.to_path_buf(),
        entries,
    }))
}

/// `data/<ns>/{powers|origins|origin_layers}/**.json`
fn is_origins_entry(name: &str) -> bool {
    let lower = name.replace('\\', "/").to_ascii_lowercase();
    if !lower.starts_with("data/") || !lower.ends_with(".json") {
        return false;
    }
    lower.contains("/powers/") || lower.contains("/origins/") || lower.contains("/origin_layers/")
}

fn dir_is_empty(path: &Path) -> bool {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .all(|e| !e.file_type().is_file())
}

fn count_files(path: &Path) -> usize {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_matches_origins_data_paths() {
        assert!(is_origins_entry("data/mypack/powers/fire_resist.json"));
        assert!(is_origins_entry("data/mypack/origins/blazeborn.json"));
        assert!(is_origins_entry("data/mypack/origin_layers/origin.json"));
        // 不是 Origins 的東西不能誤抓
        assert!(!is_origins_entry("data/mypack/recipes/thing.json"));
        assert!(!is_origins_entry("assets/mypack/lang/en_us.json"));
        assert!(!is_origins_entry("data/mypack/powers/icon.png"));
        assert!(!is_origins_entry("data/mypack/patchouli_books/b/en_us/x.json"));
    }
}
