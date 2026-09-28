//! B5a-2：設定視窗「資料與備份」用到的後端最小改動（翻譯中不搬工具資料、備份實際位置唯讀查詢、本地模型佔用大小）。

use super::*;

fn lib_src() -> String {
    std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs"))
        .unwrap()
        .replace("\r\n", "\n")
}

#[test]
fn b5a2_migrate_data_root_refuses_while_translating_before_touching_files() {
    let lib = lib_src();
    let body = &lib[lib.find("async fn migrate_data_root_cmd").unwrap()..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let guard = body.find("TRANSLATION_ACTIVE.load(").expect("要有忙碌檢查");
    let work = body.find("spawn_blocking").expect("搬移在背景執行緒");
    assert!(guard < work, "忙碌檢查要在開始複製之前");
    assert!(body.contains("正在翻譯，翻完才能搬移工具資料。"));
}

#[test]
fn b5a2_backup_location_is_read_only_and_points_at_this_packs_backup_dir() {
    let root = std::env::temp_dir().join(format!("mcpl-b5a2-backup-location-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    fs::create_dir_all(instance.join("mods")).unwrap();
    let before: Vec<_> = walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok).map(|e| e.path().to_path_buf()).collect();

    let location = apply_backup_location_cmd(instance.display().to_string()).expect("有 mods 的資料夾要查得到");
    assert!(location.ends_with(engine::apply_record::INSTANCE_BACKUP_DIR), "{location}");
    assert!(location.contains("apply-backups"), "{location}");

    let after: Vec<_> = walkdir::WalkDir::new(&root).into_iter().filter_map(Result::ok).map(|e| e.path().to_path_buf()).collect();
    assert_eq!(before, after, "查位置不可在遊戲資料夾寫任何東西（不建立識別碼記號）");
    assert!(!Path::new(&location).exists(), "只算路徑，不建立備份資料夾");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn b5a2_local_model_status_reports_size_of_what_delete_would_remove() {
    let src = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/engine/local_llm/mod.rs"),
    )
    .unwrap();
    let body = &src[src.find("pub fn status_view()").unwrap()..];
    assert!(body.contains("\"sizeBytes\": size_bytes"));
    assert!(body.contains("dir.join(\"models\")") && body.contains("dir.join(\"runtime\")"), "只算刪除時會刪的兩個子目錄");
}

fn guarded_before_work(fn_name: &str, work: &str, reason: &str) {
    let lib = lib_src();
    let body = &lib[lib.find(fn_name).unwrap_or_else(|| panic!("找不到 {fn_name}"))..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let guard = body.find("TRANSLATION_ACTIVE.load(").unwrap_or_else(|| panic!("{fn_name} 要有忙碌檢查"));
    let at = body.find(work).unwrap_or_else(|| panic!("{fn_name} 找不到 {work}"));
    assert!(guard < at, "{fn_name}：忙碌檢查要在動檔案之前");
    assert!(body.contains(reason), "{fn_name} 要回白話原因：{reason}");
}

/// 審查 F1：翻譯或套用中不能刪本地模型（翻譯正在用它）。
#[test]
fn b5a2_fix_f1_local_model_delete_refuses_while_busy() {
    guarded_before_work("async fn local_llm_delete_cmd", "spawn_blocking", "正在翻譯或套用，完成後才能刪除本地模型。");
}

/// 審查 F2：翻譯或套用中不能刪備份（套用正在寫備份）。
#[test]
fn b5a2_fix_f2_delete_backups_refuses_while_busy() {
    guarded_before_work("async fn delete_apply_backups_cmd", "spawn_blocking", "正在翻譯或套用，完成後才能刪除備份。");
}
