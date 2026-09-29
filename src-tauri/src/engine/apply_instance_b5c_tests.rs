//! B5c：整份複製的遊戲資料夾在「翻完才套用」時被擋 → 回「已翻完、還沒套用」狀態（不是翻譯失敗），
//! 而且一個檔都不動（G1.4 零寫入、G1.24 拒絕語意不變）。

use super::*;
use super::tests::{Stage, BACKUP};
use std::collections::BTreeMap;

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(list) = fs::read_dir(dir) else { return };
        for entry in list.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                walk(root, &path, out);
            } else {
                out.insert(rel, fs::read(&path).unwrap_or_default());
            }
        }
    }
    walk(root, root, &mut out);
    out
}

#[test]
fn b5c_copied_folder_blocked_at_apply_is_a_pending_status_with_zero_writes() {
    let stage = Stage::new("b5c-fork-pending");
    stage.apply(BACKUP);
    let copy = stage.root.join("minecraft_copy");
    copy_tree(&stage.mc, &copy);
    crate::engine::text_sources::mark_all_produced_for_test(&stage.work, &copy);
    let before = snapshot(&copy);
    let raw = apply_to_instance_with_game_state(&copy, &stage.work, None, BACKUP, GameRunning::No);
    assert!(raw.is_err(), "拒絕語意不變（G1.24）");
    let result = pending_when_copied(raw).expect("複製資料夾被擋要回狀態，不是錯誤");
    assert_eq!(result.status, ApplyStatus::ForkNeeded);
    assert!(!result.is_applied());
    assert!(result.player_summary.contains(super::super::apply_identity::FORK_HINT), "{}", result.player_summary);
    assert_eq!(before, snapshot(&copy), "被擋時一個檔都不能動");
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["status"], "forkNeeded");
}

#[test]
fn b5c_other_apply_errors_are_still_errors() {
    let out = pending_when_copied(Err("讀不到翻譯結果裡的檔案：x".into()));
    assert!(out.is_err());
    let ok = pending_when_copied(Ok(ApplyResult::pending(ApplyStatus::GameRunning, "m".into(), Vec::new())));
    assert_eq!(ok.unwrap().status, ApplyStatus::GameRunning);
}

#[test]
fn b5c_fix3a_latest_result_applied_is_read_only_and_tracks_the_zip() {
    let stage = Stage::new("b5c-latest-applied");
    fs::create_dir_all(stage.work.join("resourcepacks")).unwrap();
    fs::write(stage.work.join("resourcepacks").join("MCPL-test.zip"), b"zip-v1").unwrap();
    assert_eq!(crate::engine::result_owner::latest_applied(&stage.mc, &stage.work), Some(false), "還沒套用過");
    stage.apply(BACKUP);
    let before = snapshot(&stage.mc);
    assert_eq!(crate::engine::result_owner::latest_applied(&stage.mc, &stage.work), Some(true));
    assert_eq!(before, snapshot(&stage.mc), "查詢零寫入（G1.36）");
    fs::write(stage.work.join("resourcepacks").join("MCPL-test.zip"), b"zip-v2").unwrap();
    assert_eq!(crate::engine::result_owner::latest_applied(&stage.mc, &stage.work), Some(false), "最新一輪還沒套用");
}
