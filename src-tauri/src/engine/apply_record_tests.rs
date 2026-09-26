//! apply_record.rs 的測試。

use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mcpl-record-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("minecraft")).unwrap();
    dir
}

#[test]
fn record_lives_outside_game_and_backup_folders_and_round_trips() {
    let root = scratch("roundtrip");
    let mc = root.join("minecraft");
    let mut record = ApplyRecord::default();
    record.note_tool_file("mods/a.jar", true, "abc".into());
    save(&mc, &mut record).unwrap();
    let dir = record_dir(&mc);
    assert!(dir.join(RECORD_FILE).is_file());
    assert!(!dir.starts_with(&mc), "紀錄不放在遊戲資料夾裡");
    assert!(!dir.starts_with(backup_container(&mc)), "紀錄與備份分開");
    let loaded = load(&mc).unwrap();
    assert_eq!(loaded.files["mods/a.jar"].kind, FileKind::Overwritten);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn key_ignores_case_and_separators() {
    let root = scratch("key");
    let mc = root.join("minecraft");
    let upper = PathBuf::from(mc.to_string_lossy().to_uppercase());
    if cfg!(windows) {
        assert_eq!(instance_key(&mc), instance_key(&upper));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn tool_versions_are_remembered_across_reapply() {
    let mut record = ApplyRecord::default();
    record.set_entry("config/a.txt", FileKind::Added, Origin::Known, None, "v1".into());
    record.set_entry("config/a.txt", FileKind::Added, Origin::Known, None, "v2".into());
    assert_eq!(record.files["config/a.txt"].sha256, "v2");
    assert!(record.is_tool_version("config/a.txt", Some("v1")), "舊版本也認得是工具寫的");
    assert!(record.is_tool_version("config/a.txt", Some("v2")));
    assert!(!record.is_tool_version("config/a.txt", Some("整合包的")));
}

#[test]
fn not_keeping_results_does_not_mean_no_backup() {
    assert_eq!(policy_for_run_with(BackupChoice::Unset, false, false), BackupPolicy::Ask);
    assert_eq!(policy_for_run_with(BackupChoice::Always, false, false), BackupPolicy::Backup);
    assert_eq!(
        policy_for_run_with(BackupChoice::Never, false, false),
        BackupPolicy::NoBackup { overwrite_confirmed: false }
    );
}

#[test]
fn broken_record_is_explained_with_path_and_can_be_reset() {
    let root = scratch("broken");
    let mc = root.join("minecraft");
    let dir = record_dir(&mc);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(RECORD_FILE);
    fs::write(&path, "{壞掉").unwrap();
    let err = load(&mc).unwrap_err();
    assert!(err.contains(&path.display().to_string()), "要附完整路徑：{err}");
    assert!(err.contains("重設套用紀錄"), "要告訴玩家怎麼處理：{err}");
    assert!(is_broken_record_error(&err));
    let kept = reset_record(&mc).unwrap();
    assert!(kept.file_name().unwrap().to_string_lossy().contains(".broken-"), "{}", kept.display());
    assert_eq!(fs::read_to_string(&kept).unwrap(), "{壞掉", "損壞檔要保留，不能刪");
    assert!(load(&mc).unwrap().is_empty());
    assert!(was_reset(&mc));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn policy_follows_saved_choice() {
    assert_eq!(policy_from_choice(BackupChoice::Unset, false), BackupPolicy::Ask);
    assert_eq!(policy_from_choice(BackupChoice::Always, false), BackupPolicy::Backup);
    assert_eq!(
        policy_from_choice(BackupChoice::Never, false),
        BackupPolicy::NoBackup { overwrite_confirmed: false }
    );
}

#[test]
fn atomic_copy_leaves_no_temp_file() {
    let root = scratch("atomic");
    let src = root.join("a.zip");
    fs::write(&src, b"zip").unwrap();
    let dest = root.join("minecraft/b.zip");
    fs::write(&dest, b"old").unwrap();
    copy_atomic(&src, &dest).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"zip");
    assert!(!root.join("minecraft/b.zip.mcpl-tmp").exists());
    let _ = fs::remove_dir_all(root);
}
