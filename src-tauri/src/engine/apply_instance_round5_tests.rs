//! 第五輪：所有寫入遊戲資料夾的路徑都走同一套前置檢查、紀錄、標記。

use super::*;
use super::tests::{Stage, BACKUP, ORIGINAL_OPTIONS};

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

fn sha(bytes: &[u8]) -> String {
    crate::engine::hashutil::sha256_hex(bytes)
}

// ─── 12. G1.6 列出的測試要真的存在 ───

#[test]
fn backup_failure_is_reported_not_swallowed() {
    let stage = Stage::new("r5-bakfail");
    let ctx = apply_guard::Ctx::begin(&stage.mc).unwrap();
    // 備份區的位置被一個同名檔案佔住，建立備份一定失敗
    let dir = apply_record::instance_backup_dir(&stage.mc);
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    fs::write(&dir, b"file, not dir").unwrap();
    let err = apply_guard::require_original_backup(&ctx, &stage.mc.join("mods/example.jar"), "mods/example.jar", "g")
        .unwrap_err();
    assert!(err.contains("已停止套用"), "{err}");
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    let _ = fs::remove_file(&dir);
}

// ─── 6. 旁邊的舊備份清單寫明屬於別的整合包：不影響本包 ───

#[test]
fn r6_other_packs_manifest_beside_does_not_block_this_pack() {
    let stage = Stage::new("r6-other-manifest");
    let legacy = stage.root.join("翻譯套用備份_20250101_3");
    fs::create_dir_all(legacy.join("mods")).unwrap();
    fs::write(legacy.join("mods/example.jar"), b"B original").unwrap();
    fs::write(
        legacy.join(APPLY_MANIFEST),
        serde_json::json!({ "mc_dir": stage.root.join("PackB").display().to_string(), "added": [], "overwritten": ["mods/example.jar"] })
            .to_string(),
    )
    .unwrap();
    let result = stage.apply(BACKUP);
    assert!(result.quarantined_files.is_empty(), "別包的清單不能讓本包變成無法確認：{:?}", result.quarantined_files);
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original", "本包要能完整回到英文原版");
    assert!(legacy.join("mods/example.jar").exists(), "別包的備份不能動");
}

// ─── 1. 套用中途中斷 ───

#[test]
fn r1_interrupted_reapply_still_restores_original() {
    // 第一次完整套用（遊戲裡是 T1），第二次在「紀錄與標記已寫、檔案還沒寫」時中斷
    let stage = Stage::new("r1-interrupted");
    stage.apply(BACKUP);
    let t2 = sha(b"translated-v2");
    let mut record = read_json(&record_path(&stage));
    let entry = &mut record["files"]["mods/example.jar"];
    entry["history"] = serde_json::json!([entry["sha256"].clone()]);
    entry["sha256"] = serde_json::json!(t2);
    write_json(&record_path(&stage), &record);
    let marker_path = file_marker_path(&stage, "mods/example.jar");
    let mut marker = read_json(&marker_path);
    marker["toolHistory"] = serde_json::json!([marker["toolSha256"].clone()]);
    marker["toolSha256"] = serde_json::json!(t2);
    marker["state"] = serde_json::json!("pending");
    write_json(&marker_path, &marker);

    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(
        fs::read(stage.mc.join("mods/example.jar")).unwrap(),
        b"original",
        "內容等於任一版工具寫入都算工具檔，要能還原"
    );
}

#[test]
fn r1_failed_write_marks_pending_and_rerun_keeps_valid_backup() {
    let stage = Stage::new("r1-failed-write");
    fs::create_dir_all(stage.work.join("config/zz")).unwrap();
    fs::write(stage.work.join("config/zz/b.txt"), "中文").unwrap();
    // 遊戲裡同位置是一個資料夾：寫這個檔一定失敗（在寫模組檔之前）
    fs::create_dir_all(stage.mc.join("config/zz/b.txt/blocked")).unwrap();
    assert!(apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No).is_err());
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    assert_eq!(read_json(&file_marker_path(&stage, "mods/example.jar"))["state"], "pending", "還沒寫的檔要標待確認");

    fs::remove_dir_all(stage.mc.join("config/zz/b.txt")).unwrap();
    let result = stage.apply(BACKUP);
    assert_eq!(result.status, ApplyStatus::Applied);
    assert_eq!(read_json(&file_marker_path(&stage, "mods/example.jar"))["state"], "written");
    let quarantined_original = walkdir::WalkDir::new(apply_record::quarantine_dir(&stage.mc))
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_file() && fs::read(e.path()).map(|b| b == b"original").unwrap_or(false));
    assert!(!quarantined_original, "有效的原檔備份絕不能被移入隔離區");
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

