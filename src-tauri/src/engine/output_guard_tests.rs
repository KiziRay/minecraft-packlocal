//! B2#4：output guard 逐條與檔案層檢查的測試。

use super::*;
use std::collections::HashMap;
use crate::engine::output_guard_file::font_may_not_support_chinese;

fn ok(src: &str, zh: &str) -> String {
    check_entry(src, zh).unwrap_or_else(|r| panic!("「{src}」→「{zh}」應該通過，實際退回：{r:?}"))
}

fn bad(src: &str, zh: &str) -> RejectReason {
    check_entry(src, zh).expect_err(&format!("「{src}」→「{zh}」應該退回"))
}

#[test]
fn b2_normal_translations_pass() {
    ok("Yes", "是");
    ok("On", "開啟");
    ok("Cancel", "取消");
    ok("Iron Sword", "鐵劍");
    ok("%1$s was slain by %2$s", "%1$s 被 %2$s 擊殺");
    ok("Deals %s damage", "造成 %s 點傷害");
    ok("§9Mana: §f%s§7/§f%s", "§9魔力：§f%s§7/§f%s");
    ok("Welcome to the Lexica Botania, a guide to the mystical arts.", "歡迎閱讀植物魔法辭典，一本神秘藝術指南。");
    ok("Level: ", "等級： ");
}

#[test]
fn b2_format_code_mismatch_is_rejected() {
    assert_eq!(bad("Deals %s damage", "造成傷害"), RejectReason::FormatCodes);
    assert_eq!(bad("Reward: {0}", "獎勵"), RejectReason::FormatCodes);
    assert_eq!(bad("§aGreen§r text §cRed", "§a綠色文字§c紅色"), RejectReason::ColourCodes);
}

#[test]
fn b2_short_field_length_limit() {
    // 原文寬 3 → 上限 max(ceil(4.5), 4) = 5 欄：兩個中文字（4 欄）可以，三個（6 欄）不行
    ok("Axe", "斧頭");
    assert_eq!(bad("Axe", "伐木用斧頭"), RejectReason::TooLong);
    // 原文寬 2 → 上限 4
    ok("On", "開啟");
    assert_eq!(bad("On", "已經開啟"), RejectReason::TooLong);
    // 長句不受短欄位限制
    ok(
        "The forge burns hottest just before dawn and cools at noon.",
        "熔爐在黎明前燒得最旺，到了中午才會慢慢冷卻下來，這是鍛造師們代代相傳的經驗之談。",
    );
    assert_eq!(display_width("§a鐵劍"), 4);
    assert_eq!(display_width("Iron"), 4);
}

#[test]
fn b2_new_line_page_break_icons_edges() {
    // 單行原文的換行會在 placeholder 收斂；字面 \n 多出來則退回
    assert_eq!(bad("Open the door", "打開\\n門"), RejectReason::NewLine);
    assert_eq!(bad("Page one{@pagebreak}Page two", "第一頁第二頁"), RejectReason::PageBreak);
    assert_eq!(bad("\u{E001} Mana", "魔力"), RejectReason::IconGlyphs);
    ok("\u{E001} Mana", "\u{E001} 魔力");
    // 不自動改全形：全形標點照原樣留著
    assert_eq!(ok("Hello, world!", "你好，世界！"), "你好，世界！");
    // 前後空白由 placeholder 還原
    assert_eq!(ok(" Level ", "等級"), " 等級 ");
}

#[test]
fn b2_garbled_and_empty_are_rejected() {
    assert_eq!(bad("Stone", "   "), RejectReason::Empty);
    assert_eq!(bad("Stone", "石\u{FFFD}頭"), RejectReason::BadCharacters);
    assert_eq!(bad("Stone", "\u{FEFF}石頭"), RejectReason::BadCharacters);
}

