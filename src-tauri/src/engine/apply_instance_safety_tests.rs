//! 套用與移除翻譯的檔案安全測試（原則 A／B／C、前置條件、隱藏標記、標記互相關聯）。

use super::*;
use super::tests::{Stage, BACKUP, ORIGINAL_OPTIONS};

// ─── 第三輪：原則 A（只備份確定的原檔）、B（只刪／蓋確定是工具寫的）、C（紀錄）───

fn legacy_dir_in(container: &Path, stamp: &str, mc: &Path, overwritten: &[&str], added: &[&str]) -> PathBuf {
    let legacy = container.join(format!("翻譯套用備份_{stamp}"));
    fs::create_dir_all(&legacy).unwrap();
    let manifest = serde_json::json!({
        "stamp": stamp,
        "mc_dir": mc.display().to_string(),
        "backup_dir": legacy.display().to_string(),
        "added": added,
        "overwritten": overwritten
    });
    fs::write(legacy.join(APPLY_MANIFEST), manifest.to_string()).unwrap();
    legacy
}

fn instance_backup_has(stage: &Stage, rel: &str) -> bool {
    apply_record::instance_backup_dir(&stage.mc).join(rel).exists()
}

#[test]
fn a_legacy_overwrite_listed_but_backup_missing_is_not_backed_up() {
    let stage = Stage::new("a-legacy-nobak");
    fs::write(stage.mc.join("mods/example.jar"), b"translated-old").unwrap();
    stage.translated_from_current_game();
    // 舊版清單說覆蓋過，但備份裡沒有那個檔
    legacy_dir_in(&stage.root, "20250101_1", &stage.mc, &["mods/example.jar"], &[]);
    let result = stage.apply(BACKUP);
    assert_eq!(result.status, ApplyStatus::Applied);
    assert!(!instance_backup_has(&stage, "mods/example.jar"), "舊版翻譯後的內容不能當原檔備份");
    assert!(result.unknown_files.contains(&"mods/example.jar".to_string()), "{:?}", result.unknown_files);
    let restored = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(restored.uncertain.contains(&"mods/example.jar".to_string()), "{:?}", restored.uncertain);
    assert!(restored.player_summary.contains("無法確認"), "{}", restored.player_summary);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated", "來源不明的不能亂刪或蓋");
}

#[test]
fn a_reapply_after_record_reset_creates_no_new_backup_and_says_so() {
    let stage = Stage::new("a-reset");
    stage.apply(BACKUP);
    apply_record::reset_record(&stage.mc).unwrap();
    // 重設後又多了一個看起來是原檔的檔：仍無法確定，不能建立新備份
    fs::create_dir_all(stage.mc.join("config/late")).unwrap();
    fs::create_dir_all(stage.work.join("config/late")).unwrap();
    fs::write(stage.mc.join("config/late/a.txt"), "english").unwrap();
    fs::write(stage.work.join("config/late/a.txt"), "中文").unwrap();
    // 第四輪起：紀錄重設後，遊戲資料夾裡的標記仍在，紀錄從標記重建；
    // 沒有標記、也不是工具內容的檔就是確定的原檔，可以備份（備份裡必須是原檔、不是翻譯）
    stage.apply(BACKUP);
    assert_eq!(
        fs::read_to_string(apply_record::instance_backup_dir(&stage.mc).join("config/late/a.txt")).unwrap(),
        "english"
    );
    let _ = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    assert_eq!(fs::read_to_string(stage.mc.join("config/late/a.txt")).unwrap(), "english");
}

