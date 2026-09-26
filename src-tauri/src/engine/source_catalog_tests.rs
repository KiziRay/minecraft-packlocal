//! B2 第二輪 F5：英文原文全表（掃描收集、存檔、建包前載入、舊資料補齊）。

use super::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::engine::output_guard::{peek_rejected_for, peek_unverified_for, snapshot_sources};
use crate::engine::pack_out::{build_resource_pack, BuildOptions};

fn temp(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("b2_cat_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// 假的遊戲資料夾：資源包裡有 en_us 與模組自帶的 zh_tw（有中文的鍵不會進待補清單）。
fn fake_instance(root: &Path) -> PathBuf {
    let mc = root.join("game");
    fs::create_dir_all(mc.join("mods")).unwrap();
    let lang = mc.join("resourcepacks/rp/assets/b2cat/lang");
    fs::create_dir_all(&lang).unwrap();
    fs::write(mc.join("resourcepacks/rp/pack.mcmeta"), r#"{"pack":{"pack_format":15,"description":"t"}}"#).unwrap();
    fs::write(
        lang.join("en_us.json"),
        r#"{"gui.b2cat.on":"On","gui.b2cat.color":"§aGreen§r text","gui.b2cat.ok":"Cancel"}"#,
    )
    .unwrap();
    fs::write(lang.join("zh_tw.json"), r#"{"gui.b2cat.on":"開"}"#).unwrap();
    mc
}

fn opts(root: &Path, name: &str) -> BuildOptions {
    BuildOptions {
        output_dir: root.display().to_string(),
        pack_folder_name: name.into(),
        pack_description: "t".into(),
        pack_format: 15,
        target_version: Some("1.20.1".into()),
    }
}

/// 從舊資源包讀回、要重新寫出的譯文（鍵只在 zh，不在待補清單）。
fn old_pack_zh() -> LangMap {
    let mut zh: LangMap = HashMap::new();
    let z = zh.entry("b2cat".into()).or_default();
    z.insert("gui.b2cat.on".into(), "目前處於開啟狀態".into()); // 超長
    z.insert("gui.b2cat.color".into(), "§a綠色文字§a".into()); // 色碼壞
    z.insert("gui.b2cat.ok".into(), "取消".into()); // 正常
    zh
}

fn written(built_dir: &str) -> String {
    fs::read_to_string(PathBuf::from(built_dir).join("assets/b2cat/lang/zh_tw.json")).unwrap_or_default()
}

#[test]
fn b2_f5b_scan_collects_the_full_english_table() {
    let root = temp("scan");
    let mc = fake_instance(&root);
    let (_zh, en_only, _, _) = crate::engine::jar_scan::scan_instance(&mc, &HashMap::new(), false, false, |_, _| {}).unwrap();
    // 已有中文的鍵不在待補清單，但全表裡要有它的英文
    assert!(en_only.get("b2cat").map_or(true, |m| !m.contains_key("gui.b2cat.on")));
    let full = snapshot_sources();
    assert_eq!(full["b2cat"]["gui.b2cat.on"], "On");
    assert_eq!(full["b2cat"]["gui.b2cat.ok"], "Cancel");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b2_f5b_repair_rebuild_checks_old_translations_against_saved_table() {
    let root = temp("saved");
    let work = root.join("work");
    let mut en: LangMap = HashMap::new();
    let e = en.entry("b2cat".into()).or_default();
    e.insert("gui.b2cat.on".into(), "On".into());
    e.insert("gui.b2cat.color".into(), "§aGreen§r text".into());
    e.insert("gui.b2cat.ok".into(), "Cancel".into());
    save(&work, &en).unwrap();
    assert!(load(&work).is_some());

    // 重開程式後修復：原文表是空的；遊戲資料夾也不必在
    crate::engine::output_guard::reset_sources();
    assert_eq!(prepare_build_sources(&work, &root.join("no-such-game")), CatalogSource::Saved);
    let built = build_resource_pack(&old_pack_zh(), &opts(&work, "saved")).unwrap();
    let text = written(&built.pack_dir);
    assert!(!text.contains("目前處於開啟狀態"), "超長舊譯文要退回：{text}");
    assert!(!text.contains("§a綠色文字§a"), "色碼壞的舊譯文要退回：{text}");
    assert!(text.contains("取消"));
    let codes: Vec<&str> = peek_rejected_for("翻譯資源包")
        .iter()
        .filter(|r| r.key.starts_with("gui.b2cat."))
        .map(|r| r.code)
        .collect();
    assert!(codes.contains(&"too_long") && codes.contains(&"colour_codes"), "{codes:?}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b2_f5b_old_result_without_table_rescans_first() {
    let root = temp("rescan");
    let mc = fake_instance(&root);
    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    crate::engine::output_guard::reset_sources();
    assert_eq!(prepare_build_sources(&work, &mc), CatalogSource::Rescanned);
    assert!(work.join(CATALOG_FILE).is_file(), "重掃後要存下全表，下次不必再掃");
    let built = build_resource_pack(&old_pack_zh(), &opts(&work, "rescan")).unwrap();
    let text = written(&built.pack_dir);
    assert!(!text.contains("目前處於開啟狀態") && text.contains("取消"), "{text}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b2_f5b_no_table_and_no_game_folder_marks_entries_unverified() {
    let root = temp("missing");
    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    crate::engine::output_guard::reset_sources();
    assert_eq!(prepare_build_sources(&work, &root.join("no-such-game")), CatalogSource::Missing);
    let mut zh = old_pack_zh();
    zh.get_mut("b2cat").unwrap().insert("gui.b2cat.box".into(), "石頭\u{1F600}".into());
    let built = build_resource_pack(&zh, &opts(&work, "missing")).unwrap();
    let text = written(&built.pack_dir);
    // 不需原文的檢查照做：會變方框的字元擋下；結尾停在色碼上擋下
    assert!(!text.contains('\u{1F600}'), "{text}");
    assert!(!text.contains("§a綠色文字§a"), "{text}");
    assert!(text.contains("取消"));
    // 寫出的每一條都記「缺原文未完整檢查」
    let unverified = peek_unverified_for("翻譯資源包");
    assert!(unverified.iter().any(|r| r.key == "gui.b2cat.ok" && r.code == "missing_source"), "{unverified:?}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b2_f5b_every_rebuild_path_loads_the_table_first() {
    let lib = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs")).unwrap();
    for name in ["fn run_supplement(", "fn run_repair(", "async fn import_translations_cmd("] {
        let body = &lib[lib.find(name).unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        let prep = body.find("prepare_build_sources(").unwrap_or_else(|| panic!("{name} 建包前沒有載入英文原文表"));
        let build = body.find("build_resource_pack(").unwrap();
        assert!(prep < build, "{name} 要先載入原文表才建包");
        assert!(!body.contains("remember_sources(&pending)"), "{name} 不得再用會縮減的待補清單當原文");
    }
    let one = &lib[lib.find("fn run_one_click(").unwrap()..];
    let one = &one[..one.find("\n}\n").unwrap()];
    assert!(one.contains("source_catalog_save("), "整輪翻譯要把英文全表存進翻譯結果");
}

#[test]
fn b2_f5b_import_rebuild_checks_old_translations_too() {
    // 貼回翻譯：import_translations_cmd 的順序＝讀舊資源包 → 載入原文全表 → merge_imported → 建包
    let root = temp("import");
    let work = root.join("work");
    let mut en: LangMap = HashMap::new();
    let e = en.entry("b2cat".into()).or_default();
    e.insert("gui.b2cat.on".into(), "On".into());
    e.insert("gui.b2cat.color".into(), "§aGreen§r text".into());
    e.insert("gui.b2cat.ok".into(), "Cancel".into());
    e.insert("gui.b2cat.new".into(), "Done".into());
    save(&work, &en).unwrap();
    crate::engine::output_guard::reset_sources();

    let mut zh = old_pack_zh();
    assert_eq!(prepare_build_sources(&work, &root.join("no-such-game")), CatalogSource::Saved);
    let mut pending: LangMap = HashMap::new();
    pending.entry("b2cat".into()).or_default().insert("gui.b2cat.new".into(), "Done".into());
    let entries = vec![(Some("b2cat".to_string()), "gui.b2cat.new".to_string(), "完成".to_string())];
    let report = crate::engine::failed_items::merge_imported(&mut zh, &pending, &entries);
    assert_eq!(report.accepted, 1);
    let built = build_resource_pack(&zh, &opts(&work, "import")).unwrap();
    let text = written(&built.pack_dir);
    assert!(text.contains("完成") && text.contains("取消"), "{text}");
    assert!(!text.contains("目前處於開啟狀態") && !text.contains("§a綠色文字§a"), "舊譯文也要檢查：{text}");
    let _ = fs::remove_dir_all(root);
}

/// 模組 jar：en_us＋模組作者自己的 zh_tw（b 是作者自己比較長的翻譯）。
fn write_mod_jar(mods: &Path) {
    use std::io::Write;
    let file = fs::File::create(mods.join("b2fa-mod.jar")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("assets/b2fa/lang/en_us.json", opts).unwrap();
    zip.write_all(br#"{"gui.b2fa.a":"On","gui.b2fa.b":"Off"}"#).unwrap();
    zip.start_file("assets/b2fa/lang/zh_tw.json", opts).unwrap();
    zip.write_all(r#"{"gui.b2fa.b":"目前處於關閉狀態"}"#.as_bytes()).unwrap();
    zip.finish().unwrap();
}

#[test]
fn b2_fa_tool_pack_in_resourcepacks_is_not_treated_as_mod_native() {
    let root = temp("fa");
    let mc = root.join("game");
    fs::create_dir_all(mc.join("mods")).unwrap();
    write_mod_jar(&mc.join("mods"));
    // 本工具已套用進遊戲的翻譯包：AI 譯文（a 超長；b 比 jar 原文更長，驗證「較長者勝」不影響判定）
    let lang = mc.join("resourcepacks/繁體中文翻譯/assets/b2fa/lang");
    fs::create_dir_all(&lang).unwrap();
    fs::write(mc.join("resourcepacks/繁體中文翻譯/pack.mcmeta"), r#"{"pack":{"pack_format":15,"description":"t"}}"#).unwrap();
    fs::write(lang.join("zh_tw.json"), r#"{"gui.b2fa.a":"目前處於開啟狀態","gui.b2fa.b":"目前處於完全關閉的狀態"}"#).unwrap();

    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    crate::engine::output_guard::reset_sources();
    assert_eq!(prepare_build_sources(&work, &mc), CatalogSource::Rescanned);
    let mut zh: LangMap = HashMap::new();
    let z = zh.entry("b2fa".into()).or_default();
    z.insert("gui.b2fa.a".into(), "目前處於開啟狀態".into()); // 舊包 AI 譯文
    z.insert("gui.b2fa.b".into(), "目前處於關閉狀態".into()); // 模組 jar 自帶
    let built = build_resource_pack(&zh, &opts(&work, "fa")).unwrap();
    let text = fs::read_to_string(PathBuf::from(&built.pack_dir).join("assets/b2fa/lang/zh_tw.json")).unwrap_or_default();
    assert!(!text.contains("目前處於開啟狀態"), "套用進遊戲的舊包譯文不是模組自帶，要完整檢查：{text}");
    assert!(text.contains("目前處於關閉狀態"), "模組 jar 自帶的翻譯照寫：{text}");
    let _ = fs::remove_dir_all(root);
}

fn ref_setup() -> LangMap {
    crate::engine::output_guard::reset_sources();
    let mut en: LangMap = HashMap::new();
    let e = en.entry("b2ref".into()).or_default();
    e.insert("gui.b2ref.long".into(), "On".into());
    e.insert("gui.b2ref.color".into(), "§aGreen§r text".into());
    crate::engine::output_guard::remember_sources(&en);
    let mut reference: LangMap = HashMap::new();
    let r = reference.entry("b2ref".into()).or_default();
    r.insert("gui.b2ref.long".into(), "目前處於開啟狀態".into()); // 人工譯文比較長
    r.insert("gui.b2ref.color".into(), "§a綠色文字§a".into()); // 人工譯文色碼壞
    reference
}

#[test]
fn b2_ref_reference_pack_translations_skip_only_the_length_check() {
    let root = temp("ref");
    let reference = ref_setup();
    crate::engine::output_guard::remember_reference(&reference);
    let built = build_resource_pack(&reference, &opts(&root, "ref")).unwrap();
    let text = fs::read_to_string(PathBuf::from(&built.pack_dir).join("assets/b2ref/lang/zh_tw.json")).unwrap_or_default();
    assert!(text.contains("目前處於開啟狀態"), "參考包人工譯文只免長度：{text}");
    assert!(!text.contains("§a綠色文字§a"), "參考包色碼壞的照樣退回：{text}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b2_ref_reference_survives_saved_table_for_supplement() {
    let root = temp("ref_saved");
    let work = root.join("work");
    let reference = ref_setup();
    crate::engine::output_guard::remember_reference(&reference);
    save(&work, &crate::engine::output_guard::snapshot_sources()).unwrap();
    // 補翻：重開程式後從存檔讀回
    crate::engine::output_guard::reset_sources();
    assert_eq!(prepare_build_sources(&work, &root.join("no-such-game")), CatalogSource::Saved);
    let built = build_resource_pack(&reference, &opts(&work, "ref_saved")).unwrap();
    let text = fs::read_to_string(PathBuf::from(&built.pack_dir).join("assets/b2ref/lang/zh_tw.json")).unwrap_or_default();
    assert!(text.contains("目前處於開啟狀態") && !text.contains("§a綠色文字§a"), "{text}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b2_ref_only_reference_pack_entries_are_registered() {
    use crate::engine::lang_provenance::{set_source, LangSource, ProvenanceMap};
    let mut zh: LangMap = HashMap::new();
    let z = zh.entry("b2ref".into()).or_default();
    z.insert("from_ref".into(), "參考".into());
    z.insert("from_jar".into(), "模組".into());
    let mut ref_zh: LangMap = HashMap::new();
    let r = ref_zh.entry("b2ref".into()).or_default();
    r.insert("from_ref".into(), "参考".into());
    r.insert("from_jar".into(), "別的".into());
    let mut prov: ProvenanceMap = HashMap::new();
    set_source(&mut prov, "b2ref", "from_ref", LangSource::RefPack);
    set_source(&mut prov, "b2ref", "from_jar", LangSource::Tw);
    let got = reference_values(&zh, &ref_zh, &prov);
    assert_eq!(got["b2ref"].get("from_ref").map(String::as_str), Some("參考"), "登記轉繁後實際要寫出的值");
    assert!(!got["b2ref"].contains_key("from_jar"));

    // 只有使用者明確指定的參考包才登記；自動搜到的、遊戲內舊翻譯包與接續來源都不登記
    let lib = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs")).unwrap();
    assert!(lib.contains("let ref_is_user_choice = user_ref.is_some();"), "免長度只給使用者明確指定的參考包");
    assert!(lib.contains("merge_reference(&mut zh, &ref_zh, ref_is_user_choice)"));
    let at = lib.find("remember_reference_after_merge(").expect("轉繁後要登記實際寫出的值");
    let after = &lib[at..];
    assert!(after.find("source_catalog_save(").unwrap() < after.find("再合併遊戲內既有").unwrap(), "參考包合併後要再存一次原文表");
    assert_eq!(lib.matches("remember_reference").count(), 1, "lib 只在參考包那一段登記");
}

#[test]
fn b2_txt_rescan_failure_and_missing_folder_are_explained_separately() {
    let root = temp("txt");
    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    // 遊戲資料夾不在
    let gone = prepare_build_sources(&work, &root.join("no-such-game"));
    assert_eq!(gone, CatalogSource::Missing);
    let note = gone.player_note().unwrap();
    assert!(note.contains("找不到遊戲資料夾"), "{note}");
    // 資料夾還在，但讀不到（例如不是遊戲資料夾、掃描出錯）
    let broken = root.join("not-a-game");
    fs::create_dir_all(&broken).unwrap();
    let failed = prepare_build_sources(&work, &broken);
    assert_eq!(failed, CatalogSource::RescanFailed);
    let note = failed.player_note().unwrap();
    assert!(note.contains("讀取英文原文時出錯") && !note.contains("找不到遊戲資料夾"), "{note}");
    // 兩種都要說清楚後果與怎麼恢復
    for n in [gone.player_note().unwrap(), note] {
        assert!(n.contains("部分安全檢查") && n.contains("重新執行一次完整翻譯"), "{n}");
    }
    let _ = fs::remove_dir_all(root);
}


/// 走正式流程的順序：merge_reference（merge_fill_missing）→ 來源標記 → 登記實際值 → 建包。
fn merge_then_build(user_chosen: bool, name: &str) -> String {
    use crate::engine::lang_provenance::{set_source, LangSource, ProvenanceMap};
    let root = temp(name);
    let reference = ref_setup();
    let mut zh: LangMap = HashMap::new();
    merge_reference(&mut zh, &reference, user_chosen);
    let mut prov: ProvenanceMap = HashMap::new();
    for (ns, m) in &zh {
        for k in m.keys() {
            set_source(&mut prov, ns, k, LangSource::RefPack);
        }
    }
    remember_reference_after_merge(&zh, &reference, &prov, user_chosen);
    let built = build_resource_pack(&zh, &opts(&root, name)).unwrap();
    let text = fs::read_to_string(PathBuf::from(&built.pack_dir).join("assets/b2ref/lang/zh_tw.json")).unwrap_or_default();
    let _ = fs::remove_dir_all(root);
    text
}

#[test]
fn b2_ref4_user_chosen_reference_keeps_long_human_translation_end_to_end() {
    let text = merge_then_build(true, "ref4_user");
    assert!(text.contains("目前處於開啟狀態"), "使用者指定的參考包：超長人工譯文要寫出：{text}");
    assert!(!text.contains("§a綠色文字§a"), "色碼壞的照樣退回：{text}");
}

#[test]
fn b2_ref4_auto_discovered_reference_gets_full_check() {
    let text = merge_then_build(false, "ref4_auto");
    assert!(!text.contains("目前處於開啟狀態"), "自動搜到的參考包可能是工具舊輸出，照常完整檢查：{text}");
}
