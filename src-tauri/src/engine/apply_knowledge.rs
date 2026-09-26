//! 「這個檔是誰的」的判斷：新版套用紀錄＋1.0.x 舊版套用清單＋遊戲資料夾裡的工具痕跡。
//!
//! 三條原則（所有備份、還原、刪除、覆蓋都經過這裡）：
//! - **原則 A（備份）**：只有確定是原檔才建立原檔備份。確定＝不在任何新紀錄或舊版清單上、
//!   內容不等於任何已知的工具寫入版本，而且紀錄可信（沒有遺失、重設或讀不到）。
//!   其餘一律不備份，標成「來源不明」。
//! - **原則 B（刪除與還原）**：只有確定目前內容是工具寫的才刪除或蓋回。確定＝內容雜湊等於
//!   紀錄中的工具版本；舊版清單沒有雜湊，改比對翻譯結果裡的工具產出或工具產物標記，
//!   判斷不了就不動並列出來。
//! - **原則 C（紀錄）**：紀錄存不了就停下、回報，不吞掉；紀錄被重設或遺失後不建立新備份。

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::apply_record::{self, ApplyRecord};
use super::out_layout::RESULT_DIR_NAME;
use super::paths::long_path;
use super::session::is_tool_resource_pack;

pub const APPLY_MANIFEST: &str = "套用清單.json";
pub const LEGACY_BACKUP_PREFIX: &str = "翻譯套用備份_";
/// 1.0.x 起工具資源包 pack.mcmeta 的說明文字（工具產物標記）
const TOOL_PACK_DESCRIPTION: &str = "台灣用語繁體中文翻譯資源包";

/// 舊版（1.0.x～1.1.0）放在備份資料夾裡的套用清單。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ApplyManifest {
    #[serde(default)]
    pub stamp: String,
    #[serde(default)]
    pub mc_dir: String,
    #[serde(default)]
    pub backup_dir: String,
    /// 舊版新增（原本不存在）
    #[serde(default)]
    pub added: Vec<String>,
    /// 舊版覆蓋（原本存在，照理有備份）
    #[serde(default)]
    pub overwritten: Vec<String>,
}

/// 一份舊版備份。`manifest` 為 `None`＝只有備份、沒有清單（1.0.x 早期）。
/// `owned`＝清單記的遊戲資料夾就是這一個（只有 owned 的才能用、才能刪）。
#[derive(Debug, Clone)]
pub struct LegacyBackup {
    pub dir: PathBuf,
    pub manifest: Option<ApplyManifest>,
    pub owned: bool,
}

pub fn path_key(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .to_ascii_lowercase()
}

fn push_unique(list: &mut Vec<PathBuf>, path: PathBuf) {
    if !list.iter().any(|existing| existing == &path) {
        list.push(path);
    }
}

/// 所有已知的翻譯結果根目錄（存在的才列）：這次指定的、遊戲資料夾旁的、預設輸出位置、
/// 設定裡的自訂位置，以及工具資料夾裡的舊版共用 work。舊版清單可能散在任何一個裡面。
pub fn known_work_roots(mc: &Path, result_root: Option<&Path>) -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(root) = result_root {
        push_unique(&mut bases, root.to_path_buf());
    }
    let instance_root = match mc.file_name().and_then(|n| n.to_str()) {
        Some("minecraft") | Some(".minecraft") => mc.parent().unwrap_or(mc).to_path_buf(),
        _ => mc.to_path_buf(),
    };
    push_unique(&mut bases, instance_root.join("繁中翻譯輸出"));
    if let Some(parent) = mc.parent() {
        push_unique(&mut bases, parent.to_path_buf());
    }
    if !cfg!(test) {
        let settings = super::app_settings::read_settings();
        if let Some(custom) = settings
            .get("translate")
            .and_then(|t| t.get("outputCustomRoot"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            push_unique(&mut bases, PathBuf::from(custom.trim()));
        }
        push_unique(&mut bases, super::paths::resolve_file(Path::new("work")));
    }
    let mut roots = Vec::new();
    for base in bases {
        let candidates = [base.clone(), base.join(RESULT_DIR_NAME)];
        for candidate in candidates {
            if long_path(&candidate).is_dir() {
                push_unique(&mut roots, candidate.clone());
            }
        }
        // 每個整合包一個子資料夾的放法：<根>/<整合包>/翻譯結果
        if let Ok(entries) = fs::read_dir(long_path(&base)) {
            for entry in entries.flatten() {
                let nested = entry.path().join(RESULT_DIR_NAME);
                if nested.is_dir() {
                    push_unique(&mut roots, nested);
                }
            }
        }
    }
    roots
}

