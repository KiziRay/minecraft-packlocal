//! 翻譯 JAR 內非 lang 的玩家可見文字資源。
//!
//! 這一層處理的是 JAR 裡的 JSON／Markdown／properties 等資料檔，不反編譯或改寫
//! `.class` 位元碼。先抽出可讀資源，沿用 text_overlay 的路徑與欄位判斷，完成後
//! 只重建翻譯後的 JAR 副本；原始 mods JAR 永遠只讀。
//!
//! 流程：全 JAR 抽出 → **一次** overlay → 只重建有譯文的 JAR（對齊 Patchouli）。

use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::cancel;
use super::jar_scan::resolve_minecraft_dir;
use super::mech_tokens::{is_mechanism_path_segment, is_origins_powers_path};
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

struct ExtractedJar {
    jar_key: String,
    source_jar: PathBuf,
    entries_scanned: usize,
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

    // Pass A：抽出（掛在 resourcepacks/{jar_key}/ 讓共用掃描器找得到）
    let mut extracted: Vec<ExtractedJar> = Vec::new();
    for (index, jar) in jars.iter().enumerate() {
        cancel::check()?;
        let i = index + 1;
        let n = jars.len();
        on_progress(
            1 + ((index * 45) / n.max(1)) as u8,
            &format!("JAR 顯示文字：模組 {i}/{n}"),
        );
        match extract_jar_display(jar, &stage_root) {
            Ok(Some(item)) => {
                report.entries_scanned += item.entries_scanned;
                on_progress(
                    1 + ((index * 45) / n.max(1)) as u8,
                    &format!(
                        "JAR 顯示文字：模組 {i}/{n}（抽出 {} 個文字檔）",
                        item.entries_scanned
                    ),
                );
                extracted.push(item);
            }
            Ok(None) => {}
            Err(error) => report
                .skipped
                .push(format!("{}：{error}", jar.display())),
        }
    }

    if extracted.is_empty() {
        report.note = format!(
            "JAR 顯示文字：檢查 {} 個模組，沒有可抽出的顯示文字{}",
            report.jars_scanned,
            if report.skipped.is_empty() {
                String::new()
            } else {
                format!("；{} 個 JAR 略過，詳見錯誤日誌", report.skipped.len())
            }
        );
        on_progress(100, "JAR 顯示文字完成");
        return Ok(report);
    }

    // Pass B：一次 overlay
    let translated_root = stage_root.join("_translated");
    on_progress(50, "顯示文字：批次翻譯抽出檔…");
    let overlay = translate_text_overlays(&stage_root, &translated_root, use_ai, scope, |pct, msg| {
        let rewritten = rewrite_overlay_progress_msg(msg);
        on_progress(50 + (pct as u16 * 35 / 100) as u8, &rewritten);
    })?;
    report.strings_translated = overlay.strings_translated;

    // Pass C：只重建有譯文檔的 jar
    let mods = mc.join("mods");
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
                report.jars_rewritten += 1;
                report.entries_rewritten += count_files(&jar_translated);
            }
            Err(error) => report
                .skipped
                .push(format!("{}：{error}", item.source_jar.display())),
        }
        on_progress(
            85 + ((index * 14) / extracted.len().max(1)) as u8,
            &format!(
                "JAR 顯示文字：重建模組 {}/{}",
                index + 1,
                extracted.len()
            ),
        );
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

fn rewrite_overlay_progress_msg(msg: &str) -> String {
    if msg.contains("掃描多根目錄") {
        "顯示文字：批次翻譯抽出檔…".into()
    } else if let Some(rest) = msg.strip_prefix("覆寫文字：") {
        format!("顯示文字：{rest}")
    } else {
        msg.to_string()
    }
}

fn extract_jar_display(
    source_jar: &Path,
    stage_root: &Path,
) -> Result<Option<ExtractedJar>, String> {
    check_jar_size(source_jar)?;
    let file = File::open(source_jar).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("JAR 不是有效 ZIP：{e}"))?;
    let mut has_signature = false;
    let mut entries_scanned = 0usize;
    let jar_key = jar_key(source_jar);
    let stage = stage_root.join("resourcepacks").join(&jar_key);
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
        let _ = fs::remove_dir_all(&stage);
        return Err("JAR 含有簽章，不能重建後維持有效簽章；已保留原檔".into());
    }
    if entries_scanned == 0 {
        let _ = fs::remove_dir_all(&stage);
        return Ok(None);
    }
    Ok(Some(ExtractedJar {
        jar_key,
        source_jar: source_jar.to_path_buf(),
        entries_scanned,
    }))
}

