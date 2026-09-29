//! B5c 審查 1：套用前確認「這份翻譯結果屬於這個遊戲資料夾」。
//!
//! 結果的歸屬讀自兩處（任一有就用）：產出清單記下的遊戲資料夾（text_sources::game_root），
//! 以及工作階段的 instance_path。路徑鍵沿用 apply_record::normalized_key_source（大小寫、分隔符、
//! 長路徑前綴一致）。舊版結果兩處都沒有 → 照現況放行，但回一句註記（B6b 會加識別碼）。

use std::path::{Path, PathBuf};

use super::apply_record::normalized_key_source;
use super::jar_scan::resolve_minecraft_dir;

/// 比對結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    /// 屬於這個遊戲資料夾
    Matches,
    /// 舊版結果沒有歸屬紀錄（放行並註記）
    Unknown,
    /// 屬於別的遊戲資料夾
    Other(PathBuf),
}

fn key(path: &Path) -> String {
    let mc = resolve_minecraft_dir(path).unwrap_or_else(|_| path.to_path_buf());
    normalized_key_source(&mc)
}

fn session_instance(output: &Path) -> Option<PathBuf> {
    let file = super::session::find_session_file(output)?;
    let text = std::fs::read_to_string(super::paths::long_path(&file)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let raw = value.get("instancePath").or_else(|| value.get("instance_path"))?.as_str()?.trim().to_string();
    (!raw.is_empty()).then(|| PathBuf::from(raw))
}

/// 結果記下的遊戲資料夾（產出清單優先，其次工作階段）。
pub fn recorded_owner(output: &Path) -> Option<PathBuf> {
    let mut works = vec![output.to_path_buf(), output.join(super::out_layout::RESULT_DIR_NAME)];
    works.dedup();
    works
        .iter()
        .find_map(|w| super::text_sources::game_root(w))
        .or_else(|| session_instance(output))
}

pub fn ownership(instance: &Path, output: &Path) -> Ownership {
    match recorded_owner(output) {
        None => Ownership::Unknown,
        Some(owner) if key(&owner) == key(instance) => Ownership::Matches,
        Some(owner) => Ownership::Other(owner),
    }
}

fn leaf(path: &Path) -> String {
    let mc = path.to_string_lossy().to_string();
    let parts: Vec<&str> = mc.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    // `.minecraft`／`minecraft` 本身不是包名，取上一層
    match parts.as_slice() {
        [.., parent, last] if last.eq_ignore_ascii_case(".minecraft") || last.eq_ignore_ascii_case("minecraft") => (*parent).to_string(),
        [.., last] => (*last).to_string(),
        [] => mc,
    }
}

/// 套用前守門：屬於別包 → 錯誤（白話、零寫入）；沒有紀錄 → 放行並回註記。
pub fn guard(instance: &Path, output: &Path) -> Result<Option<String>, String> {
    match ownership(instance, output) {
        Ownership::Matches => Ok(None),
        Ownership::Unknown => Ok(Some(
            "這份翻譯結果是舊版工具做的，沒有記下屬於哪個模組整合包；已照你選的資料夾套用。".to_string(),
        )),
        Ownership::Other(owner) => Err(format!(
            "這份翻譯結果屬於「{}」，不是「{}」，沒有套用（一個檔都沒動）。請改選那個模組整合包，或在這個模組整合包重新翻譯。",
            leaf(&owner),
            leaf(instance)
        )),
    }
}

/// B5c 審查 3a：這份結果的**最新一輪**有沒有套用到這個遊戲資料夾（唯讀，G1.36）。
///
/// 依據：結果裡的工具資源包（`resourcepacks/*.zip`）內容雜湊，是否等於套用紀錄記下的某個
/// `resourcepacks/` 檔目前的版本（歷史版本＝更早的輪次，不算）。結果裡沒有資源包、或紀錄讀不到 → `None`（不知道，前端照舊）。
pub fn latest_applied(instance: &Path, work: &Path) -> Option<bool> {
    let mc = resolve_minecraft_dir(instance).unwrap_or_else(|_| instance.to_path_buf());
    let dir = work.join("resourcepacks");
    let zips: Vec<String> = std::fs::read_dir(super::paths::long_path(&dir))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")))
        .filter_map(|p| super::apply_record::file_sha256(&p))
        .collect();
    if zips.is_empty() {
        return None;
    }
    let record = super::apply_record::load(&mc).ok()?;
    let applied = record
        .files
        .iter()
        .any(|(rel, file)| rel.starts_with("resourcepacks/") && zips.contains(&file.sha256));
    Some(applied)
}

#[cfg(test)]
#[path = "result_owner_tests.rs"]
mod tests;
