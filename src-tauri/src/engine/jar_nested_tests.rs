use std::collections::HashMap;
use std::io::Write;

use crate::engine::tool_products::test_support::temp_game;

fn zip_bytes(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        for (name, body) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

#[test]
fn b3_nested_jar_lang_is_scanned_one_level_only() {
    let mc = temp_game("nested");
    std::fs::create_dir_all(mc.join("mods")).unwrap();
    let deepest = zip_bytes(&[("assets/deep/lang/en_us.json", br#"{"deep.key":"Too deep"}"#.to_vec())]);
    let inner = zip_bytes(&[
        ("assets/ponder/lang/en_us.json", br#"{"ponder.hold":"Hold [W] to Ponder","ponder.done":"Done"}"#.to_vec()),
        ("assets/ponder/lang/zh_tw.json", r#"{"ponder.done":"完成"}"#.as_bytes().to_vec()),
        ("assets/ponder/textures/x.png", vec![0, 1, 2]),
        ("META-INF/jars/deepest.jar", deepest),
    ]);
    let outer = zip_bytes(&[
        ("assets/create/lang/en_us.json", br#"{"create.key":"Create"}"#.to_vec()),
        ("META-INF/jars/ponder-1.0.jar", inner),
    ]);
    std::fs::write(mc.join("mods/create.jar"), outer).unwrap();

    let (zh, en_only, _, _) =
        crate::engine::jar_scan::scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    assert_eq!(
        en_only.get("ponder").and_then(|m| m.get("ponder.hold")).map(String::as_str),
        Some("Hold [W] to Ponder"),
        "子 JAR 的英文要進待翻清單"
    );
    assert_eq!(
        zh.get("ponder").and_then(|m| m.get("ponder.done")).map(String::as_str),
        Some("完成"),
        "子 JAR 自帶的 zh_tw 屬模組自帶"
    );
    assert!(en_only.contains_key("create"));
    assert!(!en_only.contains_key("deep") && !zh.contains_key("deep"), "只遞迴一層");
    // 英文原文全表（補翻／修復的完整檢查依據）包含子 JAR 的英文；子 JAR 的 zh_tw 算模組自帶
    let full = crate::engine::output_guard::snapshot_sources();
    assert_eq!(full["ponder"]["ponder.done"], "Done");
    let native = crate::engine::output_guard::snapshot_native();
    assert_eq!(native["ponder"]["ponder.done"], "完成");
    let _ = std::fs::remove_dir_all(&mc);
}
