//! B1 複審：查詢狀態零寫入；認回說明只在會動檔的動作裡產生、並帶進結果；
//! 沿用上次的原檔備份前，備份標記的原檔指紋要等於這次記下的原檔指紋。

use std::collections::BTreeMap;

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


fn instance_id(mc: &Path) -> String {
    let text = fs::read_to_string(mc.join(".mcpl/instance.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&text).unwrap()["id"].as_str().unwrap_or_default().to_string()
}

/// 翻譯結果多三個設定檔：改掉其中一個仍達「多數相符」的認回門檻。
fn stage_with_configs(tag: &str) -> Stage {
    let stage = Stage::new(tag);
    fs::create_dir_all(stage.work.join("config")).unwrap();
    for name in ["a", "b", "c"] {
        fs::write(stage.work.join(format!("config/{name}.txt")), format!("中文{name}")).unwrap();
    }
    stage
}

/// 套用過 → `.mcpl` 被刪 → 整合包更新改了一個設定檔。回傳原本的識別碼。
fn applied_then_marker_lost(stage: &Stage) -> String {
    stage.apply(BACKUP);
    let old_id = instance_id(&stage.mc);
    fs::remove_dir_all(stage.mc.join(".mcpl")).unwrap();
    fs::write(stage.mc.join("config/a.txt"), "pack updated").unwrap();
    old_id
}

/// 目錄底下每個檔（含資料夾本身）的路徑與內容；資料夾內容記為空。
fn snapshot(dirs: &[PathBuf]) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut all = BTreeMap::new();
    for dir in dirs {
        for entry in walkdir::WalkDir::new(long_path(dir)).into_iter().filter_map(|e| e.ok()) {
            let key = entry.path().display().to_string();
            let content = entry.file_type().is_file().then(|| fs::read(entry.path()).unwrap());
            all.insert(key, content);
        }
    }
    all
}

/// 這個遊戲資料夾在工具資料夾裡可能用到的位置（識別碼與舊鍵兩種）。
fn tool_areas(mc: &Path, id: &str) -> Vec<PathBuf> {
    let legacy = apply_record::legacy_path_key(mc);
    let root = apply_record::store_root();
    ["apply-records", "apply-backups", "apply-quarantine"]
        .iter()
        .flat_map(|area| [root.join(area).join(id), root.join(area).join(&legacy)])
        .collect()
}

// ─── (a) 查詢狀態：一個檔都不寫 ───

#[test]
fn ro_has_apply_backups_after_marker_deleted_writes_nothing() {
    let stage = stage_with_configs("ro-a-query");
    let old_id = applied_then_marker_lost(&stage);
    let mut watched = vec![stage.root.clone()];
    watched.extend(tool_areas(&stage.mc, &old_id));
    let before = snapshot(&watched);

    assert!(has_apply_backups_in(&stage.mc, Some(&stage.work)).unwrap(), "認得回來的紀錄要算有翻譯可以移除");

    let after = snapshot(&watched);
    assert!(!stage.mc.join(".mcpl").exists(), "查詢狀態不能補回 .mcpl");
    assert_eq!(before, after, "查詢狀態不能新增或修改遊戲資料夾與工具資料夾裡的任何檔");
}

// ─── (b) 先查詢再移除：說明還在 ───

#[test]
fn ro_query_then_remove_still_says_rest_reclaimed() {
    let stage = stage_with_configs("ro-b-remove");
    let old_id = applied_then_marker_lost(&stage);
    assert!(has_apply_backups_in(&stage.mc, Some(&stage.work)).unwrap());
    let removed = restore_last_apply_in(&stage.mc, Some(&stage.work)).unwrap();
    assert_eq!(instance_id(&stage.mc), old_id, "移除翻譯時才認回原本的識別碼");
    assert!(removed.player_summary.contains("其餘已認回"), "{}", removed.player_summary);
}

#[test]
fn ro_query_then_apply_still_says_rest_reclaimed() {
    let stage = stage_with_configs("ro-b-apply");
    applied_then_marker_lost(&stage);
    assert!(has_apply_backups_in(&stage.mc, Some(&stage.work)).unwrap());
    let result = stage.apply(BACKUP);
    assert!(result.player_summary.contains("其餘已認回"), "{}", result.player_summary);
}

// ─── (c) 字體包、修復清單：認回說明要出現在結果裡 ───

#[test]
fn ro_font_pack_result_includes_reclaim_notice() {
    let stage = stage_with_configs("ro-c-font");
    let old_id = applied_then_marker_lost(&stage);
    let font = stage.root.join("字體結果").join("resourcepacks").join("繁體中文遊戲字體");
    fs::create_dir_all(font.join("assets/minecraft/font")).unwrap();
    fs::write(font.join("assets/minecraft/font/default.json"), "{}").unwrap();
    let result =
        crate::engine::font_pack::apply_font_pack_with_game_state(&stage.mc, &font, GameRunning::No).unwrap();
    assert_eq!(instance_id(&stage.mc), old_id);
    assert!(result.player_summary.contains("其餘已認回"), "{}", result.player_summary);
    assert!(result.player_summary.contains("config/a.txt"), "{}", result.player_summary);
}

#[test]
fn ro_pack_repair_result_includes_reclaim_notice() {
    let stage = stage_with_configs("ro-c-repair");
    fs::write(stage.mc.join("resourcepacks/Lost.zip"), b"lost").unwrap();
    let old = stage.mc.join("翻譯套用備份_20250101_000000");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("options.txt"), "resourcePacks:[\"vanilla\",\"file/OtherZh.zip\",\"file/Lost.zip\"]\n").unwrap();
    let old_id = applied_then_marker_lost(&stage);
    let outcome =
        crate::engine::pack_repair::repair_pack_list_reporting(&stage.mc, GameRunning::No).unwrap();
    assert_eq!(instance_id(&stage.mc), old_id);
    assert!(outcome.added > 0);
    assert!(outcome.notice.as_deref().unwrap_or_default().contains("其餘已認回"), "{:?}", outcome.notice);
}

