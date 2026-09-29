//! S1 分享腳本安全：產生內容（清單、腳本文字）與 PowerShell 實跑（只在暫存資料夾）。

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::share_apply_script::*;

pub(super) fn scratch(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mcpl_s1_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

pub(super) fn write(path: &Path, body: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// 收件端解壓後的分享包＋一個假遊戲資料夾（名稱含空白、中文、方括號）。
pub(super) fn build_pack(root: &Path, sender_game: Option<&Path>) -> (PathBuf, PathBuf) {
    let pack = root.join("解壓 暫存");
    write(&pack.join("config/a.txt"), "新 a".as_bytes());
    write(&pack.join("kubejs/assets/x/lang/zh_tw.json"), "{\"k\":\"新\"}".as_bytes());
    write(&pack.join("resourcepacks/翻譯包.zip"), b"zip");
    write(&pack.join("ZeitFrei雲端.url"), b"[InternetShortcut]\r\nURL=https://cloud.zeitfrei.uk/\r\n");
    let game = root.join("我的 整合包 [測試]");
    write(&game.join("mods/alpha.jar"), b"aaa");
    write(&game.join("mods/beta.jar"), b"bb");
    write(&game.join("options.txt"), b"lang:en_us");
    write(&game.join("config/a.txt"), "原本 a".as_bytes());
    write_support_files(&pack, "翻譯包.zip", Some(sender_game.unwrap_or(&game))).unwrap();
    (pack, game)
}

pub(super) fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for e in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let rel = e.path().strip_prefix(dir).unwrap().to_string_lossy().to_string();
        if e.file_type().is_file() {
            out.insert(rel, fs::read(e.path()).unwrap());
        } else if e.file_type().is_dir() {
            out.insert(format!("{rel}/"), Vec::new());
        }
    }
    out
}

/// 回傳 (結束碼, 主控台輸出)；沒有 powershell 時回 None（測試照實跳過並印出）。
pub(super) fn run_ps(script: &Path, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Option<(i32, String)> {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            eprintln!("S1：本機沒有 powershell.exe，跳過實跑");
            return None;
        }
    };
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let code = out.status.code().unwrap_or(-1);
    eprintln!("[S1 實跑 {} → 結束碼 {code}]\n{text}", script.display());
    Some((code, text))
}

pub(super) fn apply(pack: &Path, game: &Path, stdin: &str, env: &[(&str, &str)]) -> Option<(i32, String)> {
    let p = pack.display().to_string();
    let g = game.display().to_string();
    run_ps(&pack.join(APPLY_SCRIPT_NAME), &[&p, "-GameDir", &g], stdin, env)
}

pub(super) fn backup_dirs(game: &Path) -> Vec<PathBuf> {
    let root = game.join(BACKUP_DIR_NAME);
    if !root.is_dir() {
        return Vec::new();
    }
    fs::read_dir(root).unwrap().map(|e| e.unwrap().path()).collect()
}

// ---------- 產生內容 ----------

