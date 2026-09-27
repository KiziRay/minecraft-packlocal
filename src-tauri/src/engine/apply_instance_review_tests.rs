//! 套用與移除翻譯：審查後補強的測試（舊版相容、玩家文字、紀錄）。

use super::*;
#[allow(unused_imports)]
use super::tests::{backup_dirs_in, Stage, BACKUP, NO_BACKUP_CONFIRMED, ORIGINAL_OPTIONS};

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


// ─── 審查後補強（B1 FAIL 修正）───

fn write_legacy_backup(stage: &Stage, overwritten: &[&str], added: &[&str]) -> PathBuf {
    let legacy = stage.root.join("翻譯套用備份_20250101_120000");
    fs::create_dir_all(legacy.join("mods")).unwrap();
    fs::write(legacy.join("mods/example.jar"), b"original").unwrap();
    let manifest = serde_json::json!({
        "stamp": "20250101_120000",
        "mc_dir": stage.mc.display().to_string(),
        "backup_dir": legacy.display().to_string(),
        "added": added,
        "overwritten": overwritten
    });
    fs::write(legacy.join(APPLY_MANIFEST), manifest.to_string()).unwrap();
    legacy
}

#[test]
fn legacy_apply_then_new_apply_then_remove_returns_legacy_english_original() {
    let stage = Stage::new("legacy-new");
    // 1.0.x 已經套用過：遊戲裡是舊版翻譯，英文原檔在舊版備份
    fs::write(stage.mc.join("mods/example.jar"), b"translated-old").unwrap();
    stage.translated_from_current_game();
    write_legacy_backup(&stage, &["mods/example.jar"], &[]);
    stage.apply(BACKUP);
    // 第四輪起舊備份會先認領進本包備份區（附標記）：裡面必須是舊備份的英文原檔，不是舊版翻譯
    assert_ne!(
        fs::read(apply_record::instance_backup_dir(&stage.mc).join("mods/example.jar")).unwrap_or_default(),
        b"translated-old",
        "不能把舊版翻譯後的內容當原檔備份"
    );
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn rerun_after_record_save_failure_does_not_back_up_translation_as_original() {
    let stage = Stage::new("save-fail");
    stage.apply(BACKUP);
    // 模擬：寫完檔案後存紀錄失敗（紀錄不見了）
    fs::remove_file(apply_record::record_dir(&stage.mc).join(apply_record::RECORD_FILE)).unwrap();
    stage.apply(BACKUP);
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn pack_update_file_replacing_tool_added_file_survives_removal() {
    let stage = Stage::new("pack-owns");
    fs::create_dir_all(stage.work.join("config/newmod")).unwrap();
    fs::write(stage.work.join("config/newmod/text.txt"), "工具加的中文").unwrap();
    stage.apply(BACKUP);
    // 整合包更新帶來自己的同名檔
    fs::write(stage.mc.join("config/newmod/text.txt"), "pack's own file").unwrap();
    stage.apply(BACKUP);
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(
        fs::read_to_string(stage.mc.join("config/newmod/text.txt")).unwrap(),
        "pack's own file",
        "整合包自己的檔案要保留"
    );
}

#[test]
fn added_file_changed_after_apply_is_not_deleted() {
    let stage = Stage::new("added-changed");
    fs::create_dir_all(stage.work.join("config/newmod")).unwrap();
    fs::write(stage.work.join("config/newmod/text.txt"), "工具加的中文").unwrap();
    stage.apply(BACKUP);
    fs::write(stage.mc.join("config/newmod/text.txt"), "玩家自己改的").unwrap();
    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read_to_string(stage.mc.join("config/newmod/text.txt")).unwrap(), "玩家自己改的");
    assert!(result.skipped_modified.contains(&"config/newmod/text.txt".to_string()));
}

#[test]
fn record_for_another_game_folder_is_refused_in_plain_words() {
    // 第四輪起紀錄以整合包識別碼為鍵：改名、搬家仍是同一個整合包（見 hidden_marker_folder_…）。
    // 會被拒絕的是「整份複製出來的資料夾」：兩邊識別碼相同、原位置也還在。
    let stage = Stage::new("record-mismatch");
    stage.apply(BACKUP);
    let copy = stage.root.join("minecraft_copy");
    for entry in walkdir::WalkDir::new(&stage.mc).into_iter().filter_map(|e| e.ok()) {
        let rel = entry.path().strip_prefix(&stage.mc).unwrap();
        let dest = copy.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest).unwrap();
        } else {
            fs::copy(entry.path(), &dest).unwrap();
        }
    }
    let err = restore_last_apply_in(&copy, Some(&stage.work)).unwrap_err();
    assert!(fs::read(copy.join("mods/example.jar")).unwrap() == b"translated", "拒絕時不能動任何檔案");
    assert!(err.contains("不符") && err.contains("遊戲資料夾"), "{err}");
    assert!(stage.mc.join("mods/example.jar").exists(), "拒絕時不能動任何檔案");
}

#[test]
fn player_text_uses_plain_words() {
    let stage = Stage::new("plain").with_zip();
    let applied = stage.apply(BACKUP);
    assert!(applied.player_summary.contains("原本是 英文"), "{}", applied.player_summary);
    let restored = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(restored.player_summary.contains("已改回 英文"), "{}", restored.player_summary);
    for text in [&applied.player_summary, &restored.player_summary] {
        for jargon in ["en_us", "options.txt", "resourcepacks", "JAR", "診斷開不了"] {
            assert!(!text.contains(jargon), "玩家文字不該出現「{jargon}」：{text}");
        }
    }
}

#[test]
fn legacy_restore_summary_is_truthful() {
    let stage = Stage::new("legacy-text");
    fs::write(stage.mc.join("mods/example.jar"), b"translated-old").unwrap();
    write_legacy_backup(&stage, &["mods/example.jar"], &[]);
    let result = restore_last_apply_in(&stage.mc, None).unwrap();
    assert!(!result.player_summary.contains("不會直接修改"), "{}", result.player_summary);
    assert!(!result.player_summary.contains("診斷開不了"), "{}", result.player_summary);
    assert!(!result.player_summary.contains("mods/*.jar"), "{}", result.player_summary);
}
