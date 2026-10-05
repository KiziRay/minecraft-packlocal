//! B6a-2：主資源包裡的書本（Patchouli／GuideME／Lavender 的 `pack-assets/assets/…`）的來源指紋。
//!
//! 書本的譯文不是整檔放進遊戲，而是被打包進主翻譯資源包 zip；整合包更新後（模組 JAR、資料包 ZIP、
//! 資料夾資源包換了新版），舊書頁譯文會蓋掉新內容。原則同 B3：必須先確認「這本書的來源還是翻譯當時那份」，
//! 才能把它放進遊戲。
//!
//! - 翻譯時：JAR／ZIP 來源的書本寫進 `pack-assets` 之後，用 [`record_pack_assets`] 記下來源指紋
//!   （資料夾資源包、kubejs 等來源由 text_overlay 自己記）。
//! - 套用前：[`filter_main_pack`] 檢查主 zip 裡每一個書頁；來源改了、已不在、讀不到、清單上沒有紀錄
//!   的書頁，從主 zip 的**過濾副本**拿掉（原 zip 不動），分類列出。語言檔（`assets/<ns>/lang/`）不動。

use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::apply_plan::{ApplyPlan, Group, ItemSource};
use super::apply_record::{self, ApplyRecord};
use super::pack_assets::PACK_ASSETS_DIR;
use super::paths::long_path;
use super::text_sources::{self, AlsoSource, SourceState, TextSource};

/// 把 `staged_root/pack-assets` 底下已搬進 `work/pack-assets` 的檔記成確認過的產出。
/// `sources_for(相對 pack-assets 的路徑)` 回傳 `(遊戲裡的來源, 實際讀的原檔)`；第一個是主要來源，其餘記為 also。
pub fn record_pack_assets(
    work: &Path,
    staged_root: &Path,
    mc: &Path,
    producer: &str,
    sources_for: &dyn Fn(&Path) -> Vec<(PathBuf, PathBuf)>,
) {
    let base = staged_root.join(PACK_ASSETS_DIR);
    if !base.is_dir() {
        return;
    }
    let mut records: Vec<(String, TextSource)> = Vec::new();
    for entry in WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(&base) else { continue };
        let out = work.join(PACK_ASSETS_DIR).join(rel);
        let Some(output_sha256) = apply_record::file_sha256(&out) else { continue };
        let srcs = sources_for(rel);
        let mut shas = Vec::new();
        for (game, read) in &srcs {
            let Some(sha) = apply_record::file_sha256(read) else { shas.clear(); break };
            shas.push((apply_record::rel_key(mc, game), sha));
        }
        if shas.is_empty() {
            continue;
        }
        let (source, sha256) = shas.remove(0);
        records.push((
            text_sources::key_of(work, &out),
            TextSource {
                source,
                sha256,
                output_sha256,
                producer: producer.to_string(),
                private: false,
                also: shas.into_iter().map(|(source, sha256)| AlsoSource { source, sha256 }).collect(),
            },
        ));
    }
    if records.is_empty() {
        return;
    }
    let result = text_sources::with_manifest(work, |m| {
        m.game_root = mc.display().to_string();
        for (key, entry) in records {
            match m.pending.get_mut(producer) {
                Some(pending) => {
                    pending.insert(key, entry);
                }
                None => {
                    m.entries.insert(key, entry);
                }
            }
        }
    });
    if let Err(e) = result {
        crate::dev_log!("translate", "書本來源指紋：{e}");
    }
}

/// 主 zip 裡書頁的檢查結果（zip 項目名，例 `assets/ns/patchouli_books/b/zh_tw/e.json`）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BookCheck {
    /// 來源在翻譯之後被改過（已變更）
    pub outdated: Vec<String>,
    /// 來源已被整合包移除（確定不在）
    pub removed: Vec<String>,
    /// 讀不到來源、或產出清單讀不到：無法確認，遊戲裡原本的書頁維持現狀
    pub unconfirmed: Vec<String>,
}

impl BookCheck {
    pub fn is_empty(&self) -> bool {
        self.outdated.is_empty() && self.removed.is_empty() && self.unconfirmed.is_empty()
    }
}

/// 主資源包只有三種東西：pack.mcmeta、`assets/<ns>/lang/zh_tw.json`（來自英文原文表，不是書）、
/// `pack-assets/assets/…` 的書本。所以 `assets/` 底下不是語言檔的，都視為書頁。
fn is_book_entry(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    name.starts_with("assets/") && !(parts.len() == 4 && parts[2] == "lang")
}

