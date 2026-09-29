//! 套用紀錄、原檔備份區、隔離區的位置與讀寫（全部以「整合包識別碼」為鍵，存在工具資料夾裡）。
//!
//! 整合包識別碼來自遊戲資料夾裡的 `.mcpl/instance.json`（見 mcpl_marker.rs）；
//! 舊的「遊戲資料夾路徑雜湊」只用於一次性遷移。
//! - 套用紀錄：`<工具資料>/apply-records/<識別碼>/套用紀錄.json`，一律寫（不論有沒有備份）。
//! - 原檔備份：`<工具資料>/apply-backups/<識別碼>/翻譯套用備份_原檔/`，每個備份檔旁有標記。
//! - 隔離區：`<工具資料>/apply-quarantine/<識別碼>/`，來源不明的檔被覆蓋前先移到這裡。
//!
//! 備份、隔離、還原、刪除的前置條件檢查在 apply_guard.rs。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::app_settings::{self, BackupChoice};
use super::hashutil::{sha256_hex, Sha256Hasher};
use super::paths::long_path;

pub const RECORD_FILE: &str = "套用紀錄.json";
/// 名稱沿用「翻譯套用備份_」開頭，既有的「刪除全部備份」「有沒有備份」都認得。
pub const INSTANCE_BACKUP_DIR: &str = "翻譯套用備份_原檔";
const RECORD_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    /// 套用前不存在 → 移除翻譯時刪掉
    Added,
    /// 套用前就有 → 移除翻譯時從備份還原
    Overwritten,
}

/// 被工具寫入之前，那個位置原本是什麼（決定移除翻譯時能不能刪／蓋回）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// 確定：新增的原本不存在；覆蓋的原檔在備份區（或當初沒備份＝無法還原）
    #[default]
    Known,
    /// 來源不明：無法確定原本是不是工具改過的內容，移除翻譯時不刪也不蓋
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedFile {
    pub kind: FileKind,
    /// 工具最後一次寫入的內容雜湊；用來判斷檔案是不是還是工具寫的版本
    pub sha256: String,
    #[serde(default)]
    pub origin: Origin,
    /// 原檔在舊版（1.0.x）備份裡的位置（新的備份區沒有時用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_backup: Option<String>,
    /// 工具以前寫過的其他版本雜湊（重跑多次時，舊版本也認得是工具寫的）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<String>,
    /// 對應的遊戲檔標記 id（`.mcpl/files/<相對路徑>.json`）
    #[serde(default)]
    pub marker_id: String,
    /// 原檔備份標記 id（沒有備份為空）
    #[serde(default)]
    pub backup_id: String,
    /// 隔離區標記 id（沒有移入隔離區為空）
    #[serde(default)]
    pub quarantine_id: String,
    /// 哪個功能放的：空＝翻譯；OWNER_FONT＝字體包（「移除翻譯」與「移除字體包」各管各的）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
}

pub const OWNER_FONT: &str = "font";
/// 修復資源包清單時建立的設定檔旁另存檔（不屬於翻譯，「移除翻譯」不動它）
pub const OWNER_REPAIR: &str = "repair";

/// 一次套用（批次）：這次寫的所有標記 id。標記也各自指回批次 id。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub id: String,
    #[serde(default)]
    pub at: u64,
    #[serde(default)]
    pub markers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsRecord {
    /// 工具有沒有改過語言（原本就是繁中時為 false）
    #[serde(default)]
    pub lang_changed: bool,
    /// 改之前的語言；`None`＝原本沒有 `lang:` 這行
    #[serde(default)]
    pub original_lang: Option<String>,
    /// 工具加進資源包清單的項目（原本就在清單的不算）
    #[serde(default)]
    pub packs_added: Vec<String>,
    /// 字體工具加進資源包清單的項目（由「移除字體包」處理）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_packs_added: Vec<String>,
    /// 語言設定標記的 id 與它那次套用的批次 id（標記不見時照這兩個重建，才連得回批次）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lang_marker_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lang_batch: String,
}

