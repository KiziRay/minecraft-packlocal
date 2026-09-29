//! S1 審查修正的測試（1-a 覆蓋中斷、1-b 再查遊戲、3 路徑白名單、4 遊戲偵測與訊息、低備註）。

use std::fs;
use std::path::Path;

use super::share_apply_script::*;
use super::share_apply_script_tests::{apply, backup_dirs, build_pack, run_ps, scratch, snapshot, write};

// ---------- S1 審查修正 ----------

fn rewrite_list_row(pack: &Path, rel: &str) {
    let path = pack.join(FILES_LIST_NAME);
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!("{}\t{rel}\n", "0".repeat(64)));
    fs::write(path, text).unwrap();
}

#[cfg(windows)]
#[test]
fn s1_fix_1a_ps_failed_overwrite_keeps_that_file_original_and_rest_restorable() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = scratch("fix1a");
    let (pack, game) = build_pack(&root, None);
    write(&pack.join("config/b.txt"), "新 b".as_bytes());
    write(&game.join("config/b.txt"), "原本 b".as_bytes());
    write_support_files(&pack, "翻譯包.zip", Some(&game)).unwrap();
    let before = snapshot(&game);
    // 只允許別人讀：備份與雜湊讀得到，覆蓋（取代）會失敗
    let lock = fs::OpenOptions::new().read(true).share_mode(1).open(game.join("config/b.txt")).unwrap();
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    drop(lock);
    assert_eq!(code, 2, "{out}");
    assert_eq!(fs::read_to_string(game.join("config/b.txt")).unwrap(), "原本 b", "失敗的檔只會是舊版：{out}");
    assert_eq!(fs::read_to_string(game.join("config/a.txt")).unwrap(), "新 a", "前面已套用的照舊：{out}");
    assert!(out.contains("維持原本的內容"), "{out}");
    let leftovers: Vec<String> = snapshot(&game).into_keys().filter(|k| k.contains(".mcpl-tmp")).collect();
    assert!(leftovers.is_empty(), "不留暫存檔：{leftovers:?}");
    let bak = backup_dirs(&game).remove(0);
    let Some((code, out)) = run_ps(&bak.join(RESTORE_SCRIPT_NAME), &[], "y\n", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    let mut after = snapshot(&game);
    after.retain(|k, _| !k.starts_with(BACKUP_DIR_NAME));
    assert_eq!(after, before, "已套用的部分可以還原：{out}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn s1_fix_1b_game_running_is_checked_again_after_confirmation_before_backup() {
    let text = apply_script_text("p.zip");
    let ask = text.rfind("Ask-Yes").unwrap();
    let backup = text.find("# 7.").unwrap();
    let recheck = text[ask..backup].find("Stop-IfGameRunning $mc");
    assert!(recheck.is_some(), "輸入 y 之後、備份之前要再查一次遊戲有沒有開");
}

#[cfg(windows)]
#[test]
fn s1_fix_3_ps_unsafe_or_unlisted_top_level_paths_are_refused() {
    for bad in [r".mcpl-share-backup.\x.txt", r".mcpl-share-backup \x.txt", r"config.\x.txt", r"mods\evil.jar", "options.txt", r"saves\w\level.dat"] {
        let root = scratch("fix3");
        let (pack, game) = build_pack(&root, None);
        rewrite_list_row(&pack, bad);
        let before = snapshot(&game);
        let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
        assert_eq!(code, 2, "{bad} 要被拒絕：{out}");
        assert!(out.contains("不正確的內容"), "{out}");
        assert_eq!(snapshot(&game), before);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn s1_fix_3_script_top_level_whitelist_matches_share_content() {
    let text = apply_script_text("p.zip");
    for dir in SHARE_TOP_LEVEL_DIRS {
        assert!(text.contains(&format!("'{dir}'")), "白名單缺 {dir}");
    }
    assert!(!SHARE_TOP_LEVEL_DIRS.contains(&"mods"), "分享內容不含 mods，不開放");
}

#[cfg(windows)]
#[test]
fn s1_fix_4a_ps_prism_instance_folder_in_java_command_line_blocks() {
    let root = scratch("fix4a");
    let (pack, _) = build_pack(&root, None);
    let instance = root.join("Prism 實例 atm10");
    let game = instance.join(".minecraft");
    write(&game.join("mods/alpha.jar"), b"aaa");
    write(&game.join("mods/beta.jar"), b"bb");
    write(&game.join("options.txt"), b"x");
    let cmdline = format!(r#""C:\java\javaw.exe" -Djava.library.path={}\natives net.minecraft.client.main.Main"#, instance.display());
    let before = snapshot(&game);
    let Some((code, out)) = apply(&pack, &game, "", &[("MCPL_SHARE_TEST_EXTRA_CMDLINE", &cmdline)]) else { return };
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("請先關閉再套用"), "{out}");
    assert_eq!(snapshot(&game), before);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn s1_fix_4b_messages_name_java_and_explain_unknown() {
    let apply = apply_script_text("p.zip");
    assert!(apply.contains("偵測到有 Minecraft／Java 程式在執行，請先關閉再$verb"));
    assert!(apply.contains("Stop-IfGameRunning $mc '套用'"));
    assert!(apply.contains("系統管理員"), "讀不到命令列時要說明可能原因與做法");
    assert!(restore_script_text().contains("Stop-IfGameRunning $mc '還原'"));
}

#[test]
fn s1_fix_low_readme_restore_order_and_launcher_pauses() {
    assert!(readme_text("p.zip").contains("請從最新的一次開始還原"));
    let apply = apply_script_text("p.zip");
    assert!(apply.contains("pause"), ".cmd 啟動器結尾要停住");
}

#[cfg(windows)]
#[test]
fn s1_fix_low_ps_restore_refuses_outside_backup_folder() {
    let root = scratch("fixlow");
    let (pack, game) = build_pack(&root, None);
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    let bak = backup_dirs(&game).remove(0);
    let moved = game.join("別的資料夾").join("x");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    fs::rename(&bak, &moved).unwrap();
    let applied = snapshot(&game);
    let Some((code, out)) = run_ps(&moved.join(RESTORE_SCRIPT_NAME), &[], "y\n", &[]) else { return };
    assert_eq!(code, 2, "{out}");
    assert_eq!(snapshot(&game), applied);
    let _ = fs::remove_dir_all(root);
}
