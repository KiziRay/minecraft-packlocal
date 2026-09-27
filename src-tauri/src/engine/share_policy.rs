//! B3#3：「不上傳共享庫」的原文登記。
//!
//! 伺服器腳本（server_scripts）與 `.tell`／`.setStatusMessage` 類字串常是伺服器公告、
//! 私訊或整合包作者的私有內容，翻譯照做、寫進遊戲，但不分享給其他玩家。
//! 翻譯腳本時在送 AI **之前**登記，所有上傳入口共用的 `guard_share_entries` 會擋掉
//! （翻譯途中的自動上傳也擋得到）。B6 若改上傳過濾，沿用這份登記即可。
//!
//! 只增不減（整個執行期）：多擋一條只是少分享，不會出錯。

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn set() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn mark_private<'a>(sources: impl IntoIterator<Item = &'a String>) {
    let mut guard = set().lock().unwrap_or_else(|e| e.into_inner());
    for s in sources {
        guard.insert(s.trim().to_string());
    }
}

pub fn is_private(source: &str) -> bool {
    set().lock().unwrap_or_else(|e| e.into_inner()).contains(source.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::shared_tm::SharedTmEntry;

    #[test]
    fn b3_private_sources_are_dropped_at_the_shared_upload_entry() {
        mark_private([&"B3 private server notice".to_string()]);
        let entry = |source: &str| SharedTmEntry {
            namespace: "kubejs".into(),
            key: "0".into(),
            source: source.into(),
            translated: "伺服器公告內容".into(),
            context: None,
            scope: None,
        };
        let kept = crate::engine::shared_contribute_queue::guard_share_entries(&[
            entry("B3 private server notice"),
            entry("B3 public words here"),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source, "B3 public words here");
    }
}
