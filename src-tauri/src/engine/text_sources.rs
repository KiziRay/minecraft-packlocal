//! B3：文字產出清單（翻譯結果裡「確認產出的檔」＋來源指紋）。
//!
//! 原則：必須先確認 X，才能做 Y。
//! - **放進遊戲前**必須確認「這個檔是確認過的產出、內容沒被動過、來源還是翻譯當時那份原檔」。
//!   不在清單上的檔（B3 以前的產物：原地改成中文的 en_us、壞掉的舊腳本、舊的 .properties 譯文……）不放。
//! - **退休**（把遊戲裡上一版譯文拿掉）是破壞性動作，必須先確認「確定不再產出」：
//!   產出者本輪**完整跑完**（commit），而且那個檔的**來源檔已不在遊戲**。
//!   中途取消／出錯（沒 commit）、逐檔跳過、AI 缺譯，都不是退休理由——上一輪的條目照樣有效。
//!
//! 流程：產出者 `begin` 開一份暫存清單 → 每寫一檔 `record` 進暫存 → 成功跑完才 `commit`：
//! 暫存取代它上一輪的條目；上一輪有、這輪沒產出的，來源還在就沿用（舊產物照樣可確認後套用），
//! 來源已不在才列入退休名單。沒 commit 的暫存不影響已確認的清單。
//! 紀錄存在翻譯結果的 `.mcpl-text-sources.json`。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::apply_plan::{ApplyPlan, Group, ItemSource};
use super::apply_record::{self, ApplyRecord};
use super::paths::long_path;

pub const SOURCES_FILE: &str = ".mcpl-text-sources.json";

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSource {
    /// 來源檔相對遊戲資料夾的路徑
    pub source: String,
    /// 來源**原檔**的指紋
    pub sha256: String,
    /// 寫出的檔的指紋（套用前比對：被動過就不放）
    #[serde(default)]
    pub output_sha256: String,
    /// 產出者
    #[serde(default)]
    pub producer: String,
    /// 含不上傳的私有字串（伺服器腳本、.tell 類）：分享包也不帶（審查 F6）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private: bool,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    /// 已確認的產出（翻譯結果相對路徑 → 來源）
    #[serde(default)]
    entries: BTreeMap<String, TextSource>,
    /// 進行中、還沒 commit 的暫存（產出者 → 條目）
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pending: BTreeMap<String, BTreeMap<String, TextSource>>,
    /// 退休候選（產出者本輪完成、當時確認來源已不在遊戲）：翻譯結果相對路徑 → 原條目（含來源路徑）。
    /// 套用時還要再確認一次來源仍不在（審查第三輪 2）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    retiring: BTreeMap<String, TextSource>,
    /// 產出者最近一次完整跑完的時間（秒）
    #[serde(default)]
    rounds: BTreeMap<String, u64>,
    /// B6a-1：產出者最近一次完整跑完時，它每個來源檔當下的指紋（產出者 → 來源 → 指紋）。
    /// 來源改了、但這一版已經處理過（沒有可翻的字所以沒有新產出）→ 不再算「要翻」。舊資料沒有這欄＝照舊行為。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    processed: BTreeMap<String, BTreeMap<String, String>>,
    /// 產出清單屬於哪個遊戲資料夾（路徑鍵）；套到別的遊戲資料夾時不做任何退休
    #[serde(default, skip_serializing_if = "String::is_empty")]
    game_key: String,
    /// 遊戲資料夾位置（分享包取遊戲原包的 pack.mcmeta 用）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    game_root: String,
    /// 第二輪格式的退休名單（只有路徑、沒有來源）：讀進來轉成 `retiring`，不再寫出
    #[serde(default, skip_serializing)]
    retire: BTreeMap<String, Vec<String>>,
}

/// 被拿掉的檔（遊戲相對路徑）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Dropped {
    /// 來源在翻譯之後被改過：需要重新翻譯
    pub outdated: Vec<String>,
    /// 不是確認過的產出（舊版產物、產出後被改過）：不放進遊戲
    pub stale: Vec<String>,
    /// 來源已被整合包移除（確定不在）：不放進遊戲；產出者下次跑完會列入退休候選
    pub source_removed: Vec<String>,
    /// 讀不到來源、無法確認：不放、也不退休，維持遊戲現狀
    pub unconfirmed: Vec<String>,
}

