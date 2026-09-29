//! B2#6：guard 沒過的譯文不存進翻譯記憶、不上傳共享庫。

use crate::engine::shared_contribute_queue::guard_share_entries;
use crate::engine::shared_tm::SharedTmEntry;
use crate::engine::tm::Tm;

fn entry(source: &str, translated: &str) -> SharedTmEntry {
    SharedTmEntry {
        namespace: "b2share".into(),
        key: "k".into(),
        source: source.into(),
        translated: translated.into(),
        context: None,
        scope: None,
    }
}

#[test]
fn b2_guard_failures_never_enter_translation_memory() {
    let mut tm = Tm::default();
    // 太長（按鈕會爆框）
    tm.insert("On", "目前處於開啟狀態");
    // 圖示字被弄丟
    tm.insert("\u{E001} Mana", "魔力");
    // Force 模式的 upsert 也一樣
    tm.upsert("Off", "目前處於關閉狀態");
    assert!(tm.get("On").is_none());
    assert!(tm.get("\u{E001} Mana").is_none());
    assert!(tm.get("Off").is_none());
    // 正常的照存
    tm.insert("Cancel", "取消");
    assert_eq!(tm.get("Cancel").as_deref(), Some("取消"));
}

#[test]
fn b2_guard_failures_are_never_uploaded() {
    let entries = vec![
        entry("On", "目前處於開啟狀態"),
        entry("Deals %s damage", "造成傷害"),
        entry("Cancel", "取消"),
    ];
    let kept = guard_share_entries(&entries);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].translated, "取消");
}

#[test]
fn b2_every_upload_and_memory_path_filters_with_guard() {
    // 上傳：佇列入口與送出入口都要過濾；deepseek 的兩個蒐集點也要
    let queue = include_str!("shared_contribute_queue.rs");
    let enqueue = &queue[queue.find("pub fn enqueue(").unwrap()..];
    let enqueue = &enqueue[..enqueue.find("\n}\n").unwrap()];
    assert!(enqueue.contains("guard_share_entries("), "enqueue 沒過濾");

    let shared = include_str!("shared_tm.rs");
    let contribute = &shared[shared.find("fn contribute_budgeted(").unwrap()..];
    let contribute = &contribute[..contribute.find("\n}\n").unwrap()];
    assert!(contribute.contains("guard_share_entries("), "contribute 沒過濾");

    // T1：deepseek.rs 拆成 deepseek/ 子模組，兩個蒐集點分在 fill.rs 與 plain.rs
    for (deepseek, anchor) in [
        (include_str!("deepseek/fill.rs"), "to_share.push(shared_tm::SharedTmEntry {"),
        (include_str!("deepseek/plain.rs"), "fn contribute_plain_job_outputs("),
    ] {
        let at = deepseek.find(anchor).unwrap_or_else(|| panic!("找不到 {anchor}"));
        let window = &deepseek[at.saturating_sub(600)..(at + 900).min(deepseek.len())];
        assert!(window.contains("output_guard::passes("), "{anchor} 附近沒有過 guard");
    }

    // 翻譯記憶：insert 與 upsert 都要擋
    let tm = include_str!("tm.rs");
    assert_eq!(tm.matches("output_guard::passes(").count(), 2, "tm insert／upsert 都要過 guard");
}
