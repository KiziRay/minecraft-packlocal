//! 共享 TM 貢獻失敗時的本機重試佇列。
//!
//! 路徑：`%APPDATA%/modpack-i18n-tool/shared_contribute_queue.json`
//! 網路／HTTP 失敗時寫入；下次 `contribute` 會先 `flush_pending`。
//! 有條數／序列化大小上限，避免佇列膨脹拖垮每次翻譯。

use std::fs;
use std::path::PathBuf;
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
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("modpack-i18n-tool")
        .join("shared_contribute_queue.json")
}

fn load_queue() -> Vec<SharedTmEntry> {
    let path = queue_path();
    fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<QueueFile>(&t).ok())
        .map(|f| f.entries)
        .unwrap_or_default()
}

fn save_queue(entries: &[SharedTmEntry]) -> bool {
    let path = queue_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let trimmed = trim_queue_to_caps(entries);
    let file = QueueFile {
        entries: trimmed,
    };
    match serde_json::to_string(&file) {
        Ok(body) => fs::write(&path, body).is_ok(),
        Err(_) => false,
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
pub(crate) fn trim_queue_to_caps(entries: &[SharedTmEntry]) -> Vec<SharedTmEntry> {
    if entries.is_empty() {
        return Vec::new();
    }
    let start = entries.len().saturating_sub(MAX_QUEUE_ENTRIES);
    let mut slice = entries[start..].to_vec();
    loop {
        let Ok(body) = serde_json::to_string(&QueueFile {
            entries: slice.clone(),
        }) else {
            break;
        };
        if body.len() <= MAX_QUEUE_BYTES || slice.len() <= 1 {
            break;
        }
        // 丟最舊約 10%，直到符合大小
        let drop_n = (slice.len() / 10).max(1);
        let keep = slice.len().saturating_sub(drop_n).max(1);
        slice = slice[slice.len().saturating_sub(keep)..].to_vec();
    }
    slice
}

/// 追加失敗條目；以 `(ns, key, source)` 雜湊去重；超出上限丟最舊。
pub fn enqueue(entries: &[SharedTmEntry]) {
    if entries.is_empty() {
        return;
    }
    let mut q = load_queue();
    for entry in entries {
        let kh = shared_tm::keyhash(&entry.namespace, &entry.key, &entry.source);
        let already = q.iter().any(|e| {
            shared_tm::keyhash(&e.namespace, &e.key, &e.source) == kh
        });
        if !already {
            q.push(entry.clone());
        }
    }
    let _ = save_queue(&q);
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
        enqueue(&entries);
        return ContributeResult {
            deferred: entries.len(),
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
        let trimmed = trim_queue_to_caps(&entries);
        assert!(trimmed.len() <= MAX_QUEUE_ENTRIES);
        // 保留較新的尾端
        assert_eq!(trimmed.last().unwrap().key, "k9999");
    }
}
