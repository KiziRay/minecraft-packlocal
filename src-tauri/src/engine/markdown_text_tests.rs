use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::*;
use crate::engine::tool_products::test_support::{temp_game, write};

const PAGE: &str = "---\nnavigation:\n  title: Getting Started\n  parent: index.md\nitem_ids:\n  - ae2:controller\n---\n\n# The ME Network\n\nBuild a controller first.\n\n<ItemImage id=\"controller\" />\n\nSee [cables](cables.md) for more.\n\n```\ncode stays\n```\n- Place it down\n";

#[test]
fn b3_markdown_only_prose_and_title_are_candidates() {
    let got = segments(PAGE);
    assert_eq!(got, vec!["Getting Started", "The ME Network", "Build a controller first.", "Place it down"]);
    let map: HashMap<String, String> = [
        ("Getting Started", "入門"),
        ("The ME Network", "ME 網路"),
        ("Build a controller first.", "先蓋控制器。"),
        ("Place it down", "放下它"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    let out = apply(PAGE, &map).unwrap();
    assert!(out.contains("  title: 入門\n  parent: index.md\n"), "{out}");
    assert!(out.contains("# ME 網路\n") && out.contains("- 放下它\n"), "{out}");
    assert!(out.contains("<ItemImage id=\"controller\" />") && out.contains("[cables](cables.md)") && out.contains("code stays"));
    assert_eq!(out.lines().count(), PAGE.lines().count());
}

#[test]
fn b3_guide_paths_map_to_zh_tw_folder() {
    assert_eq!(
        guide_zh_tw_rel(Path::new("assets/ae2/ae2guide/items/controller.md")),
        Some((PathBuf::from("assets/ae2/ae2guide/_zh_tw/items/controller.md"), 2))
    );
    assert_eq!(
        guide_zh_tw_rel(Path::new("assets/ae2/ae2guide/_zh_cn/items/controller.md")),
        Some((PathBuf::from("assets/ae2/ae2guide/_zh_tw/items/controller.md"), 0))
    );
    assert_eq!(guide_zh_tw_rel(Path::new("assets/ae2/ae2guide/_zh_tw/a.md")), None);
    assert!(guide_zh_tw_rel(Path::new("assets/x/lavender/entries/b/e.md")).is_some());
    assert!(is_guide_md_entry("assets/ae2/ae2guide/index.md"));
    assert!(!is_guide_md_entry("assets/ae2/readme.md"));
}

#[test]
fn b3_ae2_guide_in_jar_goes_to_main_pack_with_human_pages_kept() {
    let mc = temp_game("guide");
    let work = temp_game("guide-work");
    fs::create_dir_all(mc.join("mods")).unwrap();
    {
        let mut zip = zip::ZipWriter::new(fs::File::create(mc.join("mods/ae2.jar")).unwrap());
        for (name, body) in [
            ("assets/ae2/ae2guide/index.md", "# Index\n"),
            ("assets/ae2/ae2guide/_zh_cn/index.md", "# 索引页面\n"),
            ("assets/ae2/ae2guide/_zh_cn/other.md", "# 其他页面\n"),
            ("assets/ae2/ae2guide/_zh_tw/other.md", "# 人工其他\n"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    crate::engine::jar_patchouli::translate_jar_patchouli(&mc, &work, false, None, |_, _| {}).unwrap();
    let base = work.join("pack-assets/assets/ae2/ae2guide/_zh_tw");
    let index = fs::read_to_string(base.join("index.md")).expect("簡中頁轉繁放進主資源包");
    assert!(index.contains("索引頁面"), "{index}");
    assert!(!base.join("other.md").exists(), "已有人工 _zh_tw 的頁不覆蓋");
    assert!(!work.join("jar-translated/ae2.jar").exists(), "手冊不改寫模組 JAR");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_kubejs_assets_books_are_scanned() {
    let mc = temp_game("kjsassets");
    let out = temp_game("kjsassets-out");
    write(&mc.join("kubejs/assets/pack/patchouli_books/g/zh_cn/entries/e.json"), r#"{"name":"简体书"}"#);
    crate::engine::text_overlay::translate_text_overlays(&mc, &out, false, None, |_, _| {}).unwrap();
    let zh = fs::read_to_string(out.join("pack-assets/assets/pack/patchouli_books/g/zh_tw/entries/e.json")).expect("kubejs/assets 書本要掃到");
    assert!(zh.contains("簡體書"), "{zh}");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn b3_f8_multiline_tags_inline_code_and_indented_code_are_kept() {
    let page = "Intro text here.\n<ItemImage\n  id=\"ae2:controller\"\n  scale=\"2\"\n/>\nUse `/ae2 give` to get it.\n    code block line\n\tanother code line\nOutro text here.\n";
    assert_eq!(segments(page), vec!["Intro text here.", "Outro text here."]);
}
