use std::fs;

use super::test_support::*;
use super::*;
use crate::engine::apply_record::{ApplyRecord, FileKind, Origin};

#[test]
fn b3_marker_with_matching_content_is_tool_written_but_edited_file_is_not() {
    let mc = temp_game("toolprod");
    let file = mc.join("config/ftbquests/quests/lang/zh_tw.snbt");
    write(&file, "{ a: \"機翻\" }");
    assert!(ToolIndex::for_game(&mc).is_original(&file), "沒有標記、沒裝過翻譯＝原檔");
    mark_as_tool_written(&mc, "config/ftbquests/quests/lang/zh_tw.snbt");
    assert_eq!(ToolIndex::for_game(&mc).classify(&file), Provenance::Tool);
    write(&file, "{ a: \"玩家改過\" }");
    assert!(ToolIndex::for_game(&mc).is_original(&file), "玩家改過的內容要保留，不算工具產物");
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_f2_tool_version_is_read_from_backup_or_reported_as_needing_original() {
    let mc = temp_game("f2lang");
    let rel = "kubejs/assets/foo/lang/en_us.json";
    mark_overwritten_with_backup(&mc, rel, r#"{"a":"你好"}"#, Some(r#"{"a":"Hello"}"#));
    let jobs = crate::engine::lang_overlay::collect_jobs(&mc, &[mc.join(rel)]);
    assert_eq!(crate::engine::lang_overlay::candidates(&jobs), vec!["Hello".to_string()], "讀原檔備份，不拿工具中文當原文");

    let mc2 = temp_game("f2lang-nobackup");
    mark_overwritten_with_backup(&mc2, rel, r#"{"a":"你好"}"#, None);
    let jobs = crate::engine::lang_overlay::collect_jobs(&mc2, &[mc2.join(rel)]);
    assert!(jobs.is_empty(), "沒有原檔就不翻");
    assert!(crate::engine::output_guard::peek_needs_original().contains(&rel.to_string()));
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&mc2);
}

#[test]
fn b3_f2_scripts_and_quest_lang_read_the_original() {
    let mc = temp_game("f2script");
    let out = temp_game("f2script-out");
    mark_overwritten_with_backup(&mc, "kubejs/client_scripts/a.js", "// 舊版工具把腳本弄壞了\n", Some("Text.of('Hello world')\n"));
    let report = crate::engine::script_literals::translate_kubejs_literals(&mc, &out, false, None, |_, _| {}).unwrap();
    assert_eq!(report.strings_found, 1, "要讀原檔備份裡的顯示字串：{}", report.note);

    mark_overwritten_with_backup(&mc, "config/ftbquests/quests/lang/en_us.snbt", "{\n\tquest.A.title: \"舊機翻\"\n}\n", None);
    let jobs = crate::engine::ftbquests_lang::collect_jobs(&mc, &mc.join("config/ftbquests"));
    assert!(jobs.is_empty(), "工具內容沒有原檔：不能當英文來源");
    assert!(crate::engine::output_guard::peek_needs_original()
        .contains(&"config/ftbquests/quests/lang/en_us.snbt".to_string()));
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&out);
}

fn save_record(mc: &std::path::Path, entries: &[(&str, &str)]) {
    let mut record = ApplyRecord::default();
    for (rel, content) in entries {
        record.set_entry(rel, FileKind::Added, Origin::Known, None, crate::engine::hashutil::sha256_hex(content.as_bytes()));
    }
    crate::engine::apply_record::save(mc, &mut record).unwrap();
}

#[test]
fn b3_f4_without_mcpl_the_record_still_identifies_tool_files_and_the_rest_is_unconfirmed() {
    let mc = temp_game("f4");
    fs::create_dir_all(mc.join("mods")).unwrap();
    let tool_text = "{\n\tquest.A.title: \"工具機翻\"\n}\n";
    write(&mc.join("config/ftbquests/quests/lang/en_us.snbt"), "{\n\tquest.A.title: \"Hello\"\n}\n");
    write(&mc.join("config/ftbquests/quests/lang/zh_tw.snbt"), tool_text);
    write(&mc.join("kubejs/assets/foo/lang/en_us.json"), r#"{"a":"Hello"}"#);
    write(&mc.join("kubejs/assets/foo/lang/zh_tw.json"), r#"{"a":"不知道是誰寫的"}"#);
    // 裝過翻譯，但 .mcpl 被刪掉了：紀錄（依位置找回）還記得 zh_tw.snbt 是工具寫的
    save_record(&mc, &[("config/ftbquests/quests/lang/zh_tw.snbt", tool_text)]);
    assert!(!mc.join(".mcpl").exists());
    let index = ToolIndex::for_game(&mc);
    assert_eq!(index.classify(&mc.join("config/ftbquests/quests/lang/zh_tw.snbt")), Provenance::Tool);
    assert_eq!(index.classify(&mc.join("kubejs/assets/foo/lang/zh_tw.json")), Provenance::Unconfirmed);

    let quest = crate::engine::ftbquests_lang::collect_jobs(&mc, &mc.join("config/ftbquests"));
    assert_eq!(crate::engine::ftbquests_lang::candidates(&quest), vec!["Hello".to_string()], "工具的 zh_tw.snbt 不當人工");
    let lang = crate::engine::lang_overlay::collect_jobs(&mc, &[mc.join("kubejs/assets/foo/lang/en_us.json")]);
    assert_eq!(crate::engine::lang_overlay::candidates(&lang), vec!["Hello".to_string()], "無法確認的 zh_tw 不保留");
    assert!(mc.join("kubejs/assets/foo/lang/zh_tw.json").is_file(), "也不刪玩家檔");
    let _ = fs::remove_dir_all(&mc);
}

fn zip_text(files: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        for (name, body) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

#[test]
fn b3_f3_tool_written_datapack_zip_is_read_from_its_original() {
    let mc = temp_game("f3zip");
    let work = temp_game("f3zip-work");
    let rel = "datapacks/pack.zip";
    let original = zip_text(&[("data/ns/advancements/a.json", r#"{"display":{"title":"简体原文"}}"#)]);
    let tool = zip_text(&[("data/ns/advancements/a.json", r#"{"display":{"title":"工具上次的版本"}}"#)]);
    // 工具上次改寫過這個 ZIP（有標記），原檔在備份
    write(&mc.join(rel), "x");
    std::fs::write(mc.join(rel), &tool).unwrap();
    let tool_sha = crate::engine::hashutil::sha256_hex(&tool);
    let original_sha = crate::engine::hashutil::sha256_hex(&original);
    mark_overwritten_with_backup(&mc, rel, "placeholder", Some("placeholder-original"));
    // 換成真的 ZIP 內容與指紋
    std::fs::write(mc.join(rel), &tool).unwrap();
    std::fs::write(crate::engine::apply_guard::backup_file_path(&mc, rel), &original).unwrap();
    let mut marker = crate::engine::mcpl_marker::read_file_marker(&mc, rel).unwrap();
    marker.tool_sha256 = tool_sha;
    marker.original_sha256 = original_sha.clone();
    crate::engine::mcpl_marker::write_file_marker(&mc, &marker).unwrap();
    let mut backup = crate::engine::apply_guard::read_backup_marker(&mc, rel).unwrap();
    backup.original_sha256 = original_sha;
    write(&crate::engine::apply_guard::backup_marker_path(&mc, rel), &serde_json::to_string(&backup).unwrap());

    crate::engine::archive_overlay::translate_archive_overlays(&mc, &work, false, None, |_, _| {}).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(work.join(rel)).expect("用原檔重建")).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("data/ns/advancements/a.json").unwrap(), &mut text).unwrap();
    assert!(text.contains("簡體原文"), "要以原檔 ZIP 為來源：{text}");

    // 沒有原檔：不拿工具版本當來源，列入需要原檔
    fs::remove_file(crate::engine::apply_guard::backup_file_path(&mc, rel)).unwrap();
    let work2 = temp_game("f3zip-work2");
    let report = crate::engine::archive_overlay::translate_archive_overlays(&mc, &work2, false, None, |_, _| {}).unwrap();
    assert!(!work2.join(rel).exists(), "{:?}", report.skipped);
    assert!(crate::engine::output_guard::peek_needs_original().contains(&rel.to_string()));
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_dir_all(&work2);
}

#[test]
fn b3_f5_tool_jar_without_backup_keeps_author_zh_tw_and_zh_cn() {
    let root = temp_game("f5");
    let mc = root.join("minecraft");
    let work = root.join("翻譯結果");
    fs::create_dir_all(mc.join("mods")).unwrap();
    // 原本的模組：作者自帶 zh_tw（own）與 zh_cn
    let original = zip_text(&[
        ("assets/m/lang/en_us.json", r#"{"own":"Own","gap":"Gap"}"#),
        ("assets/m/lang/zh_tw.json", r#"{"own":"作者"}"#),
        ("assets/m/lang/zh_cn.json", r#"{"gap":"作者简中"}"#),
    ]);
    fs::write(mc.join("mods/m.jar"), &original).unwrap();
    // 翻譯：改寫 JAR 時記下作者自帶的 zh_tw key
    let mut translated = crate::engine::jar_scan::LangMap::new();
    translated.entry("m".into()).or_default().insert("gap".into(), "工具補的".into());
    // 正式流程的譯文表含模組自帶的中文
    translated.entry("m".into()).or_default().insert("own".into(), "作者".into());
    let mut english = crate::engine::jar_scan::LangMap::new();
    english.entry("m".into()).or_default().insert("gap".into(), "Gap".into());
    english.entry("m".into()).or_default().insert("own".into(), "Own".into());
    crate::engine::jar_translate::rewrite_translated_jars(&mc, &translated, &english, &work, |_, _, _| {}).unwrap();
    // 套用：遊戲裡換成工具翻過的 JAR（有標記），玩家選了不備份（沒有原檔）
    let tool_jar = fs::read(work.join("jar-translated/m.jar")).unwrap();
    fs::write(mc.join("mods/m.jar"), &tool_jar).unwrap();
    mark_overwritten_with_backup(&mc, "mods/m.jar", "placeholder", None);
    fs::write(mc.join("mods/m.jar"), &tool_jar).unwrap();
    let mut marker = crate::engine::mcpl_marker::read_file_marker(&mc, "mods/m.jar").unwrap();
    marker.tool_sha256 = crate::engine::hashutil::sha256_hex(&tool_jar);
    crate::engine::mcpl_marker::write_file_marker(&mc, &marker).unwrap();

    let (zh, en_only, _, _) =
        crate::engine::jar_scan::scan_instance(&mc, &std::collections::HashMap::new(), true, false, |_, _| {}).unwrap();
    assert_eq!(zh["m"].get("own").map(String::as_str), Some("作者"), "作者的 zh_tw 保留");
    assert!(zh["m"].get("gap").is_none_or(|v| v != "工具補的"), "工具補的 zh_tw 不當模組自帶：{zh:?}");
    assert!(zh["m"].contains_key("gap") || en_only["m"].contains_key("gap"));
    assert!(zh["m"].get("gap").is_none_or(|v| v.contains("作者")), "zh_cn 作者譯文仍可用來轉繁：{zh:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn b3_fd_tool_written_zh_tw_inside_a_folder_pack_is_not_human() {
    let mc = temp_game("fd");
    fs::create_dir_all(mc.join("mods")).unwrap();
    write(&mc.join("resourcepacks/UserPack/assets/m/lang/en_us.json"), r#"{"k":"Key"}"#);
    write(&mc.join("resourcepacks/UserPack/assets/m/lang/zh_tw.json"), r#"{"k":"機翻"}"#);
    mark_as_tool_written(&mc, "resourcepacks/UserPack/assets/m/lang/zh_tw.json");
    let (zh, en_only, _, _) =
        crate::engine::jar_scan::scan_instance(&mc, &std::collections::HashMap::new(), false, false, |_, _| {}).unwrap();
    assert!(zh.get("m").is_none_or(|m| !m.contains_key("k")), "{zh:?}");
    assert_eq!(en_only["m"].get("k").map(String::as_str), Some("Key"));
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_r3_backup_marker_for_another_file_is_not_used() {
    let mc = temp_game("r3bak");
    let rel = "kubejs/assets/foo/lang/en_us.json";
    mark_overwritten_with_backup(&mc, rel, r#"{"a":"你好"}"#, Some(r#"{"a":"Hello"}"#));
    let path = crate::engine::apply_guard::backup_marker_path(&mc, rel);
    let mut marker: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    marker["rel"] = serde_json::json!("some/other/file.json");
    fs::write(&path, marker.to_string()).unwrap();
    assert_eq!(ToolIndex::for_game(&mc).read_source(&mc.join(rel)), ReadSource::NeedsOriginal, "標記的 rel 對不上就不用");
    let _ = fs::remove_dir_all(&mc);
}
