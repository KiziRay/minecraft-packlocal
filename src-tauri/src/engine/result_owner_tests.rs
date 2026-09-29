use super::*;
use std::fs;

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mcpl-b5c-owner-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn game(root: &Path, name: &str) -> PathBuf {
    let mc = root.join(name);
    fs::create_dir_all(mc.join("mods")).unwrap();
    fs::write(mc.join("options.txt"), "lang:en_us\n").unwrap();
    mc
}

fn result_for(root: &Path, mc: &Path) -> PathBuf {
    let out = root.join(format!("out-{}", mc.file_name().unwrap().to_string_lossy()));
    let work = out.join(crate::engine::out_layout::RESULT_DIR_NAME);
    fs::create_dir_all(work.join("resourcepacks")).unwrap();
    fs::write(work.join("resourcepacks").join("MCPL.zip"), b"zip").unwrap();
    crate::engine::text_sources::mark_all_produced_for_test(&work, mc);
    out
}

#[test]
fn b5c_fix1_result_of_a_is_refused_for_b_in_plain_words() {
    let root = tmp("ab");
    let a = game(&root, "PackA");
    let b = game(&root, "PackB");
    let out_a = result_for(&root, &a);
    let err = guard(&b, &out_a).unwrap_err();
    assert!(err.contains("屬於「PackA」，不是「PackB」"), "{err}");
    assert_eq!(guard(&a, &out_a), Ok(None));
    // 路徑寫法不同（大小寫、結尾分隔符）也認得是同一包
    let alt = PathBuf::from(format!("{}\\", a.display().to_string().to_uppercase()));
    assert_eq!(guard(&alt, &out_a), Ok(None));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn b5c_fix1_legacy_result_without_owner_is_allowed_with_a_note() {
    let root = tmp("legacy");
    let b = game(&root, "PackB");
    let out = root.join("old");
    fs::create_dir_all(out.join(crate::engine::out_layout::RESULT_DIR_NAME)).unwrap();
    let note = guard(&b, &out).unwrap().expect("要註記");
    assert!(note.contains("舊版"), "{note}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn b5c_fix1_session_instance_path_is_used_when_no_manifest() {
    let root = tmp("session");
    let a = game(&root, "PackA");
    let b = game(&root, "PackB");
    let out = root.join("out");
    let work = out.join(crate::engine::out_layout::RESULT_DIR_NAME);
    fs::create_dir_all(&work).unwrap();
    let session = serde_json::json!({ "instancePath": a.display().to_string() });
    fs::write(work.join(crate::engine::session::SESSION_FILE), session.to_string()).unwrap();
    assert!(guard(&b, &out).is_err());
    assert_eq!(guard(&a, &out), Ok(None));
    let _ = fs::remove_dir_all(&root);
}