#[test]
fn a_renamed_game_folder_reapply_does_not_back_up_translation() {
    let stage = Stage::new("a-rename");
    stage.apply(BACKUP);
    let renamed = stage.root.join("minecraft_renamed");
    fs::rename(&stage.mc, &renamed).unwrap();
    // 第四輪起：遊戲資料夾裡的 .mcpl 識別碼讓改名後仍認得是同一個整合包，
    // 翻譯版不會被當成原檔，移除翻譯可以回到原版
    apply_to_instance_with_game_state(&renamed, &stage.work, Some("繁體中文翻譯"), BACKUP, GameRunning::No).unwrap();
    assert_eq!(
        fs::read(apply_record::instance_backup_dir(&renamed).join("mods/example.jar")).unwrap(),
        b"original",
        "翻譯版不能被當原檔"
    );
    restore_last_apply_in(&renamed, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(renamed.join("mods/example.jar")).unwrap(), b"original");
    let _ = fs::remove_dir_all(apply_record::record_dir(&renamed));
    let _ = fs::remove_dir_all(apply_record::backup_container(&renamed));
}

#[test]
fn a_orphan_legacy_backup_without_manifest_blocks_new_backups() {
    let stage = Stage::new("a-orphan");
    fs::create_dir_all(stage.root.join("翻譯套用備份_20240101_1/config")).unwrap();
    let result = stage.apply(BACKUP);
    assert!(!instance_backup_has(&stage, "mods/example.jar"));
    assert!(result.unknown_files.contains(&"mods/example.jar".to_string()));
}

#[test]
fn a_legacy_manifest_in_another_known_result_root_is_found() {
    let stage = Stage::new("a-other-root");
    fs::write(stage.mc.join("mods/example.jar"), b"translated-old").unwrap();
    stage.translated_from_current_game();
    let other_root = stage.root.join("繁中翻譯輸出").join("翻譯結果");
    let legacy = legacy_dir_in(&other_root, "20250101_2", &stage.mc, &["mods/example.jar"], &[]);
    fs::create_dir_all(legacy.join("mods")).unwrap();
    fs::write(legacy.join("mods/example.jar"), b"original").unwrap();
    stage.apply(BACKUP);
    // 認領進本包備份區的是舊備份的原檔
    assert_eq!(fs::read(apply_record::instance_backup_dir(&stage.mc).join("mods/example.jar")).unwrap(), b"original");
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn b_legacy_restore_leaves_changed_file_and_lists_it() {
    let stage = Stage::new("b-legacy-changed");
    fs::write(stage.mc.join("mods/example.jar"), b"player-changed").unwrap();
    let legacy = legacy_dir_in(&stage.root, "20250101_3", &stage.mc, &["mods/example.jar"], &[]);
    fs::create_dir_all(legacy.join("mods")).unwrap();
    fs::write(legacy.join("mods/example.jar"), b"original").unwrap();
    let result = restore_last_apply_in(&stage.mc, None).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"player-changed");
    assert!(result.uncertain.contains(&"mods/example.jar".to_string()), "{:?}", result.uncertain);
    assert!(result.player_summary.contains("無法確認"), "{}", result.player_summary);
}

#[test]
fn b_legacy_restore_does_not_delete_unidentifiable_added_file() {
    let stage = Stage::new("b-legacy-added");
    fs::create_dir_all(stage.mc.join("config/x")).unwrap();
    fs::write(stage.mc.join("config/x/a.txt"), "某個內容").unwrap();
    legacy_dir_in(&stage.root, "20250101_4", &stage.mc, &[], &["config/x/a.txt"]);
    let result = restore_last_apply_in(&stage.mc, None).unwrap();
    assert!(stage.mc.join("config/x/a.txt").exists(), "沒有雜湊又認不出是工具產物，不能刪");
    assert!(result.uncertain.contains(&"config/x/a.txt".to_string()));
}

