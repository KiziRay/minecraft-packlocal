//! 錯誤分析的記錄讀取。

use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MAX_READ_BYTES: usize = 8 * 1024 * 1024;
const HEAD_READ_BYTES: usize = 512 * 1024;
/// crash report 若比 latest.log 舊超過此時長，不納入主分類輸入。
const STALE_CRASH_BEHIND_LATEST: Duration = Duration::from_secs(48 * 60 * 60);

#[allow(dead_code)]
fn read_newest_log(mc: &Path) -> Option<(String, String)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let crash_dir = mc.join("crash-reports");
    if let Ok(entries) = fs::read_dir(&crash_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("txt") {
                candidates.push(path);
            }
        }
    }
    for path in [mc.join("logs/latest.log"), mc.join("logs/debug.log")] {
        if path.is_file() {
            candidates.push(path);
        }
    }
    candidates.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    let newest = candidates.last()?.clone();
    let text = read_bounded(&newest)?;
    Some((text, newest.display().to_string()))
}

fn read_bounded(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() <= MAX_READ_BYTES {
        return Some(String::from_utf8_lossy(&bytes).into_owned());
    }

    let head_len = HEAD_READ_BYTES.min(bytes.len());
    let tail_len = MAX_READ_BYTES.saturating_sub(head_len);
    let tail_start = bytes.len().saturating_sub(tail_len);
    let mut text = String::from_utf8_lossy(&bytes[..head_len]).into_owned();
    text.push_str("\n\n[中間記錄過長，已省略；保留檔案開頭與最後錯誤段落]\n\n");
    text.push_str(&String::from_utf8_lossy(&bytes[tail_start..]));
    Some(text)
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

/// crash report 是否比 latest.log 舊超過 48 小時（應排除於主分類）。
pub(super) fn crash_is_stale_vs_latest(
    crash_mtime: SystemTime,
    latest_mtime: SystemTime,
) -> bool {
    match latest_mtime.duration_since(crash_mtime) {
        Ok(delta) => delta > STALE_CRASH_BEHIND_LATEST,
        Err(_) => false,
    }
}

pub(super) fn read_combined_logs(mc: &Path) -> Option<(String, String)> {
    let mut crash_reports: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = fs::read_dir(mc.join("crash-reports")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("txt") {
                crash_reports.push(path);
            }
        }
    }
    crash_reports.sort_by_key(|path| {
        Reverse(
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH),
        )
    });

    let latest_log = mc.join("logs/latest.log");
    let latest_mtime = if latest_log.is_file() {
        file_mtime(&latest_log)
    } else {
        None
    };

    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut stale_crash_note: Option<String> = None;
    if let Some(path) = crash_reports.first() {
        let crash_mtime = file_mtime(path);
        let stale = match (crash_mtime, latest_mtime) {
            (Some(crash), Some(latest)) => crash_is_stale_vs_latest(crash, latest),
            _ => false,
        };
        if stale {
            stale_crash_note = Some(format!(
                "（已略過超過 48 小時的舊 crash report：{}）",
                path.display()
            ));
        } else {
            candidates.push(path.clone());
        }
    }
    let optional_candidates = vec![
        Some(latest_log),
        Some(mc.join("logs/debug.log")),
        newest_hs_err(mc),
    ];
    for path in optional_candidates.into_iter().flatten() {
        if path.is_file() && !candidates.iter().any(|item| item == &path) {
            candidates.push(path);
        }
    }

    let mut parts = Vec::new();
    let mut sources = Vec::new();
    for path in candidates {
        if let Some(text) = read_bounded(&path) {
            let label = if path.file_name().and_then(|name| name.to_str()) == Some("latest.log") {
                "latest.log"
            } else if path.file_name().and_then(|name| name.to_str()) == Some("debug.log") {
                "debug.log"
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .starts_with("hs_err_pid")
            {
                "hs_err_pid"
            } else {
                "crash report"
            };
            parts.push(format!("--- {label}: {} ---\n{text}", path.display()));
            sources.push(path.display().to_string());
        }
    }
    if let Some(note) = stale_crash_note {
        sources.push(note);
    }

    if parts.is_empty() {
        None
    } else {
        Some((parts.join("\n"), sources.join(" + ")))
    }
}

fn newest_hs_err(mc: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for dir in [mc.to_path_buf(), mc.join("logs")] {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
            if name.starts_with("hs_err_pid") && name.ends_with(".log") {
                candidates.push(path);
            }
        }
    }
    candidates.sort_by_key(|path| {
        Reverse(
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH),
        )
    });
    candidates.into_iter().next()
}
