//! B2#5：寫檔點接上 guard 後的實際行為（只退回壞的那一條，同檔其他條目照寫）。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::engine::jar_scan::LangMap;
use crate::engine::output_guard::{peek_rejected_for, remember_sources};
use crate::engine::pack_out::{build_resource_pack, BuildOptions};

fn temp(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("b2_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn b2_resource_pack_drops_only_the_bad_entry() {
    let root = temp("pack_guard");
    let ns = "b2guardmod";
    let mut en: LangMap = HashMap::new();
    let e = en.entry(ns.into()).or_default();
    e.insert("item.b2guardmod.bad".into(), "Deals %s damage".into());
    e.insert("item.b2guardmod.good".into(), "Iron Sword".into());
    e.insert("gui.b2guardmod.yes".into(), "Yes".into());
    remember_sources(&en);

    let mut zh: LangMap = HashMap::new();
    let z = zh.entry(ns.into()).or_default();
    z.insert("item.b2guardmod.bad".into(), "造成傷害".into());
    z.insert("item.b2guardmod.good".into(), "鐵劍".into());
    z.insert("gui.b2guardmod.yes".into(), "是".into());
    let opts = BuildOptions {
        output_dir: root.display().to_string(),
        pack_folder_name: "guard".into(),
        pack_description: "t".into(),
        pack_format: 15,
        target_version: Some("1.20.1".into()),
    };
    let built = build_resource_pack(&zh, &opts).unwrap();
    let text = fs::read_to_string(PathBuf::from(&built.pack_dir).join(format!("assets/{ns}/lang/zh_tw.json"))).unwrap();
    assert!(!text.contains("item.b2guardmod.bad"), "壞的那條不寫，遊戲顯示英文：{text}");
    assert!(text.contains("鐵劍") && text.contains("\"是\""), "同檔其他條目照寫：{text}");
    let rejected = peek_rejected_for("翻譯資源包");
    assert!(rejected.iter().any(|r| r.key == "item.b2guardmod.bad" && r.code == "format_codes"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b2_imported_translation_is_guarded() {
    let mut pending: LangMap = HashMap::new();
    let p = pending.entry("b2imp".into()).or_default();
    p.insert("gui.on".into(), "On".into());
    p.insert("gui.off".into(), "Off".into());
    let mut zh: LangMap = HashMap::new();
    let entries = vec![
        (Some("b2imp".to_string()), "gui.on".to_string(), "開啟".to_string()),
        // 太長：會超出按鈕
        (Some("b2imp".to_string()), "gui.off".to_string(), "目前處於關閉狀態".to_string()),
    ];
    let report = crate::engine::failed_items::merge_imported(&mut zh, &pending, &entries);
    assert_eq!(report.accepted, 1);
    assert_eq!(zh["b2imp"].get("gui.on").map(String::as_str), Some("開啟"));
    assert!(!zh["b2imp"].contains_key("gui.off"));
    assert!(report.rejected.iter().any(|r| r.contains("gui.off")));
}

#[test]
fn b2_reference_pack_entries_are_guarded() {
    let mut base: LangMap = HashMap::new();
    let mut reference: LangMap = HashMap::new();
    let r = reference.entry("b2ref".into()).or_default();
    r.insert("a".into(), "正常譯文".into());
    r.insert("b".into(), "壞\u{FFFD}掉".into());
    crate::engine::merge_ref::merge_fill_missing(&mut base, &reference);
    assert_eq!(base["b2ref"].get("a").map(String::as_str), Some("正常譯文"));
    assert!(!base["b2ref"].contains_key("b"), "亂碼條目不併入");
}
