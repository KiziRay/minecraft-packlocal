//! 共享翻譯記憶（社群，隱藏、預設開啟）。
//!
//! 讓「別人翻過的、你也用得到」——但**以 `(模組, lang key, 原文)` 為單位**，不是「同一個
//! 整合包」。命中條件是三者的雜湊全中：
//! - namespace（模組 id）→ 同一個模組
//! - lang key → 同一個上下文槽（物品名／提示／死亡訊息…）
//! - 原文 → 模組沒改過這條字串（防過期）
//! 因此重用跨整合包也安全：任何含同模組同版本的包都能受惠。
//!
//! 設計原則：
//! - **完全隱藏、無勾選、預設開**；沒有網路或服務未就緒時略過，但寫入可觀測狀態（空庫／失敗／命中）。
//! - 只送**字串**（原文＋譯文＋模組 id＋key 的雜湊），**不含任何個人資料、路徑、身分**；匿名。
//! - 貢獻免登入；若本機已有 ZeitFrei session，才附帶 session 讓雲端做濫用防護。
//! - 收回來的每條仍會過佔位符守衛才採用（呼叫端負責）。
//!
//! 儲存在開發者的 Cloudflare Worker + R2（依模組 id 分片）。端點 URL 非機密。

use std::collections::{HashMap, HashSet};
use std::cell::Cell;
#[cfg(not(test))]
use std::fs;
#[cfg(not(test))]
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;

use super::discord_auth::managed_ai_session_cookie;
use super::hashutil::sha256_hex;
use super::jar_scan::LangMap;
use super::lang_provenance::{get_source, LangSource, ProvenanceMap};
use super::mech_tokens::is_poisoned_mech_translation;
use super::placeholder;
use super::secrets::MANAGED_BASE_URL;
use super::shared_contribute_queue;
use super::shared_identity;
use super::translation_quality::is_usable_zh;
use super::translation_scope::TranslationScope;
use super::turnstile::MANAGED_AI_PROTOCOL;

/// 單次請求最多帶幾條（保護 Worker／回應大小）。
const MAX_ITEMS: usize = 3000;
/// 譯文太長不共享；書本／任務長句約 8KB。
const MAX_ZH_LEN: usize = 8192;
/// 單次 HTTP 逾時（貢獻不得拖住一鍵翻譯）。
const CONTRIBUTE_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// 一輪 contribute（含 flush）牆鐘預算。
const CONTRIBUTE_WALL_BUDGET: Duration = Duration::from_secs(10);
/// 單次呼叫最多送幾個 chunk（其餘入隊稍後再試）。
const MAX_CHUNKS_PER_CALL: usize = 2;
#[cfg(not(test))]
const SWEEP_CURSOR_LIMIT: usize = 64;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SweepCursorFile {
    #[serde(default)]
    cursors: HashMap<String, usize>,
}

#[cfg(not(test))]
fn sweep_cursor_path() -> PathBuf {
    super::paths::resolve_file(Path::new("shared_tm_sweep_cursor.json"))
}

#[cfg(not(test))]
fn load_sweep_cursor_file() -> SweepCursorFile {
    fs::read_to_string(sweep_cursor_path())
        .ok()
        .and_then(|text| serde_json::from_str::<SweepCursorFile>(&text).ok())
        .unwrap_or_default()
}

#[cfg(test)]
fn load_sweep_cursor_file() -> SweepCursorFile {
    SweepCursorFile::default()
}

#[cfg(not(test))]
fn save_sweep_cursor_file(mut file: SweepCursorFile) {
    if file.cursors.len() > SWEEP_CURSOR_LIMIT {
        let mut pairs: Vec<_> = file.cursors.into_iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        file.cursors = pairs.into_iter().take(SWEEP_CURSOR_LIMIT).collect();
    }
    let path = sweep_cursor_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(path, text + "\n");
    }
}

#[cfg(test)]
fn save_sweep_cursor_file(_file: SweepCursorFile) {}

fn sweep_cursor_key(scope: &TranslationScope) -> String {
    if scope.pack_key.trim().is_empty() {
        "unknown".into()
    } else {
        scope.pack_key.trim().to_string()
    }
}

fn load_sweep_cursor(scope: &TranslationScope, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    load_sweep_cursor_file()
        .cursors
        .get(&sweep_cursor_key(scope))
        .copied()
        .unwrap_or(0)
        % len
}

fn save_sweep_cursor(scope: &TranslationScope, cursor: usize, len: usize) {
    if len == 0 {
        return;
    }
    let mut file = load_sweep_cursor_file();
    file.cursors.insert(sweep_cursor_key(scope), cursor % len);
    save_sweep_cursor_file(file);
}

thread_local! {
    static SKIP_SHARED_LOOKUP: Cell<bool> = const { Cell::new(false) };
}

pub fn skip_shared_lookup() -> bool {
    SKIP_SHARED_LOOKUP.with(|c| c.get())
}

fn set_skip_shared_lookup(skip: bool) {
    SKIP_SHARED_LOOKUP.with(|c| c.set(skip));
}

/// 略過共享庫查找（Force／診斷後重跑）；Drop 時還原。貢獻仍可進行。
pub struct SkipSharedLookupGuard {
    prev: bool,
}

impl SkipSharedLookupGuard {
    pub fn enter(skip: bool) -> Self {
        let prev = skip_shared_lookup();
        set_skip_shared_lookup(skip);
        Self { prev }
    }
}

