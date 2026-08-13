//! JAR 內 Patchouli 書本的非破壞式翻譯。
//!
//! 多數 Patchouli 書頁位於 `data/<namespace>/patchouli_books`，不在 lang 檔。
//! 本模組只抽出這個明確路徑到工作暫存區，交給共用文字掃描器處理，再輸出
//! 成 `work/data/...` 覆寫檔；原始 JAR 不會被改寫。

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::ZipArchive;

use super::cancel;
use super::jar_scan::resolve_minecraft_dir;
use super::security::{check_jar_size, is_safe_zip_entry_name, MAX_ZIP_ENTRY_BYTES};
use super::text_overlay::translate_text_overlays;
use super::translation_scope::TranslationScope;

#[derive(Debug, Clone, Default)]
pub struct JarPatchouliReport {
    pub jars_scanned: usize,
    pub books_found: usize,
    pub files_written: usize,
    pub strings_translated: usize,
    pub skipped: Vec<String>,
    pub note: String,
}

pub fn translate_jar_patchouli<F>(
    instance_or_mc: &Path,
    work_root: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<JarPatchouliReport, String>
where
    F: FnMut(u8, &str),
{
    let mc = resolve_minecraft_dir(instance_or_mc)?;
    let mods = mc.join("mods");
    let jars = list_jars(&mods);
    let mut report = JarPatchouliReport {
        jars_scanned: jars.len(),
        ..Default::default()
    };
    if jars.is_empty() {
        report.note = "JAR 內 Patchouli：沒有找到模組 JAR".into();
        return Ok(report);
    }

    let stage = work_root.join(".jar-patchouli-stage");
    let translated_stage = work_root.join(".jar-patchouli-translated");
    let _ = fs::remove_dir_all(&stage);
    let _ = fs::remove_dir_all(&translated_stage);
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let _stage_cleanup = TempDirGuard(vec![stage.clone(), translated_stage.clone()]);
    for (index, jar) in jars.iter().enumerate() {
        cancel::check()?;
        on_progress(
            1 + ((index * 45) / jars.len()) as u8,
            &format!("JAR 書本：檢查第 {}/{} 個…", index + 1, jars.len()),
        );
        match extract_patchouli_json(jar, &stage) {
            Ok(found) => report.books_found += found,
            Err(error) => report.skipped.push(format!("{}：{error}", jar.display())),
        }
    }
    if report.books_found == 0 {
        report.note = "JAR 內 Patchouli：沒有找到 data/*/patchouli_books 文字頁面".into();
        return Ok(report);
    }
    let overlay = translate_text_overlays(&stage, &translated_stage, use_ai, scope, |pct, msg| {
        on_progress(45 + pct.saturating_mul(45) / 100, msg);
    })?;
    let source = translated_stage.join("data");
    let destination = work_root.join("data");
    if source.is_dir() {
        report.files_written = copy_tree(&source, &destination)?;
    }
    report.strings_translated = overlay.strings_translated;
    report.note = format!(
        "JAR 內 Patchouli：掃描 {} 個 JAR、找到 {} 個書本 JSON，翻譯 {} 條，寫出 {} 個覆寫檔{}",
        report.jars_scanned,
        report.books_found,
        report.strings_translated,
        report.files_written,
        if report.skipped.is_empty() {
            String::new()
        } else {
            format!("；{} 個 JAR 略過", report.skipped.len())
        }
    );
    on_progress(100, "JAR 內 Patchouli 完成");
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

fn extract_patchouli_json(jar: &Path, stage: &Path) -> Result<usize, String> {
    check_jar_size(jar)?;
    let file = File::open(jar).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("JAR 不是有效 ZIP：{e}"))?;
    let mut found = 0usize;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if !is_safe_zip_entry_name(&name)
            || !name.starts_with("data/")
            || !name.contains("/patchouli_books/")
            || !name.to_ascii_lowercase().ends_with(".json")
            || entry.size() > MAX_ZIP_ENTRY_BYTES
        {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.contains(&0) {
            continue;
        }
        let target = stage.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(target, bytes).map_err(|e| e.to_string())?;
        found += 1;
    }
    Ok(found)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<usize, String> {
    let mut written = 0usize;
    for entry in WalkDir::new(source).into_iter().filter_map(|e| e.ok()) {
        if !entry.path().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(source).map_err(|e| e.to_string())?;
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    #[test]
    fn module_has_a_narrow_patchouli_scope() {
        assert!("data/example/patchouli_books/book/en_us/entries/a.json"
            .contains("/patchouli_books/"));
        assert!(!"data/example/recipes/a.json".contains("/patchouli_books/"));
    }
}
