//! 共享 TM 貢獻失敗時的本機重試佇列。
//!
//! 路徑：`%APPDATA%/modpack-i18n-tool/shared_contribute_queue.json`
//! 網路／HTTP 失敗時寫入；下次 `contribute` 會先 `flush_pending`。
//! 有條數／序列化大小上限，避免佇列膨脹拖垮每次翻譯。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::shared_tm::{self, ContributeResult, SharedTmEntry};

/// 佇列最多保留幾條（超出丟最舊）。
const MAX_QUEUE_ENTRIES: usize = 8_000;
/// 佇列檔大約上限（超出從最舊裁到符合）。
const MAX_QUEUE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct QueueFile {
    #[serde(default)]
    entries: Vec<SharedTmEntry>,
}

pub fn queue_path() -> PathBuf {
    super::paths::resolve_file(Path::new("shared_contribute_queue.json"))
}

fn load_queue() -> Vec<SharedTmEntry> {
    let path = queue_path();
    fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<QueueFile>(&t).ok())
        .map(|f| f.entries)
        .unwrap_or_default()
}

fn save_queue(entries: &[SharedTmEntry]) -> Option<usize> {
    let path = queue_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let (trimmed, dropped_entries) = trim_queue_to_caps_detailed(entries);
    // 先把要丟的存進側錄檔，再真的丟（P1-03）。
    // 這些是使用者已經翻好的譯文，只是還沒同步給共享庫；
    // 靜默刪掉等於把他的成果扔了，而且他永遠不會知道。
    record_overflow(&dropped_entries);
    let dropped = dropped_entries.len();
    let file = QueueFile {
        entries: trimmed,
    };
    match serde_json::to_string(&file) {
        Ok(body) => {
            // 先寫暫存再換名：這個佇列實測會長到 1.7 MB 且經常重寫，
            // 直接覆蓋原檔的話，寫到一半被中斷就留下壞掉的 JSON，
            // 下次載入直接當成空佇列——累積待送的社群貢獻整批消失。
            let tmp = path.with_extension("json.tmp");
            if fs::write(&tmp, body).is_err() {
                return None;
            }
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
            if fs::rename(&tmp, &path).is_ok() {
                Some(dropped)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

fn clear_queue() {
    let path = queue_path();
    let _ = fs::remove_file(&path);
    // 若刪除失敗，寫空檔仍可避免反覆送舊資料
    if path.exists() {
        let _ = save_queue(&[]);
    }
}

/// 依條數與大約序列化大小裁切：保留較新的尾端。
/// 裁到上限，並回傳「被裁掉的那些」而不只是數量（P1-03）。
///
/// 真實執行紀錄裡出現過「佇列已達上限，丟棄最舊 20 條」——那 20 條是使用者
/// **已經翻好**的譯文，就這樣永久消失了，而且只留下一個數字。
/// 呼叫端拿到實體後會先寫進側錄檔再丟，資料至少不會不見。
pub(crate) fn trim_queue_to_caps_detailed(
    entries: &[SharedTmEntry],
) -> (Vec<SharedTmEntry>, Vec<SharedTmEntry>) {
    if entries.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let keep = trim_keep_count(entries);
    let start = entries.len().saturating_sub(keep);
    (entries[start..].to_vec(), entries[..start].to_vec())
}

pub(crate) fn trim_queue_to_caps(entries: &[SharedTmEntry]) -> (Vec<SharedTmEntry>, usize) {
    let (kept, dropped) = trim_queue_to_caps_detailed(entries);
    (kept, dropped.len())
}

fn trim_keep_count(entries: &[SharedTmEntry]) -> usize {
    if entries.is_empty() {
        return 0;
    }
    let count_start = entries.len().saturating_sub(MAX_QUEUE_ENTRIES);
    let count_limited = &entries[count_start..];
    let mut keep = count_limited.len();
    if let Ok(body) = serde_json::to_string(&QueueFile {
        entries: count_limited.to_vec(),
    }) {
        if body.len() > MAX_QUEUE_BYTES && !count_limited.is_empty() {
            let overhead = r#"{"entries":[]}"#.len();
            let payload_bytes = body.len().saturating_sub(overhead).max(1);
            let avg = payload_bytes.div_ceil(count_limited.len()).max(1);
            keep = MAX_QUEUE_BYTES
                .saturating_sub(overhead)
                .checked_div(avg)
                .unwrap_or(1)
                .clamp(1, count_limited.len());
        }
    }
    keep
}

/// 被佇列上限裁掉的譯文改存這裡，不是直接消失。
pub fn overflow_path() -> PathBuf {
    super::paths::resolve_file(Path::new("shared_contribute_overflow.jsonl"))
}

/// 把即將被丟棄的條目附加到側錄檔。
///
/// 用 JSONL 而非 JSON：附加時不必先讀進整個檔案再重寫，
/// 而且就算某一行寫壞了，其餘行仍然讀得回來。
/// 寫失敗不視為錯誤——側錄是保險，不能因為保險失敗就擋住主要流程。
pub(crate) fn record_overflow(dropped: &[SharedTmEntry]) -> usize {
    if dropped.is_empty() {
        return 0;
    }
    let path = overflow_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut body = String::new();
    for entry in dropped {
        if let Ok(line) = serde_json::to_string(entry) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    if body.is_empty() {
        return 0;
    }
    match fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => {
            use std::io::Write as _;
            if file.write_all(body.as_bytes()).is_ok() {
                dropped.len()
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

/// 追加失敗條目；以 `(ns, key, source)` 雜湊去重；超出上限丟最舊。
pub(crate) fn merge_queue_entries(
    existing: &[SharedTmEntry],
    incoming: &[SharedTmEntry],
) -> (Vec<SharedTmEntry>, usize) {
    let mut q = existing.to_vec();
    let mut seen: HashSet<String> = q
        .iter()
        .map(|e| shared_tm::keyhash(&e.namespace, &e.key, &e.source))
        .collect();
    for entry in incoming {
        let kh = shared_tm::keyhash(&entry.namespace, &entry.key, &entry.source);
        if seen.insert(kh) {
            q.push(entry.clone());
        }
    }
    trim_queue_to_caps(&q)
}

pub fn enqueue(entries: &[SharedTmEntry]) -> usize {
    if entries.is_empty() {
        return 0;
    }
    static QUEUE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _lock = QUEUE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let q = load_queue();
    let (merged, dropped) = merge_queue_entries(&q, entries);
    dropped.saturating_add(save_queue(&merged).unwrap_or(0))
}

/// 讀出佇列並嘗試貢獻；成功清空；失敗／部分失敗時由 `contribute_without_flush` 把失敗條目寫回。
pub fn flush_pending() -> ContributeResult {
    flush_pending_with_budget(Instant::now() + std::time::Duration::from_secs(10))
}

/// 在牆鐘預算內 flush；未送完的寫回佇列。
pub fn flush_pending_with_budget(deadline: Instant) -> ContributeResult {
    let entries = load_queue();
    if entries.is_empty() {
        return ContributeResult::default();
    }
    clear_queue();
    if Instant::now() >= deadline {
        // 沒時間送：整包放回（仍受 trim 上限）
        let dropped = enqueue(&entries);
        return ContributeResult {
            deferred: entries.len(),
            dropped,
            ..ContributeResult::default()
        };
    }
    shared_tm::contribute_without_flush_budget(&entries, deadline, 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::translation_scope::TranslationScope;

    fn sample_entry(src: &str, zh: &str) -> SharedTmEntry {
        SharedTmEntry {
            namespace: "create".into(),
            key: "item.create.wrench".into(),
            source: src.into(),
            translated: zh.into(),
            context: Some("物品名".into()),
            scope: Some(TranslationScope::from_name("Test Pack")),
        }
    }

    #[test]
    fn queue_file_serialize_roundtrip() {
        let file = QueueFile {
            entries: vec![
                sample_entry("Wrench", "扳手"),
                sample_entry("Cogwheel", "齒輪"),
            ],
        };
        let json = serde_json::to_string_pretty(&file).expect("serialize");
        let back: QueueFile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.entries.len(), 2);
        assert_eq!(back.entries[0].source, "Wrench");
        assert_eq!(back.entries[0].translated, "扳手");
        assert_eq!(back.entries[1].namespace, "create");
        assert!(back.entries[0].scope.as_ref().unwrap().is_known());
    }

    #[test]
    fn enqueue_dedupes_by_keyhash() {
        // 不碰真實 APPDATA：測序列化＋keyhash 去重邏輯（純函式片段）
        let a = sample_entry("Wrench", "扳手");
        let b = sample_entry("Wrench", "板手"); // 同 keyhash，應去重
        let mut q = vec![a.clone()];
        let kh = shared_tm::keyhash(&b.namespace, &b.key, &b.source);
        let already = q.iter().any(|e| {
            shared_tm::keyhash(&e.namespace, &e.key, &e.source) == kh
        });
        if !already {
            q.push(b);
        }
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].translated, "扳手");
    }

    #[test]
    fn trimming_hands_back_the_dropped_entries_so_they_can_be_saved() {
        // P1-03：真實執行紀錄出現過「佇列已達上限，丟棄最舊 20 條」。
        // 那些是使用者已經翻好的譯文，舊版只回一個數字，實體直接消失。
        // 現在裁切要交出被裁掉的實體，呼叫端才能先存檔再丟。
        let entries: Vec<_> = (0..MAX_QUEUE_ENTRIES + 20)
            .map(|i| SharedTmEntry {
                namespace: "m".into(),
                key: format!("k{i}"),
                source: format!("src{i}"),
                translated: format!("譯{i}"),
                context: None,
                scope: None,
            })
            .collect();

        let (kept, dropped) = trim_queue_to_caps_detailed(&entries);
        assert_eq!(kept.len(), MAX_QUEUE_ENTRIES);
        assert_eq!(dropped.len(), 20, "多出來的 20 條要交出來，不是只回一個數字");

        // 丟的是最舊的，留的是最新的
        assert_eq!(dropped[0].key, "k0");
        assert_eq!(dropped[19].key, "k19");
        assert_eq!(kept[0].key, "k20");

        // 每一條被丟的都還帶著完整譯文，存得回去
        for entry in &dropped {
            assert!(!entry.translated.is_empty());
            assert!(!entry.source.is_empty());
        }

        // 舊介面的數量要與新介面一致，不能兩套說法
        let (_, count) = trim_queue_to_caps(&entries);
        assert_eq!(count, dropped.len());
    }

    #[test]
    fn overflow_lines_round_trip_as_jsonl() {
        // 側錄檔用 JSONL：附加不必重寫整檔，單行壞掉也不會拖垮其餘行。
        let dropped = vec![sample_entry("Wrench", "扳手"), sample_entry("Hammer", "鎚子")];
        let body: String = dropped
            .iter()
            .map(|e| serde_json::to_string(e).unwrap() + "\n")
            .collect();
        let parsed: Vec<SharedTmEntry> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].translated, "扳手");
        assert_eq!(parsed[1].translated, "鎚子");

        // 中間插一行壞資料，其餘仍讀得回來
        let with_garbage = format!("{{壞掉的行\n{body}");
        let recovered: Vec<SharedTmEntry> = with_garbage
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        assert_eq!(recovered.len(), 2, "一行壞掉不該讓整個側錄檔報廢");
    }

    #[test]
    fn empty_input_drops_nothing() {
        let (kept, dropped) = trim_queue_to_caps_detailed(&[]);
        assert!(kept.is_empty());
        assert!(dropped.is_empty());
        assert_eq!(record_overflow(&[]), 0, "沒東西要丟就不該碰檔案");
    }

    #[test]
    fn trim_queue_caps_entry_count() {
        let entries: Vec<_> = (0..10_000)
            .map(|i| SharedTmEntry {
                namespace: "m".into(),
                key: format!("k{i}"),
                source: format!("src{i}"),
                translated: format!("譯{i}"),
                context: None,
                scope: None,
            })
            .collect();
        let (trimmed, dropped) = trim_queue_to_caps(&entries);
        assert!(trimmed.len() <= MAX_QUEUE_ENTRIES);
        assert_eq!(dropped, entries.len() - trimmed.len());
        // 保留較新的尾端
        assert_eq!(trimmed.last().unwrap().key, "k9999");
    }

    #[test]
    fn merge_queue_dedupes_5000_plus_100_and_reports_dropped() {
        let existing: Vec<_> = (0..5000)
            .map(|i| SharedTmEntry {
                namespace: "m".into(),
                key: format!("k{i}"),
                source: format!("src{i}"),
                translated: format!("譯{i}"),
                context: None,
                scope: None,
            })
            .collect();
        let incoming: Vec<_> = (0..100)
            .map(|i| SharedTmEntry {
                namespace: "m".into(),
                key: format!("k{i}"),
                source: format!("src{i}"),
                translated: format!("新譯{i}"),
                context: None,
                scope: None,
            })
            .collect();
        let (merged, dropped) = merge_queue_entries(&existing, &incoming);
        assert_eq!(merged.len(), 5000);
        assert_eq!(dropped, 0);
        assert_eq!(merged[0].translated, "譯0");
    }
}