/// 來源檔現在的狀態（審查第四輪 A）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceState {
    Present,
    /// 確定不在：檔案不存在，而且往上找到的最近一層存在的資料夾在遊戲根底下、讀得到
    Gone,
    /// 讀取錯誤、遊戲根讀不到：無法確認
    Unconfirmed,
}

pub(crate) fn source_state(mc: &Path, source: &str) -> SourceState {
    if source.is_empty() {
        return SourceState::Unconfirmed;
    }
    let path = mc.join(source);
    match long_path(&path).try_exists() {
        Ok(true) => return SourceState::Present,
        Ok(false) => {}
        Err(_) => return SourceState::Unconfirmed,
    }
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d.strip_prefix(mc).is_err() {
            return SourceState::Unconfirmed;
        }
        match long_path(d).try_exists() {
            Ok(true) => {
                return if fs::read_dir(long_path(d)).is_ok() { SourceState::Gone } else { SourceState::Unconfirmed };
            }
            Ok(false) => dir = d.parent(),
            Err(_) => return SourceState::Unconfirmed,
        }
    }
    SourceState::Unconfirmed
}

fn path_of(work: &Path) -> PathBuf {
    work.join(SOURCES_FILE)
}

fn key_of(work: &Path, output: &Path) -> String {
    output.strip_prefix(work).unwrap_or(output).to_string_lossy().replace('\\', "/")
}

fn game_key(mc: &Path) -> String {
    apply_record::legacy_path_key(mc)
}

/// 讀產出清單。只有「檔案不存在」才算空清單；讀取錯誤（網路磁碟 EPERM…）或看不懂都回錯誤，
/// 呼叫端一律中止、不寫回（審查第三輪 3：不能用空清單蓋掉其他產出者的條目）。
fn read(work: &Path) -> Result<Manifest, String> {
    let path = path_of(work);
    let text = match fs::read_to_string(long_path(&path)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Manifest::default()),
        Err(e) => return Err(format!("讀不到產出清單（{e}）：{}", path.display())),
    };
    let mut manifest =
        serde_json::from_str::<Manifest>(&text).map_err(|e| format!("產出清單看不懂（{e}）：{}", path.display()))?;
    // 第二輪格式：舊的退休名單沒有來源，轉進 retiring（來源空白＝套用時無法確認，不會退休）
    for key in std::mem::take(&mut manifest.retire).into_values().flatten() {
        manifest.retiring.entry(key).or_insert_with(|| TextSource {
            source: String::new(),
            sha256: String::new(),
            output_sha256: String::new(),
            producer: String::new(),
            private: false,
        });
    }
    Ok(manifest)
}

fn save(work: &Path, manifest: &Manifest) -> Result<(), String> {
    let text = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    fs::create_dir_all(long_path(work)).map_err(|e| e.to_string())?;
    // 暫存檔＋改名
    apply_record::write_atomic(&path_of(work), text.as_bytes()).map_err(|e| format!("無法寫入產出清單：{e}"))
}

/// 讀 → 改 → 有變才寫。讀失敗就不改、不寫，回錯誤。
fn with_manifest<R>(work: &Path, f: impl FnOnce(&mut Manifest) -> R) -> Result<R, String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut manifest = read(work)?;
    let before = serde_json::to_string(&manifest).unwrap_or_default();
    let out = f(&mut manifest);
    if serde_json::to_string(&manifest).unwrap_or_default() != before || !long_path(&path_of(work)).is_file() {
        save(work, &manifest)?;
    }
    Ok(out)
}

fn log_err(what: &str, result: Result<(), String>) {
    if let Err(e) = result {
        crate::dev_log!("translate", "{what}：{e}");
    }
}

/// 來源檔「確定不在」（見 [`source_state`]）。讀取錯誤、無法確認都不算——一律當作還在。
fn source_surely_gone(mc: &Path, source: &str) -> bool {
    source_state(mc, source) == SourceState::Gone
}

/// 產出者開始一輪：開一份空的暫存清單（已確認的清單不動）。
pub fn begin(work: &Path, producer: &str) {
    log_err(
        "產出清單",
        with_manifest(work, |m| {
            m.pending.insert(producer.to_string(), BTreeMap::new());
        }),
    );
}

