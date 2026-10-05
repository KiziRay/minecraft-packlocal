//! B6a-1：模組整合包更新偵測與「翻譯更新的部分」。
//!
//! 原則（B1 檔案安全設計）：必須先確認 X，才能做 Y。
//! - 判定「整合包已更新」一律以指紋比對（mods 檔名＋原檔大小、英文原文雜湊、任務等文字來源指紋），不猜；
//!   舊工作階段沒有對應欄位＝無法確認，不判「已更新」、也不判「版本變了」（失效安全＝舊行為）。
//! - 偵測（[`inspect`]）只讀：不建 `.mcpl`、不寫工作階段、不寫翻譯結果（G1.36、G5d.1）。
//! - 補翻前重掃（[`plan_refresh`]）：只有「新增的句子、英文改過的句子、上次還沒翻的句子」送去翻；
//!   英文改過的舊譯文不再沿用；確認被拿掉的模組（掃描沒錯誤、模組檔確實不在）的舊譯文從資源包清掉。
//! - mods/ 相關判斷在 `pack_update_mods.rs`。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::jar_scan::{resolve_minecraft_dir, LangMap};
use super::session::{filter_local_untranslatable, TranslateSession};

pub use super::pack_update_mods::{
    changed_sentences, effective_mod_files, mods_changed, mods_now, ns_jars, scan_is_clean, RemovalCheck,
};

/// 命名空間 → 鍵 → 英文原文雜湊（16 位十六進位）。
pub type SourceHashes = BTreeMap<String, BTreeMap<String, String>>;

/// 工作階段裡「判斷整合包有沒有更新」的依據（B6a-1 新增；舊工作階段都是空的）。
/// 以 `#[serde(flatten)]` 放進 TranslateSession，JSON 欄位是 `sourceHashes`、`mcVersion`、`modFiles`、`nsJars`。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBasis {
    /// 翻譯當時每一句英文的雜湊（沒有＝舊工作階段，無法確認哪些句子改了）
    #[serde(default)]
    pub source_hashes: SourceHashes,
    /// 翻譯當時偵測到的 Minecraft 版本（沒有＝舊工作階段，不判版本變了）
    #[serde(default)]
    pub mc_version: Option<String>,
    /// 翻譯當時 mods/ 的 .jar（小寫檔名 → 原檔大小），用來找出新增／更新的模組
    #[serde(default)]
    pub mod_files: BTreeMap<String, u64>,
    /// 翻譯當時每個命名空間由哪些模組檔提供（小寫檔名）：確認「模組被拿掉」用（審查 F1）
    #[serde(default)]
    pub ns_jars: BTreeMap<String, Vec<String>>,
}

/// 穩定的字串雜湊（FNV-1a 64；跨版本、跨執行不變）。
pub fn text_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub fn hashes_of(en: &LangMap) -> SourceHashes {
    let mut out = SourceHashes::new();
    for (ns, entries) in en {
        let slot = out.entry(ns.clone()).or_default();
        for (key, text) in entries {
            slot.insert(key.clone(), text_hash(text));
        }
    }
    out.retain(|_, m| !m.is_empty());
    out
}

