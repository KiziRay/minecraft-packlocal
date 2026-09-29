//! B5c：完成卡要的資料由後端帶出（OneClickResult.applyResult、interruption.cause），
//! 前端不再從中文句子猜。lib.rs 需要 AppHandle 的流程以原始碼檢查釘住關鍵點。

use crate::engine::{ApplyResult, ApplyStatus};

fn body_of<'a>(src: &'a str, name: &str) -> &'a str {
    let body = &src[src.find(name).unwrap_or_else(|| panic!("找不到 {name}"))..];
    &body[..body.find("\n}\n").unwrap_or(body.len())]
}

fn lib_src() -> String {
    std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs"))
        .unwrap()
        .replace("\r\n", "\n")
}

fn sample_apply() -> ApplyResult {
    ApplyResult {
        status: ApplyStatus::Applied,
        backup_dir: "C:/b".into(),
        backup_created: true,
        backup_reused: false,
        zip_copied: Some("MCPL.zip".into()),
        jars_copied: 3,
        quests_copied: false,
        minemenu_copied: false,
        lang_set: true,
        original_lang: Some("en_us".into()),
        pending_overwrites: vec![],
        unknown_files: vec!["u".into()],
        skipped_changed: vec!["s".into()],
        quarantined_files: vec!["q".into()],
        unconfirmed_files: vec!["c".into()],
        outdated_mods: vec!["mods/a.jar".into()],
        outdated_texts: vec!["config/b.snbt".into()],
        stale_outputs: vec!["x".into()],
        retired_files: vec!["r".into()],
        retire_skipped: vec![],
        source_removed_texts: vec!["t".into()],
        unverifiable_texts: vec!["v".into()],
        player_summary: "已把翻譯套用到遊戲".into(),
        warnings: vec!["w".into()],
    }
}

#[test]
fn b5c_apply_result_serializes_every_field_the_result_card_reads() {
    let json = serde_json::to_value(sample_apply()).unwrap();
    for key in [
        "status", "backupDir", "backupCreated", "backupReused", "zipCopied", "jarsCopied", "langSet", "originalLang",
        "unknownFiles", "skippedChanged", "quarantinedFiles", "unconfirmedFiles", "outdatedMods", "outdatedTexts",
        "staleOutputs", "retiredFiles", "retireSkipped", "sourceRemovedTexts", "unverifiableTexts", "warnings",
    ] {
        assert!(json.get(key).is_some(), "ApplyResult 少了 {key}：{json}");
    }
}

#[test]
fn b5c_one_click_result_carries_the_apply_result_and_the_interruption_cause() {
    let lib = lib_src();
    let def = body_of(&lib, "struct OneClickResult {");
    assert!(def.contains("apply_result: Option<ApplyResult>"), "OneClickResult 要帶出 ApplyResult 各欄");
    let notice = body_of(&lib, "fn with_apply_notice(");
    assert!(notice.contains("result.apply_result = Some(applied.clone())"), "with_apply_notice 要把套用結果帶進翻譯結果");
    // 每個建構 OneClickResult 的地方都走 with_apply_notice（新欄位不會漏設）
    let built = lib.matches("(OneClickResult {").count();
    let wrapped = lib.matches("with_apply_notice(OneClickResult {").count();
    assert!(wrapped >= 4 && built == wrapped, "每個建構 OneClickResult 的地方都經過 with_apply_notice");
    let view = serde_json::to_value(crate::engine::run_interrupt::view(0, 0)).unwrap();
    assert!(view.get("cause").is_some() && view.get("failureClass").is_some(), "{view}");
}

#[test]
fn b5c_copied_folder_refusal_after_a_run_is_not_a_translation_failure() {
    let lib = lib_src();
    let body = body_of(&lib, "fn apply_after_run(");
    assert!(body.contains("pending_when_copied("), "翻完套用時複製資料夾被擋要回「已翻完、還沒套用」");
}

#[test]
fn b5c_backend_conclusion_no_longer_claims_all_chinese_or_asks_where_results_are() {
    let lib = lib_src();
    let one = body_of(&lib, "fn run_one_click(");
    assert!(!one.contains("補翻／修復時「結果存哪」"), "LIB:3355 刪（規格 §5.3）");
    assert!(!lib.contains("只補缺漏"), "LIB:4685 刪（規格 §5.3）");
    assert!(!one.contains("完成！整合包裡玩得到的文字已翻成台灣繁體中文"), "停下或部分完成時不得寫全部是中文；結論以完成卡為準");
    assert!(!one.contains("裝進遊戲。\""), "用詞：套用到遊戲");
}

#[test]
fn b5c_fix1_apply_command_checks_the_result_belongs_to_this_game_folder() {
    let lib = lib_src();
    let body = body_of(&lib, "async fn apply_translation_to_game(");
    let guard = body.find("engine::result_owner::guard(").expect("套用前要比對結果屬於哪個遊戲資料夾");
    let apply = body.find("apply_to_instance(").unwrap();
    assert!(guard < apply, "比對要在任何寫入之前");
}
