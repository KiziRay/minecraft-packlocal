use std::collections::HashMap;
use std::fs;

use super::*;
use crate::engine::tool_products::test_support::{temp_game, write};

const JS: &str = r#"StartupEvents.registry('item', e => {
  e.create('magic_gem').displayName('Magic Gem').tooltip('Shiny thing')
})
ItemEvents.tooltip(e => { e.add('x:y', Text.gold('Rare')) })
PlayerEvents.loggedIn(e => { e.player.tell(`Welcome back!`); e.player.tell(`Hi ${e.player.name}`) })
if (name == 'Magic Gem') { console.log('Magic Gem') }
const t = Text.of("Say \"hi\"")
"#;

#[test]
fn b3_kubejs_display_apis_found_by_position() {
    let lits = find_literals(JS, ScriptKind::KubeJs, false);
    let texts: Vec<&str> = lits.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, vec!["Magic Gem", "Shiny thing", "Rare", "Welcome back!", "Say \"hi\""]);
    assert!(lits.iter().find(|l| l.text == "Welcome back!").unwrap().private, "tell 的字不上傳");
    assert!(!lits.iter().find(|l| l.text == "Rare").unwrap().private);
    assert!(!texts.contains(&"x:y") && !texts.iter().any(|t| t.contains("${")), "id 與含插值的反引號不翻");
}

