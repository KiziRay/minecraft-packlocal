//! B3：讀遊戲裡的檔之前，先確認「它是原檔」。
//!
//! 原則（B1 檔案安全設計）：必須先確認 X，才能做 Y。
//! - 當成**翻譯來源**讀（英文語言檔、任務檔、腳本、資料包 ZIP、模組 JAR）：必須是原檔。
//!   遊戲裡是本工具寫的版本 → 改讀原檔備份；沒有備份 → 不拿工具內容當原文，列入「需要原檔才能翻譯」。
//! - 當成**人工中文**保留（zh_tw 語言檔、任務 zh_tw.snbt、書本 zh_tw 頁、資源包）：必須確認是原檔。
//!   工具寫的、或無法確認的，都不保留、不免長度（但也不刪玩家檔）。
//!
//! 判斷（只讀，不寫任何檔）：
//! 1. `.mcpl/files/<相對路徑>.json` 標記存在：內容等於工具寫入的指紋 → 工具內容；否則 → 原檔（玩家改過）。
//! 2. 從檔案本身認得出的工具產物（工具資源包檔名／pack.mcmeta 說明）→ 工具內容。
//! 3. 套用紀錄（含 `.mcpl` 遺失時依位置或指紋認回的紀錄）記的工具版本 → 工具內容。
//! 4. 1.0.x 舊版清單提過、且內容等於某個翻譯結果裡的工具產出 → 工具內容。
//! 5. 這個遊戲裝過翻譯、但 `.mcpl` 不見了（或紀錄本身無法判斷）：內容等於某個翻譯結果的產出 → 工具內容；
//!    否則 → 無法確認。
//! 6. 其餘 → 原檔。
//!
//! （B3 審查 F1–F5 依據；G3.18–G3.21。第二輪：來源未變判定共用 original_sha，G3.28）

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::apply_guard;
use super::apply_knowledge::{self, Knowledge};
use super::apply_record;
use super::mcpl_marker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    Original,
    Tool,
    Unconfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadSource {
    /// 讀這個檔（遊戲裡的原檔，或工具版本對應的原檔備份）
    Use(PathBuf),
    /// 遊戲裡是工具寫的版本、又沒有可信的原檔：不能拿來當原文
    NeedsOriginal,
}

/// 模組 JAR 的讀法（B3#7、審查 F5）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JarRead {
    pub path: PathBuf,
    /// `Some`＝讀的是工具翻過的 JAR（沒有原檔）：zh_tw 只留這些「模組自帶」的 key（ns → keys）；
    /// 沒有紀錄時是空集合＝zh_tw 全丟。zh_cn 等其他語系不受影響（工具不寫這些）。
    pub keep_zh_tw_only: Option<HashMap<String, HashSet<String>>>,
}

pub struct ToolIndex {
    mc: PathBuf,
    /// 真的遊戲資料夾（有 mods 或 `.mcpl`）；暫存區（JAR／ZIP 解出來的）一律當原檔
    active: bool,
    mcpl_present: bool,
    knowledge: Option<Knowledge>,
    /// 讀紀錄失敗（標記壞掉等）：無法確認
    broken: bool,
    /// `.mcpl` 被刪、依紀錄認回時的原檔備份區與識別碼（審查 F-c）
    reclaimed_backup_dir: Option<(PathBuf, String)>,
}

impl ToolIndex {
    /// 為遊戲資料夾建立索引（只讀）。
    pub fn for_game(mc: &Path) -> Self {
        let mcpl_present = mc.join(mcpl_marker::MARKER_DIR).is_dir();
        let active = mcpl_present || mc.join("mods").is_dir();
        let (knowledge, broken) = if active {
            match Knowledge::peek(mc, None) {
                Ok(k) => (Some(k), false),
                Err(e) => {
                    crate::dev_log!("scan", "讀不到套用紀錄，工具產物一律視為無法確認：{e}");
                    (None, true)
                }
            }
        } else {
            (None, false)
        };
        let reclaimed_backup_dir = if active && !mcpl_present {
            super::apply_identity::find_reclaim(mc)
                .ok()
                .flatten()
                .map(|r| (apply_record::instance_backup_dir_for_id(&r.id), r.id.clone()))
        } else {
            None
        };
        Self { mc: mc.to_path_buf(), active, mcpl_present, knowledge, broken, reclaimed_backup_dir }
    }

    fn applied_before(&self) -> bool {
        self.knowledge.as_ref().is_some_and(|k| {
            k.record.has_translation() || !k.record.files.is_empty() || !k.legacy.is_empty() || k.uncertain.is_some()
        })
    }

    pub fn classify(&self, path: &Path) -> Provenance {
        if !self.active || path.strip_prefix(&self.mc).is_err() {
            return Provenance::Original;
        }
        let rel = apply_record::rel_key(&self.mc, path);
        let sha = apply_record::file_sha256(path);
        if let Some(marker) = mcpl_marker::read_file_marker(&self.mc, &rel) {
            return if marker.is_tool_content(sha.as_deref()) { Provenance::Tool } else { Provenance::Original };
        }
        if apply_knowledge::is_identifiable_tool_product(path) {
            return Provenance::Tool;
        }
        if self.broken {
            return Provenance::Unconfirmed;
        }
        let Some(k) = &self.knowledge else { return Provenance::Original };
        if k.record.is_tool_version(&rel, sha.as_deref()) {
            return Provenance::Tool;
        }
        let matches_output = || apply_knowledge::matches_known_tool_output(&k.work_roots, &rel, sha.as_deref());
        if k.legacy_mentions(&rel) && matches_output() {
            return Provenance::Tool;
        }
        if (!self.mcpl_present && self.applied_before()) || k.uncertain.is_some() {
            return if matches_output() { Provenance::Tool } else { Provenance::Unconfirmed };
        }
        if k.record.files.contains_key(&rel) || k.record.unconfirmed.contains(&rel) {
            // 紀錄上有、標記卻沒有（B1 先寫標記再寫檔）：關聯斷了，無法確認
            return Provenance::Unconfirmed;
        }
        Provenance::Original
    }

