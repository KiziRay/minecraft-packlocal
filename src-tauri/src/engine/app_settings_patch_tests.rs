//! `patch_settings`（依路徑合併寫入）的測試。拆出來是為了讓 app_settings.rs 維持在 500 行內。

use super::*;
use serde_json::json;

fn write_to(path: &Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn read_from(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn op_set(path: &str, value: serde_json::Value) -> SettingsPatchOp {
    SettingsPatchOp { path: path.into(), value: Some(value), delete: false }
}

fn op_del(path: &str) -> SettingsPatchOp {
    SettingsPatchOp { path: path.into(), value: None, delete: true }
}

#[test]
fn patch_merges_without_clobbering_other_windows_fields() {
    // 設定視窗改主題、主視窗改記憶的欄位：兩邊各送自己的欄位，彼此都要留下來
    let dir = std::env::temp_dir().join(format!("mcpl-settings-patch-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let path = dir.join(SETTINGS_FILE);
    write_to(&path, &json!({ "version": 1, "appearance": { "theme": "dark", "sfxVolume": "0.3" } }));
    patch_settings_at(&path, &[op_set("appearance.theme", json!("light"))]).unwrap();
    patch_settings_at(&path, &[op_set("translate.cacheRemind", json!("0"))]).unwrap();
    let back = read_from(&path);
    assert_eq!(back["appearance"]["theme"], "light");
    assert_eq!(back["appearance"]["sfxVolume"], "0.3", "沒送的欄位不可被蓋掉");
    assert_eq!(back["translate"]["cacheRemind"], "0");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn patch_null_value_never_overwrites() {
    let mut base = json!({ "version": 1, "translate": { "outputCustomRoot": "D:/x" } });
    let op = SettingsPatchOp {
        path: "translate.outputCustomRoot".into(),
        value: Some(json!(null)),
        delete: false,
    };
    assert_eq!(apply_patch(&mut base, &[op]).unwrap(), 0);
    assert_eq!(base["translate"]["outputCustomRoot"], "D:/x");
}

#[test]
fn patch_delete_is_explicit_and_idempotent() {
    let mut base = json!({ "version": 1, "translate": { "backupChoice": "never", "cacheRemind": "1" } });
    assert_eq!(apply_patch(&mut base, &[op_del(BACKUP_CHOICE_PATH)]).unwrap(), 1);
    assert!(base["translate"].get("backupChoice").is_none());
    assert_eq!(base["translate"]["cacheRemind"], "1");
    assert_eq!(apply_patch(&mut base, &[op_del(BACKUP_CHOICE_PATH)]).unwrap(), 0);
    assert_eq!(apply_patch(&mut base, &[op_del("appearance.theme")]).unwrap(), 0);
    assert_eq!(backup_choice_from(&base), BackupChoice::Unset);
}

#[test]
fn patch_rejects_bad_paths_before_touching_anything() {
    let mut base = json!({ "keep": 1 });
    assert!(apply_patch(&mut base, &[op_set("appearance.theme", json!("dark")), op_set("a..b", json!(1))]).is_err());
    assert!(base.get("appearance").is_none(), "有任何一筆路徑不合法就整批不套用");
    assert!(apply_patch(&mut base, &[op_set("", json!(1))]).is_err());
}

#[test]
fn patch_refuses_to_overwrite_unreadable_file() {
    // 讀不出來的設定檔不能被當成「沒有設定」而整份蓋掉
    let dir = std::env::temp_dir().join(format!("mcpl-settings-patch-bad-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(SETTINGS_FILE);
    fs::write(&path, "{ 壞掉").unwrap();
    assert!(patch_settings_at(&path, &[op_set("appearance.theme", json!("dark"))]).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{ 壞掉", "原檔必須原封不動");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn patch_ops_deserialize_from_frontend_shape() {
    let ops: Vec<SettingsPatchOp> = serde_json::from_value(json!([
        { "path": "appearance.theme", "value": "light" },
        { "path": "translate.backupChoice", "delete": true },
        { "path": "translate.outputCustomRoot", "value": null }
    ]))
    .unwrap();
    assert_eq!(ops.len(), 3);
    assert!(ops[1].delete);
    assert!(ops[2].value.is_none());
}

#[test]
fn backup_choice_reads_known_values_only() {
    assert_eq!(backup_choice_from(&json!({})), BackupChoice::Unset);
    assert_eq!(backup_choice_from(&json!({ "translate": { "backupChoice": "always" } })), BackupChoice::Always);
    assert_eq!(backup_choice_from(&json!({ "translate": { "backupChoice": "never" } })), BackupChoice::Never);
    assert_eq!(backup_choice_from(&json!({ "translate": { "backupChoice": "亂寫" } })), BackupChoice::Unset);
}


#[test]
fn write_replaces_file_in_place_without_deleting_it_first() {
    // 先刪再改名的話，兩步之間當機或 rename 失敗，設定檔就整份不見了
    let source = include_str!("app_settings.rs");
    let start = source.find("fn write_settings_to").expect("找不到 write_settings_to");
    let end = source[start..].find("\n}").expect("找不到函式結尾");
    let body = &source[start..start + end];
    assert!(!body.contains("remove_file(path)"), "寫設定檔不可先刪除原檔");

    let dir = std::env::temp_dir().join(format!("mcpl-settings-atomic-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let path = dir.join(SETTINGS_FILE);
    write_to(&path, &json!({ "version": 1, "appearance": { "theme": "dark" } }));
    for theme in ["light", "dark", "light"] {
        patch_settings_at(&path, &[op_set("appearance.theme", json!(theme))]).unwrap();
        assert!(path.is_file(), "寫入後原檔必須一直存在");
        assert_eq!(read_from(&path)["appearance"]["theme"], theme);
        assert!(!path.with_extension("json.tmp").exists(), "暫存檔不可殘留");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn patch_rejects_paths_outside_the_whitelist() {
    for bad in [
        "version",
        "migration.version",
        "__proto__.polluted",
        "appearance.__proto__",
        "constructor.prototype",
        "appearance.prototype",
        "totally.unknown",
    ] {
        let mut base = json!({ "version": 1 });
        assert!(
            apply_patch(&mut base, &[op_set(bad, json!("x"))]).is_err(),
            "不在白名單的路徑必須拒絕：{bad}"
        );
        assert!(apply_patch(&mut base, &[op_del(bad)]).is_err(), "刪除也要擋：{bad}");
        assert_eq!(base, json!({ "version": 1 }));
    }
    let mut base = json!({});
    assert!(apply_patch(&mut base, &[op_set("appearance.theme", json!("dark"))]).is_ok());
}

#[test]
fn patch_refuses_to_replace_non_object_parent() {
    // 中間節點不是物件（例如被手改成字串）時，不可以整個蓋掉
    let mut base = json!({ "version": 1, "appearance": "壞掉的值" });
    assert!(apply_patch(&mut base, &[op_set("appearance.theme", json!("dark"))]).is_err());
    assert_eq!(base["appearance"], "壞掉的值");
}

#[test]
fn whitelist_is_shared_with_frontend_list() {
    let paths = allowed_setting_paths();
    assert!(paths.len() >= 20, "清單解析失敗：{paths:?}");
    for must in ["translate.backupChoice", "developer.testMode", "appearance.uiScale", "privacy.rememberApiKey"] {
        assert!(paths.contains(&must), "白名單缺少 {must}");
    }
    assert!(!paths.contains(&"version"));
}
