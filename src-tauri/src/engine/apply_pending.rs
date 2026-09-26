//! 套用中途中斷（寫入失敗、關機、取消）後，那些「紀錄與標記已寫、檔案還沒寫」的檔怎麼認。
//!
//! 前置條件：遊戲檔標記仍是「待確認」、標記與紀錄對得上，而且目前內容等於
//! 上次記下的原檔指紋、或等於隔離區裡保存的那份——才能確定「工具還沒寫過它，它還是上次看到的樣子」。
//! 成立時：
//! - 重跑套用：沿用上次的分類與備份／隔離連結（不重新分類、不把有效備份移入隔離區、
//!   不把舊版翻譯或來源不明的檔當成原檔備份）；連結的另一端壞了、或備份標記記的原檔指紋
//!   不是這次記下的那個，就列為無法確認、不覆蓋。
//! - 移除翻譯：照實說「這次還沒被翻譯寫入，保持原樣」，不說成被改過。

use std::fs;
use std::path::{Path, PathBuf};

use super::apply_guard::{self, Ctx, QuarantineMarker, QUARANTINE_MARKER_SUFFIX, STATE_VALID};
use super::apply_record::{self, ApplyRecord, FileKind, Origin};
use super::mcpl_marker::{self as mk, FileMarker};
use super::paths::long_path;

pub const STATE_PENDING: &str = "pending";

/// 上次登記過、但還沒寫入的檔。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unwritten {
    pub kind: FileKind,
    pub origin: Origin,
    pub backup_id: String,
    pub quarantine_id: String,
    /// 上次記下的原檔指紋（寫進新的遊戲檔標記）
    pub original_sha: String,
}

/// 目前內容是不是「上次登記、還沒寫入」的那一份。是就回傳上次的分類與連結。
pub fn unwritten(mc: &Path, record: &ApplyRecord, rel: &str, current: Option<&str>) -> Option<Unwritten> {
    let entry = record.files.get(rel)?;
    let current = current?;
    let marker = mk::read_file_marker(mc, rel)?;
    if marker.state != STATE_PENDING || marker.id.is_empty() || marker.id != entry.marker_id {
        return None;
    }
    let same_as_original = !marker.original_sha256.is_empty() && marker.original_sha256 == current;
    let quarantine_id = mk::linked(&marker.links, mk::REL_QUARANTINE).unwrap_or_else(|| entry.quarantine_id.clone());
    let same_as_quarantined = !same_as_original
        && read_quarantine(mc, &quarantine_id).is_some_and(|(q, _)| q.content_sha256 == current);
    if !same_as_original && !same_as_quarantined {
        return None;
    }
    Some(Unwritten {
        kind: entry.kind,
        origin: entry.origin,
        backup_id: mk::linked(&marker.links, mk::REL_BACKUP).unwrap_or_else(|| entry.backup_id.clone()),
        quarantine_id,
        original_sha: marker.original_sha256.clone(),
    })
}

/// 前置條件：沿用的備份／隔離連結另一端還在、屬本包、指紋相符（備份還要是有效的）。
pub fn require_links_intact(ctx: &Ctx, rel: &str, previous: &Unwritten) -> Result<(), String> {
    if !previous.backup_id.is_empty() {
        let marker = apply_guard::read_backup_marker(&ctx.mc, rel).ok_or("上次的原檔備份標記不見了")?;
        let file_sha = apply_record::file_sha256(&apply_guard::backup_file_path(&ctx.mc, rel));
        if marker.id != previous.backup_id
            || marker.instance_id != ctx.instance_id
            || marker.state != STATE_VALID
            // 備份標記記的原檔，要就是這次登記時記下的那個原檔（不能只看備份自己對得上）
            || marker.original_sha256 != previous.original_sha
            || file_sha.as_deref() != Some(marker.original_sha256.as_str())
        {
            return Err("上次的原檔備份對不上".into());
        }
    }
    if !previous.quarantine_id.is_empty() {
        let (marker, saved) = read_quarantine(&ctx.mc, &previous.quarantine_id).ok_or("上次隔離的檔不見了")?;
        if marker.instance_id != ctx.instance_id
            || apply_record::file_sha256(&saved).as_deref() != Some(marker.content_sha256.as_str())
        {
            return Err("上次隔離的檔對不上".into());
        }
    }
    Ok(())
}

/// 移除翻譯時：這個還沒寫入的檔目前就是上次看到的樣子（工具沒動過它）。
pub fn is_untouched(mc: &Path, record: &ApplyRecord, game: &FileMarker, current: Option<&str>) -> bool {
    game.state == STATE_PENDING && unwritten(mc, record, &game.rel, current).is_some()
}

/// 依隔離標記 id 找隔離區裡的標記與保存的檔。
fn read_quarantine(mc: &Path, id: &str) -> Option<(QuarantineMarker, PathBuf)> {
    if id.is_empty() {
        return None;
    }
    let root = apply_record::quarantine_dir(mc);
    walkdir::WalkDir::new(long_path(&root)).into_iter().filter_map(|e| e.ok()).find_map(|e| {
        let name = e.file_name().to_string_lossy().to_string();
        let stem = name.strip_suffix(QUARANTINE_MARKER_SUFFIX)?;
        let marker: QuarantineMarker = serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok()?;
        let saved = e.path().with_file_name(stem);
        (marker.id == id && saved.is_file()).then_some((marker, saved))
    })
}

/// 不備份模式要先問玩家：沿用的是「確定是原檔、上次也沒備份」的覆蓋。
pub fn overwrites_unprotected(class: &super::apply_knowledge::Class) -> bool {
    matches!(class, super::apply_knowledge::Class::Unwritten(p)
        if p.kind == FileKind::Overwritten && p.origin == Origin::Known && p.backup_id.is_empty() && p.quarantine_id.is_empty())
}