impl OptionsRecord {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        !self.lang_changed && self.packs_added.is_empty() && self.font_packs_added.is_empty()
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRecord {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub mc_dir: String,
    #[serde(default)]
    pub updated_at: u64,
    /// 相對遊戲資料夾的路徑（`/` 分隔）→ 狀態
    #[serde(default)]
    pub files: BTreeMap<String, RecordedFile>,
    #[serde(default)]
    pub options: OptionsRecord,
    /// 這份紀錄開始時，之前的紀錄已遺失或被重設：不在紀錄上的檔一律視為來源不明
    #[serde(default)]
    pub started_uncertain: bool,
    /// 舊版（1.0.x）清單提過的檔；刪掉舊備份前記下，之後仍不會把它們當原檔
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub legacy_touched: BTreeSet<String>,
    /// 舊版清單上、已經由「移除翻譯」處理完的檔（之後不再重複列出）
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub legacy_done: BTreeSet<String>,
    /// 歷次套用（批次）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub batches: Vec<Batch>,
    /// 玩家按過「刪除備份」：之後被覆蓋的檔無法還原，移除時要照實說
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub backups_deleted: bool,
    /// 識別碼標記被刪後認回時，指紋對不上的檔：無法確認，不當原檔備份、移除時不動
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unconfirmed: BTreeSet<String>,
}

impl ApplyRecord {
    /// 紀錄完全是空的（測試用；判斷「有沒有翻譯可移除」請用 [`Self::has_translation`]）
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.options.is_empty()
    }

    /// 有沒有「翻譯」放進遊戲的東西（字體包、修復清單的另存檔不算）。
    pub fn has_translation(&self) -> bool {
        self.files.values().any(|e| e.owner.is_empty())
            || !self.options.packs_added.is_empty()
            || self.options.lang_changed
    }

    /// 記下（或更新）一個工具寫入的檔。舊的雜湊留在歷史裡，之後仍認得是工具寫的。
    pub fn set_entry(
        &mut self,
        rel: &str,
        kind: FileKind,
        origin: Origin,
        legacy_backup: Option<String>,
        sha256: String,
    ) {
        let mut history = Vec::new();
        if let Some(previous) = self.files.get(rel) {
            history = previous.history.clone();
            if previous.sha256 != sha256 && !history.contains(&previous.sha256) {
                history.push(previous.sha256.clone());
            }
            history.retain(|h| h != &sha256);
            if history.len() > 20 {
                history.drain(..history.len() - 20);
            }
        }
        let (marker_id, backup_id, quarantine_id, owner) = self
            .files
            .get(rel)
            .map(|p| (p.marker_id.clone(), p.backup_id.clone(), p.quarantine_id.clone(), p.owner.clone()))
            .unwrap_or_default();
        self.files.insert(
            rel.to_string(),
            RecordedFile { kind, sha256, origin, legacy_backup, history, marker_id, backup_id, quarantine_id, owner },
        );
    }

    /// 工具新增的檔（沿用已有的分類與來源）。給設定檔旁備份、字體包這類工具自己的檔用。
    #[cfg(test)]
    pub fn note_tool_file(&mut self, rel: &str, existed_before: bool, sha256: String) {
        let (kind, origin, legacy) = match self.files.get(rel) {
            Some(entry) => (entry.kind, entry.origin, entry.legacy_backup.clone()),
            None if existed_before => (FileKind::Overwritten, Origin::Unknown, None),
            None => (FileKind::Added, Origin::Known, None),
        };
        self.set_entry(rel, kind, origin, legacy, sha256);
    }

    /// 目前的檔案內容是不是工具寫入過的版本（最新或歷史）。
    pub fn is_tool_version(&self, rel: &str, current_sha: Option<&str>) -> bool {
        match (self.files.get(rel), current_sha) {
            (Some(entry), Some(sha)) => entry.sha256 == sha || entry.history.iter().any(|h| h == sha),
            _ => false,
        }
    }
}

