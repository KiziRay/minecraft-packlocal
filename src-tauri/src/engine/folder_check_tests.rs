//! B5d 選資料夾就判定：分類、形狀、唯讀零寫入、逾時、身分（S05–S07）、遊戲偵測。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::apply_record;
use super::folder_check::*;
use super::folder_identity::inspect_identity;
use super::game_process::command_line_targets_instance;
use super::instance_validate::{validate_instance_path, NO_MODS_REASON};
use super::mcpl_marker as mk;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mcpl-b5d-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn make_pack(dir: &Path) {
    fs::create_dir_all(dir.join("mods")).unwrap();
    fs::create_dir_all(dir.join("config")).unwrap();
    fs::create_dir_all(dir.join("resourcepacks")).unwrap();
    fs::write(dir.join("mods/jei.jar"), b"pk").unwrap();
    fs::write(dir.join("options.txt"), "lang:en_us\n").unwrap();
}

/// 樹狀快照：路徑、是不是資料夾、大小、修改時間、唯讀與（Windows）隱藏屬性。
/// 資料夾的修改時間也比：建立再刪掉測試檔會改到它。
fn snapshot(root: &Path) -> Vec<String> {
    if !root.exists() {
        return vec!["<missing>".into()];
    }
    let mut rows: Vec<String> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| {
            // 用 fs::metadata（開檔查），不用 walkdir 的快取：NTFS 目錄項目裡的資料夾修改時間是延遲更新的
            let meta = fs::metadata(e.path()).unwrap();
            #[cfg(windows)]
            let attrs = {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes()
            };
            #[cfg(not(windows))]
            let attrs = 0u32;
            format!(
                "{}|{}|{}|{:?}|{}|{}",
                e.path().display(),
                meta.is_dir(),
                if meta.is_dir() { 0 } else { meta.len() },
                meta.modified().ok(),
                meta.permissions().readonly(),
                attrs
            )
        })
        .collect();
    rows.sort();
    rows
}

fn store_root() -> PathBuf {
    crate::engine::apply_record::store_root()
}

// ─── 分類 ───────────────────────────────────────────────

#[test]
fn b5d_write_errors_are_classified_by_os_error_not_by_words() {
    let local = Path::new(r"D:\Games\ATM10");
    let err = |code| io::Error::from_raw_os_error(code);
    assert_eq!(classify_io_error(&err(5), Path::new(r"C:\Program Files\x\pack")), WriteIssue::NeedsAdmin);
    assert_eq!(classify_io_error(&err(5), local), WriteIssue::Denied);
    assert_eq!(classify_io_error(&err(5), Path::new(r"C:\Users\p\OneDrive\packs\ATM10")), WriteIssue::Cloud);
    assert_eq!(classify_io_error(&err(112), local), WriteIssue::DiskFull);
    assert_eq!(classify_io_error(&err(32), local), WriteIssue::Locked);
    assert_eq!(classify_io_error(&err(362), local), WriteIssue::Cloud);
    assert_eq!(classify_io_error(&err(225), local), WriteIssue::Antivirus);
    assert_eq!(classify_io_error(&err(53), local), WriteIssue::Network);
    assert_eq!(classify_io_error(&err(3), local), WriteIssue::Missing);
    assert_eq!(classify_io_error(&io::Error::other("怪"), local), WriteIssue::Unknown);
    assert_eq!(classify_io_error(&io::Error::from(io::ErrorKind::StorageFull), local), WriteIssue::DiskFull);
}

#[test]
fn b5d_network_paths_never_suggest_admin() {
    // 以管理員身分開啟的程式預設看不到一般身分對應的網路磁碟機（J-02，推測）：網路位置一律不給管理員鈕
    let unc = Path::new(r"\\nas\Program Files\packs\ATM10");
    assert!(is_network_path(unc));
    assert!(is_network_path(Path::new(r"\\?\UNC\nas\share\pack")));
    assert!(!is_network_path(Path::new(r"\\?\C:\Users\p\pack")));
    for code in [5, 2, 1231, 64] {
        assert_eq!(classify_io_error(&io::Error::from_raw_os_error(code), unc), WriteIssue::Network, "os error {code}");
    }
    assert_eq!(classify_io_error(&io::Error::from(io::ErrorKind::TimedOut), unc), WriteIssue::Network);
}

