use std::collections::HashMap;
use std::fs;

use super::*;
use crate::engine::ftbquests::{extract_display_strings, translate_ftbquests};
use crate::engine::tool_products::test_support::{mark_as_tool_written, temp_game, write};

const EN: &str = "{\n\tchapter.0A1B2C3D4E5F6071.title: \"入门\"\n\tquest.1111222233334444.title: \"Get Started\"\n\tquest.1111222233334444.quest_desc: [\n\t\t\"第一行说明\"\n\t\t\"\"\n\t\t\"{@pagebreak}\"\n\t]\n}\n";

#[test]
fn b3_new_format_lang_snbt_produces_zh_tw_and_never_rewrites_en_us() {
    let mc = temp_game("ftblang");
    let out = temp_game("ftblang-out");
    write(&mc.join("config/ftbquests/quests/lang/en_us.snbt"), EN);
    write(&mc.join("config/ftbquests/quests/chapters/a.snbt"), "{\n\tid: \"0A1B2C3D4E5F6071\"\n}\n");
    translate_ftbquests(&mc, &out, false, None, |_, _| {}).unwrap();
    let zh = fs::read_to_string(out.join("config/ftbquests/quests/lang/zh_tw.snbt")).expect("要產出 zh_tw.snbt");
    assert!(zh.contains("chapter.0A1B2C3D4E5F6071.title: \"入門\""), "{zh}");
    assert!(zh.contains("\"第一行說明\""), "{zh}");
    assert!(zh.contains("\"Get Started\""), "沒翻到的保留英文：{zh}");
    assert!(zh.contains("\"{@pagebreak}\""), "分頁記號不能動：{zh}");
    assert_eq!(zh.lines().count(), EN.lines().count(), "陣列行數與結構照英文檔");
    assert!(!out.join("config/ftbquests/quests/lang/en_us.snbt").exists(), "英文檔不得改寫");
    assert_eq!(fs::read_to_string(mc.join("config/ftbquests/quests/lang/en_us.snbt")).unwrap(), EN);
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn b3_human_zh_tw_is_kept_and_only_en_us_keys_are_written() {
    let mc = temp_game("ftbhuman");
    let root = mc.join("config/ftbquests");
    write(&root.join("quests/lang/en_us.snbt"), "{\n\tquest.A.title: \"Hello\"\n\tquest.B.title: \"Bye\"\n}\n");
    write(&root.join("quests/lang/zh_tw.snbt"), "{\n\tquest.A.title: \"你好人工\"\n\tquest.C.title: \"多餘\"\n}\n");
    let jobs = collect_jobs(&mc, &root);
    assert_eq!(jobs.len(), 1);
    assert_eq!(candidates(&jobs), vec!["Bye".to_string()], "人工已翻的不送 AI");
    let map = HashMap::from([("Bye".to_string(), "再見".to_string())]);
    let dest = mc.join("out");
    assert_eq!(write_outputs(&jobs, &map, &dest, &mc, &mc.join("out")).unwrap(), 1);
    let zh = fs::read_to_string(dest.join("quests/lang/zh_tw.snbt")).unwrap();
    assert!(zh.contains("\"你好人工\"") && zh.contains("\"再見\""), "{zh}");
    assert!(!zh.contains("多餘"), "英文檔沒有的 key 不寫：{zh}");
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_tool_written_zh_tw_is_not_treated_as_human_translation() {
    let mc = temp_game("ftbtool");
    let root = mc.join("config/ftbquests");
    write(&root.join("quests/lang/en_us.snbt"), "{\n\tquest.A.title: \"Hello\"\n}\n");
    write(&root.join("quests/lang/zh_tw.snbt"), "{\n\tquest.A.title: \"哈囉機翻\"\n}\n");
    mark_as_tool_written(&mc, "config/ftbquests/quests/lang/zh_tw.snbt");
    let jobs = collect_jobs(&mc, &root);
    assert_eq!(candidates(&jobs), vec!["Hello".to_string()], "工具以前的機翻要重新修正，不能跳過");
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_split_lang_folder_maps_to_zh_tw_folder() {
    let mc = temp_game("ftbsplit");
    let root = mc.join("config/ftbquests");
    write(&root.join("quests/lang/en_us/chapters/intro.snbt"), "{\n\tchapter.X.title: \"Intro\"\n}\n");
    let jobs = collect_jobs(&mc, &root);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].zh_rel, PathBuf::from("quests/lang/zh_tw/chapters/intro.snbt"));
    assert!(is_lang_file(Path::new("quests/lang/en_us/chapters/intro.snbt")));
    assert!(!is_lang_file(Path::new("quests/chapters/intro.snbt")));
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_parse_spans_handles_quoted_keys_single_quotes_and_escapes() {
    let text = "{\n\t\"quest.A.title\": 'It\\'s \"ok\"'\n\tquest.B.quest_desc: [\"a\", \"b\\nc\"]\n\tflag: 1b\n}";
    let spans = parse_spans(text).unwrap();
    assert_eq!(spans.len(), 3);
    assert_eq!(spans[0].key, "quest.A.title");
    assert_eq!(spans[0].text, "It's \"ok\"");
    assert_eq!((spans[2].key.as_str(), spans[2].index, spans[2].text.as_str()), ("quest.B.quest_desc", 1, "b\nc"));
    let out = render(text, &spans, |s| (s.index == 1).then(|| "乙\n丙".to_string()));
    assert!(out.contains("[\"a\", \"乙\\n丙\"]"), "{out}");
    assert!(parse_spans("{ a: { b: \"x\" } }").is_none(), "看不懂的結構整檔不處理");
}

#[test]
fn b3_item_name_hover_lock_message_are_display_fields() {
    let text = "{\n\titem_name: \"Magic Sword\"\n\tlock_message: \"Finish chapter one first\"\n\thover: [\"Hover line\"]\n}\n";
    let got = extract_display_strings(text);
    for want in ["Magic Sword", "Finish chapter one first", "Hover line"] {
        assert!(got.iter().any(|s| s == want), "{want} 要能翻：{got:?}");
    }
}

#[test]
fn b3_single_lowercase_word_under_display_key_is_translated_but_not_under_type() {
    let got = extract_display_strings("{\n\ttitle: \"done\"\n\ttype: \"checkmark\"\n\tsubtitle: \"has_iron\"\n}\n");
    assert!(got.iter().any(|s| s == "done"), "顯示欄的一般小寫字要翻：{got:?}");
    assert!(!got.iter().any(|s| s == "checkmark"), "結構欄不翻");
    assert!(!got.iter().any(|s| s == "has_iron"), "機制 id 不翻");
}

#[test]
fn b3_f7_human_value_is_checked_and_bad_one_falls_back() {
    let mc = temp_game("ftbf7");
    let root = mc.join("config/ftbquests");
    write(&root.join("quests/lang/en_us.snbt"), "{\n\tquest.A.title: \"§aHello\"\n}\n");
    // 人工值把色碼弄丟了：只免長度、其餘照查 → 退回英文
    write(&root.join("quests/lang/zh_tw.snbt"), "{\n\tquest.A.title: \"你好\"\n}\n");
    let jobs = collect_jobs(&mc, &root);
    let dest = mc.join("out");
    let _ = write_outputs(&jobs, &HashMap::new(), &dest, &mc, &dest);
    let zh = fs::read_to_string(dest.join("quests/lang/zh_tw.snbt")).unwrap_or_default();
    assert!(!zh.contains("你好"), "{zh}");
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_f10_single_file_and_split_folder_only_writes_the_folder_version() {
    let mc = temp_game("ftbf10");
    let root = mc.join("config/ftbquests");
    write(&root.join("quests/lang/en_us.snbt"), "{\n\tquest.A.title: \"Old single\"\n}\n");
    write(&root.join("quests/lang/en_us/chapters/c.snbt"), "{\n\tquest.A.title: \"Split\"\n}\n");
    let jobs = collect_jobs(&mc, &root);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].zh_rel, PathBuf::from("quests/lang/zh_tw/chapters/c.snbt"));
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_f11_strings_with_unknown_escapes_are_left_untouched() {
    let text = "{\n\tquest.A.title: \"Line\\u00A7 odd\"\n\tquest.B.title: \"Plain\"\n}\n";
    let spans = parse_spans(text).unwrap();
    assert!(spans[0].opaque && !spans[1].opaque);
    let out = render(text, &spans, |s| Some(format!("中{}", s.index)));
    assert!(out.contains("\"Line\\u00A7 odd\""), "看不懂的跳脫整段不動：{out}");
}