// ─── 5. 其他沒有前置檢查的寫入 ───

#[test]
fn r5_font_pack_refuses_while_game_running() {
    let stage = Stage::new("r5-font-running");
    let font = stage.root.join("字體結果").join("resourcepacks").join("繁體中文遊戲字體");
    fs::create_dir_all(font.join("assets/minecraft/font")).unwrap();
    fs::write(font.join("assets/minecraft/font/default.json"), "{}").unwrap();
    let old = stage.mc.join("resourcepacks/繁體中文遊戲字體");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("player.txt"), "玩家的").unwrap();
    let err = crate::engine::font_pack::apply_font_pack_with_game_state(
        &stage.mc,
        &font,
        GameRunning::Yes { detail: "遊戲開著".into() },
    )
    .unwrap_err();
    assert!(err.contains("關閉"), "{err}");
    assert_eq!(fs::read_to_string(old.join("player.txt")).unwrap(), "玩家的", "遊戲開著時不能刪資料夾");
    assert_eq!(stage.options(), ORIGINAL_OPTIONS);
}

#[test]
fn r5_tool_file_with_unrebuildable_marker_is_not_overwritten() {
    let stage = Stage::new("r5-broken-marker");
    stage.apply(BACKUP);
    // 遊戲檔標記不見，而且紀錄指向的備份 id 對不上：無法從另一端重建
    fs::remove_file(file_marker_path(&stage, "mods/example.jar")).unwrap();
    let mut record = read_json(&record_path(&stage));
    record["files"]["mods/example.jar"]["backupId"] = serde_json::json!("對不上的備份");
    write_json(&record_path(&stage), &record);
    fs::write(stage.work.join("jar-translated/example.jar"), b"translated-v2").unwrap();
    let result = stage.apply(BACKUP);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated", "無法確認就不覆蓋");
    assert!(result.unconfirmed_files.contains(&"mods/example.jar".to_string()), "{:?}", result.unconfirmed_files);
}

// ─── 2. 快捷選單：翻譯只輸出到結果資料夾，舊 .bak 是 1.0.x 痕跡 ───

#[test]
fn r2_old_minemenu_bak_counts_as_tool_trace() {
    let stage = Stage::new("r2-minemenu-bak");
    fs::create_dir_all(stage.mc.join("minemenu")).unwrap();
    fs::write(stage.mc.join("minemenu/menu.json"), "{\"title\":\"old-zh\"}").unwrap();
    fs::write(stage.mc.join("minemenu/menu.json.bak"), "{\"title\":\"Jade\"}").unwrap();
    fs::create_dir_all(stage.work.join("minemenu")).unwrap();
    fs::write(stage.work.join("minemenu/menu.json"), "{\"title\":\"new-zh\"}").unwrap();
    let result = stage.apply(BACKUP);
    assert!(
        result.quarantined_files.contains(&"minemenu/menu.json".to_string()),
        "舊版改過的選單不能當原檔備份：{:?}",
        result.quarantined_files
    );
}

// ─── 4. 修復資源包清單也走前置檢查、紀錄、標記 ───

fn stage_with_lost_pack(tag: &str) -> Stage {
    let stage = Stage::new(tag);
    fs::write(stage.mc.join("resourcepacks/Lost.zip"), b"lost").unwrap();
    let old = stage.mc.join("翻譯套用備份_20250101_000000");
    fs::create_dir_all(&old).unwrap();
    fs::write(
        old.join("options.txt"),
        "resourcePacks:[\"vanilla\",\"file/OtherZh.zip\",\"file/Lost.zip\"]\n",
    )
    .unwrap();
    stage
}

#[test]
fn r4_repair_pack_list_refuses_while_game_running() {
    let stage = stage_with_lost_pack("r4-repair-running");
    let err = crate::engine::pack_repair::repair_pack_list_with_game_state(
        &stage.mc,
        GameRunning::Yes { detail: "遊戲開著".into() },
    )
    .unwrap_err();
    assert!(err.contains("關閉"), "{err}");
    assert_eq!(stage.options(), ORIGINAL_OPTIONS);
}

#[test]
fn r4_repair_pack_list_keeps_first_backup_and_records_batch() {
    let stage = stage_with_lost_pack("r4-repair-record");
    fs::write(stage.mc.join("options.txt.mcpl-bak"), "FIRST").unwrap();
    let added = crate::engine::pack_repair::repair_pack_list_with_game_state(&stage.mc, GameRunning::No).unwrap();
    assert_eq!(added, 1);
    assert!(stage.options().contains("file/Lost.zip"));
    assert_eq!(
        fs::read_to_string(stage.mc.join("options.txt.mcpl-bak")).unwrap(),
        "FIRST",
        "設定檔旁的另存檔只能建立一次，不能被修復蓋掉"
    );
    let record = read_json(&record_path(&stage));
    assert!(
        record["batches"].as_array().is_some_and(|b| !b.is_empty()),
        "修復前要先有紀錄：{record}"
    );
}

