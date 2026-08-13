//! 翻譯 JAR 內非 lang 的玩家可見文字資源。
//!
//! 這一層處理的是 JAR 裡的 JSON／Markdown／properties 等資料檔，不反編譯或改寫
//! `.class` 位元碼。它會先抽出可讀資源，沿用 text_overlay 的路徑與欄位判斷，完成後
//! 只重建翻譯後的 JAR 副本；原始 mods JAR 永遠只讀。

use serde::Serialize;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::{ZipArchive, ZipWriter};

use super::cancel;
use super::jar_scan::resolve_minecraft_dir;
use super::security::{check_jar_size, is_safe_zip_entry_name};
use super::text_overlay::translate_text_overlays;
use super::translation_scope::TranslationScope;

const MAX_STAGE_ENTRY_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JarDisplayReport {
    pub jars_scanned: usize,
    pub jars_rewritten: usize,
    pub entries_scanned: usize,
    pub entries_rewritten: usize,
    pub strings_translated: usize,
    pub skipped: Vec<String>,
    pub note: String,
}

pub fn translate_jar_display_texts<F>(
    instance_or_mc: &Path,
    work_root: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<JarDisplayReport, String>
where
    F: FnMut(u8, &str),
{
    let mc = resolve_minecraft_dir(instance_or_mc)?;
    let jars = list_jars(&mc.join("mods"));
    let mut report = JarDisplayReport {
        jars_scanned: jars.len(),
        ..Default::default()
    };
    if jars.is_empty() {
        report.note = "JAR 顯示文字：沒有找到模組 JAR".into();
        return Ok(report);
    }

    let stage_root = work_root.join(".jar-display-stage");
    if stage_root.exists() {
        fs::remove_dir_all(&stage_root).map_err(|e| format!("清理 JAR 文字暫存失敗：{e}"))?;
    }
    fs::create_dir_all(&stage_root).map_err(|e| e.to_string())?;
    let _cleanup = TempDirGuard(stage_root.clone());

    for (index, jar) in jars.iter().enumerate() {
        cancel::check()?;
        on_progress(
            1 + ((index * 90) / jars.len()) as u8,
            &format!("JAR 顯示文字：檢查第 {}/{} 個…", index + 1, jars.len()),
        );
        match process_jar(jar, work_root, &stage_root, use_ai, scope, &mut on_progress) {
            Ok((entries_scanned, entries_rewritten, strings)) => {
                report.entries_scanned += entries_scanned;
                if entries_rewritten > 0 {
                    report.jars_rewritten += 1;
                    report.entries_rewritten += entries_rewritten;
                    report.strings_translated += strings;
                }
            }
            Err(error) => report
                .skipped
                .push(format!("{}：{error}", jar.display())),
        }
    }
    report.note = format!(
        "JAR 顯示文字：檢查 {} 個資源檔，重建 {} 個 JAR，改寫 {} 個檔案、翻譯 {} 條{}",
        report.entries_scanned,
        report.jars_rewritten,
        report.entries_rewritten,
        report.strings_translated,
        if report.skipped.is_empty() {
            String::new()
        } else {
            format!("；{} 個 JAR 略過，詳見錯誤日誌", report.skipped.len())
        }
    );
    on_progress(100, "JAR 顯示文字完成");
    Ok(report)
}

fn process_jar(
    source_jar: &Path,
    work_root: &Path,
    stage_root: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<(usize, usize, usize), String> {
    check_jar_size(source_jar)?;
    let file = File::open(source_jar).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("JAR 不是有效 ZIP：{e}"))?;
    let mut has_signature = false;
    let mut entries_scanned = 0usize;
    let jar_key = jar_key(source_jar);
    let stage = stage_root.join(&jar_key);
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
        if !is_display_resource(&name) || entry.size() > MAX_STAGE_ENTRY_BYTES {
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
        entries_scanned += 1;
    }
    drop(archive);
    if has_signature {
        return Err("JAR 含有簽章，不能重建後維持有效簽章；已保留原檔".into());
    }
    if entries_scanned == 0 {
        let _ = fs::remove_dir_all(&stage);
        return Ok((0, 0, 0));
    }

    let translated = stage_root.join(format!("{jar_key}-out"));
    let overlay = translate_text_overlays(&stage, &translated, use_ai, scope, |pct, msg| {
        on_progress(pct.min(95), msg);
    })?;
    if overlay.files_written == 0 {
        return Ok((entries_scanned, 0, 0));
    }

    let mods = source_jar
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name.eq_ignore_ascii_case("mods")))
        .ok_or_else(|| "無法找到 mods 根目錄".to_string())?;
    let relative = source_jar
        .strip_prefix(mods)
        .map_err(|e| format!("JAR 不在 mods 根目錄：{e}"))?;
    let output = work_root.join("jar-translated").join(relative);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // 語言檔副本可能已由 jar_translate 先建立；以現有副本為基底，避免
    // JAR 顯示文字階段把前一階段的 zh_tw 語言檔覆蓋回原文。
    let base = if output.is_file() {
        output.as_path()
    } else {
        source_jar
    };
    rebuild_jar(base, &output, &translated, &stage)?;
    Ok((entries_scanned, overlay.files_written, overlay.strings_translated))
}

