//! 工具設定檔：跟著工具走的單一設定來源。
//!
//! 背景：改版前所有偏好設定（主題、同意紀錄、備份選項、輸出位置、本地模型
//! 資料夾、新手引導是否看過…共 16 個鍵）都只存在 WebView2 的 `localStorage`。
//! 那是瀏覽器快取——清快取、換機器、重裝就全部消失，而且使用者看不到也改不了。
//!
//! 這裡改成寫成一份實體檔案 `工具設定.json`，放在資料根目錄（預設是執行檔
//! 旁的 `modpack-i18n-data\`，見 `paths::resolve_file`）。前端啟動時先讀這個
//! 檔，讀不到才回退 localStorage 並把內容遷移進來。
//!
//! 硬不變式：**設定檔讀寫失敗絕不能讓工具開不起來**。所有失敗都回傳預設值
//! 或空物件，由前端沿用既有的 localStorage 行為。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;

const SETTINGS_FILE: &str = "工具設定.json";

/// 同一個行程裡所有「讀→改→寫」設定檔的動作都排隊，避免主視窗與設定視窗
/// 同時寫入時，後寫的一方把先寫的一方整份蓋掉。
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 設定檔的實際位置（隨資料根目錄走）。
pub fn settings_path() -> PathBuf {
    super::paths::resolve_file(Path::new(SETTINGS_FILE))
}

/// 讀取整份設定。檔案不存在、內容壞掉、權限不足一律回傳 `null`，
/// 讓前端知道「還沒有設定檔」而去走 localStorage 遷移路徑。
pub fn read_settings() -> serde_json::Value {
    read_settings_health().0
}

/// 讀取結果的三種狀況。前端要能分辨，因為處理方式完全不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsStatus {
    /// 還沒有設定檔（全新安裝）——正常，不必打擾使用者
    Missing,
    /// 讀到而且是合法設定
    Ok,
    /// 檔案在，但不是合法 JSON 物件——**必須告訴使用者**
    Corrupt,
    /// 檔案在，但讀不出來（權限、被鎖住、磁碟錯誤）——**必須告訴使用者**
    Unreadable,
}

impl SettingsStatus {
    fn as_str(self) -> &'static str {
        match self {
            SettingsStatus::Missing => "missing",
            SettingsStatus::Ok => "ok",
            SettingsStatus::Corrupt => "corrupt",
            SettingsStatus::Unreadable => "unreadable",
        }
    }
}

/// 讀取設定並回報健康狀態。
///
/// 為什麼需要這個：舊版讀壞檔只是回 `null`，前端會當成「還沒有設定檔」，
/// 接著把 localStorage 現況寫成新檔——**原本那份壞檔就被蓋掉了**。
/// 使用者手改設定檔打錯一個逗號，結果是所有偏好無聲無息回到預設值，
/// 而且沒有任何訊息可循。
///
/// 現在改成：壞檔先改名成 `工具設定.損毀-<秒數>.json` 留著，
/// 並回報狀態與備份路徑，讓前端明講「設定檔壞了，已備份到這裡」。
pub fn read_settings_health() -> (serde_json::Value, SettingsStatus, Option<PathBuf>, String) {
    let path = settings_path();
    if !path.exists() {
        return (serde_json::Value::Null, SettingsStatus::Missing, None, String::new());
    }
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            return (
                serde_json::Value::Null,
                SettingsStatus::Unreadable,
                None,
                format!("{e}"),
            );
        }
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) if value.is_object() => (value, SettingsStatus::Ok, None, String::new()),
        Ok(_) => {
            let (backup, detail) = quarantine(&path, "內容不是設定物件");
            (serde_json::Value::Null, SettingsStatus::Corrupt, backup, detail)
        }
        Err(e) => {
            let (backup, detail) = quarantine(&path, &format!("第 {} 行附近格式錯誤", e.line()));
            (serde_json::Value::Null, SettingsStatus::Corrupt, backup, detail)
        }
    }
}

/// 給前端用的 JSON 版本。
pub fn read_settings_report() -> serde_json::Value {
    let (value, status, backup, detail) = read_settings_health();
    serde_json::json!({
        "settings": value,
        "status": status.as_str(),
        "path": settings_path().display().to_string(),
        "backup": backup.map(|p| p.display().to_string()),
        "detail": detail,
    })
}