// ─── 10. 看起來像工具產物、但沒有工具標記：當來源不明，先隔離 ───

#[test]
fn r10_unmarked_tool_looking_pack_is_quarantined_and_listed_on_removal() {
    // 遊戲裡有一個名字像工具產物的資源包，但沒有工具標記、也沒有舊版清單佐證
    let stage = Stage::new("r10-unmarked");
    let name = "模組包翻譯工具+0901+1.0.zip";
    let rel = format!("resourcepacks/{name}");
    fs::create_dir_all(stage.work.join("resourcepacks")).unwrap();
    fs::write(stage.work.join("resourcepacks").join(name), b"new").unwrap();
    fs::write(stage.mc.join("resourcepacks").join(name), b"player version").unwrap();
    let result = apply_to_instance_with_game_state(&stage.mc, &stage.work, Some(name), BACKUP, GameRunning::No).unwrap();
    assert!(result.quarantined_files.contains(&rel), "{:?}", result.quarantined_files);
    let record = read_json(&record_path(&stage));
    assert_eq!(record["files"][&rel]["origin"], "unknown", "沒有工具標記就不能直接當舊版工具新增的檔");
    let restored = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert!(!restored.removed_files.contains(&rel), "來源不明的檔不能直接刪");
    let listed = restored.quarantined.iter().find(|q| q.contains("模組包翻譯工具")).cloned();
    let listed = listed.unwrap_or_else(|| panic!("移除翻譯要列出隔離區的玩家版本：{:?}", restored.quarantined));
    assert!(restored.player_summary.contains("隔離區"), "{}", restored.player_summary);
    let saved = walkdir::WalkDir::new(apply_record::quarantine_dir(&stage.mc))
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| e.path().is_file() && listed.contains(&e.path().display().to_string()) && fs::read(e.path()).unwrap() == b"player version");
    assert!(saved, "列出的位置要真的放著玩家的版本：{listed}");
}

// ─── 3. 翻譯後的模組檔只放在翻譯時那一個模組上 ───

#[test]
fn r3_updated_mod_is_not_replaced_with_old_translation() {
    let stage = Stage::new("r3-updated");
    fs::write(stage.mc.join("mods/example.jar"), b"original-v2").unwrap();
    let result = stage.apply(BACKUP);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original-v2", "更新後的模組不能被舊翻譯蓋掉");
    assert!(result.outdated_mods.contains(&"mods/example.jar".to_string()), "{:?}", result.outdated_mods);
    assert!(result.player_summary.contains("需重新翻譯"), "{}", result.player_summary);
}

#[test]
fn r3_renamed_mod_is_listed_not_placed() {
    let stage = Stage::new("r3-renamed");
    fs::rename(stage.mc.join("mods/example.jar"), stage.mc.join("mods/example-1.1.jar")).unwrap();
    let result = stage.apply(BACKUP);
    assert!(!stage.mc.join("mods/example.jar").exists(), "改名後不能再放一份舊名字的翻譯版（會變成兩個同樣的模組）");
    assert!(result.outdated_mods.contains(&"mods/example.jar".to_string()), "{:?}", result.outdated_mods);
}

#[test]
fn r3_reapply_over_tool_version_still_places_translation() {
    let stage = Stage::new("r3-reapply");
    stage.apply(BACKUP);
    fs::write(stage.work.join("jar-translated/example.jar"), b"translated-v2").unwrap();
    let result = stage.apply(BACKUP);
    assert!(result.outdated_mods.is_empty(), "{:?}", result.outdated_mods);
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated-v2");
}

// ─── 7. 複製出來的資料夾：提供「把這份當成新的整合包」；原位置連不到不能當改名 ───