/// 目前偵測到的 Minecraft 版本（偵測不到回 None）。
pub fn current_mc_version(instance: &Path) -> Option<String> {
    let mc = resolve_minecraft_dir(instance).unwrap_or_else(|_| instance.to_path_buf());
    super::pack_out::detect_minecraft_version(&mc).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// 整輪翻譯掃描後的判斷依據（en＝完整英文原文表）。模組清單以原檔為準（工具放進去的翻譯版改記原檔大小）。
pub fn basis_for(instance: &Path, work: Option<&Path>, en: &LangMap) -> UpdateBasis {
    UpdateBasis {
        source_hashes: hashes_of(en),
        mc_version: current_mc_version(instance),
        mod_files: effective_mod_files(instance, work, &BTreeMap::new()),
        ns_jars: ns_jars(instance),
    }
}

/// 審查 F2：整輪翻譯「本地整理完」的中途快照不寫新依據——沿用上一份工作階段的 mods 指紋與判斷依據
/// （沒有上一份＝沒有依據，接續補完會重掃）。新依據只在整輪跑完的結尾存檔寫入。
pub fn interim_basis(previous: Option<&TranslateSession>) -> (u64, UpdateBasis) {
    previous.map(|s| (s.mods_fingerprint, s.update_basis.clone())).unwrap_or_default()
}

/// 審查 F4：接續舊譯文要用的上一輪依據（只讀）：這個翻譯結果自己的工作階段；沒有（另存新結果）時，
/// 找屬於同一個遊戲資料夾的其他翻譯結果，優先最新一輪已套用到遊戲的，其次最新的工作階段。
pub fn prior_session(instance: &Path, work: &Path) -> Option<TranslateSession> {
    if let Ok((s, _)) = super::session::load_session(work) {
        if !s.update_basis.source_hashes.is_empty() {
            return Some(s);
        }
    }
    let mc = resolve_minecraft_dir(instance).unwrap_or_else(|_| instance.to_path_buf());
    let me = super::apply_record::legacy_path_key(&mc);
    let mut best: Option<((bool, u64), TranslateSession)> = None;
    for root in super::apply_knowledge::known_work_roots(&mc, None) {
        if root == work {
            continue;
        }
        let Ok((s, path)) = super::session::load_session(&root) else { continue };
        let owner = PathBuf::from(s.instance_path.trim());
        let owner_mc = resolve_minecraft_dir(&owner).unwrap_or(owner);
        if s.update_basis.source_hashes.is_empty() || super::apply_record::legacy_path_key(&owner_mc) != me {
            continue;
        }
        let applied = super::result_owner::latest_applied(instance, &root) == Some(true);
        let mtime = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if best.as_ref().map(|(rank, _)| (applied, mtime) > *rank).unwrap_or(true) {
            best = Some(((applied, mtime), s));
        }
    }
    best.map(|(_, s)| s)
}

/// 選資料夾時給前端的更新差異（唯讀；探測 `packUpdate`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    /// 模組有變（以原檔為準；舊工作階段沒記指紋＝false）
    pub mods_changed: bool,
    /// 下面的句數、模組數算得出來嗎（舊工作階段沒記英文雜湊或模組清單＝算不出來，前端不寫數字）
    pub counts_known: bool,
    /// 新增的模組（有文字、翻譯當時沒有的命名空間）
    pub new_mods: usize,
    /// 更新過的模組（翻譯當時就有、這次有新句子或句子改了）
    pub updated_mods: usize,
    /// 約幾句要翻（新增或英文改過的句子；只看新增／更新的模組檔）
    pub sentences: usize,
    /// 任務等文字來源改了幾處（翻譯之後來源檔變了、產出者還沒重跑過）
    pub texts_changed: usize,
    /// 來源已變、但因完整度設定這一輪沒打算翻的項目數（不算「整合包已更新」，只提醒）
    pub skipped_changed: usize,
    pub mc_before: Option<String>,
    pub mc_now: Option<String>,
    /// Minecraft 版本變了（兩邊都有記錄且不同）
    pub mc_changed: bool,
}

impl UpdateView {
    /// 有沒有任何「整合包已更新」的證據。
    pub fn updated(&self) -> bool {
        self.mods_changed || self.texts_changed > 0 || self.mc_changed
    }
}

fn same_version(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// 唯讀偵測：這個遊戲資料夾在上次翻譯（工作階段）之後有沒有更新。
pub fn inspect(session: &TranslateSession, instance: &Path, work: &Path) -> UpdateView {
    let basis = &session.update_basis;
    let mods_changed = mods_changed(session.mods_fingerprint, basis, instance, Some(work));
    let mc_now = current_mc_version(instance);
    let mc_before = basis.mc_version.clone();
    let mc_changed = matches!((&mc_before, &mc_now), (Some(a), Some(b)) if !same_version(a, b));
    let mc = resolve_minecraft_dir(instance).unwrap_or_else(|_| instance.to_path_buf());
    let texts_changed = super::text_sources::count_changed_sources(work, &mc);
    let skipped_changed = super::text_sources::count_skipped_changed_sources(work, &mc);
    let mut view = UpdateView { mods_changed, texts_changed, skipped_changed, mc_before, mc_now, mc_changed, ..Default::default() };
    if mods_changed && !basis.source_hashes.is_empty() && !basis.mod_files.is_empty() {
        let (new_mods, updated_mods, sentences) = changed_sentences(&mc, Some(work), basis);
        view.counts_known = true;
        view.new_mods = new_mods;
        view.updated_mods = updated_mods;
        view.sentences = sentences;
    }
    view
}

/// 補翻前要不要重掃。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RefreshReason {
    /// 舊工作階段沒有英文雜湊（[B0] 需重掃旗標）：重掃一次補上
    NoBaseline,
    /// 翻譯之後 mods 變了
    ModsChanged,
}