#[test]
fn b_legacy_restore_works_when_content_matches_known_tool_output() {
    let stage = Stage::new("b-legacy-match");
    // 遊戲裡的內容等於翻譯結果裡工具產出的那份 → 確定是工具寫的
    fs::write(stage.mc.join("mods/example.jar"), b"translated").unwrap();
    let legacy = legacy_dir_in(&stage.root, "20250101_5", &stage.mc, &["mods/example.jar"], &[]);
    fs::create_dir_all(legacy.join("mods")).unwrap();
    fs::write(legacy.join("mods/example.jar"), b"original").unwrap();
    restore_last_apply_in(&stage.mc, None).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn c_record_save_failure_stops_before_writing() {
    let stage = Stage::new("c-save-fail");
    let dir = apply_record::record_dir(&stage.mc);
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    fs::write(&dir, b"a file where the record folder should be").unwrap();
    let err = apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No).unwrap_err();
    assert!(err.contains("套用紀錄"), "{err}");
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original", "存不了紀錄就不能寫檔");
    let _ = fs::remove_file(&dir);
}

#[test]
fn c_unreadable_record_error_includes_path() {
    let stage = Stage::new("c-unreadable");
    let path = apply_record::record_dir(&stage.mc).join(apply_record::RECORD_FILE);
    fs::create_dir_all(&path).unwrap(); // 資料夾佔住檔名 → 讀取失敗（不是「不存在」）
    let err = apply_record::load(&stage.mc).unwrap_err();
    assert!(err.contains(&path.display().to_string()), "{err}");
    let _ = fs::remove_dir_all(&path);
}

#[test]
fn delete_all_backups_keeps_other_packs_backups() {
    let stage = Stage::new("del-other");
    let shared = stage.root.join("共用結果");
    let other_mc = stage.root.join("另一個整合包").join("minecraft");
    fs::create_dir_all(other_mc.join("mods")).unwrap();
    let mine = legacy_dir_in(&shared, "20250101_6", &stage.mc, &[], &[]);
    let theirs = legacy_dir_in(&shared, "20250101_7", &other_mc, &[], &[]);
    stage.apply(BACKUP);
    let result = delete_apply_backups_in(&stage.mc, Some(&shared)).unwrap();
    assert!(!mine.exists());
    assert!(theirs.exists(), "別的整合包的備份不能刪");
    assert!(!apply_record::instance_backup_dir(&stage.mc).exists());
    assert!(result.player_summary.contains("舊版"), "{}", result.player_summary);
}

#[test]
fn legacy_knowledge_survives_deleting_legacy_backups() {
    let stage = Stage::new("del-legacy");
    fs::write(stage.mc.join("mods/example.jar"), b"translated-old").unwrap();
    let legacy = legacy_dir_in(&stage.root, "20250101_8", &stage.mc, &["mods/example.jar"], &[]);
    fs::create_dir_all(legacy.join("mods")).unwrap();
    fs::write(legacy.join("mods/example.jar"), b"original").unwrap();
    delete_apply_backups_in(&stage.mc, None).unwrap();
    assert!(!legacy.exists());
    stage.apply(BACKUP);
    assert!(!instance_backup_has(&stage, "mods/example.jar"), "刪掉舊備份後也不能把舊版翻譯當原檔");
}

#[test]
fn options_backup_beside_is_created_once() {
    let stage = Stage::new("opt-once").with_zip();
    stage.apply(BACKUP);
    let first = fs::read_to_string(stage.mc.join("options.txt.mcpl-bak")).unwrap();
    assert_eq!(first, ORIGINAL_OPTIONS);
    stage.apply(BACKUP);
    assert_eq!(fs::read_to_string(stage.mc.join("options.txt.mcpl-bak")).unwrap(), first, "已存在就不覆寫");
}

