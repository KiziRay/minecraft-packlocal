//! 舊版資料升級（B0 #6）。
//!
//! 使用者從 1.0.x 升上來時，硬碟上留著各種舊格式：設定檔裡的舊鍵名、沒有新欄位的
//! 工作階段、舊版翻譯記憶、放在遊戲資料夾旁的舊備份。這裡集中處理「讀得到、
//! 不會壞」，並在啟動時跑一次設定檔遷移。
//!
//! 硬規則：
//! - **可重入**：已遷移過就跳過；重跑任何次數結果都一樣。
//! - **不刪使用者資料**：只把舊鍵搬到新位置；新位置已有值時保留新值。
//! - **失敗不擋啟動**：任何錯誤只回報，不 panic。

use std::path::Path;

use serde_json::Value;

use super::app_settings;
use super::session::TranslateSession;

/// 目前的遷移版本。之後有新遷移時加一，並在 `migrate_settings_value` 補對應步驟。
pub const MIGRATION_VERSION: u64 = 1;

/// 舊版前端直接用 localStorage 鍵名存設定；有些舊設定檔也原樣帶著這些鍵名。
/// 對照 `src/core/settings-store.js` 的 KEY_MAP（含 1.0.6／1.0.7 的舊鍵）。
const LEGACY_KEYS: &[(&str, &str)] = &[
    ("modpack-i18n-theme", "appearance.theme"),
    ("modpack-i18n-consent-hide-v1.0.9", "consent.hideVersion"),
    ("modpack-i18n-consent-hide-v1.0.6", "consent.hideVersion"),
    ("modpack-i18n-backup-before-apply", "translate.backupBeforeApply"),
    ("modpack-i18n-font-prefs", "appearance.fontPrefs"),
    ("modpack-i18n-sfx-volume-v1", "appearance.sfxVolume"),
    ("modpack-i18n-sfx-muted-v1", "appearance.sfxMuted"),
    ("modpack-i18n-output-storage-mode-v1", "translate.outputStorageMode"),
    ("modpack-i18n-output-custom-root-v1", "translate.outputCustomRoot"),
    ("modpack-i18n-cache-remind-v1", "translate.cacheRemind"),
    ("modpack-i18n-local-cloud-topup-v1", "translate.localCloudTopUp"),
    ("modpack-i18n-last-instance-path-v1", "translate.lastInstancePath"),
    ("modpack-i18n-local-llm-consent-v1", "localModel.consented"),
    ("modpack-i18n-local-llm-dir-v1", "localModel.installDir"),
    ("modpack-i18n-onboarding-seen-v1.0.9", "onboarding.seenVersion"),
    ("modpack-i18n-onboarding-seen-v1.0.7", "onboarding.seenVersion"),
    ("modpack-i18n-coverage-ack-hard", "translate.coverageAck"),
    ("modpack-i18n-remember-api-key-v1", "privacy.rememberApiKey"),
    ("modpack-i18n-usage-feedback-client-id-v1", "usage.clientId"),
    ("modpack-i18n-usage-feedback-last-submit-at-v1", "usage.lastSubmitAt"),
    ("modpack-i18n-usage-feedback-last-nudge-at-v1", "usage.lastNudgeAt"),
];

/// 已經移除的設定（「本地模型常駐」）：遷移時直接丟掉，不再有任何程式讀它。
const DROPPED_FLAT_KEYS: &[&str] = &["modpack-i18n-keep-local-model-v1"];
const DROPPED_NESTED: &[(&str, &str)] = &[("localModel", "keepAlive")];

/// 工作階段裡「原文雜湊」欄位的名稱（B6 會開始寫入）。
pub const SESSION_SOURCE_HASH_FIELD: &str = "sourceHashes";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SettingsMigration {
    /// 搬到新位置的舊鍵數
    pub moved: usize,
    /// 丟掉的已移除設定數
    pub dropped: usize,
    /// 已經遷移過，這次什麼都沒做
    pub already: bool,
}

fn migration_version(settings: &Value) -> u64 {
    settings
        .get("migration")
        .and_then(|m| m.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn nested_get<'a>(settings: &'a Value, dotted: &str) -> Option<&'a Value> {
    dotted
        .split('.')
        .try_fold(settings, |node, part| node.get(part))
        .filter(|v| !v.is_null())
}