pub fn refresh_reason(session: &TranslateSession, instance: &Path, work: Option<&Path>) -> Option<RefreshReason> {
    if session.update_basis.source_hashes.is_empty() {
        return Some(RefreshReason::NoBaseline);
    }
    mods_changed(session.mods_fingerprint, &session.update_basis, instance, work).then_some(RefreshReason::ModsChanged)
}

pub struct RefreshInput<'a> {
    /// 上次翻譯當時的英文雜湊（空＝舊工作階段）
    pub old_hashes: &'a SourceHashes,
    /// 上次還沒翻的句子
    pub old_pending: &'a LangMap,
    /// 重掃得到的完整英文原文表
    pub en_full: &'a LangMap,
    /// 重掃得到、遊戲裡沒有任何中文的英文
    pub en_only: &'a LangMap,
    /// 重掃得到、遊戲裡本來就有的中文（模組自帶、資源包）
    pub zh_scan: &'a LangMap,
    /// 上次的翻譯（舊資源包）
    pub old_zh: &'a LangMap,
    /// 「模組被拿掉」的前置條件（審查 F1）
    pub removal: RemovalCheck<'a>,
}

#[derive(Debug, Default)]
pub struct Refresh {
    /// 舊翻譯去掉「英文改過的句子」與「確認被拿掉的模組」後
    pub zh: LangMap,
    /// 遊戲裡本來就有、舊翻譯沒有的中文（併入前要照整輪翻譯的規則轉台灣正體）
    pub scan_fill: LangMap,
    /// 這次要翻的：新增的句子、英文改過的句子、上次還沒翻的句子（已去掉免譯字串）
    pub pending: LangMap,
    /// 新的英文雜湊（存回工作階段）
    pub hashes: SourceHashes,
    /// 英文改過、舊譯文不再沿用的句子（命名空間 → 鍵 → 新英文）
    pub stale: LangMap,
    /// 被整合包拿掉的模組（命名空間），舊翻譯已從資源包清掉
    pub removed_namespaces: Vec<String>,
    /// 看起來不見、但無法確認被拿掉（掃描有錯誤、模組檔還在或暫時停用、上一輪沒記）：舊翻譯保留
    pub unconfirmed_removed: Vec<String>,
    pub new_sentences: usize,
    pub changed_sentences: usize,
}

fn has(map: &LangMap, ns: &str, key: &str) -> bool {
    map.get(ns).is_some_and(|m| m.contains_key(key))
}

/// 重掃之後決定：哪些舊譯文還能用、這次要翻哪些句子。
pub fn plan_refresh(input: RefreshInput<'_>) -> Refresh {
    let hashes = hashes_of(input.en_full);
    let baseline = !input.old_hashes.is_empty();
    let mut out = Refresh { zh: input.old_zh.clone(), ..Default::default() };
    // 被拿掉的模組：翻譯當時有記（有雜湊）、現在英文與遊戲中文都沒有，而且確認模組檔真的不在了（F1）
    if baseline {
        for ns in input.old_hashes.keys() {
            if hashes.contains_key(ns) || input.zh_scan.contains_key(ns) || !out.zh.contains_key(ns) {
                continue;
            }
            if input.removal.surely_removed(ns) {
                out.zh.remove(ns);
                out.removed_namespaces.push(ns.clone());
            } else {
                out.unconfirmed_removed.push(ns.clone());
            }
        }
    }
    let (stale, _) = drop_changed(&mut out.zh, input.old_hashes, &hashes, input.en_full);
    out.stale = stale;
    for (ns, entries) in input.zh_scan {
        for (key, text) in entries {
            if !has(&out.zh, ns, key) {
                out.scan_fill.entry(ns.clone()).or_default().insert(key.clone(), text.clone());
            }
        }
    }
    let mut pending = LangMap::new();
    for (ns, entries) in input.en_only {
        for (key, text) in entries {
            if text.trim().is_empty() || has(&out.zh, ns, key) || has(&out.scan_fill, ns, key) {
                continue;
            }
            let old = input.old_hashes.get(ns).and_then(|m| m.get(key));
            let is_new = baseline && old.is_none();
            let is_changed = old.is_some_and(|h| h != &text_hash(text));
            if !baseline || is_new || is_changed || has(input.old_pending, ns, key) {
                pending.entry(ns.clone()).or_default().insert(key.clone(), text.clone());
            }
        }
    }
    filter_local_untranslatable(&mut pending);
    for (ns, entries) in &pending {
        for (key, text) in entries {
            match input.old_hashes.get(ns).and_then(|m| m.get(key)) {
                None if baseline => out.new_sentences += 1,
                Some(h) if h != &text_hash(text) => out.changed_sentences += 1,
                _ => {}
            }
        }
    }
    out.pending = pending;
    out.hashes = hashes;
    out
}