fn read_manifest(dir: &Path) -> Option<ApplyManifest> {
    let text = fs::read_to_string(long_path(&dir.join(APPLY_MANIFEST))).ok()?;
    serde_json::from_str(&text).ok()
}

/// 看得到的舊版備份（新到舊）：放在遊戲資料夾旁的，或清單記的遊戲資料夾相符的。
/// 只有 `owned`（清單確認屬於本整合包）的才能用、才能刪；其餘只用來顯示與判斷「以前被動過」。
/// CurseForge 把多個整合包放在同一個上層資料夾，旁邊的備份可能是別包的，所以不能憑位置認定。
pub fn legacy_backups_for(mc: &Path, result_root: Option<&Path>) -> Vec<LegacyBackup> {
    let mc_key = path_key(mc);
    let beside_game = mc.parent().map(path_key);
    let mut containers = known_work_roots(mc, result_root);
    if let Some(parent) = mc.parent() {
        push_unique(&mut containers, parent.to_path_buf());
    }
    let mut found: Vec<LegacyBackup> = Vec::new();
    for container in containers {
        let Ok(entries) = fs::read_dir(long_path(&container)) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            let is_backup = dir.is_dir()
                && dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(LEGACY_BACKUP_PREFIX));
            if !is_backup || found.iter().any(|b| b.dir == dir) {
                continue;
            }
            let manifest = read_manifest(&dir);
            // 放在遊戲資料夾旁的一定是這個整合包的（整合包搬過位置時清單記的路徑會不同）；
            // 其他位置的只收清單記的遊戲資料夾相符的，別的整合包的備份不碰。
            let beside = Some(path_key(&container)) == beside_game;
            let mine = beside
                || manifest
                    .as_ref()
                    .is_some_and(|m| !m.mc_dir.is_empty() && path_key(Path::new(&m.mc_dir)) == mc_key);
            if mine {
                let owned = super::apply_guard::require_legacy_owned(
                    mc,
                    manifest.as_ref().map(|m| m.mc_dir.as_str()),
                );
                found.push(LegacyBackup { dir, manifest, owned });
            }
        }
    }
    found.sort_by(|a, b| b.dir.file_name().cmp(&a.dir.file_name()));
    found
}

/// 目前檔案相對遊戲資料夾的路徑，對應到翻譯結果裡工具產出那份的位置。
fn work_sources(work_root: &Path, rel: &str) -> Vec<PathBuf> {
    if let Some(rest) = rel.strip_prefix("mods/") {
        return vec![work_root.join("jar-translated").join(rest)];
    }
    if let Some(rest) = rel.strip_prefix("resourcepacks/") {
        return vec![
            work_root.join("resourcepacks").join(rest),
            work_root.join("resourcepacks-extra").join(rest),
        ];
    }
    vec![work_root.join(rel)]
}

/// 遊戲裡這個檔的內容，是否等於某個翻譯結果裡工具產出的那份（舊版清單沒有雜湊時用）。
pub fn matches_known_tool_output(work_roots: &[PathBuf], rel: &str, current_sha: Option<&str>) -> bool {
    let Some(current) = current_sha else {
        return false;
    };
    work_roots
        .iter()
        .flat_map(|root| work_sources(root, rel))
        .filter(|candidate| long_path(candidate).is_file())
        .any(|candidate| apply_record::file_sha256(&candidate).as_deref() == Some(current))
}

/// 從檔案本身認得出是工具產物：工具的翻譯資源包（檔名或 pack.mcmeta 說明），
/// 或資源包旁只記指紋的標記檔。
pub fn is_identifiable_tool_product(target: &Path) -> bool {
    let name = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let in_resourcepacks = target
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n == "resourcepacks");
    if !in_resourcepacks {
        return false;
    }
    if let Some(stem) = name.strip_suffix(".meta.json") {
        let Ok(text) = fs::read_to_string(long_path(target)) else {
            return false;
        };
        let only_fingerprint = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.as_object().map(|o| o.len() == 1 && o.contains_key("modsFingerprint")))
            .unwrap_or(false);
        return only_fingerprint && !stem.is_empty();
    }
    if let Some(stem) = name.strip_suffix(".zip") {
        if is_tool_resource_pack(stem) {
            return true;
        }
        return zip_has_tool_description(target);
    }
    false
}