/// 產出者**完整跑完**：暫存取代它上一輪的條目。上一輪有、這輪沒產出的——來源檔沒有「確定不在」
/// 就沿用（逐檔跳過、AI 缺譯、讀取異常都不是拿掉譯文的理由）；確定不在才列入退休候選。
/// 前置條件：遊戲根目錄讀得到，否則這輪不 commit（上一輪清單保持有效）。
pub fn commit(work: &Path, producer: &str, mc: &Path) {
    if fs::read_dir(long_path(mc)).is_err() {
        crate::dev_log!("translate", "{producer}：讀不到遊戲資料夾，這輪產出不確認（上一輪清單保持有效）");
        return;
    }
    let result = with_manifest(work, |m| {
        let Some(new) = m.pending.remove(producer) else { return };
        let old: Vec<(String, TextSource)> = m
            .entries
            .iter()
            .filter(|(_, s)| s.producer == producer)
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect();
        m.entries.retain(|_, s| s.producer != producer);
        for (key, source) in old {
            if new.contains_key(&key) {
                continue;
            }
            if source_surely_gone(mc, &source.source) {
                m.retiring.insert(key, source);
            } else {
                m.entries.insert(key, source);
            }
        }
        for key in new.keys() {
            m.retiring.remove(key);
        }
        m.entries.extend(new);
        m.rounds.insert(producer.to_string(), super::mcpl_marker::now_secs());
        let done: BTreeMap<String, String> = m
            .entries
            .values()
            .filter(|s| s.producer == producer && !s.source.is_empty())
            .filter_map(|s| apply_record::file_sha256(&mc.join(&s.source)).map(|sha| (s.source.clone(), sha)))
            .collect();
        m.processed.insert(producer.to_string(), done);
        m.game_key = game_key(mc);
        m.game_root = mc.display().to_string();
    });
    log_err("產出清單", result);
}

/// B4：產出者**沒跑完**（AI 中途停下、使用者按停止），但已經寫出的檔要能裝進遊戲。
///
/// 暫存的條目加進確認清單（同一個檔以這次為準）；**不動**它上一輪的其他條目、
/// **不產生退休候選**、不更新「最近一次完整跑完」時間——退休只能在完整跑完（`commit`）時判斷。
/// 前置條件同 `commit`：遊戲根目錄讀得到，否則這輪不確認。
pub fn confirm_partial(work: &Path, producer: &str, mc: &Path) {
    if fs::read_dir(long_path(mc)).is_err() {
        crate::dev_log!("translate", "{producer}：讀不到遊戲資料夾，部分產出不確認（上一輪清單保持有效）");
        return;
    }
    let result = with_manifest(work, |m| {
        let Some(new) = m.pending.remove(producer) else { return };
        for key in new.keys() {
            m.retiring.remove(key);
        }
        m.entries.extend(new);
        m.game_root = mc.display().to_string();
    });
    log_err("產出清單", result);
}

/// 記下 `output` 是 `producer` 從遊戲裡的 `game_source` 做出來的；`read_source` 是實際讀的原檔
/// （遊戲裡的原檔，或工具版本對應的原檔備份）。產出者已 `begin` 就記進暫存，否則直接記進清單。
pub fn record(work: &Path, output: &Path, mc: &Path, game_source: &Path, read_source: &Path, producer: &str) {
    let Some(sha256) = apply_record::file_sha256(read_source) else { return };
    let Some(output_sha256) = apply_record::file_sha256(output) else { return };
    let entry = TextSource {
        source: apply_record::rel_key(mc, game_source),
        sha256,
        output_sha256,
        producer: producer.to_string(),
        private: false,
    };
    let key = key_of(work, output);
    let result = with_manifest(work, |m| {
        m.game_root = mc.display().to_string();
        match m.pending.get_mut(producer) {
            Some(pending) => {
                pending.insert(key, entry);
            }
            None => {
                m.entries.insert(key, entry);
            }
        }
    });
    log_err("產出清單", result);
}

