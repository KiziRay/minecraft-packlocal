//! 翻譯執行紀錄：每次執行一份，不覆寫。
//!
//! 背景：`執行日誌.txt` 的設計是「每次按報告或翻譯任務結束時覆寫」。單次翻譯
//! 這樣沒問題，但使用者實測遇到「工具翻到一半被關掉、重開續翻」——重開後那一輪
//! 只跑了幾分鐘就寫檔，**把前面一小時的完整紀錄整個蓋掉**，出問題時完全沒有
//! 線索可查（實測那份檔案只剩 1981 bytes）。
//!
//! 這裡把「單一檔案覆寫」換成「每次執行一個檔」：
//! ```text
//! 翻譯結果/翻譯執行紀錄/
//!   2026-08-31_1832.txt
//!   2026-08-31_1915.txt
//!   最新.txt          ← 最近一次的副本，維持舊有「打開就看得到」的習慣
//! ```
//! 舊的 `執行日誌.txt` 仍然照寫（相容既有的回報流程與說明文件），但它不再是
//! 唯一的一份。

use std::fs;
use std::path::{Path, PathBuf};

pub const JOURNAL_DIR: &str = "翻譯執行紀錄";
/// 保留幾份歷史紀錄。太多份對使用者也沒意義，而且會佔空間。
const KEEP_RUNS: usize = 10;

/// 寫入本次執行紀錄，同時更新「最新.txt」，並清掉超量的舊紀錄。
///
/// 任何一步失敗都只回傳錯誤字串，呼叫端可以選擇忽略——**紀錄寫不進去
/// 不該讓翻譯本身失敗**。
pub fn write_run_log(work_root: &Path, stamp: &str, content: &str) -> Result<PathBuf, String> {
    let dir = work_root.join(JOURNAL_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("無法建立執行紀錄資料夾：{e}"))?;
    let path = dir.join(format!("{stamp}.txt"));
    fs::write(&path, content).map_err(|e| format!("無法寫入執行紀錄：{e}"))?;
    // 「最新」是副本而不是捷徑：Windows 上捷徑需要額外權限，副本最單純可靠
    let _ = fs::write(dir.join("最新.txt"), content);
    prune_old_runs(&dir);
    Ok(path)
}

/// 只保留最近 `KEEP_RUNS` 份（依檔名排序，命名本身就是時間序）。
fn prune_old_runs(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut runs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n != "最新.txt")
                .unwrap_or(false)
        })
        .collect();
    if runs.len() <= KEEP_RUNS {
        return;
    }
    runs.sort();
    let remove_count = runs.len() - KEEP_RUNS;
    for path in runs.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

/// 列出既有的執行紀錄（新到舊），供診斷頁與問題回報取用。
pub fn list_runs(work_root: &Path) -> Vec<PathBuf> {
    let dir = work_root.join(JOURNAL_DIR);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut runs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n != "最新.txt")
                .unwrap_or(false)
        })
        .collect();
    runs.sort();
    runs.reverse();
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mcpl-journal-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn each_run_gets_its_own_file_and_never_overwrites() {
        // 這條釘死使用者回報的資料遺失：中斷後重開續翻，不可以蓋掉前一輪的紀錄
        let work = scratch("no-overwrite");
        write_run_log(&work, "20260831_1832", "第一輪：完整跑了一小時").unwrap();
        write_run_log(&work, "20260831_1915", "第二輪：只跑了幾分鐘").unwrap();

        let first = work.join(JOURNAL_DIR).join("20260831_1832.txt");
        let second = work.join(JOURNAL_DIR).join("20260831_1915.txt");
        assert!(first.is_file(), "第一輪的紀錄必須還在");
        assert_eq!(fs::read_to_string(&first).unwrap(), "第一輪：完整跑了一小時");
        assert_eq!(fs::read_to_string(&second).unwrap(), "第二輪：只跑了幾分鐘");
        // 「最新」指向最後一次
        assert_eq!(
            fs::read_to_string(work.join(JOURNAL_DIR).join("最新.txt")).unwrap(),
            "第二輪：只跑了幾分鐘"
        );
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn keeps_only_the_most_recent_runs() {
        let work = scratch("prune");
        for i in 0..(KEEP_RUNS + 5) {
            write_run_log(&work, &format!("2026083{i:02}_1200"), &format!("run {i}")).unwrap();
        }
        let runs = list_runs(&work);
        assert_eq!(runs.len(), KEEP_RUNS, "超量的舊紀錄要被清掉");
        // 最新的排最前面
        assert!(runs[0].to_string_lossy().contains(&format!("2026083{:02}", KEEP_RUNS + 4)));
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn listing_is_empty_when_nothing_written_yet() {
        let work = scratch("empty");
        assert!(list_runs(&work).is_empty());
        let _ = fs::remove_dir_all(&work);
    }
}