fn zip_has_tool_description(zip_path: &Path) -> bool {
    let Ok(file) = fs::File::open(long_path(zip_path)) else {
        return false;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return false;
    };
    let Ok(mut entry) = archive.by_name("pack.mcmeta") else {
        return false;
    };
    let mut text = String::new();
    entry.read_to_string(&mut text).is_ok() && text.contains(TOOL_PACK_DESCRIPTION)
}

/// 分類結果：這個即將被工具寫入的目標，原本是什麼。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    /// 目標不存在
    Absent,
    /// 內容是工具寫過的版本
    ToolVersion,
    /// 確定是原檔（不在任何紀錄、不是工具內容；或紀錄上但已被整合包／玩家換成別的內容）
    Original,
    /// 舊版清單（屬本包）記載這是舊版工具新增的
    LegacyAdded,
    /// 舊版清單（屬本包）記載覆蓋過，而且舊版備份裡有原檔
    LegacyBacked { original: PathBuf, manifest_dir: PathBuf },
    /// 無法確定（舊版提過但沒備份、紀錄遺失／重設、內容剛好等於工具產出…）
    Unknown,
    /// 上次套用中途中斷、還沒寫入的檔：沿用上次的分類與備份／隔離連結
    Unwritten(super::apply_pending::Unwritten),
}

pub struct Knowledge {
    pub record: ApplyRecord,
    /// 無法判斷「不在紀錄上的檔」是不是原檔時的原因
    pub uncertain: Option<String>,
    /// 清單確認屬於本整合包的舊版備份
    pub legacy: Vec<LegacyBackup>,
    /// 看得到但無法確認屬於本整合包的舊版備份（不用、不刪）
    pub unowned_legacy: Vec<LegacyBackup>,
    pub work_roots: Vec<PathBuf>,
    /// 識別碼認回時要告訴玩家的話（部分檔案被整合包更新改過）
    pub notice: Option<String>,
}

impl Knowledge {
    /// 會動檔的動作用：識別碼標記不見時，先依紀錄位置與檔案指紋真的認回（補回 `.mcpl`、寫紀錄）。
    pub fn load(mc: &Path, result_root: Option<&Path>) -> Result<Self, String> {
        // 前置條件：識別碼標記讀得懂；不見了先試著依紀錄位置與檔案指紋認回
        let notice = super::apply_identity::require_identity(mc)?;
        Self::read(mc, result_root, None, notice)
    }

    /// 查詢狀態用：只判斷、不寫任何檔（不建 `.mcpl`、不改紀錄、不設隱藏屬性）。
    /// 認得回來時直接用「認回後會是的那份紀錄」，說明也照樣算出來。
    pub fn peek(mc: &Path, result_root: Option<&Path>) -> Result<Self, String> {
        let reclaim = super::apply_identity::find_reclaim(mc)?;
        let notice = reclaim.as_ref().and_then(|r| r.notice());
        Self::read(mc, result_root, reclaim.map(|r| r.reclaimed_record()), notice)
    }

    fn read(
        mc: &Path,
        result_root: Option<&Path>,
        reclaimed: Option<ApplyRecord>,
        notice: Option<String>,
    ) -> Result<Self, String> {
        let (mut record, record_existed) = match reclaimed {
            Some(record) => (record, true),
            None => (apply_record::load(mc)?, apply_record::record_file_exists(mc)),
        };
        let (legacy, unowned_legacy): (Vec<_>, Vec<_>) =
            legacy_backups_for(mc, result_root).into_iter().partition(|b| b.owned);
        let work_roots = known_work_roots(mc, result_root);
        // 紀錄不見但遊戲資料夾裡的標記還在：從標記重建（另一端還在就重建）
        let mut rebuilt_from_markers = 0;
        if !record_existed {
            if let Ok(Some(info)) = super::mcpl_marker::read_instance(mc) {
                rebuilt_from_markers =
                    rebuild_record_from_markers(mc, &info.id, &mut record);
            }
        }
        let uncertain = if record.started_uncertain {
            Some(record_uncertain_reason(mc))
        } else if record_existed || rebuilt_from_markers > 0 {
            None
        } else {
            missing_record_reason(mc, &legacy, &unowned_legacy)
        };
        Ok(Self { record, uncertain, legacy, unowned_legacy, work_roots, notice })
    }

