//! B1 最終審查：中途中斷重跑、移除翻譯的遊戲檢查、先認回再建身分、字體包覆蓋前的標記確認、
//! 未寫入檔的說法、另存檔的擁有者、識別碼部分認回。

use super::*;
use super::tests::{Stage, BACKUP};

/// B3 審查 F1：套用只收「本輪產出」的文字檔。這些測試直接放檔模擬翻譯結果，套用前先登記成本輪產出。
#[allow(dead_code)]
fn apply_to_instance_with_game_state(
    mc: &Path,
    work: &Path,
    hint: Option<&str>,
    policy: BackupPolicy,
    running: GameRunning,
) -> Result<ApplyResult, String> {
    let game = crate::engine::jar_scan::resolve_minecraft_dir(mc).unwrap_or_else(|_| mc.to_path_buf());
    crate::engine::text_sources::mark_all_produced_for_test(work, &game);
    super::apply_to_instance_with_game_state(mc, work, hint, policy, running)
}


fn record_path(stage: &Stage) -> PathBuf {
    apply_record::record_dir(&stage.mc).join(apply_record::RECORD_FILE)
}

fn file_marker_path(stage: &Stage, rel: &str) -> PathBuf {
    stage.mc.join(".mcpl").join("files").join(format!("{rel}.json"))
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn instance_id(mc: &Path) -> String {
    read_json(&mc.join(".mcpl/instance.json"))["id"].as_str().unwrap_or_default().to_string()
}

/// 翻譯結果多一個設定檔，遊戲裡同位置是資料夾：寫它一定失敗（在寫模組檔之前）。
fn block_a_write(stage: &Stage) {
    fs::create_dir_all(stage.work.join("config/zz")).unwrap();
    fs::write(stage.work.join("config/zz/b.txt"), "中文").unwrap();
    fs::create_dir_all(stage.mc.join("config/zz/b.txt/blocked")).unwrap();
}

fn unblock(stage: &Stage) {
    fs::remove_dir_all(stage.mc.join("config/zz/b.txt")).unwrap();
}

fn legacy_backup_beside(stage: &Stage) {
    let legacy = stage.root.join("翻譯套用備份_20250101_120000");
    fs::create_dir_all(legacy.join("mods")).unwrap();
    fs::write(legacy.join("mods/example.jar"), b"original").unwrap();
    let manifest = serde_json::json!({
        "stamp": "20250101_120000",
        "mc_dir": stage.mc.display().to_string(),
        "backup_dir": legacy.display().to_string(),
        "added": [],
        "overwritten": ["mods/example.jar"]
    });
    fs::write(legacy.join(APPLY_MANIFEST), manifest.to_string()).unwrap();
}

const FONT_FILE: &str = "resourcepacks/繁體中文遊戲字體/assets/minecraft/font/default.json";

fn font_pack(stage: &Stage, content: &str) -> PathBuf {
    let font = stage.root.join("字體結果").join("resourcepacks").join("繁體中文遊戲字體");
    fs::create_dir_all(font.join("assets/minecraft/font")).unwrap();
    fs::write(font.join("assets/minecraft/font/default.json"), content).unwrap();
    font
}

fn apply_font(stage: &Stage, content: &str) -> Result<crate::engine::font_pack::FontPackApplyResult, String> {
    crate::engine::font_pack::apply_font_pack_with_game_state(&stage.mc, &font_pack(stage, content), GameRunning::No)
}

fn with_lost_pack(stage: &Stage) {
    fs::write(stage.mc.join("resourcepacks/Lost.zip"), b"lost").unwrap();
    let old = stage.mc.join("翻譯套用備份_20250101_000000");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("options.txt"), "resourcePacks:[\"vanilla\",\"file/OtherZh.zip\",\"file/Lost.zip\"]\n").unwrap();
}

// ─── 1. 中途寫入失敗後重跑：沿用上次的分類與備份／隔離連結 ───

