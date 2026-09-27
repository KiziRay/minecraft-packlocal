use super::*;
use crate::engine::out_layout::ResultLayout;
use crate::engine::tool_products::test_support::{mark_overwritten_with_backup, temp_game, write};

fn layout(work: &Path) -> ResultLayout {
    ResultLayout {
        user_base: work.to_path_buf(),
        work_root: work.to_path_buf(),
        resourcepacks: work.join("resourcepacks"),
        config: work.join("config"),
        minemenu: work.join("minemenu"),
    }
}

fn targets(mc: &Path, plan: &ApplyPlan) -> Vec<String> {
    plan.targets().iter().map(|p| apply_record::rel_key(mc, p)).collect()
}

#[test]
fn b3_quest_lang_and_lang_siblings_are_dropped_when_their_source_changed_after_translation() {
    let mc = temp_game("textsrc");
    let work = temp_game("textsrc-work");
    write(&mc.join("config/ftbquests/quests/lang/en_us.snbt"), "{\n\tquest.A.title: \"入门\"\n}\n");
    write(&mc.join("kubejs/assets/x/lang/en_us.json"), r#"{"a":"Hi"}"#);
    write(&mc.join("kubejs/assets/x/lang/zh_cn.json"), r#"{"a":"简体"}"#);
    crate::engine::ftbquests::translate_ftbquests(&mc, &work, false, None, |_, _| {}).unwrap();
    crate::engine::text_overlay::translate_text_overlays(&mc, &work, false, None, |_, _| {}).unwrap();

    let plan_of = || crate::engine::apply_plan::build_plan(&mc, &layout(&work), "p", None, None);
    let mut plan = plan_of();
    let before = targets(&mc, &plan);
    assert!(before.contains(&"config/ftbquests/quests/lang/zh_tw.snbt".to_string()), "{before:?}");
    assert!(before.contains(&"kubejs/assets/x/lang/zh_tw.json".to_string()), "{before:?}");
    assert_eq!(drop_unconfirmed(&work, &mc, &mut plan, &ApplyRecord::default()), Dropped::default(), "來源沒變就照放");

    // 整合包更新：英文任務檔與英文語言檔都換了
    write(&mc.join("config/ftbquests/quests/lang/en_us.snbt"), "{\n\tquest.A.title: \"新版\"\n}\n");
    write(&mc.join("kubejs/assets/x/lang/en_us.json"), r#"{"a":"Hello new"}"#);
    let mut plan = plan_of();
    let dropped = drop_unconfirmed(&work, &mc, &mut plan, &ApplyRecord::default());
    assert_eq!(dropped.outdated.len(), 2, "{dropped:?}");
    assert!(!targets(&mc, &plan).iter().any(|t| t.ends_with("zh_tw.snbt") || t.ends_with("zh_tw.json")));
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_f1_leftovers_from_older_versions_in_the_result_folder_are_not_applied() {
    let mc = temp_game("f1stale");
    let work = temp_game("f1stale-work");
    write(&mc.join("kubejs/assets/x/lang/en_us.json"), r#"{"a":"Hi"}"#);
    write(&mc.join("kubejs/assets/x/lang/zh_cn.json"), r#"{"a":"简体"}"#);
    write(&mc.join("kubejs/client_scripts/a.js"), "if (x == 'Stone') Text.of('Stone')\n");
    write(&mc.join("config/mod.properties"), "mode=normal\n");
    // B3 以前的產物：原地改成中文的 en_us、整檔取代而壞掉的腳本、config 的 .properties 譯文
    write(&work.join("kubejs/assets/x/lang/en_us.json"), r#"{"a":"嗨"}"#);
    write(&work.join("kubejs/client_scripts/a.js"), "if (x == '石頭') Text.of('石頭')\n");
    write(&work.join("config/mod.properties"), "mode=\\u666E\\u901A\n");
    crate::engine::text_overlay::translate_text_overlays(&mc, &work, false, None, |_, _| {}).unwrap();
    crate::engine::script_literals::translate_kubejs_literals(&mc, &work, false, None, |_, _| {}).unwrap();

    let mut plan = crate::engine::apply_plan::build_plan(&mc, &layout(&work), "p", None, None);
    let dropped = drop_unconfirmed(&work, &mc, &mut plan, &ApplyRecord::default());
    let left = targets(&mc, &plan);
    for stale in ["kubejs/assets/x/lang/en_us.json", "kubejs/client_scripts/a.js", "config/mod.properties"] {
        assert!(dropped.stale.contains(&stale.to_string()), "{stale} 要列為舊產物：{dropped:?}");
        assert!(!left.contains(&stale.to_string()), "{stale} 不得放進遊戲");
    }
    assert!(left.contains(&"kubejs/assets/x/lang/zh_tw.json".to_string()), "本輪產出的照放：{left:?}");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_f2_fingerprint_is_the_original_so_reapplying_over_a_tool_version_is_not_outdated() {
    let mc = temp_game("f2fp");
    let work = temp_game("f2fp-work");
    // 已套用過的遊戲：腳本是工具版本，原檔在備份
    mark_overwritten_with_backup(&mc, "kubejs/client_scripts/a.js", "Text.of('你好')\n", Some("Text.of('Hello there')\n"));
    let out = work.join("kubejs/client_scripts/a.js");
    write(&out, "Text.of('哈囉')\n");
    record(&work, &out, &mc, &mc.join("kubejs/client_scripts/a.js"), &crate::engine::apply_guard::backup_file_path(&mc, "kubejs/client_scripts/a.js"), "scripts");
    let mut plan = crate::engine::apply_plan::build_plan(&mc, &layout(&work), "p", None, None);
    let dropped = drop_unconfirmed(&work, &mc, &mut plan, &ApplyRecord::default());
    assert_eq!(dropped, Dropped::default(), "來源指紋是原檔，遊戲裡的工具版本對得上原檔指紋");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_apply_checks_text_sources_before_writing() {
    let src = include_str!("apply_instance.rs");
    let at = src.find("text_sources::drop_unconfirmed(").expect("套用前要檢查文字產出清單");
    assert!(at < src.find("逐檔判斷「原本是什麼」").unwrap(), "必須在逐檔判斷與寫入之前");
    assert!(src.contains("apply_restore::restore_only("), "這輪沒產出的舊翻譯要走移除翻譯的還原流程");
}

#[test]
fn b3_r3_unreadable_game_root_does_not_commit() {
    let mc = temp_game("r3root");
    let work = temp_game("r3root-work");
    write(&mc.join("a/en.json"), "x");
    write(&work.join("a/zh.json"), "y");
    begin(&work, "p");
    record(&work, &work.join("a/zh.json"), &mc, &mc.join("a/en.json"), &mc.join("a/en.json"), "p");
    commit(&work, "p", &mc);
    // 遊戲根目錄讀不到（斷線）：這輪不 commit，上一輪條目與退休名單不動
    let gone = mc.join("offline");
    begin(&work, "p");
    commit(&work, "p", &gone);
    assert!(retire_candidates(&work).is_empty());
    assert!(is_confirmed_output(&work, &work.join("a/zh.json")), "上一輪條目仍有效");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_r3_unreadable_manifest_is_never_overwritten() {
    let mc = temp_game("r3corrupt");
    let work = temp_game("r3corrupt-work");
    write(&work.join(SOURCES_FILE), "{ 壞掉的清單");
    write(&mc.join("a/en.json"), "x");
    write(&work.join("a/zh.json"), "y");
    begin(&work, "p");
    record(&work, &work.join("a/zh.json"), &mc, &mc.join("a/en.json"), &mc.join("a/en.json"), "p");
    commit(&work, "p", &mc);
    assert_eq!(fs::read_to_string(work.join(SOURCES_FILE)).unwrap(), "{ 壞掉的清單", "讀不懂就不寫回（不能蓋掉其他產出者）");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_r3_retire_is_not_confirmed_for_another_game_folder() {
    let mc = temp_game("r3game");
    let other = temp_game("r3game-other");
    let work = temp_game("r3game-work");
    write(&mc.join("a/en.json"), "x");
    write(&work.join("a/zh.json"), "y");
    begin(&work, "p");
    record(&work, &work.join("a/zh.json"), &mc, &mc.join("a/en.json"), &mc.join("a/en.json"), "p");
    commit(&work, "p", &mc);
    fs::remove_file(mc.join("a/en.json")).unwrap();
    begin(&work, "p");
    commit(&work, "p", &mc);
    assert_eq!(retire_candidates(&work).len(), 1);
    assert!(confirm_retire(&work, &other).is_empty(), "套到別的遊戲資料夾不做任何退休");
    assert_eq!(confirm_retire(&work, &mc).len(), 1);
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&other);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_r4_removed_source_folder_is_not_reported_as_changed_and_becomes_retirable() {
    let mc = temp_game("r4gone");
    let work = temp_game("r4gone-work");
    write(&mc.join("kubejs/assets/x/lang/en_us.json"), r#"{"a":"Hi"}"#);
    write(&mc.join("kubejs/assets/x/lang/zh_cn.json"), r#"{"a":"简体"}"#);
    crate::engine::text_overlay::translate_text_overlays(&mc, &work, false, None, |_, _| {}).unwrap();
    // 整合包更新把整個來源資料夾刪了（遊戲根讀得到）
    fs::remove_dir_all(mc.join("kubejs/assets/x")).unwrap();
    let mut plan = crate::engine::apply_plan::build_plan(&mc, &layout(&work), "p", None, None);
    let dropped = drop_unconfirmed(&work, &mc, &mut plan, &ApplyRecord::default());
    assert!(dropped.outdated.is_empty(), "來源被刪不是「來源已變更」：{dropped:?}");
    assert!(dropped.source_removed.contains(&"kubejs/assets/x/lang/zh_tw.json".to_string()), "{dropped:?}");
    crate::engine::text_overlay::translate_text_overlays(&mc, &work, false, None, |_, _| {}).unwrap();
    assert!(retire_candidates(&work).contains("kubejs/assets/x/lang/zh_tw.json"), "上層整個被刪、遊戲根讀得到：確定不在");
    let _ = fs::remove_dir_all(&mc);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn b3_r4_unreachable_source_is_unconfirmed() {
    let mc = temp_game("r4unk");
    assert_eq!(source_state(&mc.join("offline-root"), "a/b.json"), SourceState::Unconfirmed, "遊戲根讀不到：無法確認");
    write(&mc.join("a/keep.txt"), "x");
    assert_eq!(source_state(&mc, "a/b.json"), SourceState::Gone);
    assert_eq!(source_state(&mc, "a/keep.txt"), SourceState::Present);
    let _ = fs::remove_dir_all(&mc);
}

#[test]
fn b3_r4_manifest_with_unknown_fields_is_refused_and_old_retire_is_migrated() {
    let work = temp_game("r4fmt");
    write(&work.join(SOURCES_FILE), r#"{"entries":{},"somethingNew":1}"#);
    begin(&work, "p");
    assert!(fs::read_to_string(work.join(SOURCES_FILE)).unwrap().contains("somethingNew"), "看不懂的欄位：不寫回");
    write(&work.join(SOURCES_FILE), r#"{"entries":{},"retire":{"overlay":["a/zh.json"]},"rounds":{}}"#);
    begin(&work, "p");
    let text = fs::read_to_string(work.join(SOURCES_FILE)).unwrap();
    assert!(text.contains("retiring") && text.contains("a/zh.json") && !text.contains("\"retire\""), "{text}");
    let _ = fs::remove_dir_all(&work);
}