/// 套用時的備份做法（由設定決定，見 [`policy_for_run`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupPolicy {
    /// 覆蓋前先備份原檔
    Backup,
    /// 不備份；`overwrite_confirmed`＝玩家這次已確認可以直接覆蓋
    NoBackup { overwrite_confirmed: bool },
    /// 還沒選過：先問玩家，這次不寫任何檔
    Ask,
}

/// 決定這次套用的備份做法：照設定檔的 `translate.backupChoice`，沒選過就先問。
pub fn policy_for_run(overwrite_confirmed: bool) -> BackupPolicy {
    policy_for_run_with(
        app_settings::backup_choice_from(&app_settings::read_settings()),
        true,
        overwrite_confirmed,
    )
}

/// 純邏輯版（可測）。`_keep_results`＝開始翻譯時「保留／不保留翻譯結果」的選擇，
/// 刻意不影響備份：那個選項只管結果資料夾；備份一律照設定（第一次問、之後照選擇、
/// 不備份時每次覆蓋原檔前確認）。參數留著是為了讓測試釘住「兩者無關」。
pub fn policy_for_run_with(
    choice: BackupChoice,
    _keep_results: bool,
    overwrite_confirmed: bool,
) -> BackupPolicy {
    policy_from_choice(choice, overwrite_confirmed)
}

pub fn policy_from_choice(choice: BackupChoice, overwrite_confirmed: bool) -> BackupPolicy {
    match choice {
        BackupChoice::Always => BackupPolicy::Backup,
        BackupChoice::Never => BackupPolicy::NoBackup { overwrite_confirmed },
        BackupChoice::Unset => BackupPolicy::Ask,
    }
}

// ─── 位置 ───────────────────────────────────────────────

pub(super) fn store_root() -> PathBuf {
    #[cfg(test)]
    {
        // 測試不寫進執行檔旁的真實資料夾。T1：放在這一輪專用的測試資料夾（行程編號＋啟動時間），
        // 只用行程編號時 Windows 重用編號會讀到上一輪留下的紀錄，認回判斷變成「兩份對得上」而偶發失敗。
        return super::paths::test_data_base().join("apply-store");
    }
    #[allow(unreachable_code)]
    super::paths::active_root()
}

/// 遊戲資料夾正規化後的字串（大小寫、斜線、長路徑前綴都不影響）。
pub fn normalized_key_source(mc: &Path) -> String {
    let resolved = fs::canonicalize(mc).unwrap_or_else(|_| mc.to_path_buf());
    let mut text = resolved.to_string_lossy().replace('\\', "/");
    if let Some(rest) = text.strip_prefix("//?/UNC/") {
        text = format!("//{rest}");
    } else if let Some(rest) = text.strip_prefix("//?/") {
        text = rest.to_string();
    }
    text.trim_end_matches('/').to_lowercase()
}

/// 舊的鍵（遊戲資料夾路徑雜湊）：只用於遷移，以及還沒建立識別碼時的位置。
pub fn legacy_path_key(mc: &Path) -> String {
    sha256_hex(normalized_key_source(mc).as_bytes())[..16].to_string()
}

/// 紀錄、備份區、隔離區的鍵：遊戲資料夾有 `.mcpl/instance.json` 就用識別碼，否則用舊鍵。
pub fn instance_key(mc: &Path) -> String {
    match super::mcpl_marker::read_instance(mc) {
        Ok(Some(info)) => info.id,
        Ok(None) => legacy_path_key(mc),
        // 標記壞了：不能默默改用舊鍵（會讀到或寫到別的位置）。讀寫前 load 會先停下並說明
        Err(_) => format!("marker-broken-{}", legacy_path_key(mc)),
    }
}

pub fn quarantine_dir(mc: &Path) -> PathBuf {
    store_root().join("apply-quarantine").join(instance_key(mc))
}