pub(crate) fn rebuild_jar(
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
    let mut written_names: HashSet<String> = HashSet::new();
    for index in 0..source.len() {
        cancel::check()?;
        let entry = source.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        written_names.insert(name.trim_end_matches('/').to_string());
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
    let _ = stage_root;
    drop(source);
    if translated_root.is_dir() {
        for entry in WalkDir::new(translated_root).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(translated_root)
                .map_err(|e| e.to_string())?;
            let name = relative.to_string_lossy().replace('\\', "/");
            if name.is_empty() || written_names.contains(&name) {
                continue;
            }
            let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
            writer
                .start_file(&name, SimpleFileOptions::default())
                .map_err(|e| e.to_string())?;
            writer.write_all(&bytes).map_err(|e| e.to_string())?;
        }
    }
    writer.finish().map_err(|e| e.to_string())?;
    if output.exists() {
        fs::remove_file(output).map_err(|e| e.to_string())?;
    }
    fs::rename(temporary, output).map_err(|e| e.to_string())?;
    Ok(())
}

fn is_display_resource(name: &str) -> bool {
    let lower = name.replace('\\', "/").to_ascii_lowercase();
    let text = [".json", ".json5", ".txt", ".md", ".properties"]
        .iter()
        .any(|ext| lower.ends_with(ext));
    if !text || lower.contains("/lang/") || lower.contains("/patchouli_books/") {
        return false;
    }
    if is_mechanism_path_segment(&lower) || is_origins_powers_path(&lower) {
        return false;
    }
    lower.starts_with("assets/") || lower.starts_with("data/")
}

pub(crate) fn is_signature_path(name: &str) -> bool {
    let upper = name.replace('\\', "/").to_ascii_uppercase();
    upper.starts_with("META-INF/")
        && matches!(
            Path::new(&upper).extension().and_then(|value| value.to_str()),
            Some("SF") | Some("RSA") | Some("DSA") | Some("EC")
        )
}

pub(crate) fn jar_key(path: &Path) -> String {
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

fn dir_is_empty(path: &Path) -> bool {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .all(|e| e.path() == path || e.file_type().is_dir())
}

fn count_files(path: &Path) -> usize {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
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
    }

    #[test]
    fn rewrite_overlay_progress_hides_misleading_scan_label() {
        assert!(rewrite_overlay_progress_msg("覆寫文字：掃描多根目錄（openloader…）…")
            .contains("批次翻譯"));
        assert_eq!(
            rewrite_overlay_progress_msg("覆寫文字：候補 7 個檔"),
            "顯示文字：候補 7 個檔"
        );
    }

    #[test]
    fn rebuild_jar_adds_new_locale_files() {
        let root = std::env::temp_dir().join(format!("rebuild_add_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let base = root.join("base.jar");
        {
            let file = File::create(&base).unwrap();
            let mut writer = ZipWriter::new(file);
            writer
                .start_file(
                    "data/ex/patchouli_books/b/en_us/categories/village.json",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(br#"{"name":"Village Module"}"#).unwrap();
            writer.finish().unwrap();
        }
        let translated = root.join("translated");
        let en = translated.join("data/ex/patchouli_books/b/en_us/categories/village.json");
        let zh = translated.join("data/ex/patchouli_books/b/zh_tw/categories/village.json");
        fs::create_dir_all(en.parent().unwrap()).unwrap();
        fs::create_dir_all(zh.parent().unwrap()).unwrap();
        fs::write(&en, "{\"name\":\"村莊模組\"}").unwrap();
        fs::write(&zh, "{\"name\":\"村莊模組\"}").unwrap();
        let output = root.join("out.jar");
        rebuild_jar(&base, &output, &translated, &root.join("stage")).unwrap();
        let file = File::open(&output).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().replace('\\', "/"))
            .collect();
        assert!(
            names.iter().any(|n| n.contains("zh_tw/categories/village.json")),
            "{names:?}"
        );
        let _ = fs::remove_dir_all(root);
    }
}