/// 主 zip 裡「必須先確認來源」的書頁，逐一判斷。清單讀不到時（無法確認哪些書頁是誰產的、來源是什麼）
/// 全部書頁都列為無法確認——與 `drop_unconfirmed`「清單讀不到全部不放」同一個方向。
pub fn check_entries(work: &Path, mc: &Path, record: &ApplyRecord, in_zip: &HashSet<String>) -> BookCheck {
    let mut out = BookCheck::default();
    let (Some(mut entries), Some(retiring)) = (text_sources::entries_for(work), text_sources::retiring_for(work)) else {
        out.unconfirmed = in_zip.iter().filter(|n| is_book_entry(n)).cloned().collect();
        out.unconfirmed.sort();
        return out;
    };
    // 已退休的書頁（產出者完整跑完時來源已不在）不在 entries，但失敗輪會把舊書頁併回 pack-assets：
    // 一樣要先確認來源，不能因為清單上「沒有條目」就直接放進遊戲。entries 有的以 entries 為準。
    for (key, source) in retiring {
        entries.entry(key).or_insert(source);
    }
    let index = super::tool_products::ToolIndex::for_game(mc);
    let prefix = format!("{PACK_ASSETS_DIR}/");
    // 主 zip 裡的書頁在清單（entries＋retiring）上完全沒有紀錄（記錄指紋失敗、舊版結果）＝無法確認來源，
    // 與 `drop_unconfirmed`「沒條目就不放」同一個方向。
    let known: HashSet<&str> = entries.keys().filter_map(|k| k.strip_prefix(prefix.as_str())).collect();
    out.unconfirmed = in_zip.iter().filter(|n| is_book_entry(n) && !known.contains(n.as_str())).cloned().collect();
    for (key, source) in entries {
        let Some(name) = key.strip_prefix(&prefix) else { continue };
        if !in_zip.contains(name) {
            continue;
        }
        match text_sources::source_state(mc, &source.source) {
            SourceState::Gone => out.removed.push(name.to_string()),
            SourceState::Unconfirmed => out.unconfirmed.push(name.to_string()),
            SourceState::Present => {
                if !text_sources::source_unchanged(mc, &source, record, &index) {
                    out.outdated.push(name.to_string());
                }
            }
        }
    }
    out.outdated.sort();
    out.removed.sort();
    out.unconfirmed.sort();
    out
}

fn zip_names(path: &Path) -> Option<HashSet<String>> {
    let file = File::open(long_path(path)).ok()?;
    let archive = zip::ZipArchive::new(file).ok()?;
    Some(archive.file_names().map(|n| n.replace('\\', "/")).collect())
}