#[test]
fn font_pack_after_translation_is_not_reported_as_player_change_and_removed_by_its_own_button() {
    let stage = Stage::new("font-then-remove").with_zip();
    stage.apply(BACKUP);
    let font = stage.root.join("字體結果").join("resourcepacks").join("繁體中文遊戲字體");
    fs::create_dir_all(font.join("assets/minecraft/font")).unwrap();
    fs::write(font.join("assets/minecraft/font/default.json"), "{}").unwrap();
    crate::engine::font_pack::apply_font_pack_to_instance(&stage.mc, &font).unwrap();
    let restored = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(restored.skipped_modified.is_empty(), "{:?}", restored.skipped_modified);
    assert!(restored.uncertain.is_empty(), "{:?}", restored.uncertain);
    // 第五輪 #9：字體包與翻譯分開，「移除翻譯」不動字體包，由「移除字體包」拿掉
    assert!(stage.options().contains("繁體中文遊戲字體"), "移除翻譯不能拿掉字體包");
    crate::engine::font_restore::remove_font_pack_in(&stage.mc).unwrap();
    assert!(!stage.options().contains("繁體中文遊戲字體"), "工具加的字體包要從清單拿掉");
    assert!(!stage.mc.join("resourcepacks/繁體中文遊戲字體").join("assets/minecraft/font/default.json").exists());
    assert!(!stage.mc.join("options.txt.mcpl-bak").exists());
}

// ─── 第四輪：檔案安全設計（前置條件、隱藏標記、標記互相關聯）───

fn backup_marker_path(stage: &Stage, rel: &str) -> PathBuf {
    apply_record::instance_backup_dir(&stage.mc).join(format!("{rel}.mcpl-backup.json"))
}