impl Drop for SkipSharedLookupGuard {
    fn drop(&mut self) {
        set_skip_shared_lookup(self.prev);
    }
}

#[derive(Clone, Debug)]
pub struct SharedTmJob {
    pub namespace: String,
    pub key: String,
    pub source: String,
    pub context: Option<String>,
    pub scope: Option<TranslationScope>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedTmEntry {
    pub namespace: String,
    pub key: String,
    pub source: String,
    pub translated: String,
    pub context: Option<String>,
    pub scope: Option<TranslationScope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupStatus {
    /// 未查詢（空 jobs 或略過）
    Skipped,
    /// 連線／HTTP／解析失敗
    Failed,
    /// 成功但無命中（庫可能為空）
    Empty,
    /// 至少一條命中
    Hits,
}

#[derive(Debug, Clone, Default)]
pub struct LookupResult {
    pub hits: HashMap<usize, String>,
    pub status: LookupStatus,
    pub queried: usize,
}

impl Default for LookupStatus {
    fn default() -> Self {
        Self::Skipped
    }
}

impl LookupResult {
    pub fn player_note(&self) -> String {
        match self.status {
            LookupStatus::Skipped => "社群共享庫：本次未查詢".into(),
            LookupStatus::Failed => "社群共享庫：查詢失敗（已略過，不影響本機翻譯）".into(),
            LookupStatus::Empty => format!("社群共享庫：查 {} 條，命中 0", self.queried),
            LookupStatus::Hits => format!(
                "社群共享庫：查 {} 條，命中 {}",
                self.queried,
                self.hits.len()
            ),
        }
    }
}

pub fn normalize_source(source: &str) -> String {
    source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .nfc()
        .collect::<String>()
}

/// `(ns, key, src)` 的穩定鍵。取前 24 hex（96 bits，碰撞機率可忽略）。
fn short_hash(parts: &[&str]) -> String {
    let mut buf = String::new();
    for part in parts {
        buf.push_str(part);
        buf.push('\0');
    }
    sha256_hex(buf.as_bytes())[..24].to_string()
}

pub fn keyhash(ns: &str, key: &str, src: &str) -> String {
    let normalized = normalize_source(src);
    short_hash(&[ns.trim(), key.trim(), normalized.as_str()])
}

pub fn semantic_hash(key: &str, src: &str, context: Option<&str>) -> String {
    let normalized = normalize_source(src);
    short_hash(&[key.trim(), normalized.as_str(), context.unwrap_or("").trim()])
}

fn client() -> Option<reqwest::blocking::Client> {
    client_with_timeout(Duration::from_secs(20))
}

fn contribute_client() -> Option<reqwest::blocking::Client> {
    client_with_timeout(CONTRIBUTE_HTTP_TIMEOUT)
}

fn client_with_timeout(timeout: Duration) -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        // 連不上要快速放棄，不能為了共享庫拖住整個翻譯
        .connect_timeout(Duration::from_secs(5))
        .timeout(timeout)
        .build()
        .ok()
}

fn base() -> String {
    MANAGED_BASE_URL.trim_end_matches('/').to_string()
}

/// 批次查詢（相容舊呼叫）：只回命中表。
pub fn lookup(jobs: &[SharedTmJob]) -> HashMap<usize, String> {
    lookup_detailed(jobs).hits
}

/// 批次查詢：回傳命中＋可觀測狀態（空庫／失敗／命中）。
pub fn lookup_detailed(jobs: &[SharedTmJob]) -> LookupResult {
    if jobs.is_empty() || skip_shared_lookup() {
        return LookupResult {
            hits: HashMap::new(),
            status: LookupStatus::Skipped,
            queried: jobs.len(),
        };
    }
    let Some(client) = client() else {
        return LookupResult {
            hits: HashMap::new(),
            status: LookupStatus::Failed,
            queried: jobs.len(),
        };
    };

    let mut kh_by_job: Vec<String> = Vec::with_capacity(jobs.len());
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut items: Vec<Value> = Vec::new();
    for job in jobs {
        let kh = keyhash(&job.namespace, &job.key, &job.source);
        let sk = semantic_hash(&job.key, &job.source, job.context.as_deref());
        if seen.insert(format!("{}:{kh}", job.namespace), ()).is_none() {
            let mut item = json!({
                "ns": job.namespace,
                "kh": kh,
                "sk": sk,
                "ctx": job.context,
            });
            if let Some(scope) = &job.scope {
                item["pk"] = json!(scope.pack_key.clone());
                item["pks"] = json!(scope.lookup_pack_keys());
                item["pn"] = json!(scope.pack_name.clone());
            }
            // 模組身分：讓雲端分辨「不同整合包，但同一個模組的同一版本」。
            // 那種情況兩份譯文說的是同一個模組的同一個字串，一票就夠。
            if let Some(mv) = super::translation_scope::mod_identity_of(&job.namespace) {
                item["mv"] = json!(mv);
            }
            items.push(item);
        }
        kh_by_job.push(kh);
    }

    let mut found: HashMap<String, String> = HashMap::new();
    let mut any_ok = false;
    let mut any_fail = false;
    for chunk in items.chunks(MAX_ITEMS) {
        let body = json!({ "items": chunk });
        let resp = match client.post(format!("{}/tm/lookup", base())).json(&body).send() {
            Ok(r) if r.status().is_success() => {
                any_ok = true;
                r
            }
            _ => {
                any_fail = true;
                continue;
            }
        };
        let v: Value = match resp.json() {
            Ok(v) => v,
            Err(_) => {
                any_fail = true;
                continue;
            }
        };
        if let Some(obj) = v.get("hits").and_then(|h| h.as_object()) {
            for (kh, zh) in obj {
                if let Some(s) = zh.as_str() {
                    if !s.trim().is_empty() {
                        found.insert(kh.clone(), s.to_string());
                    }
                }
            }
        }
    }

    let mut out = HashMap::new();
    for (i, kh) in kh_by_job.iter().enumerate() {
        if let Some(zh) = found.get(kh) {
            if is_poisoned_mech_translation(&jobs[i].source, zh) {
                continue;
            }
            out.insert(i, zh.clone());
        }
    }

    let status = if !any_ok && any_fail {
        LookupStatus::Failed
    } else if out.is_empty() {
        LookupStatus::Empty
    } else {
        LookupStatus::Hits
    };

    LookupResult {
        hits: out,
        status,
        queried: jobs.len(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct ContributeResult {
    pub attempted: usize,
    pub accepted: usize,
    pub conflicts: usize,
    pub failed: bool,
    /// 因牆鐘／chunk 上限未送出、已寫入本機佇列的條數。
    pub deferred: usize,
    /// 佇列超出上限時丟掉的舊項目數。
    pub dropped: usize,
    pub status_note: Option<String>,
}

impl ContributeResult {
    pub fn player_note(&self) -> Option<String> {
        if self.attempted == 0 && !self.failed && self.deferred == 0 {
            return None;
        }
        if let Some(note) = self.status_note.clone() {
            return Some(append_queue_note(note, self.deferred, self.dropped));
        }
        if self.failed && self.accepted == 0 && self.deferred == 0 {
            return Some("社群共享庫：貢獻失敗已略過（不影響本機翻譯）".into());
        }
        let mut note = if self.accepted > 0 {
            format!(
                "已匿名貢獻共享庫 accepted＝{}（衝突 {}，送出 {}；與「翻譯完成後分享」無關）",
                self.accepted, self.conflicts, self.attempted
            )
        } else if self.attempted > 0 {
            format!(
                "社群共享庫：送出 {} 條，雲端未接受新項目（衝突 {}；與「翻譯完成後分享」無關）",
                self.attempted, self.conflicts
            )
        } else {
            "社群共享庫：本次貢獻已略過（不影響本機翻譯）".into()
        };
        if self.deferred > 0 {
            note.push_str(&format!(
                "；另 {} 條暫存本機佇列稍後再送（避免卡住翻譯）",
                self.deferred
            ));
        }
        if self.dropped > 0 {
            note.push_str(&format!(
                "；待同步佇列已滿，最舊的 {} 條已移到側錄檔保存（你的翻譯結果不受影響）",
                self.dropped
            ));
        }
        Some(note)
    }
}

fn append_queue_note(mut note: String, deferred: usize, dropped: usize) -> String {
    if deferred > 0 {
        note.push_str(&format!("；{} 條暫存本機佇列稍後再送", deferred));
    }
    if dropped > 0 {
        note.push_str(&format!(
            "；待同步佇列已滿，最舊的 {dropped} 條已移到側錄檔保存（你的翻譯結果不受影響）"
        ));
    }
    note
}

/// 依牆鐘與 chunk 上限，決定本輪要送的條目與延後入隊的剩餘。
pub(crate) fn split_contribute_budget(
    entries: &[SharedTmEntry],
    max_chunks: usize,
    deadline: Instant,
) -> (Vec<SharedTmEntry>, Vec<SharedTmEntry>) {
    if entries.is_empty() || max_chunks == 0 || Instant::now() >= deadline {
        return (Vec::new(), entries.to_vec());
    }
    let max_items = max_chunks.saturating_mul(MAX_ITEMS);
    if entries.len() <= max_items {
        return (entries.to_vec(), Vec::new());
    }
    let (send, defer) = entries.split_at(max_items);
    (send.to_vec(), defer.to_vec())
}

/// 本輪已成功送出的 keyhash，避免結尾／停止掃尾重複灌庫。
fn contribute_tracker() -> &'static Mutex<HashSet<String>> {
    static TRACKER: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    TRACKER.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 新一輪翻譯開始時清空（一鍵／補翻／修復）。
pub fn reset_contribute_tracker() {
    if let Ok(mut set) = contribute_tracker().lock() {
        set.clear();
    }
}

fn entry_keyhash(entry: &SharedTmEntry) -> String {
    keyhash(&entry.namespace, &entry.key, &entry.source)
}

/// 去掉本輪已送過的條目，並依 keyhash 去重。
pub fn filter_unsent_entries(entries: &[SharedTmEntry]) -> Vec<SharedTmEntry> {
    let sent = contribute_tracker().lock().ok();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entry in entries {
        let kh = entry_keyhash(entry);
        if !seen.insert(kh.clone()) {
            continue;
        }
        if sent.as_ref().is_some_and(|s| s.contains(&kh)) {
            continue;
        }
        out.push(entry.clone());
    }
    out
}

fn note_sent_entries(entries: &[SharedTmEntry]) {
    if let Ok(mut set) = contribute_tracker().lock() {
        for entry in entries {
            set.insert(entry_keyhash(entry));
        }
    }
}

/// 批次貢獻：回傳 accepted／conflicts；失敗不影響翻譯。
/// 先 flush 本機佇列，再送本次條目；網路失敗時寫入佇列稍後重試。
/// 自動略過本輪已送過的 keyhash。
/// 有牆鐘與 chunk 上限：超時剩餘入隊，不阻塞一鍵翻譯。
pub fn contribute(entries: &[SharedTmEntry]) -> ContributeResult {
    contribute_budgeted(
        entries,
        Instant::now() + CONTRIBUTE_WALL_BUDGET,
        MAX_CHUNKS_PER_CALL,
        false,
    )
}

fn contribute_budgeted(
    entries: &[SharedTmEntry],
    deadline: Instant,
    max_chunks: usize,
    skip_flush: bool,
) -> ContributeResult {
    // B2：過不了 output guard 的譯文不上傳（contribute、LangMap 掃尾共用這個入口）
    let entries = &shared_contribute_queue::guard_share_entries(entries);
    let mut total = if skip_flush {
        ContributeResult::default()
    } else {
        shared_contribute_queue::flush_pending_with_budget(deadline)
    };
    if Instant::now() >= deadline {
        if !entries.is_empty() {
            let filtered = filter_unsent_entries(entries);
            if !filtered.is_empty() {
                // 停止掃尾：不把超量整包塞進佇列（避免 Drop 卡死）
                if !skip_flush {
                    total.dropped = total
                        .dropped
                        .saturating_add(shared_contribute_queue::enqueue(&filtered));
                    total.deferred = total.deferred.saturating_add(filtered.len());
                } else {
                    total.deferred = total.deferred.saturating_add(filtered.len());
                }
            }
        }
        return total;
    }
    let filtered = filter_unsent_entries(entries);
    if filtered.is_empty() {
        return total;
    }
    let result = contribute_without_flush_budget(&filtered, deadline, max_chunks);
    merge_contribute_result(&mut total, result);
    total
}

/// 掃尾選項：限制建表與上傳，避免停止時卡在 96%。
#[derive(Debug, Clone)]
pub struct ContributeLangMapsOpts {
    pub max_entries: usize,
    pub skip_flush: bool,
    pub deadline: Instant,
    pub max_chunks: usize,
}

impl ContributeLangMapsOpts {
    /// 成功路徑結尾掃尾。
    pub fn success_sweep() -> Self {
        Self {
            max_entries: MAX_CHUNKS_PER_CALL.saturating_mul(MAX_ITEMS),
            skip_flush: false,
            deadline: Instant::now() + CONTRIBUTE_WALL_BUDGET,
            max_chunks: MAX_CHUNKS_PER_CALL,
        }
    }

    /// 使用者按停止：寧可少傳，不可卡住。
    pub fn cancel_sweep() -> Self {
        Self {
            max_entries: 3_000,
            skip_flush: true,
            deadline: Instant::now() + Duration::from_secs(3),
            max_chunks: 1,
        }
    }
}

/// 一鍵結束／停止掃尾：把 en∩zh 可用譯文貢獻（略過本輪已上傳）。
///
/// `provenance` 用來認出「哪些譯文是從本機參考包合併進來的」——那些**不上傳**，
/// 理由見 [`collect_lang_map_share_entries`]。傳 `None` 代表沒有來源資訊，
/// 這時一律照舊全部上傳（舊呼叫點的相容行為）。
pub fn contribute_lang_maps(
    en: &LangMap,
    zh: &LangMap,
    scope: &TranslationScope,
    provenance: Option<&ProvenanceMap>,
) -> ContributeResult {
    contribute_lang_maps_limited(
        en,
        zh,
        scope,
        provenance,
        ContributeLangMapsOpts::success_sweep(),
    )
}

/// 有條數／時間／flush 上限的 LangMap 貢獻。
pub fn contribute_lang_maps_limited(
    en: &LangMap,
    zh: &LangMap,
    scope: &TranslationScope,
    provenance: Option<&ProvenanceMap>,
    opts: ContributeLangMapsOpts,
) -> ContributeResult {
    let entries = collect_lang_map_share_entries(
        en,
        zh,
        scope,
        provenance,
        opts.max_entries,
        opts.deadline,
    );
    if entries.is_empty() {
        return ContributeResult::default();
    }
    let before = entries.len();
    let result = contribute_budgeted(&entries, opts.deadline, opts.max_chunks, opts.skip_flush);
    if before > result.attempted && result.attempted == 0 && !result.failed && result.deferred == 0
    {
        // 全部被 tracker 略過
        return ContributeResult::default();
    }
    result
}

/// 邊掃邊略過已送 keyhash；達 max_entries 或 deadline 即停（不掃完全包）。
/// 掃出這一輪可以貢獻給共享庫的譯文。
///
/// # 為什麼要看來源
///
/// 「本機合併參考翻譯包」是掃**你自己這台電腦**的下載／文件／桌面資料夾
/// （`merge_ref.rs`），把找到的第三方翻譯包合併進來。那些內容是別人的作品，
/// 站長選擇**不把它們上傳到共享庫**。
///
/// 但這條路過去沒有任何來源過濾，參考包的內容會跟自己翻的一起被送上去。
/// 現在依 `provenance` 跳過 [`LangSource::RefPack`]，其餘五種來源
/// （原生繁中／簡中轉繁／AI 補譯／術語表／翻譯記憶）照常上傳——
/// 共享庫的機制本身不受影響。
pub(crate) fn collect_lang_map_share_entries(
    en: &LangMap,
    zh: &LangMap,
    scope: &TranslationScope,
    provenance: Option<&ProvenanceMap>,
    max_entries: usize,
    deadline: Instant,
) -> Vec<SharedTmEntry> {
    if max_entries == 0 || Instant::now() >= deadline {
        return Vec::new();
    }
    let sent = contribute_tracker().lock().ok();
    let mut seen = HashSet::new();
    let mut candidates: Vec<(String, String, String, String)> = Vec::new();
    for (ns, en_map) in en {
        let Some(zh_map) = zh.get(ns) else {
            continue;
        };
        for (key, source) in en_map {
            let Some(translated) = zh_map.get(key) else {
                continue;
            };
            let src = source.trim();
            let tr = translated.trim();
            if src.is_empty() || tr.is_empty() || tr == src {
                continue;
            }
            if !is_usable_zh(src, tr) {
                continue;
            }
            if is_poisoned_mech_translation(src, tr) {
                continue;
            }
            // 集中判定（P0-03）：本來就不該翻的東西不得寫進共享庫。
            //
            // 共享庫的污染是**永久**的：一旦 `botania.entry.bcIntegration` 這種語言 key
            // 被當成「原文」上傳，之後所有整合包查到它都會拿到一個假譯文。
            // 這是四個出口裡最不能漏的一個。
            if !super::eligibility::classify(super::eligibility::Candidate {
                source_kind: "lang",
                logical_key: key,
                text: src,
            })
            .may_store_in_shared_data()
            {
                continue;
            }
            // 本機參考包的內容不上傳（站長選定）
            if let Some(prov) = provenance {
                if get_source(prov, ns, key) == Some(LangSource::RefPack) {
                    continue;
                }
            }
            candidates.push((ns.clone(), key.clone(), source.clone(), translated.clone()));
        }
    }
    candidates.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let start = load_sweep_cursor(scope, candidates.len());
    let mut entries: Vec<SharedTmEntry> = Vec::new();
    let mut inspected = 0usize;
    for idx in 0..candidates.len() {
        if Instant::now() >= deadline || entries.len() >= max_entries {
            break;
        }
        let actual = (start + idx) % candidates.len();
        let (ns, key, source, translated) = candidates[actual].clone();
        inspected = inspected.saturating_add(1);
        let kh = keyhash(&ns, &key, &source);
        if !seen.insert(kh.clone()) {
            continue;
        }
        if sent.as_ref().is_some_and(|s| s.contains(&kh)) {
            continue;
        }
        entries.push(SharedTmEntry {
            namespace: ns,
            key,
            source,
            translated,
            context: None,
            scope: Some(scope.clone()),
        });
    }
    if inspected > 0 {
        save_sweep_cursor(scope, start.saturating_add(inspected), candidates.len());
    }
    entries
}

/// 把過閘門的字串對貢獻進模組／pack 共享庫（OpenCC 現成繁中也走這裡）。
pub fn contribute_plain_pairs(
    pairs: &HashMap<String, String>,
    ns_by_src: &HashMap<String, String>,
    key: &str,
    scope: Option<&TranslationScope>,
) -> ContributeResult {
    let mut entries = Vec::new();
    for (src, zh) in pairs {
        let source = src.trim();
        let translated = zh.trim();
        if source.is_empty() || translated.is_empty() || translated == source {
            continue;
        }
        if !is_usable_zh(source, translated) || is_poisoned_mech_translation(source, translated) {
            continue;
        }
        let ns = ns_by_src
            .get(src)
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| shared_identity::pack_namespace(scope));
        let ns = shared_identity::sanitize_share_ns(&ns, scope);
        entries.push(SharedTmEntry {
            namespace: ns,
            key: key.to_string(),
            source: src.clone(),
            translated: translated.to_string(),
            context: None,
            scope: scope.cloned(),
        });
    }
    if entries.is_empty() {
        return ContributeResult::default();
    }
    contribute(&entries)
}

fn merge_contribute_result(into: &mut ContributeResult, other: ContributeResult) {
    into.attempted = into.attempted.saturating_add(other.attempted);
    into.accepted = into.accepted.saturating_add(other.accepted);
    into.conflicts = into.conflicts.saturating_add(other.conflicts);
    into.deferred = into.deferred.saturating_add(other.deferred);
    into.dropped = into.dropped.saturating_add(other.dropped);
    if into.status_note.is_none() {
        into.status_note = other.status_note;
    }
    into.failed = into.failed || other.failed;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContributePostOutcome {
    Accepted { accepted: usize, conflicts: usize },
    Unauthorized,
    UpgradeRequired,
    RateLimited,
    Failed,
}

fn contribute_status_note(outcome: ContributePostOutcome) -> Option<String> {
    match outcome {
        ContributePostOutcome::Unauthorized => {
            Some("雲端尚未更新，貢獻暫存佇列".into())
        }
        ContributePostOutcome::UpgradeRequired => {
            Some("社群共享庫：雲端要求新版協定，請更新工具後再貢獻".into())
        }
        ContributePostOutcome::RateLimited => {
            Some("社群共享庫：雲端忙碌或速率限制（429），貢獻暫存佇列".into())
        }
        ContributePostOutcome::Failed => {
            Some("社群共享庫：貢獻失敗，已暫存本機佇列稍後再送".into())
        }
        ContributePostOutcome::Accepted { .. } => None,
    }
}

fn apply_contribute_headers(
    req: reqwest::blocking::RequestBuilder,
    session: Option<String>,
) -> reqwest::blocking::RequestBuilder {
    let req = req
        .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
        .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"));
    if let Some(session) = session.filter(|s| !s.trim().is_empty()) {
        req.header("X-Zeitfrei-Session", session)
    } else {
        req
    }
}

fn optional_session_cookie() -> Option<String> {
    managed_ai_session_cookie().ok()
}

fn post_contribute_chunk(
    client: &reqwest::blocking::Client,
    body: &Value,
) -> ContributePostOutcome {
    let req = client
        .post(format!("{}/tm/contribute", base()))
        .json(body);
    let resp = match apply_contribute_headers(req, optional_session_cookie()).send() {
        Ok(resp) => resp,
        Err(_) => return ContributePostOutcome::Failed,
    };
    let status = resp.status();
    if status.is_success() {
        if let Ok(v) = resp.json::<Value>() {
            return ContributePostOutcome::Accepted {
                accepted: v.get("accepted").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
                conflicts: v.get("conflicts").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
            };
        }
        return ContributePostOutcome::Accepted {
            accepted: 0,
            conflicts: 0,
        };
    }
    match status.as_u16() {
        401 => ContributePostOutcome::Unauthorized,
        426 => ContributePostOutcome::UpgradeRequired,
        429 => ContributePostOutcome::RateLimited,
        _ => ContributePostOutcome::Failed,
    }
}

/// 直接貢獻、不先 flush（給佇列 flush 用，避免遞迴）。仍做 payload keyhash 去重。
/// 有牆鐘／chunk 上限；超時或超過 chunk 的條目寫入佇列。
pub(crate) fn contribute_without_flush_budget(
    entries: &[SharedTmEntry],
    deadline: Instant,
    max_chunks: usize,
) -> ContributeResult {
    // B2：佇列 flush 送舊條目也走這裡，同樣先過 output guard
    let entries = &shared_contribute_queue::guard_share_entries(entries);
    if entries.is_empty() {
        return ContributeResult::default();
    }
    let mut seen = HashSet::new();
    let filtered: Vec<SharedTmEntry> = entries
        .iter()
        .filter(|entry| {
            let src = entry.source.trim();
            let zh = entry.translated.trim();
            !src.is_empty()
                && !zh.is_empty()
                && zh.len() <= MAX_ZH_LEN
                && zh != src
                && placeholder::is_compatible(src, zh)
                && is_usable_zh(src, zh)
                && !is_poisoned_mech_translation(src, zh)
        })
        .filter(|entry| seen.insert(entry_keyhash(entry)))
        .cloned()
        .collect();
    if filtered.is_empty() {
        return ContributeResult::default();
    }

    let (to_send, deferred_by_budget) =
        split_contribute_budget(&filtered, max_chunks, deadline);

    if to_send.is_empty() {
        let dropped = if deferred_by_budget.is_empty() {
            0
        } else {
            shared_contribute_queue::enqueue(&deferred_by_budget)
        };
        return ContributeResult {
            deferred: deferred_by_budget.len(),
            dropped,
            ..ContributeResult::default()
        };
    }

    let Some(client) = contribute_client() else {
        let dropped = shared_contribute_queue::enqueue(&to_send)
            .saturating_add(shared_contribute_queue::enqueue(&deferred_by_budget));
        return ContributeResult {
            failed: true,
            attempted: to_send.len(),
            deferred: deferred_by_budget.len().saturating_add(to_send.len()),
            dropped,
            status_note: contribute_status_note(ContributePostOutcome::Failed),
            ..ContributeResult::default()
        };
    };

    let mut accepted = 0usize;
    let mut conflicts = 0usize;
    let mut any_ok = false;
    let mut any_fail = false;
    let mut failed_entries: Vec<SharedTmEntry> = Vec::new();
    let mut sent_ok: Vec<SharedTmEntry> = Vec::new();
    let mut attempted = 0usize;
    let mut deferred_mid = 0usize;
    let mut dropped = 0usize;
    let mut status_note: Option<String> = None;
    let mut saw_upgrade_required = false;

    let mut chunk_iter = to_send.chunks(MAX_ITEMS).peekable();
    while let Some(chunk) = chunk_iter.next() {
        if Instant::now() >= deadline {
            let mut rest: Vec<SharedTmEntry> = chunk.to_vec();
            while let Some(more) = chunk_iter.next() {
                rest.extend_from_slice(more);
            }
            dropped = dropped.saturating_add(shared_contribute_queue::enqueue(&rest));
            deferred_mid = deferred_mid.saturating_add(rest.len());
            break;
        }
        attempted += chunk.len();
        let items: Vec<Value> = chunk
            .iter()
            .map(|entry| {
                let mut item = json!({
                    "ns": entry.namespace.clone(),
                    "kh": keyhash(&entry.namespace, &entry.key, &entry.source),
                    "sk": semantic_hash(&entry.key, &entry.source, entry.context.as_deref()),
                    "ctx": entry.context.clone(),
                    "zh": entry.translated.trim(),
                });
                if let Some(scope) = &entry.scope {
                    item["pk"] = json!(scope.pack_key.clone());
                    item["pks"] = json!(scope.lookup_pack_keys());
                    item["pn"] = json!(scope.pack_name.clone());
                }
                // 見查詢端同名欄位的說明
                if let Some(mv) = super::translation_scope::mod_identity_of(&entry.namespace) {
                    item["mv"] = json!(mv);
                }
                item
            })
            .collect();
        let body = json!({ "items": items });
        match post_contribute_chunk(&client, &body) {
            ContributePostOutcome::Accepted {
                accepted: chunk_accepted,
                conflicts: chunk_conflicts,
            } => {
                any_ok = true;
                sent_ok.extend_from_slice(chunk);
                accepted = accepted.saturating_add(chunk_accepted);
                conflicts = conflicts.saturating_add(chunk_conflicts);
            }
            outcome => {
                any_fail = true;
                if outcome == ContributePostOutcome::UpgradeRequired {
                    saw_upgrade_required = true;
                    status_note = contribute_status_note(outcome);
                } else {
                    if status_note.is_none() {
                        status_note = contribute_status_note(outcome);
                    }
                    failed_entries.extend(chunk.iter().cloned());
                }
            }
        }
    }

    if !saw_upgrade_required {
        if !deferred_by_budget.is_empty() {
            dropped = dropped.saturating_add(shared_contribute_queue::enqueue(&deferred_by_budget));
        }
        if !failed_entries.is_empty() {
            dropped = dropped.saturating_add(shared_contribute_queue::enqueue(&failed_entries));
        }
    }
    if any_ok && !sent_ok.is_empty() {
        note_sent_entries(&sent_ok);
    }

    ContributeResult {
        attempted,
        accepted,
        conflicts,
        failed: any_fail && !any_ok,
        deferred: if saw_upgrade_required {
            deferred_mid
        } else {
            deferred_by_budget
                .len()
                .saturating_add(deferred_mid)
                .saturating_add(failed_entries.len())
        },
        dropped,
        status_note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    static TRACKER_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn tracker_test_lock() -> std::sync::MutexGuard<'static, ()> {
        TRACKER_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("tracker test lock")
    }

    #[test]
    fn keyhash_is_stable_and_scoped() {
        let a = keyhash("create", "item.create.wrench", "Wrench");
        assert_eq!(a, keyhash("create", "item.create.wrench", "Wrench"));
        assert_ne!(a, keyhash("mekanism", "item.create.wrench", "Wrench"));
        assert_ne!(a, keyhash("create", "create.tooltip.wrench", "Wrench"));
        assert_ne!(a, keyhash("create", "item.create.wrench", "Spanner"));
        assert_eq!(a.len(), 24);
    }

    #[test]
    fn hashes_normalize_layout_but_keep_context_separate() {
        assert_eq!(
            keyhash("create", "item.create.wrench", "Wrench\n"),
            keyhash("create", "item.create.wrench", "  Wrench  ")
        );
        assert_ne!(
            semantic_hash("item.create.wrench", "Wrench", Some("物品名")),
            semantic_hash("item.create.wrench", "Wrench", Some("提示說明"))
        );
    }

    #[test]
    fn normalize_source_applies_nfc() {
        // NFD: e + combining acute；NFC: precomposed é
        let nfd = "cafe\u{0301}";
        let nfc = "caf\u{00e9}";
        assert_eq!(normalize_source(nfd), normalize_source(nfc));
        assert_eq!(
            keyhash("mod", "key", nfd),
            keyhash("mod", "key", nfc)
        );
    }

    #[test]
    fn contribute_rejects_poisoned_resource_path() {
        let entries = vec![SharedTmEntry {
            namespace: "alexsmobs".into(),
            key: "book.text".into(),
            source: "root.txt".into(),
            translated: "根.txt".into(),
            context: None,
            scope: None,
        }];
        let result = contribute_without_flush_budget(
            &entries,
            Instant::now() + Duration::from_secs(1),
            2,
        );
        assert_eq!(result.attempted, 0);
    }

    #[test]
    fn empty_inputs_are_noops() {
        assert!(lookup(&[]).is_empty());
        let detailed = lookup_detailed(&[]);
        assert_eq!(detailed.status, LookupStatus::Skipped);
        // 不走 public contribute（會先 flush 本機佇列，可能受 APPDATA 殘檔影響）
        assert_eq!(
            contribute_without_flush_budget(&[], Instant::now() + Duration::from_secs(1), 2)
                .attempted,
            0
        );
    }

    #[test]
    fn contribute_tracker_skips_already_noted() {
        let _lock = tracker_test_lock();
        reset_contribute_tracker();
        let entry = SharedTmEntry {
            namespace: "testmod".into(),
            key: "item.test".into(),
            source: "Hello".into(),
            translated: "哈囉".into(),
            context: None,
            scope: None,
        };
        note_sent_entries(&[entry.clone()]);
        let filtered = filter_unsent_entries(&[entry.clone(), entry]);
        assert!(filtered.is_empty());
        reset_contribute_tracker();
        let again = filter_unsent_entries(&[SharedTmEntry {
            namespace: "testmod".into(),
            key: "item.test".into(),
            source: "Hello".into(),
            translated: "哈囉".into(),
            context: None,
            scope: None,
        }]);
        assert_eq!(again.len(), 1);
    }

    fn sample(i: usize) -> SharedTmEntry {
        SharedTmEntry {
            namespace: "mod".into(),
            key: format!("k{i}"),
            source: format!("Source {i}"),
            translated: format!("譯文{i}"),
            context: None,
            scope: None,
        }
    }

    #[test]
    fn split_contribute_budget_respects_chunk_cap() {
        let entries: Vec<_> = (0..7000).map(sample).collect();
        let deadline = Instant::now() + Duration::from_secs(60);
        let (send, defer) = split_contribute_budget(&entries, 2, deadline);
        assert_eq!(send.len(), 6000);
        assert_eq!(defer.len(), 1000);
    }

    #[test]
    fn split_contribute_budget_expired_defers_all() {
        let entries = vec![sample(0), sample(1)];
        let deadline = Instant::now() - Duration::from_secs(1);
        let (send, defer) = split_contribute_budget(&entries, 2, deadline);
        assert!(send.is_empty());
        assert_eq!(defer.len(), 2);
    }

    #[test]
    fn player_note_mentions_deferred() {
        let note = ContributeResult {
            attempted: 10,
            accepted: 8,
            conflicts: 0,
            failed: false,
            deferred: 100,
            dropped: 0,
            status_note: None,
        }
        .player_note()
        .expect("note");
        assert!(note.contains("100"));
        assert!(note.contains("佇列"));
    }

    #[test]
    fn contribute_without_session_headers_still_allow_attempt_and_queue_note() {
        let client = contribute_client().expect("client");
        let request = apply_contribute_headers(
            client
                .post("https://example.com/tm/contribute")
                .json(&serde_json::json!({ "items": [] })),
            None,
        )
        .build()
        .expect("request");
        assert!(request.headers().contains_key("X-Zeitfrei-AI-Protocol"));
        assert!(request.headers().contains_key("X-Zeitfrei-Client-Version"));
        assert!(!request.headers().contains_key("X-Zeitfrei-Session"));

        let note = ContributeResult {
            attempted: 1,
            accepted: 0,
            conflicts: 0,
            failed: true,
            deferred: 1,
            dropped: 0,
            status_note: contribute_status_note(ContributePostOutcome::Unauthorized),
        }
        .player_note()
        .expect("note");
        assert!(note.contains("雲端尚未更新"));
        assert!(note.contains("暫存"));
    }

    fn lang_pair(n: usize) -> (LangMap, LangMap) {
        let mut en: LangMap = HashMap::new();
        let mut zh: LangMap = HashMap::new();
        let en_ns = en.entry("mod".into()).or_default();
        let zh_ns = zh.entry("mod".into()).or_default();
        for i in 0..n {
            en_ns.insert(format!("k{i}"), format!("Source {i}"));
            zh_ns.insert(format!("k{i}"), format!("譯文{i}"));
        }
        (en, zh)
    }

    #[test]
    fn collect_lang_map_respects_max_entries() {
        let _lock = tracker_test_lock();
        reset_contribute_tracker();
        let (en, zh) = lang_pair(50);
        let scope = TranslationScope::from_name("Test Pack");
        let got = collect_lang_map_share_entries(
            &en,
            &zh,
            &scope,
            None,
            10,
            Instant::now() + Duration::from_secs(5),
        );
        assert_eq!(got.len(), 10);
    }

    #[test]
    fn collect_lang_map_skips_already_sent() {
        let _lock = tracker_test_lock();
        reset_contribute_tracker();
        let (en, zh) = lang_pair(5);
        let scope = TranslationScope::from_name("Test Pack");
        let first = collect_lang_map_share_entries(
            &en,
            &zh,
            &scope,
            None,
            5,
            Instant::now() + Duration::from_secs(5),
        );
        assert_eq!(first.len(), 5);
        note_sent_entries(&first);
        let again = collect_lang_map_share_entries(
            &en,
            &zh,
            &scope,
            None,
            5,
            Instant::now() + Duration::from_secs(5),
        );
        assert!(again.is_empty());
        reset_contribute_tracker();
    }

    #[test]
    fn collect_lang_map_expired_deadline_returns_empty() {
        let _lock = tracker_test_lock();
        reset_contribute_tracker();
        let (en, zh) = lang_pair(20);
        let scope = TranslationScope::from_name("Test Pack");
        let got = collect_lang_map_share_entries(
            &en,
            &zh,
            &scope,
            None,
            20,
            Instant::now() - Duration::from_secs(1),
        );
        assert!(got.is_empty());
    }

    #[test]
    fn cancel_sweep_opts_are_strict() {
        let opts = ContributeLangMapsOpts::cancel_sweep();
        assert!(opts.skip_flush);
        assert_eq!(opts.max_entries, 3_000);
        assert_eq!(opts.max_chunks, 1);
    }

    #[test]
    fn long_zh_under_8kb_passes_share_len_gate() {
        let zh = "翻".repeat(2000);
        assert!(zh.len() > 400 && zh.len() <= 8192);
        let src = "A long book page about gears and quests.";
        assert!(is_usable_zh(src, &zh));
        assert!(placeholder::is_compatible(src, &zh));
    }

    #[test]
    fn skip_shared_lookup_guard_skips_query() {
        let _g = SkipSharedLookupGuard::enter(true);
        let jobs = vec![SharedTmJob {
            namespace: "create".into(),
            key: "item.x".into(),
            source: "Wrench".into(),
            context: None,
            scope: None,
        }];
        let hits = lookup(&jobs);
        assert!(hits.is_empty());
        let detailed = lookup_detailed(&jobs);
        assert_eq!(detailed.status, LookupStatus::Skipped);
    }
}

#[cfg(test)]
#[path = "shared_tm_b2_tests.rs"]
mod b2_tests;