fn copy_tree(from: &Path, to: &Path) {
    for entry in walkdir::WalkDir::new(from).into_iter().filter_map(|e| e.ok()) {
        let dest = to.join(entry.path().strip_prefix(from).unwrap());
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest).unwrap();
        } else {
            fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

fn instance_id(mc: &Path) -> String {
    read_json(&mc.join(".mcpl/instance.json"))["id"].as_str().unwrap_or_default().to_string()
}

#[test]
fn r7_copied_folder_refusal_does_not_ask_to_delete_marker_and_fork_works() {
    let stage = Stage::new("r7-fork");
    stage.apply(BACKUP);
    let copy = stage.root.join("minecraft_copy");
    copy_tree(&stage.mc, &copy);
    let err = apply_to_instance_with_game_state(&copy, &stage.work, None, BACKUP, GameRunning::No).unwrap_err();
    assert!(!err.contains("刪除"), "不能叫玩家自己刪標記：{err}");
    assert!(err.contains("當成新的整合包"), "要告訴玩家有這個選項：{err}");

    crate::engine::apply_identity::fork_instance(&copy).unwrap();
    assert_ne!(instance_id(&copy), instance_id(&stage.mc), "複製出來的那份要有新的識別碼");
    let result = apply_to_instance_with_game_state(&copy, &stage.work, None, BACKUP, GameRunning::No).unwrap();
    assert_eq!(result.status, ApplyStatus::Applied);
    // 複製來的翻譯版不能當原檔備份：要嘛先隔離、要嘛（和翻譯時的模組指紋對不上）不放
    assert!(
        result.quarantined_files.contains(&"mods/example.jar".to_string())
            || result.outdated_mods.contains(&"mods/example.jar".to_string()),
        "{:?} {:?}",
        result.quarantined_files,
        result.outdated_mods
    );
    assert!(
        !apply_record::instance_backup_dir(&copy).join("mods/example.jar").exists(),
        "複製來的翻譯版不能當原檔備份"
    );
    assert!(!result.player_summary.is_empty());
    assert!(
        read_json(&record_path(&stage))["mcDir"].as_str().unwrap_or_default().contains("minecraft"),
        "原本那份的紀錄仍指向原位置"
    );
    // 原本那份的紀錄不受影響
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
    let _ = fs::remove_dir_all(apply_record::record_dir(&copy));
    let _ = fs::remove_dir_all(apply_record::backup_container(&copy));
    let _ = fs::remove_dir_all(apply_record::quarantine_dir(&copy));
}

fn unused_drive() -> Option<String> {
    (b'D'..=b'Z').rev().map(|c| format!("{}:\\", c as char)).find(|d| !Path::new(d).exists())
}

#[test]
fn r7_unreachable_original_location_is_not_treated_as_rename() {
    let Some(drive) = unused_drive() else { return };
    let stage = Stage::new("r7-unreachable");
    stage.apply(BACKUP);
    let mut record = read_json(&record_path(&stage));
    record["mcDir"] = serde_json::json!(format!("{drive}old/minecraft"));
    write_json(&record_path(&stage), &record);
    fs::write(stage.work.join("jar-translated/example.jar"), b"translated-v2").unwrap();
    let err = apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No).unwrap_err();
    assert!(err.contains("連不到"), "{err}");
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated", "停下來時不能動檔");
}

// ─── 8. 識別碼標記壞了、或 .mcpl 被整個刪掉 ───

#[test]
fn r8_corrupt_instance_marker_stops_with_path_and_fix() {
    let stage = Stage::new("r8-corrupt");
    stage.apply(BACKUP);
    fs::write(stage.mc.join(".mcpl/instance.json"), "{ broken").unwrap();
    let err = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap_err();
    assert!(err.contains("標記壞了"), "{err}");
    assert!(err.contains("instance.json"), "要附位置：{err}");
    assert!(err.contains("修復方法"), "要附修復方法：{err}");
    let err = apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No).unwrap_err();
    assert!(err.contains("標記壞了"), "{err}");
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"translated");
}

#[test]
fn r8_deleted_marker_folder_is_repaired_by_record_path_and_fingerprints() {
    let stage = Stage::new("r8-repair");
    stage.apply(BACKUP);
    let old_id = instance_id(&stage.mc);
    fs::remove_dir_all(stage.mc.join(".mcpl")).unwrap();
    // 第一次：重新配對、補標記；第二次：還原
    let _ = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(instance_id(&stage.mc), old_id, "紀錄位置與檔案指紋都對得上，要沿用原本的識別碼");
    restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(fs::read(stage.mc.join("mods/example.jar")).unwrap(), b"original");
}

#[test]
fn r8_deleted_marker_folder_with_mismatched_files_is_orphaned_and_explained() {
    let stage = Stage::new("r8-orphan");
    stage.apply(BACKUP);
    let old_id = instance_id(&stage.mc);
    let old_record = record_path(&stage);
    fs::remove_dir_all(stage.mc.join(".mcpl")).unwrap();
    fs::write(stage.mc.join("mods/example.jar"), b"player changed").unwrap();
    stage.translated_from_current_game();
    let result = stage.apply(BACKUP);
    assert_ne!(instance_id(&stage.mc), old_id, "對不上就不能沿用");
    assert!(old_record.is_file(), "舊紀錄要保留");
    assert!(result.player_summary.contains("以前的套用紀錄"), "{}", result.player_summary);
    let _ = fs::remove_dir_all(old_record.parent().unwrap());
}
