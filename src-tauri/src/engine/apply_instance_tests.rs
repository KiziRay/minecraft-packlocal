//! 套用到遊戲／移除翻譯的整合測試（用暫存資料夾模擬遊戲資料夾）。

use super::*;

pub(super) const BACKUP: BackupPolicy = BackupPolicy::Backup;
pub(super) const NO_BACKUP_CONFIRMED: BackupPolicy = BackupPolicy::NoBackup { overwrite_confirmed: true };
pub(super) const ORIGINAL_OPTIONS: &str = "version:3465\nlang:en_us\nresourcePacks:[\"vanilla\",\"file/OtherZh.zip\"]\nguiScale:3\n";

pub(super) struct Stage {
    pub(super) root: PathBuf,
    pub(super) mc: PathBuf,
    pub(super) work: PathBuf,
}

impl Stage {
    /// 一個玩過的遊戲資料夾（有 options.txt、有其他中文資源包）＋一份翻譯結果（翻譯 JAR）。
    pub(super) fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("apply_b1_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(long_path(&root));
        let mc = root.join("minecraft");
        let work = root.join("翻譯結果");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(mc.join("resourcepacks")).unwrap();
        fs::create_dir_all(work.join("jar-translated")).unwrap();
        fs::write(mc.join("mods/example.jar"), b"original").unwrap();
        fs::write(mc.join("options.txt"), ORIGINAL_OPTIONS).unwrap();
        fs::write(mc.join("resourcepacks/OtherZh.zip"), b"other").unwrap();
        fs::write(work.join("jar-translated/example.jar"), b"translated").unwrap();
        crate::engine::jar_sources::record_source(&work, Path::new("example.jar"), &mc.join("mods/example.jar")).unwrap();
        let stage = Self { root, mc, work };
        stage.cleanup_store();
        stage
    }

    /// 翻譯時遊戲裡的模組檔就是現在這一個（翻譯會記下它的指紋）。
    pub(super) fn translated_from_current_game(&self) {
        crate::engine::jar_sources::record_source(&self.work, Path::new("example.jar"), &self.mc.join("mods/example.jar"))
            .unwrap();
    }

    pub(super) fn with_zip(self) -> Self {
        fs::create_dir_all(self.work.join("resourcepacks")).unwrap();
        fs::write(self.work.join("resourcepacks/繁體中文翻譯.zip"), b"zip-v1").unwrap();
        self
    }

    pub(super) fn apply(&self, policy: BackupPolicy) -> ApplyResult {
        apply_to_instance_with_game_state(&self.mc, &self.work, Some("繁體中文翻譯"), policy, GameRunning::No)
            .unwrap()
    }

    pub(super) fn options(&self) -> String {
        fs::read_to_string(self.mc.join("options.txt")).unwrap()
    }

    fn cleanup_store(&self) {
        let _ = fs::remove_dir_all(apply_record::record_dir(&self.mc));
        let _ = fs::remove_dir_all(apply_record::backup_container(&self.mc));
        let _ = fs::remove_dir_all(apply_record::quarantine_dir(&self.mc));
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        self.cleanup_store();
        let _ = fs::remove_dir_all(long_path(&self.root));
    }
}

pub(super) fn backup_dirs_in(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("翻譯套用備份_"))
                .count()
        })
        .unwrap_or(0)
}

// ─── 既有保證（改用新結構後仍成立）───