#[test]
fn b5d_only_the_admin_case_mentions_admin_or_permission() {
    let path = Path::new(r"D:\Games\ATM10");
    for issue in [
        WriteIssue::Denied,
        WriteIssue::DiskFull,
        WriteIssue::Network,
        WriteIssue::Cloud,
        WriteIssue::Antivirus,
        WriteIssue::Locked,
        WriteIssue::Missing,
        WriteIssue::Unknown,
    ] {
        let text = issue_message(issue, path, "os error 5");
        let head = text.lines().next().unwrap();
        assert!(!head.contains("系統管理員") && !head.contains("權限"), "{issue:?}：{head}");
    }
    assert!(issue_message(WriteIssue::NeedsAdmin, path, "").contains("系統管理員"));
    let codes: std::collections::HashSet<_> = [
        WriteIssue::NeedsAdmin,
        WriteIssue::Denied,
        WriteIssue::DiskFull,
        WriteIssue::Network,
        WriteIssue::Cloud,
        WriteIssue::Antivirus,
        WriteIssue::Locked,
        WriteIssue::Missing,
        WriteIssue::Unknown,
    ]
    .iter()
    .map(|i| i.code())
    .collect();
    assert_eq!(codes.len(), 9, "每種原因一個分類碼");
}

#[test]
fn b5d_apply_time_write_probe_names_the_real_cause() {
    // 套用時的實寫探測：失敗訊息不再固定是「請改選你有權限的位置」
    let root = scratch("probe-writable");
    let file = root.join("not-a-dir.txt");
    fs::write(&file, b"x").unwrap();
    let err = super::disk::probe_writable(&file.join("sub")).unwrap_err();
    assert!(!err.contains("請改選你有權限的位置"), "{err}");
    assert!(!err.contains("系統管理員"), "{err}");
    let _ = fs::remove_dir_all(root);
}

// ─── 唯讀零寫入（G1.36） ─────────────────────────────────

