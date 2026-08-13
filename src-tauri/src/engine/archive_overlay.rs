//! 安全處理資料包／資源包 ZIP 裡的可讀文字。
//!
//! 原始 ZIP 只讀：先把可安全讀取的文字檔解到工作暫存區，沿用鬆散檔案的
//! 顯示欄位白名單與格式護盾翻譯，再以原項目重建一份 ZIP。遇到超限或危險
//! 路徑時整個 archive 略過並回報，不把半成品套進遊戲。

use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::{ZipArchive, ZipWriter};

use super::cancel;
use super::security::{is_safe_zip_entry_name, MAX_ZIP_ENTRY_BYTES};
use super::text_overlay::translate_text_overlays;
use super::translation_scope::TranslationScope;

const MAX_ARCHIVE_BYTES: u64 = 200 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const MAX_ARCHIVE_TOTAL_UNCOMPRESSED: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveOverlayReport {
    pub archives_scanned: usize,
    pub archives_rewritten: usize,
    pub entries_rewritten: usize,
    pub strings_translated: usize,
    pub skipped: Vec<String>,
}

/// 掃描 datapack／global_packs／openloader／resourcepacks 中的 ZIP 文字。
/// 產出會放在工作根目錄的對應資料夾，套用時由 apply_instance 一起合併。
pub fn translate_archive_overlays<F>(
    minecraft_dir: &Path,
    work_root: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<ArchiveOverlayReport, String>
where
    F: FnMut(u8, &str),
{
    let archives = collect_archives(minecraft_dir);
    let mut report = ArchiveOverlayReport {
        archives_scanned: archives.len(),
        ..Default::default()
    };
    if archives.is_empty() {
        return Ok(report);
    }

    let stage_root = work_root.join(".archive-overlay-stage");
    if stage_root.exists() {
        fs::remove_dir_all(&stage_root).map_err(|e| format!("清理 ZIP 暫存區失敗：{e}"))?;
    }
    fs::create_dir_all(&stage_root).map_err(|e| e.to_string())?;
    let _cleanup = TempDirGuard(stage_root.clone());

    for (index, archive) in archives.iter().enumerate() {
        cancel::check()?;
        let name = archive.display().to_string();
        on_progress(
            1 + ((index * 80) / archives.len()) as u8,
            &format!("ZIP 文字：檢查第 {}/{} 個…", index + 1, archives.len()),
        );
        match process_archive(
            minecraft_dir,
            archive,
            &stage_root,
            work_root,
            use_ai,
            scope,
            &mut on_progress,
        ) {
            Ok((rewritten, strings)) if rewritten > 0 => {
                report.archives_rewritten += 1;
                report.entries_rewritten += rewritten;
                report.strings_translated += strings;
            }
            Ok(_) => {}
            Err(error) => report.skipped.push(format!("{name}：{error}")),
        }
    }

    on_progress(100, "ZIP 文字檢查完成");
    Ok(report)
}

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn collect_archives(mc: &Path) -> Vec<PathBuf> {
    const ROOTS: &[&str] = &[
        "datapacks",
        "global_packs",
        "resourcepacks",
        "config/openloader",
        "config",
        "defaultconfigs",
        "paxi/datapacks",
        "kubejs",
    ];
    let mut out = Vec::new();
    for root in ROOTS {
        let path = mc.join(root);
        if !path.is_dir() {
            continue;
        }
        for entry in WalkDir::new(path)
            .min_depth(1)
            .max_depth(8)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("zip"))
                    .unwrap_or(false)
            {
                out.push(p.to_path_buf());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn process_archive(
    mc: &Path,
    archive_path: &Path,
    stage_root: &Path,
    work_root: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<(usize, usize), String> {
    let metadata = fs::metadata(archive_path).map_err(|e| e.to_string())?;
    if metadata.len() == 0 || metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(format!("ZIP 超過 {} MB 大小上限", MAX_ARCHIVE_BYTES / 1024 / 1024));
    }
    let source_file = File::open(archive_path).map_err(|e| e.to_string())?;
    let mut source = ZipArchive::new(source_file).map_err(|e| format!("ZIP 格式錯誤：{e}"))?;
    if source.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!("ZIP 項目超過 {} 個上限", MAX_ARCHIVE_ENTRIES));
    }

    let relative = archive_path
        .strip_prefix(mc)
        .map_err(|e| format!("ZIP 不在遊戲目錄內：{e}"))?;
    let mut has_text = false;
    let mut total = 0u64;
    let mut hasher = DefaultHasher::new();
    relative.to_string_lossy().hash(&mut hasher);
    let id = format!("{:x}", hasher.finish());
    // 統一掛在 resourcepacks 根，讓共用掃描器能同時看見 data/assets/lang。
    let stage = stage_root.join("resourcepacks").join(&id);
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

    for index in 0..source.len() {
        let mut entry = source
            .by_index(index)
            .map_err(|e| format!("讀取 ZIP 目錄失敗：{e}"))?;
        let name = entry.name().replace('\\', "/");
        if !is_safe_zip_entry_name(&name) {
            return Err(format!("包含不安全的 ZIP 路徑：{name}"));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_ARCHIVE_TOTAL_UNCOMPRESSED {
            return Err("ZIP 解壓後總大小超過安全上限".into());
        }
        if !entry.is_file() || !is_translatable_extension(&name) {
            continue;
        }
        if entry.size() > MAX_ZIP_ENTRY_BYTES {
            continue;
        }
        let output = stage.join(&name);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.contains(&0) {
            continue;
        }
        fs::write(output, bytes).map_err(|e| e.to_string())?;
        has_text = true;
    }
    drop(source);
    if !has_text {
        return Ok((0, 0));
    }

    let translated_root = stage_root.join(format!("{id}-out"));
    let overlay = translate_text_overlays(&stage, &translated_root, use_ai, scope, |pct, msg| {
        on_progress(10 + pct.saturating_mul(70) / 100, msg);
    })?;
    if overlay.files_written == 0 {
        return Ok((0, 0));
    }

    let relative_text = relative.to_string_lossy().replace('\\', "/");
    let output_path = archive_output_path(work_root, &relative_text, archive_path);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temporary = output_path.with_extension("zip.tmp");
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|e| e.to_string())?;
    }
    let source_file = File::open(archive_path).map_err(|e| e.to_string())?;
    let mut source = ZipArchive::new(source_file).map_err(|e| e.to_string())?;
    let output_file = File::create(&temporary).map_err(|e| e.to_string())?;
    let mut writer = ZipWriter::new(output_file);
    let mut rewritten = 0usize;
    for index in 0..source.len() {
        cancel::check()?;
        let entry = source.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        let candidate = translated_root
            .join("resourcepacks")
            .join(&id)
            .join(&name);
        let options = entry.options();
        if entry.is_file() && candidate.is_file() {
            let bytes = fs::read(candidate).map_err(|e| e.to_string())?;
            writer
                .start_file(&name, options)
                .map_err(|e| format!("寫入 ZIP 項目失敗：{e}"))?;
            writer.write_all(&bytes).map_err(|e| e.to_string())?;
            rewritten += 1;
        } else {
            writer
                .raw_copy_file(entry)
                .map_err(|e| format!("複製 ZIP 項目失敗：{e}"))?;
        }
    }
    writer
        .finish()
        .map_err(|e| format!("完成 ZIP 重建失敗：{e}"))?;
    if output_path.exists() {
        fs::remove_file(&output_path).map_err(|e| e.to_string())?;
    }
    fs::rename(&temporary, &output_path).map_err(|e| e.to_string())?;
    Ok((rewritten, overlay.strings_translated))
}

fn archive_output_path(work_root: &Path, relative: &str, archive: &Path) -> PathBuf {
    if relative
        .split('/')
        .next()
        .is_some_and(|root| root.eq_ignore_ascii_case("resourcepacks"))
    {
        return work_root.join("resourcepacks-extra").join(
            archive
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("overlay.zip")),
        );
    }
    work_root.join(relative)
}

fn is_translatable_extension(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".json")
        || lower.ends_with(".txt")
        || lower.ends_with(".md")
        || lower.ends_with(".properties")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resourcepack_archives_use_separate_output_root() {
        let p = archive_output_path(Path::new("work"), "resourcepacks/base.zip", Path::new("x/base.zip"));
        assert_eq!(p, Path::new("work/resourcepacks-extra/base.zip"));
    }

    #[test]
    fn ordinary_archives_keep_their_game_relative_root() {
        let p = archive_output_path(Path::new("work"), "datapacks/base.zip", Path::new("x/base.zip"));
        assert_eq!(p, Path::new("work/datapacks/base.zip"));
    }
}