/// 拿掉英文改過的句子的舊譯文；回（被拿掉的句子：命名空間 → 鍵 → 新英文, 句數）。
/// 兩邊都有雜湊才比；舊工作階段沒雜湊時什麼都不拿（無法確認）。
fn drop_changed(zh: &mut LangMap, old: &SourceHashes, new: &SourceHashes, en: &LangMap) -> (LangMap, usize) {
    let mut stale = LangMap::new();
    let mut n = 0usize;
    for (ns, entries) in zh.iter_mut() {
        let (Some(old_ns), Some(new_ns)) = (old.get(ns), new.get(ns)) else { continue };
        entries.retain(|key, _| {
            let changed = matches!((old_ns.get(key), new_ns.get(key)), (Some(a), Some(b)) if a != b);
            if changed {
                n += 1;
                let text = en.get(ns).and_then(|m| m.get(key)).cloned().unwrap_or_default();
                stale.entry(ns.clone()).or_default().insert(key.clone(), text);
            }
            !changed
        });
    }
    zh.retain(|_, m| !m.is_empty());
    (stale, n)
}

/// 整輪翻譯接續舊譯文前：拿掉英文改過的句子與確認被拿掉的模組（只在兩邊都有記錄時）。回（句數, 模組數）。
pub fn drop_stale(prior: &mut LangMap, old: &SourceHashes, new: &SourceHashes, zh_scan: &LangMap, removal: &RemovalCheck<'_>) -> (usize, usize) {
    let mut removed = 0usize;
    if !old.is_empty() {
        let gone: Vec<String> = prior
            .keys()
            .filter(|ns| old.contains_key(*ns) && !new.contains_key(*ns) && !zh_scan.contains_key(*ns) && removal.surely_removed(ns))
            .cloned()
            .collect();
        for ns in gone {
            prior.remove(&ns);
            removed += 1;
        }
    }
    let (_, n) = drop_changed(prior, old, new, &LangMap::new());
    (n, removed)
}

/// 從清單拿掉這些句子（英文改了：品質暫緩要重新給機會）。
pub fn forget_keys(map: &mut LangMap, keys: &LangMap) {
    for (ns, entries) in keys {
        if let Some(slot) = map.get_mut(ns) {
            for key in entries.keys() {
                slot.remove(key);
            }
        }
    }
    map.retain(|_, m| !m.is_empty());
}

/// 補翻／翻譯結果回給前端的更新摘要（完成卡「已幫你做的事」）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSummary {
    /// 這一輪有重掃整合包
    pub rescanned: bool,
    pub reason: Option<RefreshReason>,
    /// 新增幾句（算不出來＝None：整輪翻譯、舊工作階段沒雜湊）
    pub new_sentences: Option<usize>,
    pub changed_sentences: usize,
    /// 模組整合包拿掉的模組數（舊翻譯已從資源包清掉）
    pub removed_mods: usize,
    /// 看起來不見、但無法確認被拿掉的模組數（舊翻譯保留）
    pub unconfirmed_removed: usize,
}

impl UpdateSummary {
    pub fn from_refresh(reason: RefreshReason, r: &Refresh) -> Self {
        Self {
            rescanned: true,
            reason: Some(reason),
            new_sentences: (reason != RefreshReason::NoBaseline).then_some(r.new_sentences),
            changed_sentences: r.changed_sentences,
            removed_mods: r.removed_namespaces.len(),
            unconfirmed_removed: r.unconfirmed_removed.len(),
        }
    }

    /// 給玩家看的一行（日誌）。算不出來的數字不寫。
    pub fn log_line(&self) -> String {
        let mut parts = Vec::new();
        if let Some(n) = self.new_sentences {
            parts.push(format!("新增 {n} 句"));
        }
        parts.push(format!("英文改過 {} 句", self.changed_sentences));
        if self.removed_mods > 0 {
            parts.push(format!("拿掉的 {} 個模組，舊翻譯已清掉", self.removed_mods));
        }
        if self.unconfirmed_removed > 0 {
            parts.push(format!("{} 個模組讀不到或暫時停用，舊翻譯先保留（無法確認是被拿掉）", self.unconfirmed_removed));
        }
        format!("模組整合包已更新：{}；其他照舊。", parts.join("、"))
    }
}

#[cfg(test)]
#[path = "pack_update_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pack_update_fix_tests.rs"]
mod fix_tests;

#[cfg(test)]
#[path = "pack_update_fix2_tests.rs"]
mod fix2_tests;
