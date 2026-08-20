//! JAR 內 Patchouli 書本的非破壞式翻譯。
//!
//! 多數 Patchouli 書頁位於 `data/<namespace>/patchouli_books`，不在 lang 檔。
//! 抽出後交給共用文字掃描器，再嵌回 `jar-translated` 副本；原始 JAR 不會被改寫。
//! `work/data` 僅作除錯產出，不會套用到遊戲的 `minecraft/data/`。

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::ZipArchive;

use super::cancel;
use super::jar_display::{is_signature_path, jar_key, rebuild_jar};
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

struct ExtractedJar {
    jar_key: String,
    source_jar: PathBuf,
    entries_scanned: usize,
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

    let stage_root = work_root.join(".jar-patchouli-stage");
    let translated_root = work_root.join(".jar-patchouli-translated");
    let _ = fs::remove_dir_all(&stage_root);
    let _ = fs::remove_dir_all(&translated_root);
    fs::create_dir_all(&stage_root).map_err(|e| e.to_string())?;
    let _stage_cleanup = TempDirGuard(vec![stage_root.clone(), translated_root.clone()]);

    let mut extracted: Vec<ExtractedJar> = Vec::new();
    for (index, jar) in jars.iter().enumerate() {
        cancel::check()?;
        on_progress(
            1 + ((index * 40) / jars.len().max(1)) as u8,
            &format!("JAR 書本：模組 {}/{}", index + 1, jars.len()),
        );
        match extract_patchouli(jar, &stage_root) {
            Ok(Some(item)) => {
                report.books_found += item.entries_scanned;
                extracted.push(item);
            }
            Ok(None) => {}
            Err(error) => report.skipped.push(format!("{}：{error}", jar.display())),
        }
    }
    if extracted.is_empty() {
        report.note = "JAR 內 Patchouli：沒有找到 data/*/patchouli_books 文字頁面".into();
        return Ok(report);
    }

    let overlay = translate_text_overlays(&stage_root, &translated_root, use_ai, scope, |pct, msg| {
        on_progress(42 + pct.saturating_mul(40) / 100, msg);
    })?;
    report.strings_translated = overlay.strings_translated;

    for (index, item) in extracted.iter().enumerate() {
        cancel::check()?;
        let jar_translated = translated_root
            .join("resourcepacks")
            .join(&item.jar_key);
        if !jar_translated.is_dir() || dir_is_empty(&jar_translated) {
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
        let base = if output.is_file() {
            output.as_path()
        } else {
            item.source_jar.as_path()
        };
        let stage = stage_root.join("resourcepacks").join(&item.jar_key);
        match rebuild_jar(base, &output, &jar_translated, &stage) {
            Ok(()) => {
                report.files_written += count_files(&jar_translated);
            }
            Err(error) => report
                .skipped
                .push(format!("{}：{error}", item.source_jar.display())),
        }
        on_progress(
            84 + ((index * 12) / extracted.len().max(1)) as u8,
            &format!(
                "JAR 書本：重建模組 {}/{}",
                index + 1,
                extracted.len()
            ),
        );
    }

    let debug_data = work_root.join("data");
    let _ = copy_translated_data_for_debug(&translated_root, &debug_data);

    report.note = format!(
        "JAR 內 Patchouli：掃描 {} 個 JAR、找到 {} 個書本檔，翻譯 {} 條，寫入 jar-translated 副本（不改原 jar）{}",
        report.jars_scanned,
        report.books_found,
        report.strings_translated,
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

fn extract_patchouli(source_jar: &Path, stage_root: &Path) -> Result<Option<ExtractedJar>, String> {
    check_jar_size(source_jar)?;
    let file = File::open(source_jar).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("JAR 不是有效 ZIP：{e}"))?;
    let mut has_signature = false;
    let mut entries_scanned = 0usize;
    let key = jar_key(source_jar);
    let stage = stage_root.join("resourcepacks").join(&key);
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
        if !is_patchouli_book_entry(&name) || entry.size() > MAX_ZIP_ENTRY_BYTES {
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
        let _ = fs::remove_dir_all(&stage);
        return Err("JAR 含有簽章，不能重建後維持有效簽章；已保留原檔".into());
    }
    if entries_scanned == 0 {
        let _ = fs::remove_dir_all(&stage);
        return Ok(None);
    }
    Ok(Some(ExtractedJar {
        jar_key: key,
        source_jar: source_jar.to_path_buf(),
        entries_scanned,
    }))
}

fn is_patchouli_book_entry(name: &str) -> bool {
    let lower = name.replace('\\', "/").to_ascii_lowercase();
    lower.starts_with("data/")
        && lower.contains("/patchouli_books/")
        && (lower.ends_with(".json") || lower.ends_with(".txt"))
}

fn copy_translated_data_for_debug(translated_root: &Path, destination: &Path) -> Result<usize, String> {
    let packs = translated_root.join("resourcepacks");
    if !packs.is_dir() {
        return Ok(0);
    }
    let mut written = 0usize;
    for jar_dir in fs::read_dir(&packs).map_err(|e| e.to_string())? {
        let jar_dir = jar_dir.map_err(|e| e.to_string())?.path();
        let data = jar_dir.join("data");
        if !data.is_dir() {
            continue;
        }
        written += copy_tree(&data, destination)?;
    }
    Ok(written)
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
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn module_has_a_narrow_patchouli_scope() {
        assert!(is_patchouli_book_entry(
            "data/example/patchouli_books/book/en_us/entries/a.json"
        ));
        assert!(is_patchouli_book_entry(
            "data/example/patchouli_books/book/en_us/root.txt"
        ));
        assert!(!is_patchouli_book_entry("data/example/recipes/a.json"));
        assert!(!is_patchouli_book_entry(
            "assets/example/book/animal_dictionary/en_us/root.txt"
        ));
    }

    #[test]
    fn embeds_translated_patchouli_into_jar_translated_not_minecraft_data() {
        let root = std::env::temp_dir().join(format!("jar_patchouli_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("work");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(&work).unwrap();

        let jar = mc.join("mods/guidebook.jar");
        {
            let file = File::create(&jar).unwrap();
            let mut writer = ZipWriter::new(file);
            writer
                .start_file(
                    "data/example/patchouli_books/guide/book.json",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(
                    r#"{
  "name": "测试书",
  "landing_text": "这是首页说明"
}"#
                    .as_bytes(),
                )
                .unwrap();
            writer
                .start_file(
                    "data/example/patchouli_books/guide/en_us/categories/village.json",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(r#"{"name":"村庄模块"}"#.as_bytes())
                .unwrap();
            writer.finish().unwrap();
        }

        let report = translate_jar_patchouli(&mc, &work, false, None, |_, _| {}).unwrap();
        assert!(report.books_found >= 2, "{report:?}");
        assert!(report.strings_translated >= 1, "{}", report.note);

        let out_jar = work.join("jar-translated/guidebook.jar");
        assert!(out_jar.is_file(), "{}", report.note);
        let file = File::open(&out_jar).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().replace('\\', "/"))
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("zh_tw/categories/village.json")),
            "{names:?}"
        );

        let mut book = zip
            .by_name("data/example/patchouli_books/guide/book.json")
            .unwrap();
        let mut bytes = Vec::new();
        book.read_to_end(&mut bytes).unwrap();
        drop(book);
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("這是首頁說明") || text.contains("测试书") || text.contains("測試"),
            "{text}"
        );

        assert!(
            !mc.join("data").exists(),
            "不得把 Patchouli 寫進遊戲 minecraft/data"
        );
        let _ = fs::remove_dir_all(root);
    }
}