/// 審查 F6：這個產出含私有字串（先 `record` 再標記）。
pub fn mark_private_output(work: &Path, output: &Path) {
    let key = key_of(work, output);
    let result = with_manifest(work, |m| {
        for entries in m.pending.values_mut().chain(std::iter::once(&mut m.entries)) {
            if let Some(entry) = entries.get_mut(&key) {
                entry.private = true;
            }
        }
    });
    log_err("產出清單", result);
}

/// 分享包用：`rel`（相對翻譯結果）是標了私有的產出。清單讀不到時一律當私有（不分享）。
pub fn is_private_output(work: &Path, rel: &str) -> bool {
    match read(work) {
        Ok(m) => m.entries.get(&rel.replace('\\', "/")).is_some_and(|s| s.private),
        Err(_) => true,
    }
}

/// 審查 F-e：分享包只帶確認過的產出（在清單上、內容指紋相符）。清單讀不到時一律不算。
pub fn is_confirmed_output(work: &Path, path: &Path) -> bool {
    read(work).is_ok_and(|manifest| {
        manifest
            .entries
            .get(&key_of(work, path))
            .is_some_and(|s| apply_record::file_sha256(path).as_deref() == Some(s.output_sha256.as_str()))
    })
}

/// 分享包用：產出清單記下的遊戲資料夾。
pub fn game_root(work: &Path) -> Option<PathBuf> {
    read(work).ok().map(|m| m.game_root).filter(|r| !r.is_empty()).map(PathBuf::from)
}

/// 退休候選（翻譯結果相對路徑＝遊戲相對路徑）。測試檢查用；套用一律走 [`confirm_retire`]。
#[cfg(test)]
pub fn retire_candidates(work: &Path) -> HashSet<String> {
    read(work).map(|m| m.retiring.into_keys().collect()).unwrap_or_default()
}

/// 套用時的最後確認（審查第三輪 2）：只回傳「產出清單屬於這個遊戲資料夾、而且來源現在仍確定不在」的檔。
/// 來源又回來的（例如模組裝回去）從候選移除並恢復舊條目——之後照一般規則確認（來源變了就列需重翻）。
pub fn confirm_retire(work: &Path, mc: &Path) -> HashSet<String> {
    let result = with_manifest(work, |m| {
        if m.game_key.is_empty() || m.game_key != game_key(mc) {
            return HashSet::new();
        }
        let mut confirmed = HashSet::new();
        let keys: Vec<String> = m.retiring.keys().cloned().collect();
        for key in keys {
            let source = m.retiring[&key].source.clone();
            match source_state(mc, &source) {
                SourceState::Present => {
                    if let Some(entry) = m.retiring.remove(&key) {
                        m.entries.insert(key, entry);
                    }
                }
                SourceState::Gone => {
                    confirmed.insert(key);
                }
                SourceState::Unconfirmed => {}
            }
        }
        confirmed
    });
    result.unwrap_or_else(|e| {
        crate::dev_log!("apply", "讀不到產出清單，這次不退休任何檔：{e}");
        HashSet::new()
    })
}

/// 套用前：只留「確認過的產出、內容未動、來源未變」的文字檔。
pub fn drop_unconfirmed(work: &Path, mc: &Path, plan: &mut ApplyPlan, record: &ApplyRecord) -> Dropped {
    // 清單讀不到：無法確認任何文字檔，全部不放（列為舊產物）
    let sources = read(work).map(|m| m.entries).unwrap_or_default();
    let index = super::tool_products::ToolIndex::for_game(mc);
    let mut dropped = Dropped::default();
    plan.items.retain(|item| {
        let ItemSource::File(file) = &item.source else { return true };
        // 模組檔（jar_sources 另外檢查）、主翻譯資源包（每輪重建）不在這份清單管
        if matches!(item.group, Group::Mods | Group::Zip | Group::ZipMeta) {
            return true;
        }
        let dest_rel = apply_record::rel_key(mc, &item.dest);
        let Some(source) = sources.get(&key_of(work, file)) else {
            crate::dev_log!("apply", "{dest_rel}：不是確認過的產出，不放進遊戲");
            dropped.stale.push(dest_rel);
            return false;
        };
        if apply_record::file_sha256(file).as_deref() != Some(source.output_sha256.as_str()) {
            crate::dev_log!("apply", "{dest_rel}：翻譯結果裡的檔在產出後被改過，不放進遊戲");
            dropped.stale.push(dest_rel);
            return false;
        }
        // 審查第四輪 A：來源不在不是「來源已變更」
        match source_state(mc, &source.source) {
            SourceState::Gone => {
                crate::dev_log!("apply", "{dest_rel}：來源 {} 已被整合包移除，不放進遊戲", source.source);
                dropped.source_removed.push(dest_rel);
                return false;
            }
            SourceState::Unconfirmed => {
                crate::dev_log!("apply", "{dest_rel}：讀不到來源 {}，無法確認，這次不處理", source.source);
                dropped.unconfirmed.push(dest_rel);
                return false;
            }
            SourceState::Present => {}
        }
        if !source_unchanged(mc, source, record, &index) {
            crate::dev_log!("apply", "{dest_rel}：翻譯之後來源 {} 已變更，不放進遊戲", source.source);
            dropped.outdated.push(dest_rel);
            return false;
        }
        true
    });
    dropped
}

