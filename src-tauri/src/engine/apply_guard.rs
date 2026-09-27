//! 前置條件檢查（B1 檔案安全設計，原則一：每個動作都是「必須先確認 X，才能做 Y」）。
//!
//! | 動作 | 檢查函式 | 條件不符時 |
//! |---|---|---|
//! | 建立原檔備份 | [`require_original_backup`] | 舊備份過期／用過／不屬本包 → 先移入隔離區，再為目前原檔建新備份 |
//! | 覆蓋來源不明的檔 | [`require_quarantined`] | 先複製進隔離區（附標記）才覆蓋 |
//! | 使用 1.0.x 舊備份 | [`require_legacy_owned`]、[`require_legacy_claimed`] | 清單不屬本包就不用、不刪；屬本包就複製進備份區並補標記 |
//! | 寫入遊戲檔、改設定檔 | [`require_record_and_markers`] | 先存紀錄與標記，存不了就一個檔都不寫 |
//! | 處理任何已記錄的檔 | [`require_game_marker`] | 標記缺一端能重建就重建（這次不動檔），否則列為無法確認 |
//! | 還原備份 | [`require_restorable`] | 同上；備份用過、指紋不符、互指不一致 → 不動 |
//! | 刪除工具新增檔 | [`require_deletable`] | 不是工具新增或來源不明 → 不刪 |
//! | 刪除備份 | [`require_backup_owned`] | 標記不屬本包或缺標記 → 不刪 |

use std::fs;
use std::path::{Path, PathBuf};

use super::apply_record::{self, ApplyRecord, Batch};
use super::mcpl_marker::{self as mk, FileMarker, Link};
use super::paths::long_path;

const BACKUP_MARKER_SUFFIX: &str = ".mcpl-backup.json";
pub const QUARANTINE_MARKER_SUFFIX: &str = ".mcpl-quarantine.json";
pub const STATE_VALID: &str = "valid";
pub const STATE_USED: &str = "used";

/// 備份區裡每個備份檔旁的標記。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupMarker {
    pub id: String,
    pub instance_id: String,
    pub rel: String,
    pub original_sha256: String,
    /// "valid"＝可用；"used"＝已經還原過，不能再用
    pub state: String,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub created_at: u64,
    /// 認領自 1.0.x 舊備份時：舊版清單的位置與指紋
    #[serde(default)]
    pub legacy_manifest: String,
    #[serde(default)]
    pub legacy_manifest_sha256: String,
}

/// 隔離區裡每個檔旁的標記（不拿來自動還原）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineMarker {
    pub id: String,
    pub instance_id: String,
    pub rel: String,
    pub content_sha256: String,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub created_at: u64,
}

/// 一次動作的環境：遊戲資料夾、整合包識別碼、這次的批次 id。
pub struct Ctx {
    pub mc: PathBuf,
    pub instance_id: String,
    pub batch_id: String,
}

impl Ctx {
    /// 前置條件：遊戲資料夾要有整合包識別碼（沒有就建立，舊資料一併遷移）。
    pub fn begin(mc: &Path) -> Result<Self, String> {
        let (instance_id, _) = apply_record::ensure_instance(mc)?;
        Ok(Self { mc: mc.to_path_buf(), instance_id, batch_id: mk::new_marker_id() })
    }