/// 把壞掉的設定檔改名保存，讓下一次寫入不會直接蓋掉它。
/// 改名失敗（檔案被鎖住之類）不是致命問題——照樣回報狀態，只是沒有備份可指。
fn quarantine(path: &Path, why: &str) -> (Option<PathBuf>, String) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path.with_file_name(format!("工具設定.損毀-{stamp}.json"));
    match fs::rename(path, &backup) {
        Ok(()) => (Some(backup), why.to_string()),
        Err(e) => (None, format!("{why}；另存備份也失敗：{e}")),
    }
}

/// 實際寫檔：先寫暫存檔再改名，避免寫到一半斷電留下半份壞檔（呼叫端負責持有 `WRITE_LOCK`）。
///
/// 不再提供「整份覆寫」的公開函式：所有寫入都走 [`patch_settings`] 或 [`update_settings_at`]。
fn write_settings_to(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if !value.is_object() {
        return Err("設定內容必須是物件".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("無法建立設定資料夾：{e}"))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| format!("設定序列化失敗：{e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text.as_bytes()).map_err(|e| format!("寫入設定失敗：{e}"))?;
    // 直接改名覆蓋：Rust 在 Windows 上的 rename 會取代既有檔案（MoveFileEx REPLACE_EXISTING）。
    // 不可先刪原檔——兩步之間當機或改名失敗，設定檔就整份不見了。
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("儲存設定失敗：{e}"));
    }
    Ok(())
}

/// 一筆設定補丁：設定某個路徑的值，或明確刪除它。
///
/// `value` 是 `null`（或沒給）而且沒有 `delete` 時什麼都不做——
/// 前端讀不到值時送來的 null 不可以把使用者原本的設定蓋成空的。
/// 要「清除」「改回每次詢問」一律用 `delete: true`。
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsPatchOp {
    pub path: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub delete: bool,
}

/// 前端可寫入的設定路徑。唯一來源是 `src/core/settings-paths.js` 的 `SETTING_PATHS`，
/// 這裡在編譯期把那個檔案讀進來解析，前後端不會各寫一份而分歧。
const SETTING_PATHS_SOURCE: &str = include_str!("../../../src/core/settings-paths.js");

pub fn allowed_setting_paths() -> &'static [&'static str] {
    static PATHS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    PATHS.get_or_init(|| {
        let start = SETTING_PATHS_SOURCE
            .find("SETTING_PATHS = [")
            .map(|i| i + "SETTING_PATHS = [".len())
            .unwrap_or(SETTING_PATHS_SOURCE.len());
        let block = &SETTING_PATHS_SOURCE[start..];
        let block = &block[..block.find("];").unwrap_or(0)];
        block
            .lines()
            .filter_map(|line| {
                let line = line.trim().trim_end_matches(',');
                line.strip_prefix('"')?.strip_suffix('"')
            })
            .collect()
    })
}

const FORBIDDEN_SEGMENTS: &[&str] = &["__proto__", "constructor", "prototype"];

fn split_path(dotted: &str) -> Result<Vec<&str>, String> {
    let parts: Vec<&str> = dotted.split('.').collect();
    if dotted.trim().is_empty()
        || parts
            .iter()
            .any(|p| p.trim().is_empty() || FORBIDDEN_SEGMENTS.contains(p))
    {
        return Err(format!("設定路徑不正確：「{dotted}」"));
    }
    if !allowed_setting_paths().contains(&dotted) {
        return Err(format!("不允許寫入這個設定：「{dotted}」"));
    }
    Ok(parts)
}

/// 把補丁套到記憶體裡的設定物件上，回傳實際改動的筆數。純函式，方便測試。
/// 整批要嘛全部套用、要嘛完全不動（任何一筆失敗就回錯誤，`base` 保持原樣）。
pub fn apply_patch(base: &mut serde_json::Value, ops: &[SettingsPatchOp]) -> Result<usize, String> {
    for op in ops {
        split_path(&op.path)?;
    }
    let mut draft = if base.is_object() {
        base.clone()
    } else {
        serde_json::json!({ "version": 1 })
    };
    let changed = apply_patch_to(&mut draft, ops)?;
    *base = draft;
    Ok(changed)
}

fn apply_patch_to(base: &mut serde_json::Value, ops: &[SettingsPatchOp]) -> Result<usize, String> {
    let mut changed = 0;
    for op in ops {
        let parts = split_path(&op.path)?;
        let Some((last, parents)) = parts.split_last() else {
            continue;
        };
        if op.delete {
            if remove_at(base, parents, last) {
                changed += 1;
            }
            continue;
        }
        let Some(value) = op.value.as_ref().filter(|v| !v.is_null()) else {
            continue;
        };
        if set_at(base, parents, last, value)? {
            changed += 1;
        }
    }
    Ok(changed)
}