/// B6a-1（唯讀）：翻譯之後來源檔確定被改過的有幾個（任務、腳本等；不在、讀不到的不算）。
/// 清單屬於別的遊戲資料夾、讀不到清單都回 0（無法確認＝不判已更新）。
pub fn count_changed_sources(work: &Path, mc: &Path) -> usize {
    let Ok(manifest) = read(work) else { return 0 };
    if manifest.entries.is_empty() || (!manifest.game_key.is_empty() && manifest.game_key != game_key(mc)) {
        return 0;
    }
    let index = super::tool_products::ToolIndex::for_game(mc);
    let empty = ApplyRecord::default();
    let mut seen = HashSet::new();
    // 審查第二輪 3：來源改了之後，產出者已經完整跑過一輪處理「這一版」（已處理指紋＝現在的指紋）——
    // 沒有新產出是因為沒有可翻的字，不再算「要翻」，S15 才會消失。以指紋比對（不看修改時間）；舊資料沒有已處理指紋＝照舊。
    let processed_after_change = |s: &TextSource| {
        let Some(done) = manifest.processed.get(&s.producer) else { return false };
        let Some(sha) = done.get(&s.source) else { return false };
        apply_record::file_sha256(&mc.join(&s.source)).as_deref() == Some(sha.as_str())
    };
    manifest
        .entries
        .values()
        .filter(|s| seen.insert(s.source.clone()))
        .filter(|s| source_state(mc, &s.source) == SourceState::Present && !source_unchanged(mc, s, &empty, &index))
        .filter(|s| !processed_after_change(s))
        .count()
}

/// 來源還是翻譯當時那份原檔（審查 F-c：與 ToolIndex 同一套判斷）：
/// 遊戲裡就是它；或遊戲裡是工具版本（標記、套用紀錄、1.0.x 舊版清單），而它對應的原檔就是它。
fn source_unchanged(mc: &Path, source: &TextSource, record: &ApplyRecord, index: &super::tool_products::ToolIndex) -> bool {
    let game = mc.join(&source.source);
    let current = apply_record::file_sha256(&game);
    if current.as_deref() == Some(source.sha256.as_str()) {
        return true;
    }
    let tool_version = index.classify(&game) == super::tool_products::Provenance::Tool
        || record.is_tool_version(&source.source, current.as_deref());
    tool_version && index.original_sha(&game).as_deref() == Some(source.sha256.as_str())
}

#[cfg(test)]
#[path = "text_sources_tests.rs"]
mod tests;

/// 測試：把翻譯結果裡現有的文字檔都登記成確認過的產出（B1 的套用測試直接放檔模擬翻譯結果）。
#[cfg(test)]
pub(crate) fn mark_all_produced_for_test(work: &Path, mc: &Path) {
    for entry in walkdir::WalkDir::new(work).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !entry.file_type().is_file() || path.file_name().is_some_and(|n| n == SOURCES_FILE) {
            continue;
        }
        let rel = key_of(work, path);
        let game = mc.join(&rel);
        // 遊戲裡有同位置的檔就當來源；沒有（新增的檔）就用產出本身當來源
        let source = if long_path(&game).is_file() { game } else { path.to_path_buf() };
        record(work, path, mc, &source, &source, "test");
    }
}


