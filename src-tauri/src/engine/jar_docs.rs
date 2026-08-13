//! Safe, read-only inspection of documentation and hard-coded text inside JARs.
//!
//! A JAR is a ZIP file.  We inspect its resources without modifying the JAR or
//! executing its classes.  Class files are only reduced to printable string
//! clues; they are never treated as trusted translations.  This catches text
//! that a normal `lang/*.json` scan cannot see while keeping the translation
//! pipeline deterministic and offline-first.

use serde::Serialize;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::ZipArchive;

use super::jar_scan::resolve_minecraft_dir;
use super::security::sanitize_folder_name;

const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CLASS_CLUES: usize = 300;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JarDocumentationReport {
    pub jars_scanned: usize,
    pub text_entries: usize,
    pub class_files_inspected: usize,
    pub files_written: usize,
    pub bytes_written: u64,
    pub skipped_entries: usize,
    pub notes: Vec<String>,
}

pub fn extract_jar_documentation(
    instance_or_minecraft: &Path,
    work_root: &Path,
) -> Result<JarDocumentationReport, String> {
    let mc = resolve_minecraft_dir(instance_or_minecraft)
        .unwrap_or_else(|_| instance_or_minecraft.to_path_buf());
    let mods = mc.join("mods");
    if !mods.is_dir() {
        return Ok(JarDocumentationReport {
            notes: vec!["找不到 mods 資料夾，略過 JAR 文件複查。".to_string()],
            ..Default::default()
        });
    }

    let out = work_root.join("jar-documentation");
    if out.exists() {
        fs::remove_dir_all(&out).map_err(|e| format!("清理舊的 JAR 文件複查失敗：{e}"))?;
    }
    fs::create_dir_all(&out).map_err(|e| format!("建立 JAR 文件複查目錄失敗：{e}"))?;
    let mut report = JarDocumentationReport::default();
    for entry in WalkDir::new(&mods).max_depth(1).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("jar")) != Some(true) {
            continue;
        }
        report.jars_scanned += 1;
        match inspect_jar(path, &out, &mut report) {
            Ok(()) => {}
            Err(error) => report.notes.push(format!("{}：{}", path.display(), error)),
        }
    }
    Ok(report)
}

fn inspect_jar(
    jar_path: &Path,
    out_root: &Path,
    report: &mut JarDocumentationReport,
) -> Result<(), String> {
    let file = File::open(jar_path).map_err(|e| format!("無法讀取：{e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("不是可讀取的 JAR：{e}"))?;
    let stem = jar_path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
    let jar_dir = out_root.join(sanitize_folder_name(stem).unwrap_or_else(|_| "unknown".into()));
    let mut total = 0u64;
    let mut class_clues = Vec::new();

    for index in 0..archive.len() {
        let mut item = archive.by_index(index).map_err(|e| e.to_string())?;
        if item.is_dir() || item.size() > MAX_ENTRY_BYTES {
            report.skipped_entries += 1;
            continue;
        }
        let name = item.name().replace('\\', "/");
        if name.contains("..") || name.starts_with('/') {
            report.skipped_entries += 1;
            continue;
        }
        let is_class = name.ends_with(".class");
        if !is_interesting_text(&name) && !is_class {
            continue;
        }
        if total.saturating_add(item.size()) > MAX_TOTAL_BYTES {
            report.notes.push(format!("{} 超過文件複查大小上限，後續項目略過。", jar_path.display()));
            break;
        }
        let mut bytes = Vec::with_capacity(item.size() as usize);
        item.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        total = total.saturating_add(bytes.len() as u64);

        if is_class {
            report.class_files_inspected += 1;
            for clue in printable_strings(&bytes) {
                if class_clues.len() >= MAX_CLASS_CLUES {
                    break;
                }
                class_clues.push(format!("{}: {}", name, clue));
            }
            continue;
        }

        let relative = safe_relative_path(&name);
        let target = jar_dir.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(target, &bytes).map_err(|e| e.to_string())?;
        report.text_entries += 1;
        report.files_written += 1;
        report.bytes_written = report.bytes_written.saturating_add(bytes.len() as u64);
    }

    if !class_clues.is_empty() {
        fs::create_dir_all(&jar_dir).map_err(|e| e.to_string())?;
        let path = jar_dir.join("class-text-clues.txt");
        fs::write(path, class_clues.join("\n") + "\n").map_err(|e| e.to_string())?;
        report.files_written += 1;
    }
    Ok(())
}

fn is_interesting_text(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let extension = ["json", "json5", "lang", "txt", "md", "toml", "properties", "yml", "yaml"]
        .iter()
        .any(|suffix| lower.ends_with(&format!(".{suffix}")));
    extension && (lower.starts_with("assets/") || lower.starts_with("data/") || lower.contains("doc") || lower.contains("config"))
}

fn safe_relative_path(name: &str) -> PathBuf {
    name.split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(|part| sanitize_folder_name(part).unwrap_or_else(|_| "entry".to_string()))
        .collect()
}

fn printable_strings(bytes: &[u8]) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = Vec::new();
    for byte in bytes.iter().copied().chain(std::iter::once(0)) {
        if byte.is_ascii_graphic() || byte == b' ' {
            current.push(byte);
        } else {
            if current.len() >= 4 {
                if let Ok(value) = String::from_utf8(std::mem::take(&mut current)) {
                    let trimmed = value.trim();
                    if !trimmed.is_empty() && !result.iter().any(|item| item == trimmed) {
                        result.push(trimmed.to_string());
                    }
                }
            }
            current.clear();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn extracts_text_resources_and_class_clues_without_executing_jar() {
        let root = std::env::temp_dir().join(format!("jar_docs_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mods = root.join("minecraft/mods");
        fs::create_dir_all(&mods).unwrap();
        let jar_path = mods.join("example.jar");
        let file = File::create(&jar_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        zip.start_file("assets/example/lang/en_us.json", opts).unwrap();
        zip.write_all(br#"{"demo.title":"Hello"}"#).unwrap();
        zip.start_file("assets/example/Example.class", opts).unwrap();
        zip.write_all(b"\0Hello from class\0").unwrap();
        zip.finish().unwrap();

        let report = extract_jar_documentation(&root, &root.join("work")).unwrap();
        assert_eq!(report.jars_scanned, 1);
        assert_eq!(report.text_entries, 1);
        assert_eq!(report.class_files_inspected, 1);
        assert!(root.join("work/jar-documentation/example/class-text-clues.txt").is_file());
        let _ = fs::remove_dir_all(root);
    }
}
