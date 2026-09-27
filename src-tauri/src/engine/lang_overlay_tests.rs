use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::engine::tool_products::test_support::{mark_as_tool_written, temp_game, write};

fn write_zip(path: &Path, files: &[(&str, &str)]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, body) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn read_json(path: &Path) -> HashMap<String, String> {
    serde_json::from_str(&fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

#[test]
fn b3_loose_lang_gets_zh_tw_sibling_and_en_us_is_never_rewritten() {
    let mc = temp_game("langov");
    let out = temp_game("langov-out");
    let lang = mc.join("kubejs/assets/foo/lang");
    write(&lang.join("en_us.json"), r#"{"a":"Hello","b":"Tool"}"#);
    write(&lang.join("zh_cn.json"), r#"{"b":"工具软件"}"#);
    write(&lang.join("zh_tw.json"), r#"{"a":"你好人工","extra":"多的"}"#);
    write(&lang.join("ja_jp.json"), r#"{"a":"こんにちは"}"#);
    crate::engine::text_overlay::translate_text_overlays(&mc, &out, false, None, |_, _| {}).unwrap();
    let out_lang = out.join("kubejs/assets/foo/lang");
    assert!(!out_lang.join("en_us.json").exists(), "英文語言檔不得原地改寫");
    assert!(!out_lang.join("zh_cn.json").exists() && !out_lang.join("ja_jp.json").exists(), "其他語系不改寫");
    let zh = read_json(&out_lang.join("zh_tw.json"));
    assert_eq!(zh.get("a").map(String::as_str), Some("你好人工"), "人工 zh_tw 保留");
    assert_eq!(zh.get("b").map(String::as_str), Some("工具軟體"), "簡中轉繁補缺");
    assert!(!zh.contains_key("extra"), "只寫英文檔有的 key");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn b3_tool_written_zh_tw_lang_is_not_human() {
    let mc = temp_game("langtool");
    let lang = mc.join("kubejs/assets/foo/lang");
    write(&lang.join("en_us.json"), r#"{"a":"Hello"}"#);
    write(&lang.join("zh_tw.json"), r#"{"a":"舊機翻"}"#);
    mark_as_tool_written(&mc, "kubejs/assets/foo/lang/zh_tw.json");
    let jobs = super::collect_jobs(&mc, &[lang.join("en_us.json")]);
    assert_eq!(super::candidates(&jobs), vec!["Hello".to_string()], "工具以前的機翻要重翻");
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_scan_ignores_tool_pack_and_user_pack_outranks_mod() {
    let mc = temp_game("hier");
    write_zip(
        &mc.join("mods/m.jar"),
        &[
            ("assets/m/lang/en_us.json", r#"{"k":"Key","r":"Original long mod text","z":"Zed"}"#),
            ("assets/m/lang/zh_tw.json", r#"{"z":"模組自帶較長的翻譯"}"#),
        ],
    );
    write_zip(&mc.join("resourcepacks/模組包翻譯工具-x.zip"), &[("assets/m/lang/zh_tw.json", r#"{"k":"機翻"}"#)]);
    write(&mc.join("resourcepacks/UserPack/assets/m/lang/en_us.json"), r#"{"r":"Pack text"}"#);
    write(&mc.join("resourcepacks/UserPack/assets/m/lang/zh_tw.json"), r#"{"z":"玩家包"}"#);
    // 工具寫進資料夾型資源包的 zh_tw（有標記）也不算
    write(&mc.join("resourcepacks/UserPack/assets/m/lang/zh_cn.json"), r#"{"k":"机翻"}"#);
    mark_as_tool_written(&mc, "resourcepacks/UserPack/assets/m/lang/zh_cn.json");
    let (zh, en_only, _, _) =
        crate::engine::jar_scan::scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    assert!(zh.get("m").is_none_or(|m| !m.contains_key("k")), "工具資源包的機翻不當成已有中文：{zh:?}");
    assert_eq!(en_only["m"].get("k").map(String::as_str), Some("Key"));
    assert_eq!(en_only["m"].get("r").map(String::as_str), Some("Pack text"), "資源包的英文蓋過模組（來源階層）");
    assert_eq!(zh["m"].get("z").map(String::as_str), Some("玩家包"), "玩家資源包的中文蓋過模組");
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_datapack_zip_text_is_really_rewritten_and_new_zh_tw_entries_added() {
    let mc = temp_game("zipov");
    let work = temp_game("zipov-work");
    write_zip(
        &mc.join("datapacks/pack.zip"),
        &[
            ("pack.mcmeta", r#"{"pack":{"pack_format":15,"description":"x"}}"#),
            ("data/ns/advancements/a.json", r#"{"display":{"title":"简体标题","description":"说明"}}"#),
            ("data/ns/patchouli_books/b/zh_cn/entries/e.json", r#"{"name":"简体书页","pages":[]}"#),
        ],
    );
    crate::engine::archive_overlay::translate_archive_overlays(&mc, &work, false, None, |_, _| {}).unwrap();
    let out = work.join("datapacks/pack.zip");
    let mut zip = zip::ZipArchive::new(fs::File::open(&out).expect("要重建 zip")).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("data/ns/advancements/a.json").unwrap(), &mut text).unwrap();
    assert!(text.contains("簡體標題"), "{text}");
    let mut book = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("data/ns/patchouli_books/b/zh_tw/entries/e.json").expect("新增 zh_tw 書頁"), &mut book).unwrap();
    assert!(book.contains("簡體書頁"), "{book}");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}