// ─── (d) 沿用上次的原檔備份：備份標記的原檔指紋要等於這次記下的 ───

#[test]
fn ro_rerun_refuses_backup_whose_original_fingerprint_differs() {
    let stage = Stage::new("ro-d-backup-sha");
    // 翻譯結果多一個設定檔，遊戲裡同位置是資料夾：寫它一定失敗（在寫模組檔之前）
    fs::create_dir_all(stage.work.join("config/zz")).unwrap();
    fs::write(stage.work.join("config/zz/b.txt"), "中文").unwrap();
    fs::create_dir_all(stage.mc.join("config/zz/b.txt/blocked")).unwrap();
    assert!(apply_to_instance_with_game_state(&stage.mc, &stage.work, None, BACKUP, GameRunning::No).is_err());

    // 備份檔與備份標記自己對得上，但原檔指紋跟遊戲檔標記記下的不同
    let rel = "mods/example.jar";
    let backup = apply_guard::backup_file_path(&stage.mc, rel);
    assert_eq!(fs::read(&backup).unwrap(), b"original", "中斷前已先建立原檔備份");
    fs::write(&backup, b"someone else").unwrap();
    let marker_path = apply_guard::backup_marker_path(&stage.mc, rel);
    let mut marker: serde_json::Value = serde_json::from_str(&fs::read_to_string(&marker_path).unwrap()).unwrap();
    marker["originalSha256"] = serde_json::json!(sha256_hex(b"someone else"));
    fs::write(&marker_path, marker.to_string()).unwrap();

    fs::remove_dir_all(stage.mc.join("config/zz/b.txt")).unwrap();
    let result = stage.apply(BACKUP);
    assert!(result.unconfirmed_files.contains(&rel.to_string()), "{:?}", result.unconfirmed_files);
    assert_eq!(fs::read(stage.mc.join(rel)).unwrap(), b"original", "無法確認就不覆蓋");
    assert_eq!(fs::read(&backup).unwrap(), b"someone else", "對不上的備份不動");
}