fn file_marker_path(stage: &Stage, rel: &str) -> PathBuf {
    stage.mc.join(".mcpl").join("files").join(format!("{rel}.json"))
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn p1_remove_then_pack_update_then_reapply_restores_the_new_original() {
    let stage = Stage::new("p1-a");
    stage.apply(BACKUP);
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    // 整合包更新：原檔換成新版
    fs::write(stage.mc.join("mods/example.jar"), b"original-v2").unwrap();
    stage.apply(BACKUP);
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(
        fs::read(stage.mc.join("mods/example.jar")).unwrap(),
        b"original-v2",
        "用過的舊備份不能再拿來還原，否則整合包的新版本會遺失"
    );
}

#[test]
fn p1_pack_update_before_removal_then_reapply_keeps_player_version() {
    let stage = Stage::new("p1-b");
    fs::create_dir_all(stage.mc.join("config/q")).unwrap();
    fs::create_dir_all(stage.work.join("config/q")).unwrap();
    fs::write(stage.mc.join("config/q/a.txt"), "O").unwrap();
    fs::write(stage.work.join("config/q/a.txt"), "中文").unwrap();
    stage.apply(BACKUP);
    fs::write(stage.mc.join("config/q/a.txt"), "U（玩家自己改的）").unwrap();
    let first = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(first.skipped_modified.contains(&"config/q/a.txt".to_string()));
    stage.apply(BACKUP);
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(
        fs::read_to_string(stage.mc.join("config/q/a.txt")).unwrap(),
        "U（玩家自己改的）",
        "不能蓋回過期的舊原檔"
    );
}

#[test]
fn p2_shared_parent_backups_of_other_pack_are_not_used_or_deleted() {
    // CurseForge：instances/ 底下並排多個整合包，遊戲資料夾本身就有 mods
    let root = std::env::temp_dir().join(format!("apply_b1_p2_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let instances = root.join("instances");
    let a = instances.join("PackA");
    let b = instances.join("PackB");
    let work = root.join("翻譯結果");
    for mc in [&a, &b] {
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::write(mc.join("options.txt"), ORIGINAL_OPTIONS).unwrap();
    }
    fs::write(a.join("mods/shared.jar"), b"A original").unwrap();
    fs::create_dir_all(work.join("jar-translated")).unwrap();
    fs::write(work.join("jar-translated/shared.jar"), b"A translated").unwrap();
    // 共用上層資料夾裡有 B 的舊備份（沒有清單、以及清單記的是 B）
    let orphan = instances.join("翻譯套用備份_20250101_1");
    fs::create_dir_all(orphan.join("mods")).unwrap();
    fs::write(orphan.join("mods/shared.jar"), b"B original").unwrap();
    let of_b = instances.join("翻譯套用備份_20250101_2");
    fs::create_dir_all(of_b.join("mods")).unwrap();
    fs::write(of_b.join("mods/shared.jar"), b"B original 2").unwrap();
    fs::write(
        of_b.join(APPLY_MANIFEST),
        serde_json::json!({ "mc_dir": b.display().to_string(), "added": [], "overwritten": ["mods/shared.jar"] }).to_string(),
    )
    .unwrap();

    apply_to_instance_with_game_state(&a, &work, None, BACKUP, GameRunning::No).unwrap();
    delete_apply_backups_in(&a, None).unwrap();
    assert!(orphan.join("mods/shared.jar").exists(), "無法確認屬於 A 的舊備份不能刪");
    assert!(of_b.join("mods/shared.jar").exists(), "B 的舊備份不能刪");
    restore_last_apply_in(&a, Some(&work)).unwrap();
    assert_ne!(fs::read(a.join("mods/shared.jar")).unwrap(), b"B original", "不能拿別的整合包的備份還原");
    assert_ne!(fs::read(a.join("mods/shared.jar")).unwrap(), b"B original 2");
    for mc in [&a, &b] {
        let _ = fs::remove_dir_all(apply_record::record_dir(mc));
        let _ = fs::remove_dir_all(apply_record::backup_container(mc));
        let _ = fs::remove_dir_all(apply_record::quarantine_dir(mc));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn p3_unknown_file_is_quarantined_before_overwrite() {
    let stage = Stage::new("p3");
    fs::write(stage.mc.join("mods/example.jar"), b"who knows").unwrap();
    stage.translated_from_current_game();
    // 舊版清單提過、但舊備份沒有原檔 → 來源不明
    let legacy = stage.root.join("翻譯套用備份_20250101_9");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(
        legacy.join(APPLY_MANIFEST),
        serde_json::json!({ "mc_dir": stage.mc.display().to_string(), "added": [], "overwritten": ["mods/example.jar"] }).to_string(),
    )
    .unwrap();
    let result = stage.apply(BACKUP);
    assert!(result.quarantined_files.contains(&"mods/example.jar".to_string()), "{:?}", result.quarantined_files);
    let kept = walk_files(&apply_record::quarantine_dir(&stage.mc))
        .into_iter()
        .any(|p| p.file_name().is_some_and(|n| n == "example.jar") && fs::read(&p).unwrap() == b"who knows");
    assert!(kept, "被覆蓋前的內容要留在隔離區");
    assert!(result.player_summary.contains("隔離區"), "{}", result.player_summary);
}

fn walk_files(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect()
}

#[test]
fn p4_font_pack_touches_nothing_when_record_cannot_be_saved() {
    let stage = Stage::new("p4");
    let font = stage.root.join("字體結果").join("resourcepacks").join("繁體中文遊戲字體");
    fs::create_dir_all(font.join("assets/minecraft/font")).unwrap();
    fs::write(font.join("assets/minecraft/font/default.json"), "{}").unwrap();
    // 先讓遊戲資料夾有識別碼，再把紀錄位置堵住
    crate::engine::apply_record::ensure_instance(&stage.mc).unwrap();
    let dir = apply_record::record_dir(&stage.mc);
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    fs::write(&dir, b"blocked").unwrap();
    assert!(crate::engine::font_pack::apply_font_pack_to_instance(&stage.mc, &font).is_err());
    assert!(!stage.mc.join("resourcepacks/繁體中文遊戲字體").exists(), "存不了紀錄就不能動遊戲檔");
    assert_eq!(stage.options(), ORIGINAL_OPTIONS);
    assert!(!stage.mc.join("options.txt.mcpl-bak").exists());
    let _ = fs::remove_file(&dir);
}

#[test]
fn hidden_marker_folder_identifies_the_pack_after_rename() {
    let stage = Stage::new("mcpl-rename");
    stage.apply(BACKUP);
    let marker_dir = stage.mc.join(".mcpl");
    assert!(marker_dir.join("instance.json").is_file());
    assert!(crate::engine::mcpl_marker::is_hidden(&marker_dir));
    assert!(file_marker_path(&stage, "mods/example.jar").is_file(), "工具寫過的檔要有標記");
    let renamed = stage.root.join("minecraft_renamed");
    fs::rename(&stage.mc, &renamed).unwrap();
    restore_last_apply_in(&renamed, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(renamed.join("mods/example.jar")).unwrap(), b"original", "改名後仍認得是同一個整合包");
    fs::rename(&renamed, &stage.mc).unwrap();
}

#[test]
fn links_backup_marker_deleted_but_backup_file_kept_is_repaired_not_used() {
    let stage = Stage::new("link-a");
    stage.apply(BACKUP);
    fs::remove_file(backup_marker_path(&stage, "mods/example.jar")).unwrap();
    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated", "標記有缺時這次不動檔");
    assert!(result.repaired.contains(&"mods/example.jar".to_string()), "{:?}", result.repaired);
    assert!(backup_marker_path(&stage, "mods/example.jar").is_file(), "能從遊戲檔標記補回備份標記");
    // 補好之後再按一次就能還原
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn links_backup_file_deleted_but_marker_kept_is_not_touched() {
    let stage = Stage::new("link-b");
    stage.apply(BACKUP);
    fs::remove_file(apply_record::instance_backup_dir(&stage.mc).join("mods/example.jar")).unwrap();
    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
    assert!(result.uncertain.contains(&"mods/example.jar".to_string()), "{:?}", result.uncertain);
}

#[test]
fn links_game_marker_pointing_to_missing_backup_is_not_touched() {
    let stage = Stage::new("link-c");
    stage.apply(BACKUP);
    let path = file_marker_path(&stage, "mods/example.jar");
    let mut marker = read_json(&path);
    for link in marker["links"].as_array_mut().unwrap() {
        if link["relation"] == "backup" {
            link["id"] = serde_json::json!("不存在的備份");
        }
    }
    fs::write(&path, marker.to_string()).unwrap();
    fs::remove_file(backup_marker_path(&stage, "mods/example.jar")).unwrap();
    fs::remove_file(apply_record::instance_backup_dir(&stage.mc).join("mods/example.jar")).unwrap();
    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
    assert!(result.uncertain.contains(&"mods/example.jar".to_string()), "{:?}", result.uncertain);
}

#[test]
fn links_mismatched_back_references_are_not_touched() {
    let stage = Stage::new("link-d");
    stage.apply(BACKUP);
    let path = backup_marker_path(&stage, "mods/example.jar");
    let mut marker = read_json(&path);
    for link in marker["links"].as_array_mut().unwrap() {
        if link["relation"] == "gameFile" {
            link["id"] = serde_json::json!("別人的標記");
        }
    }
    fs::write(&path, marker.to_string()).unwrap();
    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
    assert!(result.uncertain.contains(&"mods/example.jar".to_string()), "{:?}", result.uncertain);
    assert!(result.player_summary.contains("無法確認"), "{}", result.player_summary);
}

#[test]
fn restore_lists_restored_and_removed_files_in_plain_words() {
    let stage = Stage::new("lists").with_zip();
    stage.apply(BACKUP);
    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(result.restored_files.contains(&"mods/example.jar".to_string()), "{:?}", result.restored_files);
    assert!(result.removed_files.contains(&"resourcepacks/繁體中文翻譯.zip".to_string()), "{:?}", result.removed_files);
    assert!(result.player_summary.contains("已還原"), "{}", result.player_summary);
    assert!(result.player_summary.contains("已刪除"), "{}", result.player_summary);
}
