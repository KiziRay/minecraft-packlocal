//! B3 審查：套用只收本輪產出；「退休」（把遊戲裡上一版譯文拿掉）是破壞性動作，
//! 必須先確認「確定不再產出」——產出者本輪已完成、而且來源檔已不在遊戲。

use super::tests::{Stage, BACKUP};
use super::*;

const EN: &str = "kubejs/assets/x/lang/en_us.json";
const ZH: &str = "kubejs/assets/x/lang/zh_tw.json";

fn produce(stage: &Stage) {
    crate::engine::text_overlay::translate_text_overlays(&stage.mc, &stage.work, false, None, |_, _| {}).unwrap();
}

/// 真正的套用（不經測試包裝，不會把工作資料夾全部登記成本輪產出）。
fn apply(stage: &Stage) -> ApplyResult {
    super::apply_to_instance_with_game_state(&stage.mc, &stage.work, Some("繁體中文翻譯"), BACKUP, GameRunning::No).unwrap()
}

/// 遊戲裡有英文＋簡中語言檔，翻一輪並套用：zh_tw.json 由工具新增進遊戲。
fn applied_stage(tag: &str) -> Stage {
    let stage = Stage::new(tag);
    fs::create_dir_all(stage.mc.join("kubejs/assets/x/lang")).unwrap();
    fs::write(stage.mc.join(EN), r#"{"a":"Hi"}"#).unwrap();
    fs::write(stage.mc.join("kubejs/assets/x/lang/zh_cn.json"), r#"{"a":"简体"}"#).unwrap();
    produce(&stage);
    apply(&stage);
    assert!(fs::read_to_string(stage.mc.join(ZH)).unwrap().contains("簡體"), "前提：譯文已放進遊戲");
    stage
}

fn remove_source(stage: &Stage) {
    fs::remove_file(stage.mc.join(EN)).unwrap();
    fs::remove_file(stage.mc.join("kubejs/assets/x/lang/zh_cn.json")).unwrap();
}

#[test]
fn b3_fa_cancelled_rerun_then_apply_keeps_the_previous_translation() {
    let stage = applied_stage("b3-fa-cancel");
    // 重翻到一半被取消：產出者開始了、沒有完成
    crate::engine::text_sources::begin(&stage.work, "overlay");
    let result = apply(&stage);
    assert!(stage.mc.join(ZH).is_file(), "中途取消不能把遊戲裡上一版譯文拿掉：{:?}", result.retired_files);
    assert!(result.retired_files.is_empty());
}

#[test]
fn b3_fa_only_a_removed_source_retires_the_translation() {
    let stage = applied_stage("b3-fa-removed");
    remove_source(&stage);
    produce(&stage);
    let result = apply(&stage);
    assert!(result.retired_files.contains(&ZH.to_string()), "來源被整合包移除才退休：{:?}", result.retired_files);
    assert!(!stage.mc.join(ZH).exists());
}

#[test]
fn b3_fa_source_still_there_but_not_translated_this_time_is_not_retired() {
    let stage = applied_stage("b3-fa-skip");
    // 這輪沒產出（例如簡中檔不見、又沒開 AI）但英文來源還在：不是退休理由
    fs::remove_file(stage.mc.join("kubejs/assets/x/lang/zh_cn.json")).unwrap();
    produce(&stage);
    let result = apply(&stage);
    assert!(result.retired_files.is_empty(), "{:?}", result.retired_files);
    assert!(stage.mc.join(ZH).is_file());
}

#[test]
fn b3_fb_stale_and_outdated_files_are_listed_not_retired() {
    let stage = applied_stage("b3-fb-stale");
    // 產出後被改過（stale）
    fs::write(stage.work.join(ZH), r#"{"a":"被改過"}"#).unwrap();
    let result = apply(&stage);
    assert!(result.stale_outputs.contains(&ZH.to_string()), "{:?}", result.stale_outputs);
    assert!(result.retired_files.is_empty(), "stale 只列出、不退休：{:?}", result.retired_files);
    assert!(fs::read_to_string(stage.mc.join(ZH)).unwrap().contains("簡體"), "遊戲維持現狀");

    let stage = applied_stage("b3-fb-outdated");
    // 翻譯之後來源改了（outdated）
    fs::write(stage.mc.join(EN), r#"{"a":"Hello new"}"#).unwrap();
    let result = apply(&stage);
    assert!(result.outdated_texts.contains(&ZH.to_string()), "{:?}", result.outdated_texts);
    assert!(result.retired_files.is_empty(), "outdated 只列出、不退休：{:?}", result.retired_files);
    assert!(stage.mc.join(ZH).is_file());
}

#[test]
fn b3_fc_legacy_in_place_en_us_can_be_retranslated_and_applied() {
    let stage = Stage::new("b3-fc-legacy");
    fs::create_dir_all(stage.mc.join("kubejs/assets/x/lang")).unwrap();
    // 1.0.x：en_us 被原地改成中文，舊版備份（遊戲資料夾旁）有原檔，舊結果裡也是那份中文
    let chinese = r#"{"a":"嗨"}"#;
    fs::write(stage.mc.join(EN), chinese).unwrap();
    fs::write(stage.mc.join("kubejs/assets/x/lang/zh_cn.json"), r#"{"a":"简体"}"#).unwrap();
    let legacy = stage.root.join("翻譯套用備份_20250101_1");
    fs::create_dir_all(legacy.join("kubejs/assets/x/lang")).unwrap();
    fs::write(legacy.join(EN), r#"{"a":"Hi"}"#).unwrap();
    fs::write(
        legacy.join(APPLY_MANIFEST),
        serde_json::json!({ "mc_dir": stage.mc.display().to_string(), "added": [], "overwritten": [EN] }).to_string(),
    )
    .unwrap();
    fs::create_dir_all(stage.work.join("kubejs/assets/x/lang")).unwrap();
    fs::write(stage.work.join(EN), chinese).unwrap();
    produce(&stage);
    let result = apply(&stage);
    assert!(!result.outdated_texts.contains(&ZH.to_string()), "讀舊版備份做出的譯文不能永遠判過期：{:?}", result.outdated_texts);
    assert!(stage.mc.join(ZH).is_file(), "{:?}", result.stale_outputs);
}

#[test]
fn b3_fc_first_apply_after_mcpl_deleted_is_not_all_outdated() {
    let stage = Stage::new("b3-fc-mcpl");
    let adv = "data/ns/advancements/a.json";
    fs::create_dir_all(stage.mc.join("data/ns/advancements")).unwrap();
    fs::write(stage.mc.join(adv), r#"{"display":{"title":"简体标题"}}"#).unwrap();
    produce(&stage);
    apply(&stage);
    assert!(fs::read_to_string(stage.mc.join(adv)).unwrap().contains("簡體"), "前提：工具覆蓋了原檔");
    fs::remove_dir_all(stage.mc.join(".mcpl")).unwrap();
    produce(&stage);
    let result = apply(&stage);
    assert!(!result.outdated_texts.contains(&adv.to_string()), "{:?}", result.outdated_texts);
    assert!(!result.stale_outputs.contains(&adv.to_string()), "讀得到原檔備份，本輪有產出：{:?}", result.stale_outputs);
    assert!(fs::read_to_string(stage.mc.join(adv)).unwrap().contains("簡體"));
}

#[test]
fn b3_retire_keeps_player_edits_font_owner_and_other_packs_backups() {
    // 玩家改過的檔：來源移除也不退休（列在沒動的清單）
    let stage = applied_stage("b3-retire-player");
    fs::write(stage.mc.join(ZH), r#"{"a":"玩家自己改的"}"#).unwrap();
    remove_source(&stage);
    produce(&stage);
    let result = apply(&stage);
    assert!(fs::read_to_string(stage.mc.join(ZH)).unwrap().contains("玩家自己改的"));
    assert!(result.retire_skipped.contains(&ZH.to_string()), "{:?}", result.retire_skipped);

    // 字體包的檔：退休只處理翻譯放的檔
    let stage = applied_stage("b3-retire-font");
    let mut record = apply_record::load(&stage.mc).unwrap();
    record.files.get_mut(ZH).unwrap().owner = apply_record::OWNER_FONT.to_string();
    apply_record::save(&stage.mc, &mut record).unwrap();
    remove_source(&stage);
    produce(&stage);
    apply(&stage);
    assert!(stage.mc.join(ZH).is_file(), "字體包的檔不動");

    // 覆蓋了原有 zh_tw、備份卻屬於別的整合包：不拿來還原
    let stage = Stage::new("b3-retire-otherpack");
    fs::create_dir_all(stage.mc.join("kubejs/assets/x/lang")).unwrap();
    fs::write(stage.mc.join(EN), r#"{"a":"Hi","b":"Bye"}"#).unwrap();
    fs::write(stage.mc.join("kubejs/assets/x/lang/zh_cn.json"), r#"{"b":"再见"}"#).unwrap();
    fs::write(stage.mc.join(ZH), r#"{"a":"人工"}"#).unwrap();
    produce(&stage);
    apply(&stage);
    let marker_path = apply_guard::backup_marker_path(&stage.mc, ZH);
    let mut marker: serde_json::Value = serde_json::from_str(&fs::read_to_string(&marker_path).unwrap()).unwrap();
    marker["instanceId"] = serde_json::json!("another-pack");
    fs::write(&marker_path, marker.to_string()).unwrap();
    remove_source(&stage);
    produce(&stage);
    let result = apply(&stage);
    assert!(result.retire_skipped.contains(&ZH.to_string()), "別包的備份不用：{:?}", result.retire_skipped);
}

// ─── 第三輪 ───

#[test]
fn b3_r3_source_back_before_apply_is_not_retired() {
    let stage = applied_stage("b3-r3-back");
    let en = fs::read(stage.mc.join(EN)).unwrap();
    let cn = fs::read(stage.mc.join("kubejs/assets/x/lang/zh_cn.json")).unwrap();
    remove_source(&stage);
    produce(&stage);
    // 翻完之後玩家把模組裝回來：套用時來源又在了
    fs::write(stage.mc.join(EN), en).unwrap();
    fs::write(stage.mc.join("kubejs/assets/x/lang/zh_cn.json"), cn).unwrap();
    let result = apply(&stage);
    assert!(result.retired_files.is_empty(), "套用時再確認來源仍不在才退休：{:?}", result.retired_files);
    assert!(stage.mc.join(ZH).is_file());
    assert!(crate::engine::text_sources::retire_candidates(&stage.work).is_empty(), "來源回來就從退休名單移除");
}

#[test]
fn b3_r3_vanished_source_folder_under_a_readable_game_is_a_removal() {
    // 第四輪 A（取代第三輪「整個資料夾不見當讀取異常」）：往上找到的最近一層存在的資料夾在遊戲根底下
    // 且讀得到 → 確定不在；讀取錯誤／遊戲根讀不到才是無法確認（見 text_sources::tests::b3_r4_unreachable_source_is_unconfirmed、
    // b3_r3_unreadable_game_root_does_not_commit）
    let stage = applied_stage("b3-r3-vanish");
    fs::remove_dir_all(stage.mc.join("kubejs/assets/x")).unwrap();
    produce(&stage);
    assert!(crate::engine::text_sources::retire_candidates(&stage.work).contains(ZH));
}