#[test]
fn ro_b5d_folder_inspection_writes_nothing_and_creates_no_marker() {
    let root = scratch("ro-inspect");
    let pack = root.join("ATM10");
    make_pack(&pack);
    let before = snapshot(&root);
    // 工具紀錄只看這一包會用到的位置（其他測試同時在寫共用的測試紀錄根目錄）
    let legacy = apply_record::legacy_path_key(&pack);
    let areas: Vec<PathBuf> =
        ["apply-records", "apply-backups", "apply-quarantine"].iter().map(|a| store_root().join(a).join(&legacy)).collect();

    let inspection = inspect_folder(&pack);
    assert!(inspection.reachable && inspection.validation.ok, "{inspection:?}");
    let write = inspection.write.as_ref().unwrap();
    assert!(write.writable, "{write:?}");
    assert_eq!(write.code, "ok");
    assert_eq!(inspection.shape.kind, "ok");
    assert!(inspection.shape.has_options);
    let identity = inspect_identity(&pack, INSPECT_TIMEOUT);
    assert_eq!(identity.state, "new");
    let _ = inspect_folder(&pack.join("mods"));
    let _ = inspect_folder(&root);

    assert_eq!(before, snapshot(&root), "選資料夾的檢查不可在遊戲資料夾寫任何東西（含建了又刪）");
    assert!(!pack.join(".mcpl").exists(), "不可建立 .mcpl");
    assert!(areas.iter().all(|a| !a.exists()), "不可建立工具紀錄");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ro_b5d_elevate_check_write_access_is_read_only_too() {
    let root = scratch("ro-elevate");
    make_pack(&root);
    let before = snapshot(&root);
    let report = super::elevate::check_write_access(&root);
    assert!(report.writable && !report.needs_admin, "{report:?}");
    assert_eq!(before, snapshot(&root));
    let _ = fs::remove_dir_all(root);
}

// ─── 形狀（S02、S03） ────────────────────────────────────

#[test]
fn b5d_selecting_mods_itself_offers_the_parent() {
    let root = scratch("mods-selected");
    make_pack(&root);
    let inspection = inspect_folder(&root.join("mods"));
    assert!(!inspection.validation.ok);
    assert_eq!(inspection.shape.kind, "mods_selected");
    assert_eq!(inspection.shape.parent.as_deref(), Some(root.display().to_string().as_str()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b5d_launcher_list_folder_lists_at_most_five_packs() {
    let root = scratch("launcher-list");
    for name in ["g", "b", "a", "f", "c", "e", "d"] {
        make_pack(&root.join(format!("pack-{name}")));
    }
    fs::create_dir_all(root.join("not-a-pack")).unwrap();
    let inspection = inspect_folder(&root);
    assert_eq!(inspection.shape.kind, "launcher_list");
    let names: Vec<_> = inspection.shape.candidates.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["pack-a", "pack-b", "pack-c", "pack-d", "pack-e"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b5d_folder_without_mods_says_so_in_plain_words() {
    let root = scratch("no-mods");
    fs::create_dir_all(root.join("screenshots")).unwrap();
    let inspection = inspect_folder(&root);
    assert_eq!(inspection.shape.kind, "no_mods");
    assert_eq!(inspection.validation.reason, NO_MODS_REASON);
    assert!(!inspection.validation.reason.contains("實例"));
    assert_eq!(inspect_folder(&root.join("gone")).shape.kind, "invalid");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b5d_server_folder_is_recognised_and_options_decides_override() {
    let root = scratch("server");
    fs::create_dir_all(root.join("mods")).unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("mods/a.jar"), b"pk").unwrap();
    fs::write(root.join("server.properties"), "motd=x\n").unwrap();
    let shape = inspect_folder(&root).shape;
    assert_eq!((shape.kind.as_str(), shape.has_options), ("server", false));
    fs::write(root.join("options.txt"), "lang:en_us\n").unwrap();
    let shape = inspect_folder(&root).shape;
    assert_eq!((shape.kind.as_str(), shape.has_options), ("server", true), "有 options.txt 時前端給「仍要翻」");
    fs::remove_file(root.join("server.properties")).unwrap();
    fs::write(root.join("eula.txt"), "eula=true\n").unwrap();
    assert_eq!(inspect_folder(&root).shape.kind, "server");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn b5d_validation_strings_use_the_glossary() {
    let root = scratch("strings");
    make_pack(&root);
    let ok = validate_instance_path(&root);
    let empty = validate_instance_path(Path::new(""));
    let gone = validate_instance_path(&root.join("gone"));
    for text in [&ok.reason, &empty.reason, &gone.reason]
        .into_iter()
        .chain(ok.hints.iter())
        .chain(empty.hints.iter())
        .chain(gone.hints.iter())
    {
        assert!(!text.contains("實例") && !text.contains("jar"), "{text}");
    }
    assert_eq!(empty.reason, "尚未選擇遊戲資料夾。", "前端靠這句判斷 S01");
    let _ = fs::remove_dir_all(root);
}

// ─── 逾時與起始位置 ───────────────────────────────────────

#[test]
fn b5d_timeout_gives_up_waiting_quickly() {
    let started = Instant::now();
    let out = run_with_timeout(Duration::from_millis(80), || {
        std::thread::sleep(Duration::from_secs(3));
        1
    });
    assert!(out.is_none());
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    assert_eq!(run_with_timeout(Duration::from_secs(2), || 7), Some(7));
}

#[test]
fn b5d_browse_starts_at_a_detected_launcher_folder() {
    let root = scratch("launcher-dir");
    let home = root.join("home");
    let appdata = root.join("appdata");
    fs::create_dir_all(appdata.join("PrismLauncher/instances")).unwrap();
    assert_eq!(first_launcher_dir(Some(&home), Some(&appdata)), Some(appdata.join("PrismLauncher").join("instances")));
    fs::create_dir_all(home.join("curseforge/minecraft/Instances")).unwrap();
    assert_eq!(
        first_launcher_dir(Some(&home), Some(&appdata)),
        Some(home.join("curseforge").join("minecraft").join("Instances")),
        "CurseForge 優先"
    );
    assert_eq!(first_launcher_dir(Some(&root.join("x")), None), None);
    let _ = fs::remove_dir_all(root);
}

// ─── 身分（S05–S07），全部唯讀 ─────────────────────────────

fn write_record(id: &str, mc_dir: &Path) {
    let dir = store_root().join("apply-records").join(id);
    fs::create_dir_all(&dir).unwrap();
    let record = apply_record::ApplyRecord { mc_dir: mc_dir.display().to_string(), ..Default::default() };
    fs::write(dir.join(apply_record::RECORD_FILE), serde_json::to_string(&record).unwrap()).unwrap();
}

#[test]
fn ro_b5d_identity_copied_folder_is_detected_without_writing() {
    let root = scratch("id-copied");
    let original = root.join("ATM10");
    let copy = root.join("ATM10 - 複製");
    make_pack(&original);
    make_pack(&copy);
    let id = format!("b5d-copied-{}", std::process::id());
    mk::create_instance_with_id(&original, &id).unwrap();
    mk::create_instance_with_id(&copy, &id).unwrap();
    write_record(&id, &original);
    let before = snapshot(&root);
    let record_dir = store_root().join("apply-records").join(&id);
    let store_before = snapshot(&record_dir);

    let copied = inspect_identity(&copy, INSPECT_TIMEOUT);
    assert_eq!(copied.state, "copied", "{copied:?}");
    assert_eq!(copied.origin_name, "ATM10");
    assert_eq!(inspect_identity(&original, INSPECT_TIMEOUT).state, "ok");

    assert_eq!(before, snapshot(&root));
    assert_eq!(store_before, snapshot(&record_dir));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ro_b5d_identity_unreachable_original_answers_within_seconds() {
    let root = scratch("id-unreachable");
    make_pack(&root);
    let id = format!("b5d-unreachable-{}", std::process::id());
    mk::create_instance_with_id(&root, &id).unwrap();
    write_record(&id, Path::new(r"\\mcpl-b5d-no-such-host.invalid\share\ATM10"));
    let before = snapshot(&root);
    let started = Instant::now();
    let check = inspect_identity(&root, Duration::from_millis(500));
    assert_eq!(check.state, "unreachable", "{check:?}");
    assert!(started.elapsed() < Duration::from_millis(2500), "{:?}", started.elapsed());
    assert_eq!(before, snapshot(&root));

    // 原位置確定不在（上層連得到）＝搬家或改名，照常沿用
    write_record(&id, &root.parent().unwrap().join("moved-away-b5d"));
    assert_eq!(inspect_identity(&root, INSPECT_TIMEOUT).state, "ok");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ro_b5d_identity_broken_marker_and_record_stop_with_path() {
    let root = scratch("id-broken");
    make_pack(&root);
    fs::create_dir_all(root.join(".mcpl")).unwrap();
    fs::write(root.join(".mcpl/instance.json"), b"{broken").unwrap();
    let before = snapshot(&root);
    let check = inspect_identity(&root, INSPECT_TIMEOUT);
    assert_eq!(check.state, "marker_broken");
    assert!(check.path.ends_with("instance.json"), "{}", check.path);
    assert_eq!(before, snapshot(&root));

    let id = format!("b5d-broken-{}", std::process::id());
    fs::remove_dir_all(root.join(".mcpl")).unwrap();
    mk::create_instance_with_id(&root, &id).unwrap();
    let dir = store_root().join("apply-records").join(&id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(apply_record::RECORD_FILE), b"not json").unwrap();
    let store_before = snapshot(&dir);
    let check = inspect_identity(&root, INSPECT_TIMEOUT);
    assert_eq!(check.state, "record_broken");
    assert!(check.path.ends_with(apply_record::RECORD_FILE), "{}", check.path);
    assert_eq!(store_before, snapshot(&dir), "不自動重設紀錄");
    let _ = fs::remove_dir_all(root);
}

// ─── 遊戲偵測（deferred [S1 審查 4-a]，與分享腳本 GS1.14 同演算法） ─────

#[test]
fn b5d_prism_instance_folder_in_command_line_counts_as_running() {
    let game = r"C:\Users\p\AppData\Roaming\PrismLauncher\instances\ATM10\.minecraft";
    let cmd = r#"javaw.exe -Djava.library.path=C:/Users/p/AppData/Roaming/PrismLauncher/instances/ATM10/natives net.minecraft.client.main.Main"#;
    assert!(command_line_targets_instance(cmd, game));
    let other = r#"javaw.exe -Djava.library.path=C:/Users/p/AppData/Roaming/PrismLauncher/instances/Other/natives"#;
    assert!(!command_line_targets_instance(other, game));
}

#[test]
fn b5d_similar_names_do_not_match() {
    assert!(!command_line_targets_instance(r"javaw.exe --gameDir D:\packs\atm10-2", r"D:\packs\atm10"));
    assert!(command_line_targets_instance(r#"javaw.exe --gameDir "D:\packs\atm10""#, r"D:\packs\atm10"));
    // .minecraft 本身太常見：只有上一層夠獨特時才比名稱
    assert!(!command_line_targets_instance(r"javaw.exe --gameDir C:\Users\p\AppData\Roaming\.minecraft", r"D:\a\.minecraft"));
}