#[test]
fn fin1_legacy_pack_interrupted_rerun_then_remove_returns_english_original() {
    // 1.0.x 套用過：遊戲裡是舊版翻譯，舊備份裡是英文原檔
    let stage = Stage::new("f1-legacy");
    fs::write(stage.mc.join("mods/example.jar"), b"old-zh").unwrap();
    stage.translated_from_current_game();
    legacy_backup_beside(&stage);
    block_a_write(&stage);
    assert!(apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No).is_err());
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"old-zh", "寫入失敗前還沒動模組檔");

    unblock(&stage);
    let result = stage.apply(BACKUP);
    assert_eq!(result.status, ApplyStatus::Applied);
    let backup = apply_record::instance_backup_dir(&stage.mc).join("mods/example.jar");
    assert_eq!(fs::read(&backup).unwrap(), b"original", "剛認領的英文原檔備份不能被換成舊版翻譯");
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original", "移除翻譯要回到英文原檔");
}

#[test]
fn fin1_quarantined_unknown_file_stays_linked_after_rerun() {
    // 遊戲資料夾有工具痕跡但沒有紀錄：模組檔來源不明，覆蓋前先隔離
    let stage = Stage::new("f1-unknown");
    fs::write(stage.mc.join("options.txt.mcpl-bak"), "earlier").unwrap();
    block_a_write(&stage);
    assert!(apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No).is_err());
    let first = read_json(&record_path(&stage))["files"]["mods/example.jar"].clone();
    assert_eq!(first["origin"], "unknown", "{first}");

    unblock(&stage);
    stage.apply(BACKUP);
    let again = read_json(&record_path(&stage))["files"]["mods/example.jar"].clone();
    assert_eq!(again["origin"], "unknown", "重跑不能把來源不明的檔當原檔：{again}");
    assert_eq!(again["quarantineId"], first["quarantineId"], "要連回同一份隔離");
    assert!(
        !apply_record::instance_backup_dir(&stage.mc).join("mods/example.jar").exists(),
        "來源不明的檔不能變成原檔備份"
    );
    let restored = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(restored.quarantined.iter().any(|q| q.contains("example.jar")), "{:?}", restored.quarantined);
}

// ─── 2. 移除翻譯：遊戲開著就一個檔都不動 ───

#[test]
fn fin2_remove_translation_refuses_while_game_running() {
    let stage = Stage::new("f2-running");
    stage.apply(BACKUP);
    let before = stage.options();
    let err = restore_last_apply_with_game_state(
        &stage.mc,
        Some(&stage.work),
        GameRunning::Yes { detail: "遊戲開著".into() },
    )
    .unwrap_err();
    assert!(err.contains("關閉"), "{err}");
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated", "遊戲開著時不能動檔");
    assert_eq!(stage.options(), before);
}

// ─── 3. .mcpl 被刪後先套字體包／先修復清單：要先認回原本的紀錄 ───