    fn legacy_added(&self, rel: &str) -> bool {
        self.legacy
            .iter()
            .filter_map(|b| b.manifest.as_ref())
            .any(|m| m.added.iter().any(|a| a.replace('\\', "/") == rel))
    }

    fn legacy_overwritten(&self, rel: &str) -> bool {
        self.legacy
            .iter()
            .filter_map(|b| b.manifest.as_ref())
            .any(|m| m.overwritten.iter().any(|a| a.replace('\\', "/") == rel))
    }

    /// 任何舊版紀錄（還在的清單，或刪備份前記下的）提到這個檔。
    pub fn legacy_mentions(&self, rel: &str) -> bool {
        self.legacy_added(rel) || self.legacy_overwritten(rel) || self.record.legacy_touched.contains(rel)
    }

    /// 屬本包的舊版備份裡這個檔的原檔（清單記載覆蓋過才算）。新的優先。回傳 (原檔, 舊備份資料夾)。
    pub fn legacy_original(&self, rel: &str) -> Option<(PathBuf, PathBuf)> {
        self.legacy
            .iter()
            .filter(|b| {
                b.manifest
                    .as_ref()
                    .is_some_and(|m| m.overwritten.iter().any(|a| a.replace('\\', "/") == rel))
            })
            .map(|b| (b.dir.join(rel), b.dir.clone()))
            .find(|(p, _)| long_path(p).is_file())
    }

    /// 新紀錄以外、舊版清單上的全部檔（相對路徑）。
    pub fn legacy_listed(&self) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = Vec::new();
        for manifest in self.legacy.iter().filter_map(|b| b.manifest.as_ref()) {
            for rel in &manifest.added {
                let rel = rel.replace('\\', "/");
                if !out.iter().any(|(r, _)| *r == rel) {
                    out.push((rel, true));
                }
            }
            for rel in &manifest.overwritten {
                let rel = rel.replace('\\', "/");
                if !out.iter().any(|(r, _)| *r == rel) {
                    out.push((rel, false));
                }
            }
        }
        out
    }

    /// 即將寫入 `target` 時，判斷它現在是什麼。`planned_sha`＝工具這次要寫進去的內容雜湊。
    pub fn classify(&self, mc: &Path, target: &Path, planned_sha: &str) -> Class {
        if !long_path(target).is_file() {
            return Class::Absent;
        }
        let rel = apply_record::rel_key(mc, target);
        let current = apply_record::file_sha256(target);
        if self.record.files.contains_key(&rel) {
            if self.record.is_tool_version(&rel, current.as_deref()) {
                return Class::ToolVersion;
            }
            if let Some(previous) = super::apply_pending::unwritten(mc, &self.record, &rel, current.as_deref()) {
                return Class::Unwritten(previous);
            }
            // 識別碼認回時指紋對不上的檔：無法確認，先隔離
            if self.record.unconfirmed.contains(&rel) {
                return Class::Unknown;
            }
            // 紀錄上有、但內容已經不是工具寫的：整合包更新或玩家改過的新原檔（要重新備份）
            return Class::Original;
        }
        if self.legacy_added(&rel) {
            return Class::LegacyAdded;
        }
        if let Some((original, manifest_dir)) = self.legacy_original(&rel) {
            return Class::LegacyBacked { original, manifest_dir };
        }
        if self.legacy_mentions(&rel)
            || self.uncertain.is_some()
            // 看起來像工具產物但沒有標記：原則 A，來源不明
            || is_identifiable_tool_product(target)
            || current.as_deref() == Some(planned_sha)
            || matches_known_tool_output(&self.work_roots, &rel, current.as_deref())
        {
            return Class::Unknown;
        }
        Class::Original
    }
}

fn record_uncertain_reason(mc: &Path) -> String {
    if apply_record::was_reset(mc) {
        "套用紀錄曾經被重設".into()
    } else {
        "先前的套用紀錄不完整".into()
    }
}