fn rebuild_jar(
    base_jar: &Path,
    output: &Path,
    translated_root: &Path,
    stage_root: &Path,
) -> Result<(), String> {
    let source_file = File::open(base_jar).map_err(|e| e.to_string())?;
    let mut source = ZipArchive::new(source_file).map_err(|e| e.to_string())?;
    let temporary = output.with_extension("jar.display.tmp");
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|e| e.to_string())?;
    }
    let output_file = File::create(&temporary).map_err(|e| e.to_string())?;
    let mut writer = ZipWriter::new(output_file);
    for index in 0..source.len() {
        cancel::check()?;
        let entry = source.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        let candidate = translated_root.join(&name);
        if entry.is_file() && candidate.is_file() {
            let bytes = fs::read(candidate).map_err(|e| e.to_string())?;
            writer
                .start_file(&name, entry.options())
                .map_err(|e| e.to_string())?;
            writer.write_all(&bytes).map_err(|e| e.to_string())?;
        } else {
            writer.raw_copy_file(entry).map_err(|e| e.to_string())?;
        }
    }
    // 只把 stage 中原本存在、但基底 JAR 沒有的檔案加入；目前主要用來保留
    // text_overlay 正規化後的相同路徑，避免意外把暫存目錄或報告塞進 JAR。
    let _ = stage_root;
    // Windows 可能仍把基底 JAR 視為開啟中；先釋放 ZipArchive 的檔案控制代碼，
    // 才能安全替換同名的翻譯副本。
    drop(source);
    writer.finish().map_err(|e| e.to_string())?;
    if output.exists() {
        fs::remove_file(output).map_err(|e| e.to_string())?;
    }
    fs::rename(temporary, output).map_err(|e| e.to_string())?;
    Ok(())
}

fn is_display_resource(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let text = [".json", ".json5", ".txt", ".md", ".properties"]
        .iter()
        .any(|ext| lower.ends_with(ext));
    if !text || lower.contains("/lang/") || lower.contains("/patchouli_books/") {
        return false;
    }
    if [
        "/recipes/",
        "/loot_tables/",
        "/tags/",
        "/structures/",
        "/worldgen/",
        "/functions/",
        "/predicates/",
        "/item_modifiers/",
        "/dimension/",
        "/biome/",
    ]
    .iter()
    .any(|segment| lower.contains(segment))
    {
        return false;
    }
    lower.starts_with("assets/") || lower.starts_with("data/")
}

fn is_signature_path(name: &str) -> bool {
    let upper = name.replace('\\', "/").to_ascii_uppercase();
    upper.starts_with("META-INF/")
        && matches!(
            Path::new(&upper).extension().and_then(|value| value.to_str()),
            Some("SF") | Some("RSA") | Some("DSA") | Some("EC")
        )
}

fn jar_key(path: &Path) -> String {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("jar");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    let stem = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    format!("{stem}_{:x}", hasher.finish())
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
                    .is_some_and(|s| s.eq_ignore_ascii_case("jar"))
        })
        .collect()
}

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_display_resources_but_not_language_or_binary_files() {
        assert!(is_display_resource("data/example/quests/chapter.json"));
        assert!(is_display_resource("assets/example/manual/readme.md"));
        assert!(!is_display_resource("assets/example/lang/en_us.json"));
        assert!(!is_display_resource("data/example/recipes/iron.json"));
        assert!(!is_display_resource("assets/example/icon.png"));
    }
}