/// 過濾副本：排除 `exclude`；`carry`（無法確認的書頁）改從遊戲裡現有的同名主 zip 帶過來，維持現狀。
fn write_filtered(src: &Path, dest: &Path, exclude: &HashSet<String>, carry_from: Option<(&Path, &HashSet<String>)>) -> Result<(), String> {
    #[cfg(test)]
    if FORCE_FAIL.with(|f| f.get()) {
        return Err("測試：強制失敗".into());
    }
    let file = File::open(long_path(src)).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let out = File::create(dest).map_err(|e| e.to_string())?;
    let mut writer = zip::ZipWriter::new(out);
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if exclude.contains(&entry.name().replace('\\', "/")) {
            continue;
        }
        writer.raw_copy_file(entry).map_err(|e| e.to_string())?;
    }
    if let Some((game_zip, carry)) = carry_from {
        // 遊戲裡還沒有主 zip（第一次套用）＝沒有現狀可維持，這些書頁不放。
        // 有但讀不到 → 回錯誤停下，不能靜默少放（呼叫端已先檢查過一次，這裡防檢查後才壞掉）。
        if !carry.is_empty() && game_pack_present(&game_zip)? {
            let f = File::open(long_path(game_zip)).map_err(|e| unreadable_game_pack(game_zip, &e.to_string()))?;
            let mut old = zip::ZipArchive::new(f).map_err(|e| unreadable_game_pack(game_zip, &e.to_string()))?;
            for i in 0..old.len() {
                let entry = old.by_index(i).map_err(|e| unreadable_game_pack(game_zip, &e.to_string()))?;
                if carry.contains(&entry.name().replace('\\', "/")) {
                    writer.raw_copy_file(entry).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(())
}

const UNREADABLE_GAME_PACK: &str = "讀不到遊戲裡現有的資源包";

/// 遊戲裡有沒有同名主 zip。判斷不了（權限、網路磁碟抖動）不能當成「沒有」，回錯誤停下。
fn game_pack_present(game_zip: &Path) -> Result<bool, String> {
    long_path(game_zip).try_exists().map_err(|e| unreadable_game_pack(game_zip, &e.to_string()))
}

fn unreadable_game_pack(game_zip: &Path, detail: &str) -> String {
    format!(
        "{UNREADABLE_GAME_PACK}（{}：{detail}），有些書頁無法確認來源、也無法維持原狀，這次沒有套用（一個檔都沒動）。\
         請確認遊戲已關閉、這個檔沒有被其他程式佔用，再按一次「套用到遊戲」",
        game_zip.display()
    )
}

#[cfg(test)]
thread_local! {
    static FORCE_FAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LAST_TEMP: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn force_filter_failure_for_test(on: bool) {
    FORCE_FAIL.with(|f| f.set(on));
}

#[cfg(test)]
pub(crate) fn last_temp_dir_for_test() -> Option<PathBuf> {
    LAST_TEMP.with(|t| t.borrow().clone())
}

/// 過濾副本的暫存資料夾；套用結束（成功或失敗）時清掉。
pub struct TempCopy(Option<PathBuf>);

impl Drop for TempCopy {
    fn drop(&mut self) {
        if let Some(dir) = self.0.take() {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

fn unique_temp_dir() -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("mcpl-apply-filtered-{}-{n}-{nanos}", std::process::id()))
}

/// 套用前：主 zip 裡來源已變更／已被移除的書頁拿掉（換成過濾副本）；無法確認的維持遊戲現狀
/// （從遊戲裡現有的主 zip 帶過來，沒有就不放；有但讀不到就停下）。三類分別併入 `dropped`（白話列出）。
/// 要拿掉書頁卻寫不出過濾副本 → 回錯誤、整個套用停下（還沒寫任何遊戲檔）。
/// 回傳的 [`TempCopy`] 要留到套用結束（Drop 時清掉暫存副本）。
pub fn filter_main_pack(
    work: &Path,
    mc: &Path,
    plan: &mut ApplyPlan,
    record: &ApplyRecord,
    dropped: &mut text_sources::Dropped,
) -> Result<TempCopy, String> {
    let Some(item) = plan.items.iter_mut().find(|i| i.group == Group::Zip) else { return Ok(TempCopy(None)) };
    let ItemSource::File(zip) = &item.source else { return Ok(TempCopy(None)) };
    let zip = zip.clone();
    let Some(names) = zip_names(&zip) else { return Ok(TempCopy(None)) };
    let check = check_entries(work, mc, record, &names);
    if check.is_empty() {
        return Ok(TempCopy(None));
    }
    let Some(file_name) = zip.file_name() else { return Ok(TempCopy(None)) };
    let dir = unique_temp_dir();
    #[cfg(test)]
    LAST_TEMP.with(|t| *t.borrow_mut() = Some(dir.clone()));
    let temp = TempCopy(Some(dir.clone()));
    let dest = dir.join(file_name);
    let exclude: HashSet<String> =
        check.outdated.iter().chain(&check.removed).chain(&check.unconfirmed).cloned().collect();
    let carry: HashSet<String> = check.unconfirmed.iter().cloned().collect();
    let game_zip = item.dest.clone();
    if !carry.is_empty() && game_pack_present(&game_zip)? {
        let readable = File::open(long_path(&game_zip))
            .map_err(|e| e.to_string())
            .and_then(|f| zip::ZipArchive::new(f).map(|_| ()).map_err(|e| e.to_string()));
        if let Err(e) = readable {
            return Err(unreadable_game_pack(&game_zip, &e));
        }
    }
    write_filtered(&zip, &dest, &exclude, Some((&game_zip, &carry))).map_err(|e| {
        if e.starts_with(UNREADABLE_GAME_PACK) {
            e
        } else {
            format!("書本的來源有變，但無法產生排除它們的資源包，沒有套用（一個檔都沒動）：{e}")
        }
    })?;
    remember_filtered(work, &zip, &dest);
    let label = |name: &String| format!("主資源包：{name}");
    for name in &check.outdated {
        crate::dev_log!("apply", "主資源包裡的書頁 {name}：翻譯之後來源已變更，這次不放進遊戲");
    }
    for name in &check.removed {
        crate::dev_log!("apply", "主資源包裡的書頁 {name}：來源已被整合包移除，這次不放進遊戲");
    }
    for name in &check.unconfirmed {
        crate::dev_log!("apply", "主資源包裡的書頁 {name}：讀不到來源或產出清單，無法確認，維持遊戲現狀");
    }
    dropped.outdated.extend(check.outdated.iter().map(label));
    dropped.source_removed.extend(check.removed.iter().map(label));
    dropped.unconfirmed.extend(check.unconfirmed.iter().map(label));
    item.source = ItemSource::File(dest);
    Ok(temp)
}

pub const FILTERED_FILE: &str = ".mcpl-filtered-packs.json";

/// 過濾副本的指紋 → 翻譯結果裡原 zip 的指紋（B5c「最新一輪有沒有套用」認得過濾後的版本）。
/// 記不下來不影響套用（只會讓那個判斷偏保守）。
fn remember_filtered(work: &Path, original: &Path, copy: &Path) {
    let (Some(orig), Some(copy_sha)) = (apply_record::file_sha256(original), apply_record::file_sha256(copy)) else { return };
    let path = work.join(FILTERED_FILE);
    let mut map: std::collections::BTreeMap<String, String> = fs::read_to_string(long_path(&path))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    map.insert(copy_sha, orig);
    if let Ok(text) = serde_json::to_string_pretty(&map) {
        if let Err(e) = apply_record::write_atomic(&path, text.as_bytes()) {
            crate::dev_log!("apply", "無法記下過濾副本的指紋：{e}");
        }
    }
}

/// `sha`（遊戲裡主 zip 的指紋）若是某次過濾副本，回傳它對應的原 zip 指紋。
pub fn original_of_filtered(work: &Path, sha: &str) -> Option<String> {
    let text = fs::read_to_string(long_path(&work.join(FILTERED_FILE))).ok()?;
    let map: std::collections::BTreeMap<String, String> = serde_json::from_str(&text).ok()?;
    map.get(sha).cloned()
}