#[test]
fn backup_failure_stops_the_apply_instead_of_overwriting_unprotected() {
    // 備份失敗必須讓整個套用停下來：舊版曾「沒有備份卻照樣覆蓋」並回報成功。
    let stage = Stage::new("bakfail");
    let container = apply_record::backup_container(&stage.mc);
    fs::create_dir_all(&container).unwrap();
    // 備份資料夾的位置被一個同名檔案佔住，建立／複製一定失敗
    fs::write(apply_record::instance_backup_dir(&stage.mc), b"I am a file").unwrap();
    let result = apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No);
    assert!(result.is_err(), "備份失敗時必須回傳錯誤，不能靜默繼續");
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn game_running_returns_translated_but_not_applied_with_zero_writes() {
    // 遊戲開著：不是翻譯失敗，而是「已翻完、未套用」，而且一個檔案都不能動。
    let stage = Stage::new("running");
    let result = apply_to_instance_with_game_state(
        &stage.mc,
        &stage.work,
        None,
        BACKUP,
        GameRunning::Yes { detail: "偵測到這個整合包的遊戲行程正在執行。".into() },
    )
    .unwrap();
    assert_eq!(result.status, ApplyStatus::GameRunning);
    assert!(!result.is_applied());
    assert!(result.player_summary.contains("翻譯已完成"), "{}", result.player_summary);
    assert!(result.player_summary.contains("套用到遊戲"), "要告訴玩家下一步：{}", result.player_summary);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    assert_eq!(stage.options(), ORIGINAL_OPTIONS);
    assert_eq!(backup_dirs_in(&stage.work), 0);
    assert!(!apply_record::instance_backup_dir(&stage.mc).exists(), "被擋下時不得留下半成品備份");
    assert!(apply_record::load(&stage.mc).unwrap().is_empty(), "被擋下時不得寫套用紀錄");
}

#[test]
fn unknown_game_state_proceeds_but_says_so() {
    let stage = Stage::new("unknown");
    let result =
        apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::Unknown).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated", "Unknown 應放行");
    assert!(result.warnings.iter().any(|w| w.contains("無法確認")), "{:?}", result.warnings);
}

#[test]
fn confirmed_closed_game_has_no_unconfirmed_warning() {
    let stage = Stage::new("closed");
    let result = stage.apply(BACKUP);
    assert!(!result.warnings.iter().any(|w| w.contains("無法確認")));
}

#[test]
fn only_yes_blocks_apply() {
    assert!(GameRunning::Yes { detail: String::new() }.blocks_apply());
    assert!(!GameRunning::No.blocks_apply());
    assert!(!GameRunning::Unknown.blocks_apply());
}

