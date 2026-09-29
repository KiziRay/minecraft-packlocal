//! B6a-1：已覆蓋的模組 JAR 被整合包更新後，「翻譯更新的部分」以新版原檔為底重新改寫，不沿用工具舊版。

use super::tests::{Stage, BACKUP};
use super::*;
use std::io::{Read, Write};

fn jar(en: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let body: serde_json::Map<String, serde_json::Value> =
            en.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect();
        zip.start_file("assets/example/lang/en_us.json", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(serde_json::Value::Object(body).to_string().as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn entry(jar: &Path, name: &str) -> Option<String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(jar).ok()?).ok()?;
    let mut e = zip.by_name(name).ok()?;
    let mut s = String::new();
    e.read_to_string(&mut s).ok()?;
    Some(s)
}

fn zh(entries: &[(&str, &str)]) -> crate::engine::jar_scan::LangMap {
    let mut m = crate::engine::jar_scan::LangMap::new();
    let slot = m.entry("example".to_string()).or_default();
    for (k, v) in entries {
        slot.insert(k.to_string(), v.to_string());
    }
    m
}

fn rewrite(stage: &Stage, translated: &crate::engine::jar_scan::LangMap) {
    crate::engine::jar_translate::rewrite_translated_jars(&stage.mc, translated, &Default::default(), &stage.work, |_, _, _| {})
        .unwrap();
}

#[test]
fn b6a1_updated_jar_is_rewritten_from_the_new_version_not_the_tool_copy() {
    let stage = Stage::new("b6a1-jar-update");
    let v1 = jar(&[("item.example.apple", "Apple")]);
    fs::write(stage.mc.join("mods/example.jar"), &v1).unwrap();
    rewrite(&stage, &zh(&[("item.example.apple", "蘋果")]));
    let first = stage.apply(BACKUP);
    assert!(first.outdated_mods.is_empty(), "{:?}", first.outdated_mods);
    let placed = stage.mc.join("mods/example.jar");
    assert!(entry(&placed, "assets/example/lang/zh_tw.json").unwrap().contains("蘋果"), "第一次套用放進翻譯版");

    // 整合包更新：同名模組換成新版（多一句）
    let v2 = jar(&[("item.example.apple", "Apple"), ("item.example.banana", "Banana")]);
    fs::write(&placed, &v2).unwrap();
    let stale = stage.apply(BACKUP);
    assert!(stale.outdated_mods.contains(&"mods/example.jar".to_string()), "舊翻譯版不能蓋掉新版：{:?}", stale.outdated_mods);
    assert_eq!(fs::read(&placed).unwrap(), v2);

    // 翻譯更新的部分：以新版原檔為底重新改寫（遊戲裡的新版不是工具版，照原檔讀）
    rewrite(&stage, &zh(&[("item.example.apple", "蘋果"), ("item.example.banana", "香蕉")]));
    let out = stage.work.join("jar-translated/example.jar");
    assert!(entry(&out, "assets/example/lang/en_us.json").unwrap().contains("Banana"), "改寫的底是新版");
    assert!(entry(&out, "assets/example/lang/zh_tw.json").unwrap().contains("香蕉"), "新句子有翻");
    let again = stage.apply(BACKUP);
    assert!(
        !again.outdated_mods.contains(&"mods/example.jar".to_string()),
        "重新改寫後不再列「模組已更新，需重新翻譯」：{:?}",
        again.outdated_mods
    );
    assert_eq!(again.status, ApplyStatus::Applied);
    assert!(again.skipped_changed.is_empty(), "{:?}", again.skipped_changed);
    assert!(entry(&placed, "assets/example/lang/zh_tw.json").unwrap().contains("香蕉"), "遊戲裡換成新版的翻譯版");
}

/// 審查 F3：套用把翻譯版 JAR 放進 mods/（大小變了）不能被當成「整合包已更新」；移除翻譯後也不能；
/// 整合包真的換了新版才算。判斷以原檔為準，工具版不會被記成原檔。
#[test]
fn b6a1_f3_applying_the_translation_is_not_a_pack_update() {
    let stage = Stage::new("b6a1-f3");
    fs::write(stage.mc.join("mods/example.jar"), jar(&[("item.example.apple", "Apple")])).unwrap();
    let original_size = fs::metadata(stage.mc.join("mods/example.jar")).unwrap().len();
    rewrite(&stage, &zh(&[("item.example.apple", "蘋果")]));
    // 翻譯當時（套用前）記下的判斷依據
    let basis = crate::engine::pack_update::basis_for(&stage.mc, Some(&stage.work), &Default::default());
    assert_eq!(basis.mod_files.get("example.jar"), Some(&original_size));
    let fingerprint = crate::engine::session::mods_fingerprint(&stage.mc);
    stage.apply(BACKUP);
    let tool_size = fs::metadata(stage.mc.join("mods/example.jar")).unwrap().len();
    assert_ne!(tool_size, original_size, "前提：翻譯版大小不同");
    let changed = |work: Option<&Path>| crate::engine::pack_update::mods_changed(fingerprint, &basis, &stage.mc, work);
    assert!(!changed(Some(&stage.work)), "套用翻譯不是整合包更新");
    assert!(!changed(None), "沒有翻譯結果可比時，靠 .mcpl 標記認得工具內容");
    // 套用後再記一次依據（接續補完重掃後）：記的是原檔大小，不是工具版
    let again = crate::engine::pack_update::basis_for(&stage.mc, Some(&stage.work), &Default::default());
    assert_eq!(again.mod_files.get("example.jar"), Some(&original_size), "工具版不會被記成原檔");
    // 整合包真的換了新版
    fs::write(stage.mc.join("mods/example.jar"), jar(&[("item.example.apple", "Apple"), ("item.example.kiwi", "Kiwi")])).unwrap();
    assert!(changed(Some(&stage.work)), "換成新版要算更新");
}

/// 審查 F3：移除翻譯（放回原檔）後也不算更新。
#[test]
fn b6a1_f3_removing_the_translation_is_not_a_pack_update() {
    let stage = Stage::new("b6a1-f3-restore");
    fs::write(stage.mc.join("mods/example.jar"), jar(&[("item.example.apple", "Apple")])).unwrap();
    rewrite(&stage, &zh(&[("item.example.apple", "蘋果")]));
    let basis = crate::engine::pack_update::basis_for(&stage.mc, Some(&stage.work), &Default::default());
    stage.apply(BACKUP);
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(!crate::engine::pack_update::mods_changed(0, &basis, &stage.mc, Some(&stage.work)));
}

/// 第二輪 2：原檔大小只用確認過的原檔備份（ToolIndex：識別碼、位置、原檔指紋都要對）；備份被動過就不採用。
#[test]
fn b6a1_fix2_2_tampered_backup_size_is_not_trusted() {
    let stage = Stage::new("b6a1-fix2-backup");
    fs::write(stage.mc.join("mods/example.jar"), jar(&[("item.example.apple", "Apple")])).unwrap();
    rewrite(&stage, &zh(&[("item.example.apple", "蘋果")]));
    let basis = crate::engine::pack_update::basis_for(&stage.mc, Some(&stage.work), &Default::default());
    stage.apply(BACKUP);
    let backup = crate::engine::apply_guard::backup_paths_at(&apply_record::instance_backup_dir(&stage.mc), "mods/example.jar").0;
    assert!(backup.is_file(), "前提：有原檔備份 {}", backup.display());
    let mut bytes = fs::read(&backup).unwrap();
    bytes.extend_from_slice(b"tampered");
    fs::write(&backup, bytes).unwrap();
    let now = crate::engine::pack_update::effective_mod_files(&stage.mc, None, &basis.mod_files);
    assert_eq!(now.get("example.jar"), basis.mod_files.get("example.jar"), "指紋不符的備份不採用，改用上一輪記下的原檔大小");
}