/// 確保遊戲資料夾有整合包識別碼；第一次建立時，把舊鍵（路徑雜湊）底下的紀錄與備份搬到新鍵。
/// 回傳 (識別碼, 這次才建立)。
pub fn ensure_instance(mc: &Path) -> Result<(String, bool), String> {
    let old_key = legacy_path_key(mc);
    let (info, created) = super::mcpl_marker::create_instance(mc)?;
    if created {
        for area in ["apply-records", "apply-backups", "apply-quarantine"] {
            let from = store_root().join(area).join(&old_key);
            let to = store_root().join(area).join(&info.id);
            if long_path(&from).exists() && !long_path(&to).exists() {
                if let Some(parent) = to.parent() {
                    let _ = fs::create_dir_all(long_path(parent));
                }
                fs::rename(long_path(&from), long_path(&to)).map_err(|e| {
                    format!("無法把舊的套用紀錄搬到新位置（{e}）：{} → {}", from.display(), to.display())
                })?;
            }
        }
    }
    Ok((info.id, created))
}

pub fn record_dir(mc: &Path) -> PathBuf {
    store_root().join("apply-records").join(instance_key(mc))
}

/// 「刪除全部備份」「有沒有備份」要掃的容器（裡面放 `翻譯套用備份_原檔`）。
pub fn backup_container(mc: &Path) -> PathBuf {
    store_root().join("apply-backups").join(instance_key(mc))
}

/// B3 審查 F-c：`.mcpl` 被刪、依紀錄認回的識別碼對應的原檔備份區（只讀用）。
pub fn instance_backup_dir_for_id(id: &str) -> PathBuf {
    store_root().join("apply-backups").join(id).join(INSTANCE_BACKUP_DIR)
}

pub fn instance_backup_dir(mc: &Path) -> PathBuf {
    backup_container(mc).join(INSTANCE_BACKUP_DIR)
}

// ─── 讀寫 ───────────────────────────────────────────────

pub fn load(mc: &Path) -> Result<ApplyRecord, String> {
    // 前置條件：識別碼標記讀得懂（壞了就停下，附位置與修復方法）
    super::mcpl_marker::read_instance(mc)?;
    let path = record_dir(mc).join(RECORD_FILE);
    let text = match fs::read_to_string(long_path(&path)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(ApplyRecord::default()),
        Err(error) => {
            return Err(format!(
                "讀不到套用紀錄（{error}），為了不改錯檔案，已經停止，沒有動任何東西。\n\
常見原因是權限不足或被防毒軟體擋住；請確認工具資料夾可以讀寫。\n紀錄檔位置：{}",
                path.display()
            ))
        }
    };
    let record: ApplyRecord = serde_json::from_str(&text).map_err(|_| broken_record_message(&path))?;
    // 紀錄以識別碼為鍵，所以遊戲資料夾改名或搬家仍找得到；整份複製、原位置連不到則停下
    super::apply_identity::check_recorded_location(mc, &record.mc_dir)?;
    Ok(record)
}

pub fn save(mc: &Path, record: &mut ApplyRecord) -> Result<(), String> {
    let dir = record_dir(mc);
    let path = dir.join(RECORD_FILE);
    // 全部移除後也保留（空的）紀錄檔：它在＝這個遊戲資料夾的紀錄是連續、可信的
    record.version = RECORD_VERSION;
    record.mc_dir = mc.display().to_string();
    record.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let fail = |e: &dyn std::fmt::Display| {
        format!(
            "無法儲存套用紀錄（{e}），為了之後能正確移除翻譯，已經停止。\n\
常見原因是權限不足或磁碟空間不足。\n紀錄檔位置：{}",
            path.display()
        )
    };
    fs::create_dir_all(long_path(&dir)).map_err(|e| fail(&e))?;
    let json = serde_json::to_string_pretty(record).map_err(|e| fail(&e))?;
    write_atomic(&path, format!("{json}\n").as_bytes()).map_err(|e| fail(&e))?;
    // 給人看的：這份紀錄是哪個遊戲資料夾的
    let _ = fs::write(long_path(&dir.join("遊戲資料夾.txt")), format!("{}\n", mc.display()));
    Ok(())
}