#[test]
fn b2_only_the_bad_entry_falls_back_to_english() {
    let file = "b2-test-guard-map";
    let mut map: HashMap<String, String> = HashMap::new();
    map.insert("Deals %s damage".into(), "造成傷害".into());
    map.insert("Iron Sword".into(), "鐵劍".into());
    map.insert("Cancel".into(), "取消".into());
    guard_map(file, &mut map);
    assert_eq!(map.len(), 2, "只拿掉壞的那一條：{map:?}");
    assert_eq!(map.get("Iron Sword").map(String::as_str), Some("鐵劍"));
    let rejected = peek_rejected_for(file);
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].code, "format_codes");
    assert_eq!(rejected[0].source, "Deals %s damage");
}

#[test]
fn b2_lang_entries_use_known_english() {
    let file = "b2-test-lang";
    let mut en: LangMap = HashMap::new();
    en.entry("demo".into()).or_default().insert("a".into(), "Deals %s damage".into());
    en.entry("demo".into()).or_default().insert("b".into(), "Stone".into());
    let mut zh: LangMap = HashMap::new();
    zh.entry("demo".into()).or_default().insert("a".into(), "造成傷害".into());
    zh.entry("demo".into()).or_default().insert("b".into(), "石頭".into());
    guard_langmap(file, &mut zh, Some(&en));
    assert_eq!(zh["demo"].len(), 1);
    assert_eq!(zh["demo"]["b"], "石頭");
    // 沒有原文時只擋空譯文與亂碼
    assert_eq!(lang_entry(file, "none", "x", "任何譯文", None).as_deref(), Some("任何譯文"));
    assert!(lang_entry(file, "none", "y", "壞\u{FFFD}", None).is_none());
}

#[test]
fn b2_array_line_count_must_match() {
    // 多行文字（手冊段落、lore）被 AI 併成一行＝行數不同，退回
    assert_eq!(bad("Line one\nLine two", "第一行第二行"), RejectReason::LineCount);
    ok("Line one\nLine two", "第一行\n第二行");
}

#[test]
fn b2_file_layer_encoding_and_reparse() {
    // BOM 去掉
    let out = finish_file("a.json", b"{\"a\":\"b\"}", "\u{FEFF}{\"a\":\"乙\"}".as_bytes().to_vec());
    assert_eq!(String::from_utf8(out).unwrap(), "{\"a\":\"乙\"}");
    // 寫完讀不回來 → 整檔保留原文
    let orig = b"{\"a\":\"b\"}";
    assert_eq!(finish_file("b2-test-broken.json", orig, b"{\"a\":\"\xe4\xb9\x99\"".to_vec()), orig.to_vec());
    // properties 用 \u 跳脫
    let out = finish_file("x.properties", b"title=Hello\n", "title=你好\n".as_bytes().to_vec());
    assert_eq!(String::from_utf8(out).unwrap(), "title=\\u4F60\\u597D\n");
    // SNBT 引號沒關 → 保留原文
    let snbt = b"{ title: \"Hello\" }";
    assert_eq!(finish_file("q.snbt", snbt, "{ title: \"你好 }".as_bytes().to_vec()), snbt.to_vec());
    // 非 UTF-8 → 保留原文
    assert_eq!(finish_file("t.txt", b"hi", vec![0xff, 0xfe]), b"hi".to_vec());
}

#[test]
fn b2_zip_entry_names() {
    assert!(zip_entry_ok("assets/demo/lang/zh_tw.json"));
    assert!(zip_entry_ok("assets/demo/書本/頁.json"));
    assert!(!zip_entry_ok("../evil.json"));
    assert!(!zip_entry_ok("assets\\demo.json"));
    assert!(!zip_entry_ok("/abs.json"));
}