fn nested_set(settings: &mut Value, dotted: &str, value: Value) {
    let parts: Vec<&str> = dotted.split('.').collect();
    let Some((last, parents)) = parts.split_last() else {
        return;
    };
    let mut node = settings;
    for part in parents {
        let Some(obj) = node.as_object_mut() else {
            return;
        };
        let entry = obj
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Default::default()));
        if !entry.is_object() {
            *entry = Value::Object(Default::default());
        }
        node = entry;
    }
    if let Some(obj) = node.as_object_mut() {
        obj.insert(last.to_string(), value);
    }
}

/// 把一份設定物件升到目前結構。純函式：只改傳進來的值，不碰檔案。
pub fn migrate_settings_value(settings: &mut Value) -> SettingsMigration {
    let mut report = SettingsMigration::default();
    if !settings.is_object() {
        return report;
    }
    if migration_version(settings) >= MIGRATION_VERSION {
        report.already = true;
        return report;
    }
    for (flat, dotted) in LEGACY_KEYS {
        let Some(value) = settings.as_object_mut().and_then(|o| o.remove(*flat)) else {
            continue;
        };
        report.moved += 1;
        // 新位置已經有值＝使用者在新版改過，新值優先
        if !value.is_null() && nested_get(settings, dotted).is_none() {
            nested_set(settings, dotted, value);
        }
    }
    for flat in DROPPED_FLAT_KEYS {
        if settings.as_object_mut().and_then(|o| o.remove(*flat)).is_some() {
            report.dropped += 1;
        }
    }
    for (parent, key) in DROPPED_NESTED {
        if let Some(obj) = settings.get_mut(*parent).and_then(Value::as_object_mut) {
            if obj.remove(*key).is_some() {
                report.dropped += 1;
            }
        }
    }
    nested_set(settings, "migration.version", Value::from(MIGRATION_VERSION));
    report
}

/// 對指定設定檔跑一次遷移。檔案不存在時什麼都不做（全新安裝由前端建立第一份）。
pub fn migrate_settings_file(path: &Path) -> Result<SettingsMigration, String> {
    let mut outcome = SettingsMigration::default();
    app_settings::update_settings_at(path, |settings| {
        outcome = migrate_settings_value(settings);
        !outcome.already
    })?;
    Ok(outcome)
}

/// 啟動時呼叫一次。任何失敗只寫診斷紀錄，不擋工具開啟。
pub fn run_startup_migration() {
    match migrate_settings_file(&app_settings::settings_path()) {
        Ok(outcome) if !outcome.already && (outcome.moved > 0 || outcome.dropped > 0) => {
            crate::dev_log!(
                "migrate",
                "設定檔已升級：搬移舊鍵 {} 個、移除停用設定 {} 個",
                outcome.moved,
                outcome.dropped
            );
        }
        Ok(_) => {}
        Err(e) => {
            crate::dev_log!("migrate", "設定檔升級略過：{e}");
        }
    }
}

/// 舊工作階段沒有原文雜湊時，譯文是否還對得上現在的英文無從判斷，必須重掃。
pub fn session_needs_rescan(raw: &Value) -> bool {
    match raw.get(SESSION_SOURCE_HASH_FIELD) {
        None | Some(Value::Null) => true,
        Some(Value::Object(map)) => map.is_empty(),
        Some(_) => true,
    }
}