    /// 只讀：已有識別碼才回傳（移除、刪備份不替遊戲資料夾建立新身分）。
    pub fn existing(mc: &Path) -> Result<Option<Self>, String> {
        Ok(mk::read_instance(mc)?.map(|info| Self {
            mc: mc.to_path_buf(),
            instance_id: info.id,
            batch_id: mk::new_marker_id(),
        }))
    }
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(long_path(parent)).map_err(|e| format!("無法建立資料夾（{e}）：{}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    apply_record::write_atomic(path, json.as_bytes()).map_err(|e| format!("無法寫入標記（{e}）：{}", path.display()))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(long_path(path)).ok()?).ok()
}

pub fn backup_marker_path(mc: &Path, rel: &str) -> PathBuf {
    apply_record::instance_backup_dir(mc).join(format!("{rel}{BACKUP_MARKER_SUFFIX}"))
}

pub fn backup_file_path(mc: &Path, rel: &str) -> PathBuf {
    apply_record::instance_backup_dir(mc).join(rel)
}

/// B3 審查 F-c：指定備份區裡某個原檔備份的（檔案, 標記）位置（`.mcpl` 被刪時用認回的識別碼找）。
pub fn backup_paths_at(backup_dir: &Path, rel: &str) -> (PathBuf, PathBuf) {
    (backup_dir.join(rel), backup_dir.join(format!("{rel}{BACKUP_MARKER_SUFFIX}")))
}

pub fn read_backup_marker(mc: &Path, rel: &str) -> Option<BackupMarker> {
    read_json(&backup_marker_path(mc, rel))
}

/// 目前這份原檔已有「有效、屬本包、指紋相符」的備份嗎？有就回傳它的標記。
fn usable_backup_of(ctx: &Ctx, rel: &str, current_sha: &str) -> Option<BackupMarker> {
    let marker = read_backup_marker(&ctx.mc, rel)?;
    let file_sha = apply_record::file_sha256(&backup_file_path(&ctx.mc, rel))?;
    (marker.instance_id == ctx.instance_id
        && marker.state == STATE_VALID
        && marker.original_sha256 == current_sha
        && file_sha == current_sha)
        .then_some(marker)
}

/// 把備份區裡同路徑的舊備份（過期、用過、不屬本包、缺標記）移進隔離區——絕不直接刪。
fn move_stale_backup_to_quarantine(ctx: &Ctx, rel: &str) -> Result<(), String> {
    let file = backup_file_path(&ctx.mc, rel);
    let marker = backup_marker_path(&ctx.mc, rel);
    let dest_root = apply_record::quarantine_dir(&ctx.mc).join(&ctx.batch_id).join("舊備份");
    for source in [&file, &marker] {
        if !long_path(source).is_file() {
            continue;
        }
        let name = source
            .strip_prefix(apply_record::instance_backup_dir(&ctx.mc))
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| PathBuf::from(source.file_name().unwrap_or_default()));
        let dest = dest_root.join(name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(long_path(parent)).map_err(|e| e.to_string())?;
        }
        fs::rename(long_path(source), long_path(&dest))
            .or_else(|_| apply_record::copy_atomic(source, &dest).and_then(|_| fs::remove_file(long_path(source))))
            .map_err(|e| format!("無法把舊備份移到隔離區（{e}）：{}", source.display()))?;
    }
    Ok(())
}

/// 前置條件：覆蓋「確定是原檔」的檔之前，備份區要有**目前這份原檔**的有效備份。
/// 已有且指紋相符 → 沿用並把它指向新的遊戲檔標記；否則舊的移入隔離區、為目前原檔建立新備份。
/// 回傳備份標記 id。任何一步失敗都回錯，呼叫端必須停止套用。
pub fn require_original_backup(ctx: &Ctx, target: &Path, rel: &str, game_marker_id: &str) -> Result<String, String> {
    let current = apply_record::file_sha256(target)
        .ok_or_else(|| format!("讀不到要備份的檔案，已停止套用：{}", target.display()))?;
    if let Some(mut marker) = usable_backup_of(ctx, rel, &current) {
        mk::set_link(&mut marker.links, mk::REL_GAME_FILE, game_marker_id);
        write_json(&backup_marker_path(&ctx.mc, rel), &marker)?;
        return Ok(marker.id);
    }
    move_stale_backup_to_quarantine(ctx, rel)?;
    let dest = backup_file_path(&ctx.mc, rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(long_path(parent)).map_err(|e| backup_error(target, &e))?;
    }
    apply_record::copy_atomic(target, &dest).map_err(|e| backup_error(target, &e))?;
    let marker = BackupMarker {
        id: mk::new_marker_id(),
        instance_id: ctx.instance_id.clone(),
        rel: rel.to_string(),
        original_sha256: current,
        state: STATE_VALID.into(),
        links: vec![mk::link(game_marker_id, mk::REL_GAME_FILE), mk::link(&ctx.batch_id, mk::REL_BATCH)],
        created_at: mk::now_secs(),
        legacy_manifest: String::new(),
        legacy_manifest_sha256: String::new(),
    };
    write_json(&backup_marker_path(&ctx.mc, rel), &marker)?;
    Ok(marker.id)
}

fn backup_error(target: &Path, error: &dyn std::fmt::Display) -> String {
    format!(
        "備份 {} 失敗：{error}\n為了不讓原始檔案在沒有備份的情況下被覆蓋，已停止套用。\
常見原因是遊戲還開著把檔案鎖住，或磁碟空間不足。",
        target.display()
    )
}

/// 前置條件：覆蓋來源不明的檔之前，先把它複製進隔離區（附標記）。回傳隔離標記 id。
pub fn require_quarantined(ctx: &Ctx, target: &Path, rel: &str, game_marker_id: &str) -> Result<String, String> {
    let sha = apply_record::file_sha256(target)
        .ok_or_else(|| format!("讀不到要移入隔離區的檔案，已停止套用：{}", target.display()))?;
    let dest = apply_record::quarantine_dir(&ctx.mc).join(&ctx.batch_id).join(rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(long_path(parent)).map_err(|e| e.to_string())?;
    }
    apply_record::copy_atomic(target, &dest)
        .map_err(|e| format!("無法把檔案移入隔離區，已停止套用（{e}）：{}", target.display()))?;
    let marker = QuarantineMarker {
        id: mk::new_marker_id(),
        instance_id: ctx.instance_id.clone(),
        rel: rel.to_string(),
        content_sha256: sha,
        links: vec![mk::link(game_marker_id, mk::REL_GAME_FILE), mk::link(&ctx.batch_id, mk::REL_BATCH)],
        created_at: mk::now_secs(),
    };
    let marker_path = dest.with_file_name(format!(
        "{}{QUARANTINE_MARKER_SUFFIX}",
        dest.file_name().unwrap_or_default().to_string_lossy()
    ));
    write_json(&marker_path, &marker)?;
    Ok(marker.id)
}

/// 前置條件：1.0.x 舊備份要能用，它的清單必須存在，而且清單記的遊戲資料夾就是這一個。
/// 沒有清單、或是別的整合包的清單 → 不用、不刪（CurseForge 會把多個整合包放在同一個上層資料夾）。
pub fn require_legacy_owned(mc: &Path, manifest_mc_dir: Option<&str>) -> bool {
    match manifest_mc_dir {
        Some(dir) if !dir.trim().is_empty() => {
            super::apply_knowledge::path_key(Path::new(dir)) == super::apply_knowledge::path_key(mc)
        }
        _ => false,
    }
}

/// 前置條件：使用 1.0.x 舊備份前先認領——複製進本包備份區並補寫標記（連到舊版清單位置與指紋）。
/// 回傳備份標記 id。
pub fn require_legacy_claimed(
    ctx: &Ctx,
    rel: &str,
    legacy_original: &Path,
    manifest_dir: &Path,
    game_marker_id: &str,
) -> Result<String, String> {
    let sha = apply_record::file_sha256(legacy_original)
        .ok_or_else(|| format!("讀不到舊版備份：{}", legacy_original.display()))?;
    if let Some(mut marker) = usable_backup_of(ctx, rel, &sha) {
        mk::set_link(&mut marker.links, mk::REL_GAME_FILE, game_marker_id);
        write_json(&backup_marker_path(&ctx.mc, rel), &marker)?;
        return Ok(marker.id);
    }
    move_stale_backup_to_quarantine(ctx, rel)?;
    let dest = backup_file_path(&ctx.mc, rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(long_path(parent)).map_err(|e| e.to_string())?;
    }
    apply_record::copy_atomic(legacy_original, &dest)
        .map_err(|e| format!("無法認領舊版備份（{e}）：{}", legacy_original.display()))?;
    let manifest = manifest_dir.join(super::apply_knowledge::APPLY_MANIFEST);
    let marker = BackupMarker {
        id: mk::new_marker_id(),
        instance_id: ctx.instance_id.clone(),
        rel: rel.to_string(),
        original_sha256: sha,
        state: STATE_VALID.into(),
        links: vec![
            mk::link(game_marker_id, mk::REL_GAME_FILE),
            mk::link(&ctx.batch_id, mk::REL_BATCH),
            mk::link(&manifest.display().to_string(), mk::REL_LEGACY),
        ],
        created_at: mk::now_secs(),
        legacy_manifest: manifest.display().to_string(),
        legacy_manifest_sha256: apply_record::file_sha256(&manifest).unwrap_or_default(),
    };
    write_json(&backup_marker_path(&ctx.mc, rel), &marker)?;
    Ok(marker.id)
}

/// 前置條件：動遊戲檔、改設定檔之前，先存好紀錄（含這次批次）與所有標記。存不了就回錯。
pub fn require_record_and_markers(
    ctx: &Ctx,
    record: &mut ApplyRecord,
    markers: &[FileMarker],
    options: Option<&mk::OptionsMarker>,
    extra_ids: &[String],
) -> Result<(), String> {
    let mut ids: Vec<String> = markers.iter().map(|m| m.id.clone()).collect();
    ids.extend(extra_ids.iter().cloned());
    record.batches.retain(|b| b.id != ctx.batch_id);
    record.batches.push(Batch { id: ctx.batch_id.clone(), at: mk::now_secs(), markers: ids });
    apply_record::save(&ctx.mc, record)?;
    for marker in markers {
        mk::write_file_marker(&ctx.mc, marker)?;
    }
    if let Some(options) = options {
        mk::write_options_marker(&ctx.mc, options)?;
    }
    Ok(())
}

/// 檢查結果：可以做／標記已補回（這次不動檔）／無法確認（不動檔）。
#[derive(Debug, Clone)]
pub enum Check<T> {
    Ready(T),
    Repaired(String),
    Broken(String),
}

fn batch_lists(record: &ApplyRecord, batch_id: &str, marker_id: &str) -> bool {
    record
        .batches
        .iter()
        .any(|b| b.id == batch_id && b.markers.iter().any(|m| m == marker_id))
}

/// 前置條件：處理一個已記錄的檔之前，遊戲檔標記與紀錄、批次要互相對得上。
/// 標記不見但紀錄還在 → 從紀錄（與指回它的備份標記）重建標記，這次不動檔。
pub fn require_game_marker(ctx: &Ctx, record: &ApplyRecord, rel: &str) -> Check<FileMarker> {
    let Some(entry) = record.files.get(rel) else {
        return Check::Broken("套用紀錄裡沒有這個檔".into());
    };
    match mk::read_file_marker(&ctx.mc, rel) {
        Some(marker) => {
            let batch = mk::linked(&marker.links, mk::REL_BATCH);
            let batch_ok = batch.as_deref().is_some_and(|b| batch_lists(record, b, &marker.id));
            if marker.id != entry.marker_id || marker.instance_id != ctx.instance_id || !batch_ok {
                return Check::Broken("遊戲檔標記與套用紀錄對不上".into());
            }
            Check::Ready(marker)
        }
        None => {
            if entry.marker_id.is_empty() {
                return Check::Broken("遊戲檔標記不見了，紀錄也沒有它的 id".into());
            }
            let Some(batch) = record.batches.iter().find(|b| b.markers.contains(&entry.marker_id)) else {
                return Check::Broken("遊戲檔標記不見了，也找不到它屬於哪一次套用".into());
            };
            let mut links = vec![mk::link(&batch.id, mk::REL_BATCH)];
            let mut original = String::new();
            if !entry.backup_id.is_empty() {
                match read_backup_marker(&ctx.mc, rel) {
                    Some(b) if b.id == entry.backup_id && mk::links_to(&b.links, mk::REL_GAME_FILE, &entry.marker_id) => {
                        original = b.original_sha256.clone();
                        links.push(mk::link(&b.id, mk::REL_BACKUP));
                    }
                    _ => return Check::Broken("遊戲檔標記不見了，備份那一端也對不上".into()),
                }
            }
            if !entry.quarantine_id.is_empty() {
                links.push(mk::link(&entry.quarantine_id, mk::REL_QUARANTINE));
            }
            let marker = FileMarker {
                id: entry.marker_id.clone(),
                instance_id: ctx.instance_id.clone(),
                rel: rel.to_string(),
                role: match entry.kind {
                    apply_record::FileKind::Added => "added".into(),
                    apply_record::FileKind::Overwritten => "overwritten".into(),
                },
                tool_sha256: entry.sha256.clone(),
                original_sha256: original,
                backup: String::new(),
                tool_version: env!("CARGO_PKG_VERSION").into(),
                written_at: mk::now_secs(),
                origin: match entry.origin {
                    apply_record::Origin::Known => "known".into(),
                    apply_record::Origin::Unknown => "unknown".into(),
                },
                links,
                tool_history: entry.history.clone(),
                state: "written".into(),
            };
            match mk::write_file_marker(&ctx.mc, &marker) {
                Ok(()) => Check::Repaired("遊戲檔標記不見了，已從套用紀錄補回".into()),
                Err(e) => Check::Broken(e),
            }
        }
    }
}

/// 前置條件：還原備份——備份標記屬本包、狀態有效、兩端互指、內容指紋相符
/// （遊戲裡目前內容等於工具版本由呼叫端先確認）。
pub fn require_restorable(ctx: &Ctx, game: &FileMarker) -> Check<(PathBuf, BackupMarker)> {
    let Some(backup_id) = mk::linked(&game.links, mk::REL_BACKUP) else {
        return Check::Broken("沒有備份".into());
    };
    let rel = &game.rel;
    let file = backup_file_path(&ctx.mc, rel);
    let file_sha = apply_record::file_sha256(&file);
    match read_backup_marker(&ctx.mc, rel) {
        None => {
            // 備份標記不見：備份檔還在、指紋等於遊戲檔標記記的原檔指紋，才從另一端補回
            if !game.original_sha256.is_empty() && file_sha.as_deref() == Some(game.original_sha256.as_str()) {
                let marker = BackupMarker {
                    id: backup_id,
                    instance_id: ctx.instance_id.clone(),
                    rel: rel.clone(),
                    original_sha256: game.original_sha256.clone(),
                    state: STATE_VALID.into(),
                    links: vec![mk::link(&game.id, mk::REL_GAME_FILE)],
                    created_at: mk::now_secs(),
                    legacy_manifest: String::new(),
                    legacy_manifest_sha256: String::new(),
                };
                return match write_json(&backup_marker_path(&ctx.mc, rel), &marker) {
                    Ok(()) => Check::Repaired("備份標記不見了，已從遊戲檔標記補回".into()),
                    Err(e) => Check::Broken(e),
                };
            }
            Check::Broken("遊戲檔標記指向的備份不存在".into())
        }
        Some(marker) => {
            if marker.id != backup_id || !mk::links_to(&marker.links, mk::REL_GAME_FILE, &game.id) {
                return Check::Broken("備份標記與遊戲檔標記互指不一致".into());
            }
            if marker.instance_id != ctx.instance_id {
                return Check::Broken("備份不屬於這個整合包".into());
            }
            if marker.state != STATE_VALID {
                return Check::Broken("備份已經用過".into());
            }
            let Some(sha) = file_sha else {
                return Check::Broken("備份檔不見了".into());
            };
            if sha != marker.original_sha256 || marker.original_sha256 != game.original_sha256 {
                return Check::Broken("備份內容的指紋對不上".into());
            }
            Check::Ready((file, marker))
        }
    }
}

/// 還原後把備份標成「已用過」：同一份備份不會再被拿來還原第二次。
pub fn mark_backup_used(ctx: &Ctx, marker: &BackupMarker) -> Result<(), String> {
    let mut used = marker.clone();
    used.state = STATE_USED.into();
    write_json(&backup_marker_path(&ctx.mc, &marker.rel), &used)
}

/// 前置條件：刪除工具新增檔——標記是「工具新增」而且來源確定（內容指紋由呼叫端先比對）。
pub fn require_deletable(game: &FileMarker) -> bool {
    game.role == "added" && game.origin != "unknown"
}

/// 前置條件：刪除備份——備份標記存在且屬於本整合包。
pub fn require_backup_owned(ctx: &Ctx, rel: &str) -> bool {
    read_backup_marker(&ctx.mc, rel).is_some_and(|m| m.instance_id == ctx.instance_id)
}

/// 備份區裡所有備份檔（不含標記本身），相對路徑。
pub fn backup_rels(mc: &Path) -> Vec<String> {
    let root = apply_record::instance_backup_dir(mc);
    let long_root = long_path(&root);
    walkdir::WalkDir::new(&long_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(&long_root).ok()?.to_string_lossy().replace('\\', "/");
            (!rel.ends_with(BACKUP_MARKER_SUFFIX)).then_some(rel)
        })
        .collect()
}

pub fn delete_backup(mc: &Path, rel: &str) -> Result<(), String> {
    for path in [backup_file_path(mc, rel), backup_marker_path(mc, rel)] {
        match fs::remove_file(long_path(&path)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}：{e}", path.display())),
        }
    }
    Ok(())
}

/// 刪完備份後把空掉的資料夾收掉（有東西的不動）。
pub fn remove_empty_backup_dirs(mc: &Path) {
    let root = apply_record::instance_backup_dir(mc);
    let mut dirs: Vec<PathBuf> = walkdir::WalkDir::new(long_path(&root))
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.path().to_path_buf())
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in dirs {
        let _ = fs::remove_dir(&dir);
    }
    if let Some(container) = root.parent() {
        let _ = fs::remove_dir(long_path(container));
    }
}