#[test]
fn b2_font_scan_reuses_apply_logic_and_skips_our_font_pack() {
    let root = std::env::temp_dir().join(format!("b2_font_scan_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mc = root.join("minecraft");
    for name in ["PixelFont", "繁體中文遊戲字體"] {
        let dir = mc.join("resourcepacks").join(name).join("assets/minecraft/font");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("default.json"), "{}").unwrap();
    }
    std::fs::write(
        mc.join("options.txt"),
        "resourcePacks:[\"vanilla\",\"file/PixelFont\",\"file/繁體中文遊戲字體\"]\n",
    )
    .unwrap();
    assert_eq!(font_may_not_support_chinese(&mc), vec!["PixelFont".to_string()]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn b2_text_component_split_keeps_structure() {
    let parts = split_text_component(r#"{"text":"Hi","extra":[{"text":"there","bold":true}]}"#).unwrap();
    let texts: Vec<_> = parts
        .iter()
        .filter_map(|p| match p {
            ComponentPart::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["Hi", "there"]);
    assert!(split_text_component("Plain text").is_none());
    assert!(split_text_component(r#"{"color":"red"}"#).is_none());
}

#[test]
fn b2_f2_characters_that_render_as_boxes_are_rejected() {
    for zh in [
        "石\u{200B}頭",   // 零寬空白
        "石\u{200D}頭",   // 零寬連接
        "石\u{2060}頭",   // word joiner
        "石\u{FE0F}頭",   // 異體選擇符
        "石\u{0085}頭",   // C1 控制
        "石\u{007F}頭",   // DEL
        "石\u{2028}頭",   // 行分隔
        "石頭\u{1F600}",  // emoji
        "石頭\u{2764}",   // 雜項符號
        "\u{20BB7}石",    // 擴展 B（遊戲字型多半沒有）
    ] {
        assert_eq!(bad("Stone", zh), RejectReason::BadCharacters, "{zh:?}");
    }
    // 原文本來就有的不擋
    ok("Love \u{2764}", "愛 \u{2764}");
    // 一般全形標點與常用中文照常
    ok("Hello, world! (test)", "你好，世界！（測試）");
    ok("Note: \"quoted\"", "注意：「引號」");
}

#[test]
fn b2_f3_carriage_return_is_rejected_unless_in_source() {
    assert_eq!(bad("Line one\nLine two", "第一行\r\n第二行"), RejectReason::BadCharacters);
    ok("Line one\r\nLine two", "第一行\r\n第二行");
}

#[test]
fn b2_f6_each_run_starts_with_an_empty_rejection_list() {
    record_rejection("b2-f6-old", "k", "Stone", "壞", RejectReason::BadCharacters);
    begin_run();
    let report = take_run_report();
    assert!(report.rejected.iter().all(|r| r.file != "b2-f6-old"), "上一輪的退回不得混進這一輪");

    // 去重鍵含檔案：同一條在兩個檔案被退回要列兩次（玩家要知道兩處都保留英文）
    begin_run();
    record_rejection("b2-f6-a.json", "k", "Stone", "壞", RejectReason::BadCharacters);
    record_rejection("b2-f6-b.json", "k", "Stone", "壞", RejectReason::BadCharacters);
    record_rejection("b2-f6-b.json", "k", "Stone", "壞", RejectReason::BadCharacters);
    let report = take_run_report();
    assert_eq!(report.rejected.len(), 2);
    assert_eq!(report.rejected_count, 2);

    let lib = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs")).unwrap();
    for name in ["fn run_one_click(", "fn run_supplement(", "fn run_repair(", "async fn import_translations_cmd("] {
        let body = &lib[lib.find(name).unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        let head: String = body.lines().take(25).collect::<Vec<_>>().join("\n");
        assert!(head.contains("begin_guard_run()"), "{name} 開頭要清空退回紀錄");
    }
}

#[test]
fn b2_f7_scanning_another_pack_clears_the_english_table() {
    let mut first: LangMap = HashMap::new();
    first.entry("b2f7".into()).or_default().insert("gui.b2f7.on".into(), "On".into());
    remember_sources(&first);
    assert_eq!(source_of("b2f7", "gui.b2f7.on").as_deref(), Some("On"));

    // 同一個執行期換翻第二個整合包：掃描開始前清空，第二包不會拿到第一包的英文
    let root = std::env::temp_dir().join(format!("b2_f7_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("mods")).unwrap();
    let _ = crate::engine::jar_scan::scan_instance(&root, &HashMap::new(), false, false, |_, _| {});
    assert!(source_of("b2f7", "gui.b2f7.on").is_none(), "第一包的英文原文要清掉");
    let _ = std::fs::remove_dir_all(root);
}