    /// 可以當成「本來就有的人工／模組自帶內容」保留嗎？只有確認是原檔才可以。
    pub fn is_original(&self, path: &Path) -> bool {
        self.classify(path) == Provenance::Original
    }

    /// 當成翻譯來源讀的位置。
    pub fn read_source(&self, path: &Path) -> ReadSource {
        match self.classify(path) {
            Provenance::Tool => match self.original_backup(path) {
                Some(original) => ReadSource::Use(original),
                None => ReadSource::NeedsOriginal,
            },
            _ => ReadSource::Use(path.to_path_buf()),
        }
    }

    /// 工具版本對應的原檔：備份區（備份標記的原檔指紋要相符；`.mcpl` 被刪時用認回的識別碼找）
    /// 或 1.0.x 舊版備份。
    pub(crate) fn original_backup(&self, path: &Path) -> Option<PathBuf> {
        let rel = apply_record::rel_key(&self.mc, path);
        let current_id = mcpl_marker::read_instance(&self.mc).ok().flatten().map(|i| i.id);
        let current = (apply_guard::backup_paths_at(&apply_record::instance_backup_dir(&self.mc), &rel), current_id);
        let reclaimed = self
            .reclaimed_backup_dir
            .as_ref()
            .map(|(dir, id)| (apply_guard::backup_paths_at(dir, &rel), Some(id.clone())));
        for ((backup, marker_path), instance_id) in std::iter::once(current).chain(reclaimed) {
            let marker: Option<apply_guard::BackupMarker> =
                std::fs::read_to_string(&marker_path).ok().and_then(|t| serde_json::from_str(&t).ok());
            if let Some(marker) = marker {
                // 審查第三輪 5：備份標記要屬於這個整合包、而且就是這個檔
                if marker.rel != rel || instance_id.as_deref() != Some(marker.instance_id.as_str()) {
                    continue;
                }
                if !marker.original_sha256.is_empty()
                    && apply_record::file_sha256(&backup).as_deref() == Some(marker.original_sha256.as_str())
                {
                    return Some(backup);
                }
            }
        }
        self.knowledge.as_ref().and_then(|k| k.legacy_original(&rel)).map(|(p, _)| p)
    }

    /// 工具版本對應的原檔指紋（審查 F-c：來源指紋比對用）：原檔備份的指紋，或標記記下的原檔指紋。
    pub fn original_sha(&self, path: &Path) -> Option<String> {
        if let Some(backup) = self.original_backup(path) {
            return apply_record::file_sha256(&backup);
        }
        let rel = apply_record::rel_key(&self.mc, path);
        mcpl_marker::read_file_marker(&self.mc, &rel).map(|m| m.original_sha256).filter(|s| !s.is_empty())
    }

    /// 模組 JAR 的讀法：原檔照讀；工具翻過的讀原檔備份；沒有備份時照讀，但 zh_tw 只留模組自帶的 key。
    pub fn jar_read(&self, jar: &Path) -> JarRead {
        match self.read_source(jar) {
            ReadSource::Use(path) => JarRead { path, keep_zh_tw_only: None },
            ReadSource::NeedsOriginal => {
                let rel = apply_record::rel_key(&self.mc, jar);
                let keep = self
                    .knowledge
                    .as_ref()
                    .and_then(|k| super::jar_sources::native_zh_tw_keys(&k.work_roots, &rel))
                    .unwrap_or_default();
                JarRead { path: jar.to_path_buf(), keep_zh_tw_only: Some(keep) }
            }
        }
    }
}

/// 翻譯器共用：讀遊戲裡的文字檔當來源（原檔或原檔備份）；工具內容又沒原檔 → 記「需要原檔」並回 `None`。
pub fn read_game_text(index: &ToolIndex, mc: &Path, path: &Path) -> Option<(PathBuf, String)> {
    match index.read_source(path) {
        ReadSource::Use(read) => std::fs::read_to_string(&read).ok().map(|text| (read, text)),
        ReadSource::NeedsOriginal => {
            super::output_guard::record_needs_original(&apply_record::rel_key(mc, path));
            None
        }
    }
}

// ─── 掃描鬆散語言檔時共用（jar_scan 的內部函式拿不到遊戲資料夾）───

thread_local! {
    static SCAN_INDEX: RefCell<Option<Rc<ToolIndex>>> = const { RefCell::new(None) };
}

/// 掃描期間讓同一執行緒的鬆散檔判斷用這個索引；離開範圍自動還原。
pub struct ScanIndexGuard(Option<Rc<ToolIndex>>);

impl ScanIndexGuard {
    pub fn set(index: Rc<ToolIndex>) -> Self {
        Self(SCAN_INDEX.with(|s| s.replace(Some(index))))
    }
}

impl Drop for ScanIndexGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        SCAN_INDEX.with(|s| *s.borrow_mut() = previous);
    }
}

/// 掃描中的鬆散語言檔：不是確認過的原檔就不收（工具寫的、無法確認的都不算已有內容）。
pub fn is_tool_written_file(path: &Path) -> bool {
    SCAN_INDEX.with(|s| s.borrow().as_ref().map(|index| !index.is_original(path))).unwrap_or(false)
}

#[cfg(test)]
#[path = "tool_products_test_support.rs"]
pub(crate) mod test_support;

#[cfg(test)]
#[path = "tool_products_tests.rs"]
mod tests;