#[test]
fn fin3_font_pack_after_marker_folder_deleted_reclaims_record() {
    let stage = Stage::new("f3-font");
    stage.apply(BACKUP);
    let old_id = instance_id(&stage.mc);
    fs::remove_dir_all(stage.mc.join(".mcpl")).unwrap();
    apply_font(&stage, "{}").unwrap();
    assert_eq!(instance_id(&stage.mc), old_id, "要沿用原本的識別碼，不能先建新的");
    // 第一次補標記、第二次還原
    let _ = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn fin3_pack_repair_after_marker_folder_deleted_reclaims_record() {
    let stage = Stage::new("f3-repair");
    with_lost_pack(&stage);
    stage.apply(BACKUP);
    let old_id = instance_id(&stage.mc);
    fs::remove_dir_all(stage.mc.join(".mcpl")).unwrap();
    crate::engine::pack_repair::repair_pack_list_with_game_state(&stage.mc, GameRunning::No).unwrap();
    assert_eq!(instance_id(&stage.mc), old_id, "要沿用原本的識別碼，不能先建新的");
    let _ = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

// ─── 4. 字體包覆蓋工具版本前，標記要能確認 ───

#[test]
fn fin4_font_pack_does_not_overwrite_tool_file_with_unrebuildable_marker() {
    let stage = Stage::new("f4-font-marker");
    apply_font(&stage, "{}").unwrap();
    fs::remove_file(file_marker_path(&stage, FONT_FILE)).unwrap();
    let mut record = read_json(&record_path(&stage));
    record["files"][FONT_FILE]["markerId"] = serde_json::json!("");
    write_json(&record_path(&stage), &record);
    let err = apply_font(&stage, "{\"v\":2}").unwrap_err();
    assert!(err.contains("無法確認"), "{err}");
    assert_eq!(fs::read_to_string(stage.mc.join(FONT_FILE)).unwrap(), "{}", "無法確認就不覆蓋");
}

// ─── 5. 中途中斷後移除：還沒寫入的檔不能說成被改過 ───

#[test]
fn fin5_interrupted_then_removed_says_not_written_yet() {
    let stage = Stage::new("f5-unwritten");
    block_a_write(&stage);
    assert!(apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No).is_err());
    unblock(&stage);
    let restored = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(restored.player_summary.contains("這次還沒被翻譯寫入，保持原樣"), "{}", restored.player_summary);
    assert!(!restored.player_summary.contains("被你或整合包更新改過"), "{}", restored.player_summary);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

// ─── 6. 設定檔旁的另存檔記在實際擁有者名下 ───

#[test]
fn fin6_font_only_pack_has_no_translation_to_remove() {
    let stage = Stage::new("f6-font-only");
    apply_font(&stage, "{}").unwrap();
    let record = read_json(&record_path(&stage));
    assert_eq!(record["files"]["options.txt.mcpl-bak"]["owner"], "font", "{record}");
    assert!(!has_apply_backups_in(&stage.mc, Some(&stage.work)).unwrap(), "只裝過字體包時不該出現「移除翻譯」");
    let err = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap_err();
    assert!(err.contains("沒有翻譯可以移除"), "{err}");
}

#[test]
fn fin6_repair_owns_its_options_backup() {
    let stage = Stage::new("f6-repair");
    with_lost_pack(&stage);
    crate::engine::pack_repair::repair_pack_list_with_game_state(&stage.mc, GameRunning::No).unwrap();
    let record = read_json(&record_path(&stage));
    assert_eq!(record["files"]["options.txt.mcpl-bak"]["owner"], "repair", "{record}");
    assert!(!has_apply_backups_in(&stage.mc, Some(&stage.work)).unwrap(), "只修復過清單時不該出現「移除翻譯」");
}

// ─── 7. 識別碼被刪：多數檔對得上就認回，對不上的列為無法確認 ───

fn stage_with_configs(tag: &str) -> Stage {
    let stage = Stage::new(tag);
    fs::create_dir_all(stage.work.join("config")).unwrap();
    for name in ["a", "b", "c"] {
        fs::write(stage.work.join(format!("config/{name}.txt")), format!("中文{name}")).unwrap();
    }
    stage
}

#[test]
fn fin7_partial_fingerprint_match_reclaims_and_lists_changed_file() {
    let stage = stage_with_configs("f7-partial");
    stage.apply(BACKUP);
    let old_id = instance_id(&stage.mc);
    fs::remove_dir_all(stage.mc.join(".mcpl")).unwrap();
    fs::write(stage.mc.join("config/a.txt"), "pack updated").unwrap();
    let first = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(instance_id(&stage.mc), old_id, "多數檔對得上要認回");
    assert!(first.player_summary.contains("部分檔案被整合包更新改過"), "{}", first.player_summary);
    assert!(first.player_summary.contains("其餘已認回"), "{}", first.player_summary);
    let second = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    assert!(second.uncertain.contains(&"config/a.txt".to_string()), "{:?}", second.uncertain);
    assert_eq!(fs::read_to_string(stage.mc.join("config/a.txt")).unwrap(), "pack updated");
}

#[test]
fn fin7_partial_reclaim_then_apply_quarantines_changed_file() {
    let stage = stage_with_configs("f7-apply");
    stage.apply(BACKUP);
    let old_id = instance_id(&stage.mc);
    fs::remove_dir_all(stage.mc.join(".mcpl")).unwrap();
    fs::write(stage.mc.join("config/a.txt"), "pack updated").unwrap();
    let result = stage.apply(BACKUP);
    assert_eq!(instance_id(&stage.mc), old_id);
    assert!(result.quarantined_files.contains(&"config/a.txt".to_string()), "{:?}", result.quarantined_files);
    assert!(!apply_record::instance_backup_dir(&stage.mc).join("config/a.txt").exists(), "無法確認的檔不能當原檔備份");
    assert!(result.player_summary.contains("其餘已認回"), "{}", result.player_summary);
}
