//! B6a-1 第二輪修正：mods/ 讀不到或是空的不清譯文、JAR 內反斜線路徑、任務來源以指紋判斷已處理。

use super::*;
use crate::engine::jar_scan::scan_instance;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

fn temp(name: &str) -> PathBuf {
    let root = crate::engine::paths::test_data_base().join(format!("b6a1-fix2-{name}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn jar_bytes(entry: &str, en: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let body: serde_json::Map<String, serde_json::Value> =
            en.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect();
        zip.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(serde_json::Value::Object(body).to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn lm(entries: &[(&str, &str, &str)]) -> LangMap {
    let mut m = LangMap::new();
    for (ns, k, v) in entries {
        m.entry(ns.to_string()).or_default().insert(k.to_string(), v.to_string());
    }
    m
}

fn pack(name: &str) -> (PathBuf, UpdateBasis) {
    let mc = temp(name).join("game");
    fs::create_dir_all(mc.join("mods")).unwrap();
    fs::write(mc.join("mods/alpha.jar"), jar_bytes("assets/alpha/lang/en_us.json", &[("item.alpha.apple", "Apple")])).unwrap();
    scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let basis = basis_for(&mc, None, &crate::engine::output_guard::snapshot_sources());
    (mc, basis)
}

/// 重掃後整輪翻譯接續舊譯文（drop_stale）與補翻（plan_refresh）都不能清掉。
fn assert_nothing_wiped(mc: &Path, basis: &UpdateBasis, why: &str) {
    let (zh_scan, en_only, _, report) = scan_instance(mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    let en_full = crate::engine::output_guard::snapshot_sources();
    let now = mods_now(mc);
    let old_zh = lm(&[("alpha", "item.alpha.apple", "人工補翻的蘋果")]);
    let r = plan_refresh(RefreshInput {
        old_hashes: &basis.source_hashes,
        old_pending: &LangMap::new(),
        en_full: &en_full,
        en_only: &en_only,
        zh_scan: &zh_scan,
        old_zh: &old_zh,
        removal: RemovalCheck { scan_clean: scan_is_clean(&report.errors), old_ns_jars: &basis.ns_jars, mods_now: &now },
    });
    assert!(r.zh.contains_key("alpha") && r.removed_namespaces.is_empty(), "{why}：補翻不能清掉 {:?}", r.removed_namespaces);
    let mut prior = old_zh.clone();
    let removal = RemovalCheck { scan_clean: scan_is_clean(&report.errors), old_ns_jars: &basis.ns_jars, mods_now: &now };
    assert_eq!(drop_stale(&mut prior, &basis.source_hashes, &hashes_of(&en_full), &zh_scan, &removal), (0, 0), "{why}");
    assert!(prior.contains_key("alpha"), "{why}：整輪翻譯不能清掉");
}

#[test]
fn b6a1_fix2_1_unreadable_mods_folder_wipes_nothing_and_is_reported() {
    let (mc, basis) = pack("mods-unreadable");
    // mods/ 讀不到（網路磁碟斷線時的樣子：不是資料夾）：重掃直接失敗（接續補完停下），而且移除判斷一律無法確認
    fs::remove_dir_all(mc.join("mods")).unwrap();
    fs::write(mc.join("mods"), b"not a folder").unwrap();
    assert!(scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).is_err());
    let now = mods_now(&mc);
    assert!(now.is_empty());
    let removal = RemovalCheck { scan_clean: true, old_ns_jars: &basis.ns_jars, mods_now: &now };
    assert!(!removal.surely_removed("alpha"), "mods/ 讀不到＝無法確認");
    let mut prior = lm(&[("alpha", "item.alpha.apple", "人工補翻的蘋果")]);
    assert_eq!(drop_stale(&mut prior, &basis.source_hashes, &SourceHashes::new(), &LangMap::new(), &removal), (0, 0));
    assert!(prior.contains_key("alpha"));
}

#[test]
fn b6a1_fix2_1_empty_mods_folder_wipes_nothing() {
    let (mc, basis) = pack("mods-empty");
    fs::remove_file(mc.join("mods/alpha.jar")).unwrap();
    assert_nothing_wiped(&mc, &basis, "mods/ 是空的");
}

#[test]
fn b6a1_fix2_backslash_paths_inside_jars_are_recognised() {
    let mc = temp("backslash").join("game");
    fs::create_dir_all(mc.join("mods")).unwrap();
    fs::write(mc.join("mods/win.jar"), jar_bytes("assets\\winmod\\lang\\en_us.json", &[("k", "Text")])).unwrap();
    assert_eq!(ns_jars(&mc).get("winmod"), Some(&vec!["win.jar".to_string()]));
}

/// 第二輪 3：來源內容改了但修改時間比較舊（解壓保留時間戳）→ 仍報 S15；產出者重跑後消失。
#[test]
fn b6a1_fix2_3_quest_change_with_an_old_timestamp_is_still_reported() {
    let root = temp("quest-mtime");
    let mc = root.join("game");
    let work = root.join("work");
    let quest = mc.join("config/ftbquests/quests/chapters/intro.snbt");
    fs::create_dir_all(quest.parent().unwrap()).unwrap();
    fs::create_dir_all(&work).unwrap();
    let run = || crate::engine::ftbquests::translate_ftbquests(&mc, &work, false, None, |_, _| {}).unwrap();
    fs::write(&quest, "{ title: \"苹果任务\" }").unwrap();
    run();
    assert_eq!(crate::engine::text_sources::count_changed_sources(&work, &mc), 0);
    fs::write(&quest, "{ title: \"Banana quest\" }").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * 30);
    fs::File::options().write(true).open(&quest).unwrap().set_modified(old).unwrap();
    assert_eq!(crate::engine::text_sources::count_changed_sources(&work, &mc), 1, "修改時間較舊也要報");
    run();
    assert_eq!(crate::engine::text_sources::count_changed_sources(&work, &mc), 0, "重跑（這版沒有可翻的字）後消失");
}