/// 這個遊戲資料夾有沒有紀錄檔（有＝紀錄連續可信）。
pub fn record_file_exists(mc: &Path) -> bool {
    long_path(&record_dir(mc).join(RECORD_FILE)).is_file()
}

/// 紀錄曾經被重設過（留有 `.broken-*` 檔）。
pub fn was_reset(mc: &Path) -> bool {
    let prefix = format!("{RECORD_FILE}.broken-");
    fs::read_dir(long_path(&record_dir(mc)))
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        })
        .unwrap_or(false)
}

const BROKEN_RECORD_MARK: &str = "套用紀錄損壞";

fn broken_record_message(path: &Path) -> String {
    format!(
        "{BROKEN_RECORD_MARK}：工具記錄「裝進遊戲的哪些檔案」的那份清單讀不懂了（可能是寫到一半停電或被其他程式改過）。\n\
為了不刪錯或改錯你的檔案，已經停止，沒有動任何東西。\n\
可以按「重設套用紀錄」重新開始記錄；壞掉的那份會改名保留，不會刪除。\n\
紀錄檔位置：{}",
        path.display()
    )
}

/// 這個錯誤是不是「套用紀錄損壞」（前端的 isBrokenRecordError 用同一個標記）。
#[cfg(test)]
pub fn is_broken_record_error(message: &str) -> bool {
    message.contains(BROKEN_RECORD_MARK)
}

/// 紀錄損壞時的出口：把損壞的紀錄改名保留為 `.broken-時間戳`（不刪），之後重新開始記錄。
/// 回傳改名後的位置。重設後，已經在遊戲裡的翻譯檔要靠備份或重新安裝整合包才能完全移除。
pub fn reset_record(mc: &Path) -> Result<PathBuf, String> {
    let path = record_dir(mc).join(RECORD_FILE);
    if !long_path(&path).is_file() {
        return Err("找不到套用紀錄，不需要重設。".into());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let kept = path.with_file_name(format!("{RECORD_FILE}.broken-{stamp}"));
    fs::rename(long_path(&path), long_path(&kept))
        .map_err(|e| format!("無法重設套用紀錄（{}）：{e}", path.display()))?;
    Ok(kept)
}

// ─── 檔案工具 ───────────────────────────────────────────

/// 先寫同資料夾的暫存檔再改名：寫到一半斷掉也不會留下半個檔。
pub fn write_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = tmp_sibling(dest);
    fs::write(long_path(&tmp), bytes)?;
    rename_over(&tmp, dest)
}

/// 複製檔案（暫存檔＋改名）。
pub fn copy_atomic(src: &Path, dest: &Path) -> std::io::Result<()> {
    let tmp = tmp_sibling(dest);
    fs::copy(long_path(src), long_path(&tmp))?;
    rename_over(&tmp, dest)
}

fn rename_over(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    match fs::rename(long_path(tmp), long_path(dest)) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(long_path(tmp));
            Err(error)
        }
    }
}

fn tmp_sibling(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    dest.with_file_name(format!("{name}.mcpl-tmp"))
}

/// 檔案內容雜湊；讀不到回 `None`。大檔（模組 JAR）用串流讀。
pub fn file_sha256(path: &Path) -> Option<String> {
    let mut file = fs::File::open(long_path(path)).ok()?;
    let mut hasher = Sha256Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finalize_hex())
}

/// 相對遊戲資料夾、以 `/` 分隔的路徑（紀錄與備份共用的鍵）。
pub fn rel_key(mc: &Path, target: &Path) -> String {
    target
        .strip_prefix(mc)
        .unwrap_or(target)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
#[path = "apply_record_tests.rs"]
mod tests;