#[test]
fn b3_replacement_only_touches_the_display_call_not_other_same_strings() {
    let lits = find_literals(JS, ScriptKind::KubeJs, false);
    let map: HashMap<&str, &str> = [("Magic Gem", "魔法寶石"), ("Welcome back!", "歡迎回來！"), ("Say \"hi\"", "說「嗨」")].into();
    let out = replace_literals(JS, &lits, |l| map.get(l.text.as_str()).map(|s| s.to_string()));
    assert!(out.contains(".displayName('魔法寶石')"), "{out}");
    assert!(out.contains("if (name == 'Magic Gem') { console.log('Magic Gem') }"), "同字串的邏輯比對不能被換掉：{out}");
    assert!(out.contains("tell(`歡迎回來！`)") && out.contains("tell(`Hi ${e.player.name}`)"), "{out}");
    assert!(out.contains(r#"Text.of("說「嗨」")"#), "{out}");
}

#[test]
fn b3_crafttweaker_zs_display_apis() {
    let zs = "<item:minecraft:stick>.addTooltip(\"Pointy\");\n<item:x:y>.displayName = \"Fancy Stick\";\nif (\"Pointy\" == a) {}\n";
    let lits = find_literals(zs, ScriptKind::CraftTweaker, false);
    let texts: Vec<&str> = lits.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, vec!["Pointy", "Fancy Stick"]);
    let out = replace_literals(zs, &lits, |l| Some(format!("中{}", l.text.len())));
    assert!(out.contains("addTooltip(\"中6\")") && out.contains("if (\"Pointy\" == a)"), "{out}");
}

#[test]
fn b3_script_outputs_written_by_position_and_server_scripts_marked_private() {
    let mc = temp_game("kjs");
    let out = temp_game("kjs-out");
    write(&mc.join("kubejs/server_scripts/a.js"), "e.player.tell('Server hello')\nText.of('Shared words')\n");
    write(&mc.join("scripts/tip.zs"), "<item:minecraft:stick>.addTooltip(\"Pointy stick\");\n");
    let report = crate::engine::script_literals::translate_kubejs_literals(&mc, &out, false, None, |_, _| {}).unwrap();
    assert_eq!(report.files_scanned, 2, "{}", report.note);
    assert_eq!(report.private_sources.len(), 2, "伺服器腳本的字全部標記不上傳：{:?}", report.private_sources);
    assert!(crate::engine::share_policy::is_private("Server hello"));
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn b3_f6_text_nested_inside_tell_is_private() {
    let js = "e.player.tell(Text.of('Server secret').gold())\nText.of('Public words')\ne.player.setStatusMessage(Text.red('Status here'))\n";
    let lits = find_literals(js, ScriptKind::KubeJs, false);
    let private: Vec<&str> = lits.iter().filter(|l| l.private).map(|l| l.text.as_str()).collect();
    assert_eq!(private, vec!["Server secret", "Status here"], "{lits:?}");
}

#[test]
fn b3_f6_share_package_excludes_server_scripts_and_private_script_outputs() {
    let mc = temp_game("f6share");
    let work = temp_game("f6share-work");
    write(&mc.join("kubejs/client_scripts/a.js"), "e.player.tell(Text.of('Hi there'))\n");
    write(&mc.join("kubejs/client_scripts/b.js"), "Text.of('Hello world')\n");
    write(&work.join("kubejs/client_scripts/a.js"), "e.player.tell(Text.of('你好'))\n");
    write(&work.join("kubejs/client_scripts/b.js"), "Text.of('哈囉')\n");
    write(&work.join("kubejs/server_scripts/s.js"), "// 伺服器腳本全文\n");
    crate::engine::text_sources::record(&work, &work.join("kubejs/client_scripts/a.js"), &mc, &mc.join("kubejs/client_scripts/a.js"), &mc.join("kubejs/client_scripts/a.js"), "scripts");
    crate::engine::text_sources::mark_private_output(&work, &work.join("kubejs/client_scripts/a.js"));
    crate::engine::text_sources::record(&work, &work.join("kubejs/client_scripts/b.js"), &mc, &mc.join("kubejs/client_scripts/b.js"), &mc.join("kubejs/client_scripts/b.js"), "scripts");
    let zip = crate::engine::share_pack::package_translation(&work, &mc.join("out"), "t").unwrap();
    let archive = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    let names: Vec<String> = archive.file_names().map(|n| n.replace('\\', "/")).collect();
    assert!(names.iter().any(|n| n.ends_with("kubejs/client_scripts/b.js")), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("server_scripts")), "伺服器腳本不分享：{names:?}");
    assert!(!names.iter().any(|n| n.ends_with("client_scripts/a.js")), "含私有字串的腳本產物不分享：{names:?}");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_fe_share_package_only_carries_confirmed_outputs() {
    let mc = temp_game("feshare");
    let work = temp_game("feshare-work");
    write(&mc.join("kubejs/assets/x/lang/en_us.json"), r#"{"a":"Hi"}"#);
    write(&mc.join("kubejs/assets/x/lang/zh_cn.json"), r#"{"a":"简体"}"#);
    // 舊版產物：原地中文 en_us、舊的 client 腳本產物（不在產出清單）
    write(&work.join("kubejs/assets/x/lang/en_us.json"), r#"{"a":"嗨"}"#);
    write(&work.join("kubejs/client_scripts/old.js"), "Text.of('舊')
");
    crate::engine::text_overlay::translate_text_overlays(&mc, &work, false, None, |_, _| {}).unwrap();
    let zip = crate::engine::share_pack::package_translation(&work, &mc.join("out"), "t").unwrap();
    let archive = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    let names: Vec<String> = archive.file_names().map(|n| n.replace('\\', "/")).collect();
    assert!(names.iter().any(|n| n.ends_with("lang/zh_tw.json")), "{names:?}");
    assert!(!names.iter().any(|n| n.ends_with("lang/en_us.json")), "舊版產物不分享：{names:?}");
    assert!(!names.iter().any(|n| n.ends_with("old.js")), "{names:?}");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_private_call_spans_skip_parentheses_in_strings_and_comments() {
    let js = "e.player.tell(Text.of('a ) b') /* ) */ // )
 + Text.of('Inner secret'))
Text.of('Outside')
";
    let lits = find_literals(js, ScriptKind::KubeJs, false);
    let private: Vec<&str> = lits.iter().filter(|l| l.private).map(|l| l.text.as_str()).collect();
    assert_eq!(private, vec!["a ) b", "Inner secret"], "{lits:?}");
}