fn remove_at(base: &mut serde_json::Value, parents: &[&str], last: &str) -> bool {
    let mut node = base;
    for part in parents {
        match node.get_mut(*part) {
            Some(next) if next.is_object() => node = next,
            _ => return false,
        }
    }
    node.as_object_mut()
        .map(|obj| obj.remove(last).is_some())
        .unwrap_or(false)
}

/// 中間節點不是物件時拒絕（不覆蓋）：那可能是使用者手改的值，蓋掉就找不回來了。
fn set_at(
    base: &mut serde_json::Value,
    parents: &[&str],
    last: &str,
    value: &serde_json::Value,
) -> Result<bool, String> {
    let mut node = base;
    for part in parents {
        let obj = node
            .as_object_mut()
            .ok_or_else(|| format!("設定檔的「{part}」上一層格式不對，這次沒有儲存"))?;
        let entry = obj
            .entry(part.to_string())
            .or_insert_with(|| serde_json::json!({}));
        if !entry.is_object() {
            return Err(format!("設定檔的「{part}」格式不對，這次沒有儲存，以免蓋掉原本的內容"));
        }
        node = entry;
    }
    let obj = node
        .as_object_mut()
        .ok_or_else(|| "設定檔格式不對，這次沒有儲存".to_string())?;
    if obj.get(last) == Some(value) {
        return Ok(false);
    }
    obj.insert(last.to_string(), value.clone());
    Ok(true)
}

/// 讀取「準備合併」的現檔。檔案不存在＝從空白開始；
/// 檔案在但讀不出來或格式壞掉＝**拒絕寫入**，絕不拿空白蓋掉使用者的設定。
fn read_for_patch(path: &Path) -> Result<serde_json::Value, String> {
    if !path.exists() {
        return Ok(serde_json::json!({ "version": 1 }));
    }
    let text = fs::read_to_string(path)
        .map_err(|e| format!("設定檔暫時打不開，這次沒有儲存，以免蓋掉原本的設定（{e}）"))?;
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) if value.is_object() => Ok(value),
        _ => Err("設定檔內容壞掉了，這次沒有儲存，以免蓋掉原本的設定。請重新開啟工具。".into()),
    }
}

/// 在指定路徑上做「讀現檔 → 合併補丁 → 原子寫回」。
pub fn patch_settings_at(
    path: &Path,
    ops: &[SettingsPatchOp],
) -> Result<serde_json::Value, String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut current = read_for_patch(path)?;
    let changed = apply_patch(&mut current, ops)?;
    if changed > 0 {
        write_settings_to(path, &current)?;
    }
    Ok(current)
}

/// 在同一把鎖裡讀現檔、交給 `edit` 修改，`edit` 回 true 才寫回。
/// 給遷移這類「要改鍵名而不只是設值」的後端工作使用。檔案不存在時不建立。
pub fn update_settings_at<F>(path: &Path, edit: F) -> Result<bool, String>
where
    F: FnOnce(&mut serde_json::Value) -> bool,
{
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if !path.exists() {
        return Ok(false);
    }
    let mut current = read_for_patch(path)?;
    if !edit(&mut current) {
        return Ok(false);
    }
    write_settings_to(path, &current)?;
    Ok(true)
}

/// 依路徑合併寫入工具設定檔。兩個視窗各改各的欄位，不會再互相蓋掉。
pub fn patch_settings(ops: &[SettingsPatchOp]) -> Result<(PathBuf, serde_json::Value), String> {
    let path = settings_path();
    let merged = patch_settings_at(&path, ops)?;
    Ok((path, merged))
}

/// 第一次套用翻譯時要不要備份的長期選擇。B0 先定好欄位與預設值，B1 的套用流程才開始讀。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupChoice {
    /// 還沒選過：第一次套用時要問
    Unset,
    Always,
    Never,
}

/// 設定檔裡的位置。沒有這個欄位＝未設定（首次詢問）；「改回每次詢問」＝刪除它。
#[allow(dead_code)]
pub const BACKUP_CHOICE_PATH: &str = "translate.backupChoice";