#[test]
fn applies_translated_jars_after_backing_up_originals() {
    let stage = Stage::new("jars");
    let result = stage.apply(BACKUP);
    assert_eq!(result.status, ApplyStatus::Applied);
    assert_eq!(result.jars_copied, 1);
    assert!(result.backup_created);
    assert!(!result.backup_reused);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
    let backup = PathBuf::from(&result.backup_dir);
    assert_eq!(backup, apply_record::instance_backup_dir(&stage.mc), "備份以遊戲資料夾為單位");
    assert!(!backup.starts_with(&stage.mc), "備份不放在遊戲資料夾裡");
    assert_eq!(fs::read(backup.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn reuses_matching_backup_on_repeated_apply() {
    let stage = Stage::new("reuse");
    let first = stage.apply(BACKUP);
    fs::write(stage.work.join("jar-translated/example.jar"), b"translated-v2").unwrap();
    let second = stage.apply(BACKUP);
    assert!(!second.backup_created);
    assert!(second.backup_reused);
    assert_eq!(second.backup_dir, first.backup_dir);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated-v2");
    assert_eq!(backup_dirs_in(&apply_record::backup_container(&stage.mc)), 1);
    assert_eq!(backup_dirs_in(&stage.work), 0, "不再每次在結果資料夾另建備份");
    // 重跑兩次後，備份裡仍是英文原版，不是上一次的翻譯版
    assert_eq!(
        fs::read(PathBuf::from(&second.backup_dir).join("mods/example.jar")).unwrap(),
        b"original"
    );
}

#[test]
fn skips_backup_when_player_disables_it() {
    let stage = Stage::new("nobackup");
    let result = stage.apply(NO_BACKUP_CONFIRMED);
    assert!(!result.backup_created);
    assert!(result.backup_dir.is_empty());
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
    assert!(!apply_record::instance_backup_dir(&stage.mc).exists());
    assert_eq!(backup_dirs_in(&stage.root), 0);
}

#[test]
fn deletes_only_tool_backup_directories() {
    let root = std::env::temp_dir().join(format!("delete_backups_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let mc = root.join("minecraft");
    fs::create_dir_all(mc.join("mods")).unwrap();
    fs::create_dir_all(root.join("翻譯套用備份_20260811_1")).unwrap();
    fs::create_dir_all(root.join("翻譯套用備份_20260811_2")).unwrap();
    fs::create_dir_all(root.join("player-backup")).unwrap();

    // 沒有清單的舊備份無法確認屬於這個整合包（CurseForge 會把多個整合包放在同一個上層資料夾）：
    // B1 第四輪起不刪，列在「沒有刪」裡；玩家自己的資料夾一律不碰
    let result = delete_apply_backups_in(&mc, None).unwrap();
    assert_eq!(result.deleted, 0);
    assert_eq!(result.kept.len(), 2, "{:?}", result.kept);
    assert!(root.join("player-backup").is_dir());
    assert!(root.join("翻譯套用備份_20260811_1").exists());
    assert!(result.player_summary.contains("沒有刪"), "{}", result.player_summary);
    let _ = fs::remove_dir_all(apply_record::record_dir(&mc));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn applies_and_detects_config_text_overlays() {
    let stage = Stage::new("config");
    fs::create_dir_all(stage.mc.join("config/unknown_display_mod")).unwrap();
    fs::create_dir_all(stage.work.join("config/unknown_display_mod")).unwrap();
    fs::write(stage.mc.join("config/unknown_display_mod/start.txt"), "原文").unwrap();
    fs::write(stage.work.join("config/unknown_display_mod/start.txt"), "繁中").unwrap();
    let result = stage.apply(BACKUP);
    assert!(result.backup_created);
    assert_eq!(
        fs::read_to_string(stage.mc.join("config/unknown_display_mod/start.txt")).unwrap(),
        "繁中"
    );
    assert!(has_apply_backups_in(&stage.mc, Some(&stage.work)).unwrap());
}

#[test]
fn restore_rejects_legacy_manifest_for_different_mc_dir() {
    let stage = Stage::new("mismatch");
    let other = stage.root.join("other_minecraft");
    fs::create_dir_all(other.join("mods")).unwrap();
    // 1.1.0 以前的備份：放在結果資料夾，清單記的是另一個遊戲資料夾
    let legacy = stage.work.join("翻譯套用備份_1700000000");
    fs::create_dir_all(&legacy).unwrap();
    let manifest = serde_json::json!({
        "stamp": "1700000000",
        "mc_dir": stage.mc.display().to_string(),
        "backup_dir": legacy.display().to_string(),
        "added": [],
        "overwritten": []
    });
    fs::write(legacy.join(APPLY_MANIFEST), manifest.to_string()).unwrap();
    fs::write(other.join("mods/example.jar"), b"other pack's file").unwrap();
    // 別的遊戲資料夾的舊版清單一律不採用：什麼都不動
    let err = restore_last_apply_in(&other, Some(&stage.work)).unwrap_err();
    assert!(err.contains("沒有"), "{err}");
    assert!(legacy.is_dir());
    assert_eq!(fs::read(other.join("mods/example.jar")).unwrap(), b"other pack's file");
}

#[test]
fn does_not_copy_work_data_into_minecraft_data() {
    // JAR 書本的除錯副本放在翻譯結果的 data/，遊戲資料夾沒有對應來源，不能搬進去
    let stage = Stage::new("nodata");
    fs::create_dir_all(stage.work.join("data/example")).unwrap();
    fs::write(stage.work.join("data/example/book.json"), b"{}").unwrap();
    stage.apply(NO_BACKUP_CONFIRMED);
    assert!(!stage.mc.join("data/example/book.json").exists());
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
}

#[test]
fn enables_pack_and_keeps_existing_resource_packs() {
    let stage = Stage::new("enable").with_zip();
    stage.apply(BACKUP);
    let options = stage.options();
    assert!(options.contains("\"vanilla\""));
    assert!(options.contains("\"file/OtherZh.zip\""));
    assert!(options.contains("\"file/繁體中文翻譯.zip\""));
    assert!(stage.mc.join("options.txt.mcpl-bak").is_file(), "動 options.txt 前另存一份");
    let record = apply_record::load(&stage.mc).unwrap();
    assert_eq!(record.options.packs_added, vec!["file/繁體中文翻譯.zip".to_string()]);
}

// ─── B1 新行為 ───

#[test]
fn apply_sets_zh_tw_and_remembers_original_language() {
    let stage = Stage::new("lang").with_zip();
    let result = stage.apply(BACKUP);
    assert!(result.lang_set);
    assert_eq!(result.original_lang.as_deref(), Some("en_us"));
    assert_eq!(options_txt::read_lang(&stage.options()).as_deref(), Some("zh_tw"));
    let record = apply_record::load(&stage.mc).unwrap();
    assert!(record.options.lang_changed);
    assert_eq!(record.options.original_lang.as_deref(), Some("en_us"));
}

#[test]
fn missing_options_txt_asks_to_launch_game_first_and_writes_nothing() {
    let stage = Stage::new("fresh").with_zip();
    fs::remove_file(stage.mc.join("options.txt")).unwrap();
    let result = stage.apply(BACKUP);
    assert_eq!(result.status, ApplyStatus::NoOptionsTxt);
    assert!(result.player_summary.contains("請先用啟動器開一次遊戲"), "{}", result.player_summary);
    assert!(!stage.mc.join("options.txt").exists(), "不能自己建 options.txt");
    assert!(!stage.mc.join("resourcepacks/繁體中文翻譯.zip").exists());
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn rerun_keeps_tool_pack_last_without_touching_other_chinese_pack() {
    let stage = Stage::new("rerun").with_zip();
    stage.apply(BACKUP);
    // 玩家之後把別的包拖到我們上面
    let moved = stage
        .options()
        .replace("[\"vanilla\",\"file/OtherZh.zip\",\"file/繁體中文翻譯.zip\"]", "[\"vanilla\",\"file/繁體中文翻譯.zip\",\"file/OtherZh.zip\"]");
    fs::write(stage.mc.join("options.txt"), &moved).unwrap();
    fs::write(stage.work.join("resourcepacks/繁體中文翻譯.zip"), b"zip-v2").unwrap();
    stage.apply(BACKUP);
    let packs = super::super::resource_pack_guard::parse_pack_list(&stage.options());
    assert_eq!(packs.last().map(String::as_str), Some("file/繁體中文翻譯.zip"), "{packs:?}");
    assert!(packs.contains(&"file/OtherZh.zip".to_string()));
    assert_eq!(packs.iter().filter(|p| p.contains("繁體中文翻譯")).count(), 1);
    assert!(stage.options().contains("guiScale:3"));
}

#[test]
fn record_is_written_even_without_backup_and_outside_game_folder() {
    let stage = Stage::new("record").with_zip();
    stage.apply(NO_BACKUP_CONFIRMED);
    let dir = apply_record::record_dir(&stage.mc);
    assert!(dir.join(apply_record::RECORD_FILE).is_file());
    assert!(!dir.starts_with(&stage.mc));
    assert!(!dir.starts_with(apply_record::backup_container(&stage.mc)));
    let record = apply_record::load(&stage.mc).unwrap();
    assert_eq!(record.files["mods/example.jar"].kind, apply_record::FileKind::Overwritten);
    assert_eq!(record.files["resourcepacks/繁體中文翻譯.zip"].kind, apply_record::FileKind::Added);
    assert!(record.files.contains_key("resourcepacks/繁體中文翻譯.meta.json"), "指紋標記也是工具加的");
}

#[test]
fn remove_translation_after_two_runs_returns_english_original_and_settings() {
    let stage = Stage::new("restore").with_zip();
    stage.apply(BACKUP);
    fs::write(stage.work.join("jar-translated/example.jar"), b"translated-v2").unwrap();
    stage.apply(BACKUP);
    // 玩家玩了一陣子，自己改了畫面大小
    let played = stage.options().replace("guiScale:3", "guiScale:2");
    fs::write(stage.mc.join("options.txt"), played).unwrap();

    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    assert!(!stage.mc.join("resourcepacks/繁體中文翻譯.zip").exists());
    assert!(!stage.mc.join("resourcepacks/繁體中文翻譯.meta.json").exists());
    assert!(!stage.mc.join("options.txt.mcpl-bak").exists());
    let options = stage.options();
    assert_eq!(options_txt::read_lang(&options).as_deref(), Some("en_us"));
    assert!(!options.contains("繁體中文翻譯"));
    assert!(options.contains("file/OtherZh.zip"), "玩家原本的中文資源包要留著");
    assert!(options.contains("guiScale:2"), "不整檔覆蓋：玩家後來的設定要留著");
    assert!(result.player_summary.contains("已移除翻譯"));
    assert!(apply_record::load(&stage.mc).unwrap().is_empty(), "全部還原後紀錄清空");
}

#[test]
fn remove_translation_works_without_backup_and_reports_what_cannot_be_restored() {
    let stage = Stage::new("restore-nobak").with_zip();
    stage.apply(NO_BACKUP_CONFIRMED);
    assert!(has_apply_backups_in(&stage.mc, Some(&stage.work)).unwrap(), "沒有備份也要能按移除翻譯");
    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(!stage.mc.join("resourcepacks/繁體中文翻譯.zip").exists(), "工具新增的一律刪除");
    assert_eq!(options_txt::read_lang(&stage.options()).as_deref(), Some("en_us"));
    assert_eq!(result.unrestorable, vec!["mods/example.jar".to_string()]);
    assert!(result.player_summary.contains("無法還原"), "{}", result.player_summary);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
}

#[test]
fn remove_translation_leaves_files_changed_after_apply() {
    let stage = Stage::new("restore-changed");
    fs::create_dir_all(stage.mc.join("config/q")).unwrap();
    fs::create_dir_all(stage.work.join("config/q")).unwrap();
    fs::write(stage.mc.join("config/q/a.snbt"), "english").unwrap();
    fs::write(stage.work.join("config/q/a.snbt"), "中文").unwrap();
    stage.apply(BACKUP);
    // 整合包更新換掉了這個檔
    fs::write(stage.mc.join("config/q/a.snbt"), "english v2 from pack update").unwrap();
    let result = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(
        fs::read_to_string(stage.mc.join("config/q/a.snbt")).unwrap(),
        "english v2 from pack update"
    );
    assert_eq!(result.skipped_modified, vec!["config/q/a.snbt".to_string()]);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn unset_backup_choice_asks_before_writing_anything() {
    let stage = Stage::new("ask").with_zip();
    let result = stage.apply(BackupPolicy::Ask);
    assert_eq!(result.status, ApplyStatus::NeedsBackupChoice);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    assert_eq!(stage.options(), ORIGINAL_OPTIONS);
}

#[test]
fn no_backup_mode_confirms_before_overwriting_originals() {
    let stage = Stage::new("confirm").with_zip();
    let result = stage.apply(BackupPolicy::NoBackup { overwrite_confirmed: false });
    assert_eq!(result.status, ApplyStatus::NeedsOverwriteConfirm);
    assert_eq!(result.pending_overwrites, vec!["mods/example.jar".to_string()]);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    // 確認後才覆蓋
    let result = stage.apply(BackupPolicy::NoBackup { overwrite_confirmed: true });
    assert_eq!(result.status, ApplyStatus::Applied);
    // 之後只剩工具自己的檔案會被蓋，不用再問
    let again = stage.apply(BackupPolicy::NoBackup { overwrite_confirmed: false });
    assert_eq!(again.status, ApplyStatus::Applied);
}

#[test]
fn legacy_backup_beside_game_folder_still_restores_originals() {
    // 1.0.x：備份在遊戲資料夾旁，沒有新的套用紀錄。遊戲裡的內容等於翻譯結果裡的工具產出
    // （舊版清單沒有雜湊，原則 B：比對得上才蓋回；比對不上見 b_legacy_restore_leaves_changed_file_and_lists_it）
    let stage = Stage::new("legacy");
    fs::write(stage.mc.join("mods/example.jar"), b"translated").unwrap();
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
    assert!(has_apply_backups_in(&stage.mc, None).unwrap());
    restore_last_apply_in(&stage.mc, None).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn long_paths_over_260_characters_apply_and_restore() {
    let mut deep = std::env::temp_dir().join(format!("apply_b1_long_{}", std::process::id()));
    let root = deep.clone();
    let _ = fs::remove_dir_all(long_path(&root));
    while deep.to_string_lossy().len() < 200 {
        deep = deep.join("a_really_long_modpack_folder_name");
    }
    let mc = deep.join("minecraft");
    let work = deep.join("翻譯結果");
    let nested = "config/some_mod_with_a_long_name/nested_folder_for_quests/chapter_folder";
    fs::create_dir_all(long_path(&mc.join(nested))).unwrap();
    fs::create_dir_all(long_path(&work.join(nested))).unwrap();
    let target = mc.join(nested).join("the_final_quest_file_name.snbt");
    assert!(target.to_string_lossy().len() > 260, "測試路徑要真的超過 260 字元");
    fs::write(long_path(&target), "english").unwrap();
    fs::write(long_path(&work.join(nested).join("the_final_quest_file_name.snbt")), "中文").unwrap();
    fs::create_dir_all(long_path(&mc.join("mods"))).unwrap();
    fs::write(long_path(&mc.join("options.txt")), ORIGINAL_OPTIONS).unwrap();

    let result = apply_to_instance_with_game_state(&mc, &work, None, BACKUP, GameRunning::No).unwrap();
    assert_eq!(result.status, ApplyStatus::Applied);
    assert_eq!(fs::read_to_string(long_path(&target)).unwrap(), "中文");
    restore_last_apply_in(&mc, Some(&work)).unwrap();
    assert_eq!(fs::read_to_string(long_path(&target)).unwrap(), "english");
    let _ = fs::remove_dir_all(apply_record::record_dir(&mc));
    let _ = fs::remove_dir_all(apply_record::backup_container(&mc));
    let _ = fs::remove_dir_all(long_path(&root));
}

#[test]
fn old_tool_pack_already_in_game_is_quarantined_and_listed_on_removal() {
    // 1.0.x 已經把「模組包翻譯工具+…」放進遊戲並啟用；升級後第一次套用沒有新紀錄
    let stage = Stage::new("oldpack");
    let name = "模組包翻譯工具+0901+1.0.zip";
    fs::create_dir_all(stage.work.join("resourcepacks")).unwrap();
    fs::write(stage.work.join("resourcepacks").join(name), b"new").unwrap();
    fs::write(stage.mc.join("resourcepacks").join(name), b"old-translation").unwrap();
    fs::write(
        stage.mc.join("options.txt"),
        format!("lang:en_us\nresourcePacks:[\"vanilla\",\"file/{name}\"]\n"),
    )
    .unwrap();
    let result = apply_to_instance_with_game_state(
        &stage.mc,
        &stage.work,
        Some(name),
        BackupPolicy::NoBackup { overwrite_confirmed: false },
        GameRunning::No,
    )
    .unwrap();
    // 遊戲裡有舊版工具的翻譯資源包、卻沒有任何紀錄：無法確定模組檔是不是以前翻過的，
    // 覆蓋前一律先移入隔離區（不需要不備份確認，因為原內容已保存）
    assert_eq!(result.status, ApplyStatus::Applied);
    assert!(result.quarantined_files.contains(&"mods/example.jar".to_string()), "{:?}", result.quarantined_files);
    assert!(result.quarantined_files.contains(&format!("resourcepacks/{name}")), "舊翻譯包覆蓋前也先保存");
    let restored = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    // 第五輪：沒有工具標記的舊翻譯包是來源不明——不直接刪，列出隔離區裡原本那份的位置
    assert!(restored.quarantined.iter().any(|q| q.contains("模組包翻譯工具")), "{:?}", restored.quarantined);
    assert!(!stage.options().contains(name), "資源包清單裡工具加的項目要拿掉");
}
