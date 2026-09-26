//! B2 審查 F8：所有上傳路徑（contribute、LangMap 掃尾、佇列 flush）都過 output guard。
//! 用「已過期的期限＋不 flush」讓 contribute_budgeted 只計算會送出的條數，不碰網路。

use super::*;

fn entry(key: &str, source: &str, translated: &str) -> SharedTmEntry {
    SharedTmEntry {
        namespace: "b2f8".into(),
        key: key.into(),
        source: source.into(),
        translated: translated.into(),
        context: None,
        scope: None,
    }
}

#[test]
fn b2_f8_budgeted_path_drops_guard_failures() {
    // contribute_lang_maps_limited 走這條（繞過 contribute()）
    let entries = vec![
        entry("b2f8.a", "On", "目前處於開啟狀態"),
        entry("b2f8.b", "\u{E001} Mana", "魔力"),
        entry("b2f8.c", "Cancel", "取消"),
    ];
    let past = Instant::now() - Duration::from_secs(1);
    let result = contribute_budgeted(&entries, past, 1, true);
    assert_eq!(result.deferred, 1, "只有合格的那條會被送出／暫存：{result:?}");
}

#[test]
fn b2_f8_queue_flush_path_drops_guard_failures() {
    // flush_pending 送舊佇列走 contribute_without_flush_budget
    let entries = vec![entry("b2f8.q", "Off", "目前處於關閉狀態")];
    let result = contribute_without_flush_budget(&entries, Instant::now() + Duration::from_secs(1), 2);
    assert_eq!(result.attempted, 0);
    assert_eq!(result.deferred, 0);
    let queue = include_str!("shared_contribute_queue.rs");
    let flush = &queue[queue.find("pub fn flush_pending_with_budget(").unwrap()..];
    assert!(flush[..flush.find("\n}\n").unwrap()].contains("contribute_without_flush_budget("));
}

#[test]
fn b2_f8_single_entry_points_call_guard() {
    let src = include_str!("shared_tm.rs");
    for name in ["fn contribute_budgeted(", "pub(crate) fn contribute_without_flush_budget("] {
        let body = &src[src.find(name).unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        let head: String = body.lines().take(12).collect::<Vec<_>>().join("\n");
        assert!(head.contains("guard_share_entries("), "{name} 開頭要先過 guard");
    }
}