#[allow(dead_code)]
pub fn backup_choice_from(settings: &serde_json::Value) -> BackupChoice {
    match settings
        .get("translate")
        .and_then(|t| t.get("backupChoice"))
        .and_then(|v| v.as_str())
    {
        Some("always") => BackupChoice::Always,
        Some("never") => BackupChoice::Never,
        _ => BackupChoice::Unset,
    }
}

#[cfg(test)]
#[path = "app_settings_patch_tests.rs"]
mod patch_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 用假的根目錄測純邏輯：真實的 `settings_path()` 依賴執行檔位置，
    /// 測試裡不能安全覆寫，所以這裡直接驗證讀寫在指定路徑上的行為。
    fn write_to(path: &Path, value: &serde_json::Value) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(path, serde_json::to_string_pretty(value).unwrap()).map_err(|e| e.to_string())
    }

    fn read_from(path: &Path) -> serde_json::Value {
        let Ok(text) = fs::read_to_string(path) else {
            return serde_json::Value::Null;
        };
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) if v.is_object() => v,
            _ => serde_json::Value::Null,
        }
    }

    #[test]
    fn missing_file_reads_as_null_not_error() {
        // 全新安裝：沒有設定檔不是錯誤，前端要能據此走 localStorage 遷移
        let dir = std::env::temp_dir().join(format!("mcpl-settings-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(read_from(&dir.join(SETTINGS_FILE)).is_null());
    }

    #[test]
    fn broken_file_reads_as_null_and_is_not_deleted() {
        // 設定檔壞掉時最重要的是「工具還開得起來」，而不是保住這份設定
        let dir = std::env::temp_dir().join(format!("mcpl-settings-broken-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SETTINGS_FILE);
        fs::write(&path, "{ 這不是 JSON").unwrap();
        assert!(read_from(&path).is_null());
        assert!(path.is_file(), "壞掉的設定檔不該被刪除，使用者可能想自己救");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn roundtrip_preserves_nested_structure() {
        let dir = std::env::temp_dir().join(format!("mcpl-settings-rt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join(SETTINGS_FILE);
        let value = json!({
            "version": 1,
            "appearance": { "theme": "dark" },
            "localModel": { "consented": true, "installDir": "D:\\models" },
            "onboarding": { "seenVersion": "1.0.9", "step": 2 }
        });
        write_to(&path, &value).unwrap();
        let back = read_from(&path);
        assert_eq!(back["appearance"]["theme"], "dark");
        assert_eq!(back["localModel"]["consented"], true);
        assert_eq!(back["onboarding"]["step"], 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_quarantined_not_overwritten() {
        // 舊行為：讀壞檔回 null → 前端當成「沒有設定檔」→ 寫新檔蓋掉原本那份。
        // 使用者手改設定檔少一個逗號，所有偏好無聲無息回到預設值。
        let dir = std::env::temp_dir().join(format!("mcpl-settings-q-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SETTINGS_FILE);
        fs::write(&path, "{ \"theme\": \"dark\", }").unwrap();

        let (backup, detail) = quarantine(&path, "測試");
        let backup = backup.expect("壞檔要改名保存，不能就地被蓋掉");
        assert!(backup.is_file(), "備份檔要真的存在");
        assert!(!path.is_file(), "原路徑要讓出來，下次寫入才不會撞到壞檔");
        assert!(
            backup.file_name().unwrap().to_string_lossy().contains("損毀"),
            "備份檔名要讓使用者一眼看出這是壞掉的那份"
        );
        assert!(!detail.is_empty(), "要有可以顯示給使用者的原因");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn health_report_carries_every_status_the_ui_needs() {
        // 前端要靠 status 分辨「全新安裝」與「設定檔壞了」，兩者處理方式完全相反
        let report = read_settings_report();
        let status = report["status"].as_str().unwrap_or_default();
        assert!(
            ["missing", "ok", "corrupt", "unreadable"].contains(&status),
            "未知狀態：{status}"
        );
        assert!(report["path"].as_str().is_some_and(|p| !p.is_empty()));
        assert!(report.get("settings").is_some());
    }

    #[test]
    fn non_object_is_rejected() {
        let path = std::env::temp_dir().join(format!("mcpl-settings-nonobj-{}.json", std::process::id()));
        assert!(write_settings_to(&path, &json!("字串不是設定")).is_err());
        assert!(write_settings_to(&path, &json!([1, 2, 3])).is_err());
        assert!(!path.exists());
    }
}