/// 沒有新紀錄（也沒有標記可重建）時，看得出「工具以前動過這裡」卻說不清動了哪些檔的情況。
fn missing_record_reason(mc: &Path, legacy: &[LegacyBackup], unowned: &[LegacyBackup]) -> Option<String> {
    if apply_record::was_reset(mc) {
        return Some("套用紀錄曾經被重設".into());
    }
    if long_path(&apply_record::instance_backup_dir(mc)).is_dir() {
        return Some("找不到套用紀錄，但這個遊戲資料夾有工具的備份".into());
    }
    // 清單寫明屬於別的整合包的舊備份與本包無關；只有「沒有清單」的舊備份才無法判斷
    if unowned.iter().any(|b| b.manifest.is_none()) {
        return Some("找到舊版工具的備份，但沒有清單，無法確認是不是這個整合包的".into());
    }
    if let Some(old) = super::apply_identity::orphan_records(mc).first() {
        return Some(format!(
            "找到這個遊戲資料夾以前的套用紀錄，但裡面記的檔案和現在對不上，這次不沿用（舊紀錄與備份保留在：{}）",
            old.display()
        ));
    }
    let has_manifest = !legacy.is_empty();
    if !has_manifest && game_has_tool_traces(mc) {
        return Some("這個遊戲資料夾以前被工具改過，但找不到當時的紀錄（可能搬過或改過名）".into());
    }
    None
}

/// 遊戲資料夾裡的工具痕跡：設定檔旁的備份、資源包旁的指紋標記、工具的翻譯資源包。
fn game_has_tool_traces(mc: &Path) -> bool {
    if long_path(&mc.join("options.txt.mcpl-bak")).is_file() {
        return true;
    }
    // 從別的整合包複製過來、已改成新整合包的舊標記
    let copied = fs::read_dir(long_path(&super::mcpl_marker::marker_dir(mc))).ok().is_some_and(|entries| {
        entries.flatten().any(|e| e.file_name().to_string_lossy().starts_with(super::mcpl_marker::COPIED_FROM_PREFIX))
    });
    if copied {
        return true;
    }
    // 1.0.x 翻譯快捷選單時直接改遊戲裡的 menu.json 並留下 .bak
    if long_path(&mc.join("minemenu").join("menu.json.bak")).is_file() {
        return true;
    }
    let Ok(entries) = fs::read_dir(long_path(&mc.join("resourcepacks"))) else {
        return false;
    };
    entries.flatten().any(|entry| is_identifiable_tool_product(&entry.path()))
}

/// 已知舊版清單上、新增過的檔集合（給刪備份前記下）。
pub fn legacy_touched_set(legacy: &[LegacyBackup]) -> HashSet<String> {
    legacy
        .iter()
        .filter_map(|b| b.manifest.as_ref())
        .flat_map(|m| m.added.iter().chain(m.overwritten.iter()))
        .map(|rel| rel.replace('\\', "/"))
        .collect()
}

/// 紀錄遺失時，從遊戲資料夾裡的標記重建（另一端還在就重建）。回傳重建了幾個檔。
pub fn rebuild_record_from_markers(mc: &Path, instance_id: &str, record: &mut ApplyRecord) -> usize {
    let mut count = 0usize;
    for marker in super::mcpl_marker::all_file_markers(mc) {
        if marker.instance_id != instance_id || record.files.contains_key(&marker.rel) {
            continue;
        }
        let kind = if marker.role == "added" {
            apply_record::FileKind::Added
        } else {
            apply_record::FileKind::Overwritten
        };
        let origin = if marker.origin == "unknown" {
            apply_record::Origin::Unknown
        } else {
            apply_record::Origin::Known
        };
        record.set_entry(&marker.rel, kind, origin, None, marker.tool_sha256.clone());
        if let Some(entry) = record.files.get_mut(&marker.rel) {
            entry.history = marker.tool_history.clone();
            entry.marker_id = marker.id.clone();
            entry.backup_id = super::mcpl_marker::linked(&marker.links, super::mcpl_marker::REL_BACKUP).unwrap_or_default();
            entry.quarantine_id = super::mcpl_marker::linked(&marker.links, super::mcpl_marker::REL_QUARANTINE).unwrap_or_default();
        }
        if let Some(batch) = super::mcpl_marker::linked(&marker.links, super::mcpl_marker::REL_BATCH) {
            match record.batches.iter_mut().find(|b| b.id == batch) {
                Some(b) => b.markers.push(marker.id.clone()),
                None => record.batches.push(apply_record::Batch { id: batch, at: 0, markers: vec![marker.id.clone()] }),
            }
        }
        count += 1;
    }
    count
}