/// 讀一份工作階段文字：舊欄位缺漏靠 serde 預設值補齊，並回報是否需要重掃。
pub fn read_session_text(text: &str) -> Result<(TranslateSession, bool), String> {
    let raw: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let needs_rescan = session_needs_rescan(&raw);
    let session: TranslateSession =
        serde_json::from_value(raw).map_err(|e| e.to_string())?;
    Ok((session, needs_rescan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    const LEGACY_SETTINGS: &str = include_str!("../../fixtures/migration/settings-legacy-flat.json");
    const LEGACY_SESSION: &str = include_str!("../../fixtures/migration/session-legacy-v1.0.8.json");
    const LEGACY_APPLY_MANIFEST: &str =
        include_str!("../../fixtures/migration/legacy-apply-manifest.json");

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("migration")
            .join(name)
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mcpl-migrate-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn legacy_settings_keys_move_to_new_structure_and_rerun_is_noop() {
        let dir = temp_dir("settings");
        let path = dir.join("工具設定.json");
        fs::write(&path, LEGACY_SETTINGS).unwrap();

        let first = migrate_settings_file(&path).unwrap();
        assert!(!first.already);
        assert!(first.moved >= 4, "舊鍵應搬到新位置：{first:?}");
        assert_eq!(first.dropped, 2, "本地模型常駐的新舊兩種寫法都要丟掉");

        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after["appearance"]["theme"], "light");
        assert_eq!(after["appearance"]["sfxVolume"], "0.3");
        assert_eq!(after["appearance"]["sfxMuted"], "1", "原本就在新位置的值要保留");
        assert_eq!(after["consent"]["hideVersion"], "1", "1.0.6 的同意鍵要對應過來");
        assert_eq!(after["onboarding"]["seenVersion"], "1", "1.0.7 的引導鍵要對應過來");
        assert_eq!(after["localModel"]["installDir"], "D:/models");
        assert!(after["localModel"].get("keepAlive").is_none());
        assert!(after.get("modpack-i18n-theme").is_none(), "舊鍵名不可殘留");
        assert!(after.get("modpack-i18n-keep-local-model-v1").is_none());
        assert_eq!(after["migration"]["version"], MIGRATION_VERSION);

        let text_before = fs::read_to_string(&path).unwrap();
        let second = migrate_settings_file(&path).unwrap();
        assert!(second.already, "第二次必須跳過");
        assert_eq!(fs::read_to_string(&path).unwrap(), text_before, "重跑不得改動檔案");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_value_wins_over_legacy_key() {
        let mut settings = json!({
            "modpack-i18n-theme": "light",
            "appearance": { "theme": "dark" }
        });
        migrate_settings_value(&mut settings);
        assert_eq!(settings["appearance"]["theme"], "dark");
    }

    #[test]
    fn missing_settings_file_is_left_for_frontend_first_run() {
        let dir = temp_dir("missing");
        let path = dir.join("工具設定.json");
        let outcome = migrate_settings_file(&path).unwrap();
        assert_eq!(outcome.moved, 0);
        assert!(!path.exists(), "全新安裝不可由遷移建立設定檔，否則前端不會搬 localStorage");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_session_without_source_hash_loads_and_needs_rescan() {
        let (session, needs_rescan) = read_session_text(LEGACY_SESSION).unwrap();
        assert_eq!(session.pack_name, "OldPack 繁中");
        assert_eq!(session.pending_count, 1);
        assert_eq!(session.review_pass, 0, "舊檔缺欄位用預設值");
        assert!(session.target_version.is_none());
        assert!(needs_rescan, "沒有原文雜湊的舊工作階段必須標記需重掃");

        let mut with_hash: Value = serde_json::from_str(LEGACY_SESSION).unwrap();
        // B6a-1：實際格式是 命名空間 → 鍵 → 英文雜湊（pack_update::SourceHashes）
        with_hash[SESSION_SOURCE_HASH_FIELD] = json!({ "create": { "block.create.gear": "abc123" } });
        let (_, rescan) = read_session_text(&with_hash.to_string()).unwrap();
        assert!(!rescan);
    }

    #[test]
    fn legacy_translation_memory_is_readable() {
        let mut tm = super::super::tm::Tm::load_from(&fixture_path("tm-legacy.json"));
        assert_eq!(tm.get("Diamond Sword").as_deref(), Some("鑽石劍"));
        assert_eq!(tm.get("Iron Ingot").as_deref(), Some("鐵錠"));
        // 不存在的檔案不可讓翻譯流程失敗
        let mut empty = super::super::tm::Tm::load_from(&fixture_path("不存在.json"));
        assert!(empty.get("Diamond Sword").is_none());
    }

    #[test]
    fn legacy_backup_beside_game_folder_is_still_found() {
        // 1.0.x 把備份放在 minecraft 資料夾旁（實例根目錄），不在翻譯結果裡
        let dir = temp_dir("backup");
        let instance = dir.join("OldPack");
        fs::create_dir_all(instance.join("minecraft").join("mods")).unwrap();
        let backup = instance.join("翻譯套用備份_20250101_120000");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("套用清單.json"), LEGACY_APPLY_MANIFEST).unwrap();

        let found = super::super::apply_instance::has_apply_backups_in(&instance, None).unwrap();
        assert!(found, "舊位置的備份必須仍能被找到");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_choice_defaults_to_unset_for_old_settings() {
        let mut settings: Value = serde_json::from_str(LEGACY_SETTINGS).unwrap();
        migrate_settings_value(&mut settings);
        assert_eq!(
            app_settings::backup_choice_from(&settings),
            app_settings::BackupChoice::Unset,
            "舊使用者升級後，第一次套用仍要問一次"
        );
        assert!(
            settings["translate"].get("backupChoice").is_none(),
            "遷移不可替使用者做備份決定"
        );
    }
}
