//! B2：output guard 的英文原文表與模組自帶 zh_tw 表（從 output_guard.rs 拆出，控制檔案大小）。

use std::collections::HashMap;
#[cfg(not(test))]
use std::sync::{Mutex, OnceLock};

use super::jar_scan::LangMap;

// ─── 英文原文登記（lang 條目寫出時查原文用）─────────────────────

#[derive(Default)]
struct SourceTable {
    /// (命名空間, 鍵) → 英文原文
    en: HashMap<(String, String), String>,
    /// (命名空間, 鍵) → 模組自帶 zh_tw
    native: HashMap<(String, String), String>,
    /// (命名空間, 鍵) → 使用者選的參考包／CFPA 人工譯文（實際寫出的值）
    reference: HashMap<(String, String), String>,
}

/// 正式執行：整個程式共用一張表（掃描與建包可能在不同執行緒）。
#[cfg(not(test))]
fn with_sources<R>(f: impl FnOnce(&mut SourceTable) -> R) -> R {
    static SRC: OnceLock<Mutex<SourceTable>> = OnceLock::new();
    let mut map = SRC
        .get_or_init(|| Mutex::new(SourceTable::default()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    f(&mut map)
}

/// 測試：每個測試執行緒各一張表，平行測試（含會呼叫 scan_instance 的測試）互不清掉對方。
#[cfg(test)]
fn with_sources<R>(f: impl FnOnce(&mut SourceTable) -> R) -> R {
    thread_local! {
        static SRC: std::cell::RefCell<SourceTable> = std::cell::RefCell::new(SourceTable::default());
    }
    SRC.with(|m| f(&mut m.borrow_mut()))
}

/// 登記英文原文（掃描完成、修復／補翻建包前），資源包寫出時才查得到每個 key 的原文。
pub fn remember_sources(en: &LangMap) {
    with_sources(|map| {
        for (ns, entries) in en {
            for (key, text) in entries {
                map.en.insert((ns.clone(), key.clone()), text.clone());
            }
        }
    });
}

/// 目前的英文原文表（存成「英文原文表.json」用）。
pub fn snapshot_sources() -> LangMap {
    with_sources(|map| to_langmap(&map.en))
}

/// 目前的模組自帶 zh_tw 表。
pub fn snapshot_native() -> LangMap {
    with_sources(|map| to_langmap(&map.native))
}

fn to_langmap(table: &HashMap<(String, String), String>) -> LangMap {
    let mut out = LangMap::new();
    for ((ns, key), text) in table {
        out.entry(ns.clone()).or_default().insert(key.clone(), text.clone());
    }
    out
}

/// 登記模組自帶的 zh_tw：寫出的譯文就是模組作者自己的翻譯時，不是工具產生的，不套用長度等檢查。
pub fn remember_native(zh_tw: &LangMap) {
    with_sources(|map| {
        for (ns, entries) in zh_tw {
            for (key, text) in entries {
                map.native.insert((ns.clone(), key.clone()), text.clone());
            }
        }
    });
}

pub(crate) fn native_of(ns: &str, key: &str) -> Option<String> {
    with_sources(|map| map.native.get(&(ns.to_string(), key.to_string())).cloned())
}


/// 清空英文原文表（每次掃描整合包前），避免第二個整合包拿到第一包的英文。
pub fn reset_sources() {
    with_sources(|map| {
        map.en.clear();
        map.native.clear();
        map.reference.clear();
    });
}

pub(crate) fn source_of(ns: &str, key: &str) -> Option<String> {
    with_sources(|map| map.en.get(&(ns.to_string(), key.to_string())).cloned())
}

/// 登記參考包（人工譯文）實際要寫出的值：寫出時只免長度檢查，其餘照常。
pub fn remember_reference(zh: &LangMap) {
    with_sources(|map| {
        for (ns, entries) in zh {
            for (key, text) in entries {
                map.reference.insert((ns.clone(), key.clone()), text.clone());
            }
        }
    });
}

pub(crate) fn reference_of(ns: &str, key: &str) -> Option<String> {
    with_sources(|map| map.reference.get(&(ns.to_string(), key.to_string())).cloned())
}

/// 目前的參考包譯文表（存進英文原文表用）。
pub fn snapshot_reference() -> LangMap {
    with_sources(|map| to_langmap(&map.reference))
}