#[test]
fn s1_files_list_hashes_every_payload_file_and_skips_support_files() {
    let root = scratch("list");
    let (pack, _game) = build_pack(&root, None);
    let text = fs::read_to_string(pack.join(FILES_LIST_NAME)).unwrap();
    let rows: Vec<(&str, &str)> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_once('\t').unwrap())
        .collect();
    let rels: Vec<&str> = rows.iter().map(|r| r.1).collect();
    assert!(rels.contains(&"config\\a.txt"), "{rels:?}");
    assert!(rels.contains(&"kubejs\\assets\\x\\lang\\zh_tw.json"), "{rels:?}");
    assert!(rels.contains(&"resourcepacks\\翻譯包.zip"), "{rels:?}");
    assert!(rels.contains(&"ZeitFrei雲端.url"), "{rels:?}");
    for name in SUPPORT_FILES {
        assert!(!rels.contains(name), "支援檔不可當成要放進遊戲的檔：{name}");
    }
    let a = rows.iter().find(|r| r.1 == "config\\a.txt").unwrap();
    assert_eq!(a.0, sha256_file(&pack.join("config/a.txt")).unwrap());
    assert_eq!(a.0.len(), 64);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn s1_mods_list_records_sender_jars_with_size() {
    let root = scratch("mods");
    let sender = root.join("sender");
    write(&sender.join("mods/Alpha-1.0.jar"), b"12345");
    write(&sender.join("mods/off.jar.disabled"), b"x");
    write(&sender.join("mods/readme.txt"), b"x");
    let text = mods_list_text(&sender.join("mods")).unwrap();
    assert!(text.contains("5\tAlpha-1.0.jar"), "{text}");
    assert!(!text.contains("disabled") && !text.contains("readme"), "{text}");
    assert!(mods_list_text(&root.join("nope")).is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn s1_no_sender_game_means_no_mods_list_and_script_asks_before_applying() {
    let root = scratch("nomods");
    let pack = root.join("p");
    write(&pack.join("config/a.txt"), b"x");
    write_support_files(&pack, "p.zip", None).unwrap();
    assert!(!pack.join(MODS_LIST_NAME).exists());
    assert!(pack.join(FILES_LIST_NAME).is_file());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn s1_scripts_are_utf8_bom_stop_on_error_and_never_swallow_errors() {
    for text in [apply_script_text("x'y.zip"), restore_script_text()] {
        assert!(text.starts_with('\u{FEFF}'), "Windows PowerShell 5.1 需要 BOM 才不會中文亂碼");
        assert!(text.trim_start_matches('\u{FEFF}').starts_with("param("), "param 必須在第一行");
        assert!(text.contains("$ErrorActionPreference = 'Stop'"));
        assert!(!text.contains("SilentlyContinue"), "不可吞錯");
        assert!(text.contains("OutputEncoding"));
        assert!(text.contains("Wait-Close"), "結束要停住讓人看到結果");
        assert!(text.contains("\r\n"));
    }
    let apply = apply_script_text("x'y.zip");
    assert!(apply.contains("'x''y.zip'"), "檔名單引號要跳脫");
    assert!(apply.contains(BACKUP_DIR_NAME) && apply.contains(FILES_LIST_NAME) && apply.contains(MODS_LIST_NAME));
    assert!(apply.contains("options.txt") && apply.contains("Test-GameRunning"));
}

#[test]
fn s1_readme_explains_apply_backup_restore_and_mod_mismatch() {
    let text = readme_text("翻譯包.zip");
    for must in [BACKUP_DIR_NAME, RESTORE_LAUNCHER_NAME, "清單.txt", "模組", "輸入 y", "翻譯包.zip", "關閉 Minecraft"] {
        assert!(text.contains(must), "說明缺「{must}」");
    }
}

// ---------- PowerShell 實跑（暫存資料夾） ----------

#[cfg(windows)]
#[test]
fn s1_ps_apply_backs_up_then_restore_returns_game_to_original() {
    let root = scratch("ps-roundtrip");
    let (pack, game) = build_pack(&root, None);
    let before = snapshot(&game);
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("完成"), "主控台要是看得懂的中文：{out}");
    assert_eq!(fs::read_to_string(game.join("config/a.txt")).unwrap(), "新 a");
    assert!(game.join("kubejs/assets/x/lang/zh_tw.json").is_file());
    let baks = backup_dirs(&game);
    assert_eq!(baks.len(), 1, "{out}");
    let bak = &baks[0];
    assert_eq!(fs::read_to_string(bak.join("files/config/a.txt")).unwrap(), "原本 a");
    let manifest = fs::read_to_string(bak.join("manifest.tsv")).unwrap();
    assert!(manifest.contains("overwritten\t") && manifest.contains("config\\a.txt"), "{manifest}");
    assert!(manifest.contains("added\t") && manifest.contains("kubejs\\assets\\x\\lang\\zh_tw.json"), "{manifest}");
    assert!(bak.join("清單.txt").is_file() && bak.join(RESTORE_LAUNCHER_NAME).is_file() && bak.join(RESTORE_SCRIPT_NAME).is_file());
    assert!(out.contains(&bak.display().to_string()), "完成訊息要寫備份位置：{out}");

    let Some((code, out)) = run_ps(&bak.join(RESTORE_SCRIPT_NAME), &[], "y\n", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    let mut after = snapshot(&game);
    after.retain(|k, _| !k.starts_with(BACKUP_DIR_NAME));
    assert_eq!(after, before, "還原後要回到套用前（新增的檔與資料夾都刪掉）：{out}");
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_restore_leaves_files_changed_after_apply_untouched() {
    let root = scratch("ps-changed");
    let (pack, game) = build_pack(&root, None);
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    fs::write(game.join("config/a.txt"), "玩家後來自己改的").unwrap();
    let bak = backup_dirs(&game).remove(0);
    let Some((code, out)) = run_ps(&bak.join(RESTORE_SCRIPT_NAME), &[], "y\n", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read_to_string(game.join("config/a.txt")).unwrap(), "玩家後來自己改的");
    assert!(out.contains("config\\a.txt"), "被改過的檔要列出來：{out}");
    assert!(!game.join("kubejs").exists(), "沒被改過的新增檔照樣刪除：{out}");
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_restore_cancel_by_default_changes_nothing() {
    let root = scratch("ps-restore-cancel");
    let (pack, game) = build_pack(&root, None);
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    let applied = snapshot(&game);
    let bak = backup_dirs(&game).remove(0);
    let Some((code, out)) = run_ps(&bak.join(RESTORE_SCRIPT_NAME), &[], "\n", &[]) else { return };
    assert_eq!(code, 1, "{out}");
    assert_eq!(snapshot(&game), applied, "{out}");
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_mods_mismatch_stops_by_default_and_continues_only_on_y() {
    let root = scratch("ps-mods");
    let sender = root.join("sender");
    write(&sender.join("mods/alpha.jar"), b"aaa");
    write(&sender.join("mods/beta.jar"), b"bbbbbb");
    write(&sender.join("mods/gamma.jar"), b"g");
    let (pack, game) = build_pack(&root, Some(&sender));
    let before = snapshot(&game);
    let Some((code, out)) = apply(&pack, &game, "\n", &[]) else { return };
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("gamma.jar") && out.contains("beta.jar"), "要列出不同的模組：{out}");
    assert_eq!(snapshot(&game), before, "預設不繼續，什麼都不動");
    let Some((code, out)) = apply(&pack, &game, "y\n", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read_to_string(game.join("config/a.txt")).unwrap(), "新 a");
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_not_a_game_folder_stops_without_changes() {
    let root = scratch("ps-notgame");
    let (pack, _game) = build_pack(&root, None);
    let prism = root.join("Prism 實例");
    write(&prism.join(".minecraft/mods/alpha.jar"), b"aaa");
    write(&prism.join("instance.cfg"), b"x");
    let before = snapshot(&prism);
    let Some((code, out)) = apply(&pack, &prism, "", &[]) else { return };
    assert_eq!(code, 1, "{out}");
    assert!(out.contains(".minecraft"), "要指出該選哪一層：{out}");
    assert_eq!(snapshot(&prism), before);
    // 有 mods 但沒有 options.txt 也沒有 config：判斷不了就停
    let odd = root.join("只有 mods");
    write(&odd.join("mods/alpha.jar"), b"aaa");
    let Some((code, out)) = apply(&pack, &odd, "", &[]) else { return };
    assert_eq!(code, 1, "{out}");
    assert!(!odd.join("config").exists() && !odd.join(BACKUP_DIR_NAME).exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_game_running_or_unknown_stops_without_changes() {
    let root = scratch("ps-running");
    let (pack, game) = build_pack(&root, None);
    let before = snapshot(&game);
    for (value, expect) in [("yes", "請先關閉再套用"), ("unknown", "系統管理員")] {
        let Some((code, out)) = apply(&pack, &game, "y\n", &[("MCPL_SHARE_TEST_GAME_RUNNING", value)]) else { return };
        assert_eq!(code, 1, "{out}");
        assert!(out.contains(expect), "{out}");
        assert_eq!(snapshot(&game), before);
    }
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_corrupted_payload_stops_without_changes() {
    let root = scratch("ps-corrupt");
    let (pack, game) = build_pack(&root, None);
    fs::write(pack.join("config/a.txt"), "被竄改").unwrap();
    let before = snapshot(&game);
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("重新下載"), "{out}");
    assert_eq!(snapshot(&game), before);
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_backup_failure_aborts_before_any_overwrite() {
    let root = scratch("ps-bakfail");
    let (pack, game) = build_pack(&root, None);
    // 備份資料夾位置被一個同名檔案占住 → 建不出備份
    write(&game.join(BACKUP_DIR_NAME), b"not a dir");
    let before = snapshot(&game);
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("備份失敗"), "{out}");
    assert_eq!(snapshot(&game), before, "備份失敗就不覆蓋任何檔");
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_locked_file_fails_loudly_without_overwrite() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = scratch("ps-locked");
    let (pack, game) = build_pack(&root, None);
    let lock = fs::OpenOptions::new().read(true).write(true).share_mode(0).open(game.join("config/a.txt")).unwrap();
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    drop(lock);
    assert_eq!(code, 2, "{out}");
    assert!(!out.contains("完成："), "不可假裝成功：{out}");
    assert_eq!(fs::read_to_string(game.join("config/a.txt")).unwrap(), "原本 a");
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn s1_ps_long_paths_are_handled() {
    let root = scratch("ps-long");
    let pack = root.join("解壓 暫存");
    let deep: PathBuf = (0..6).map(|i| format!("很長的資料夾名稱-{i}-{}", "x".repeat(30))).collect();
    let rel = Path::new("config").join(&deep).join("深層 設定.json");
    write(&pack.join(&rel), b"{}");
    let game = root.join("我的 整合包 [測試]");
    write(&game.join("mods/alpha.jar"), b"aaa");
    write(&game.join("options.txt"), b"x");
    write(&game.join(&rel), b"old");
    assert!(game.join(&rel).display().to_string().len() > 260);
    write_support_files(&pack, "p.zip", Some(&game)).unwrap();
    let Some((code, out)) = apply(&pack, &game, "", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read(game.join(&rel)).unwrap(), b"{}");
    let bak = backup_dirs(&game).remove(0);
    let Some((code, out)) = run_ps(&bak.join(RESTORE_SCRIPT_NAME), &[], "y\n", &[]) else { return };
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read(game.join(&rel)).unwrap(), b"old");
    let _ = fs::remove_dir_all(root);
}

// ---------- 接進分享包 ----------

#[test]
fn s1_share_stage_carries_support_files_and_sender_mods_without_changing_content_scope() {
    let root = scratch("stage");
    let work = root.join("翻譯結果");
    let game = root.join("game");
    write(&game.join("mods/alpha.jar"), b"aaa");
    write(&work.join("config/ftbquests/quests/chapter.snbt"), b"title: \"hi\"");
    write(&work.join("kubejs/server_scripts/private.js"), b"x");
    write(&work.join("覆蓋範圍說明.txt"), b"x");
    crate::engine::text_sources::mark_all_produced_for_test(&work, &game);
    let stage = root.join("stage");
    fs::create_dir_all(&stage).unwrap();
    crate::engine::share_pack::stage_share_payload(&work, &stage).unwrap();
    for name in [APPLY_SCRIPT_NAME, RESTORE_SCRIPT_NAME, FILES_LIST_NAME, MODS_LIST_NAME, README_NAME] {
        assert!(stage.join(name).is_file(), "缺 {name}");
    }
    let list = fs::read_to_string(stage.join(FILES_LIST_NAME)).unwrap();
    assert!(list.contains(r"config\ftbquests\quests\chapter.snbt"), "{list}");
    assert!(!list.contains("server_scripts") && !list.contains("覆蓋範圍說明"), "分享內容範圍照 G3.x：{list}");
    assert!(fs::read_to_string(stage.join(MODS_LIST_NAME)).unwrap().contains("3\talpha.jar"));
    let args = crate::engine::share_pack::shareable_top_level_args(&stage).unwrap();
    for a in &args {
        let n = a.file_name().unwrap().to_string_lossy().to_string();
        assert!(!is_support_file(&n), "支援檔另外加進 7z，不重複：{n}");
    }
    let _ = fs::remove_dir_all(root);
}
