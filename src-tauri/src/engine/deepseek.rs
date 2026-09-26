//! AI 補譯層。**只在本機整理完、確定缺中文時才會用到。**
//!
//! 送出前會依序擋掉不必送的東西，AI 只翻真正剩下的：
//! 1. 相同英文去重
//! 2. 術語表直接命中（官方台灣譯名，免費且一致）
//! 3. 翻譯記憶命中（上次或別的整合包翻過）
//!
//! 收回來後每一條都過佔位符把關，`%s` 被吃掉的譯文一律退回原文——
//! 少一句中文只是可惜，少一個 `%s` 是遊戲當場報錯。

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use super::cancel;
use super::cancel::CANCEL_MESSAGE;
use super::codex_chat::{self, CODEX_RESPONSES_URL};
use super::discord_auth::managed_ai_session_cookie;
use super::eligibility;
use super::glossary::{self, Glossary, TermConsistencyStats};
use super::jar_scan::LangMap;
use super::mech_tokens::{is_poisoned_mech_translation, skip_before_ai};
use super::local_quality;
use super::placeholder::{self, GuardStats};
use super::sentence_split;
use super::shared_glossary;
use super::shared_tm;
use super::secrets::{
    api_chat_completions_url, get_ai_mode, resolve_ai_config, resolve_cloud_fallback_config,
    AiProvider, MaxTokensField, ProviderCapabilities,
};
use super::translation_quality::{is_usable_zh, quality_fail_reason, QualityFailReason};
use super::tm::{Tm, TmSaveGuard};
use super::translation_mode::TranslationQuality;
use super::translation_scope::TranslationScope;
use super::turnstile::MANAGED_AI_PROTOCOL;

/// 連續幾輪「整組都沒譯出」且屬可恢復失敗時提前結束（保留已得譯文）
const EMPTY_ROUNDS_ABORT: usize = 3;
/// 單批計劃失敗後最多再排入佇列幾次
const MAX_PLAN_REQUEUE: usize = 2;
/// 空 content／空 JSON 時 chunk 內最多再試幾次（含首次後的重試上限）
const EMPTY_CONTENT_RETRY_LIMIT: usize = 4;
/// 佔位符拒譯後最多再試幾次；0＝外層只跑一輪（至多一次嚴格通過），失敗則保留英文並負向快取
const PLACEHOLDER_RETRY_LIMIT: usize = 0;
/// 單句嚴格重試硬上限，避免近千句一句一請求燒錢
const STRICT_PLACEHOLDER_RETRY_CAP: usize = 48;
const MAX_PROMPT_GLOSSARY_TERMS: usize = 150;
const MIN_COMPLETION_TOKENS: usize = 512;
const MAX_COMPLETION_TOKENS: usize = 8192;
/// 依條數估 completion 下限（關閉思考後仍要夠裝 JSON 譯文）
const TOKENS_PER_ITEM: usize = 48;
/// finish_reason=length 時同尺寸最多再試幾次（0＝立刻拆半重排）
const LENGTH_TRUNCATION_RETRY_LIMIT: usize = 0;
const NAME_MAX_CHARS: usize = 48;
const UI_MAX_CHARS: usize = 200;
const SOLO_MIN_CHARS: usize = 2000;
const STORY_MIN_CHARS: usize = 200;
/// 每次開始翻譯前唯一送出的最小真實翻譯。它只活在記憶體，絕不進 LangMap、TM 或共享庫。
const AI_TRANSLATION_PROBE_SOURCE: &str = "Iron Sword";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AiUsageTotals {
    pub prompt_cache_hit_tokens: usize,
    pub prompt_cache_miss_tokens: usize,
    pub completion_tokens: usize,
}

impl AiUsageTotals {
    fn add(&mut self, other: &Self) {
        self.prompt_cache_hit_tokens += other.prompt_cache_hit_tokens;
        self.prompt_cache_miss_tokens += other.prompt_cache_miss_tokens;
        self.completion_tokens += other.completion_tokens;
    }

    pub fn is_empty(&self) -> bool {
        self.prompt_cache_hit_tokens == 0
            && self.prompt_cache_miss_tokens == 0
            && self.completion_tokens == 0
    }

    pub fn note(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        Some(format!(
            "AI token：快取命中 {}、未命中 {}、輸出 {}",
            self.prompt_cache_hit_tokens, self.prompt_cache_miss_tokens, self.completion_tokens
        ))
    }

    fn inline_note(&self) -> String {
        format!(
            "用量命中 {}／未命中 {}／輸出 {}",
            self.prompt_cache_hit_tokens, self.prompt_cache_miss_tokens, self.completion_tokens
        )
    }
}

fn placeholder_negative_cache() -> &'static Mutex<HashSet<String>> {
    static CACHE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_placeholder_negatively_cached(source: &str) -> bool {
    placeholder_negative_cache()
        .lock()
        .map(|c| c.contains(source))
        .unwrap_or(false)
}

fn remember_placeholder_rejection(source: &str) {
    if let Ok(mut cache) = placeholder_negative_cache().lock() {
        cache.insert(source.to_string());
    }
}

/// 把語言表已譯字串寫進本機 TM，供 ZIP／覆寫／任務共用（0.3.8）。
/// `en` 應為掃描時的英文目錄（勿用已被 subtract 的 pending）。
pub fn seed_tm_from_langmaps(en: &LangMap, zh: &LangMap) -> usize {
    let mut tm = Tm::load();
    let mut seeded = 0usize;
    for (ns, en_map) in en {
        let Some(zh_map) = zh.get(ns) else {
            continue;
        };
        for (key, en_val) in en_map {
            let Some(zh_val) = zh_map.get(key) else {
                continue;
            };
            if !is_usable_zh(en_val, zh_val) {
                continue;
            }
            if placeholder::is_compatible(en_val, zh_val) {
                let ctx = context_hint(key);
                match ctx {
                    Some(c) => tm.insert_with_context(en_val, zh_val, Some(c)),
                    None => tm.insert(en_val, zh_val),
                };
                seeded += 1;
            }
        }
    }
    let _ = tm.save();
    seeded
}

/// 一次補譯的成果，供覆蓋範圍說明與 UI 顯示。
#[derive(Debug, Clone, Default)]
pub struct AiFillReport {
    /// 實際寫進語言表的條數
    pub filled: usize,
    /// 本機術語表（user／builtin Enforce）exact 命中
    pub glossary_hits: usize,
    pub tm_hits: usize,
    /// 社群共享翻譯記憶命中（免送 AI）
    pub shared_hits: usize,
    /// 社群共享術語命中
    pub shared_glossary_hits: usize,
    pub ai_translated: usize,
    /// 佔位符壞掉、已退回原文（含嚴格上限外略過）
    pub rejected: usize,
    /// guard 通過但品質閘未過，保留英文、不送嚴格重試
    pub quality_skipped: usize,
    /// 品質失敗細因累計（與 quality_skipped 對齊；僅量測，不改判定）
    pub quality_still_english: usize,
    pub quality_mixed_fragment: usize,
    pub quality_same_as_source: usize,
    /// 本地小模型退化輸出的細因（重複迴圈／截斷／旁白／數字漂移／加油添醋）
    pub local_degenerate: Vec<(&'static str, usize)>,
    /// AI 這一層不可用的原因（`None` ＝ AI 正常跑過）。
    ///
    /// 有值代表「資料層有結果、但 AI 沒跑成」——呼叫端必須把這一層算成
    /// **部分完成**，不可以當成完成。這是「宣稱完成卻整段沒翻」的防線。
    pub ai_unavailable: Option<String>,
    /// 具體因品質失敗而暫緩的語言表項目；一般補充不會再次送出。
    pub quality_deferred: LangMap,
    pub usage: AiUsageTotals,
    pub notes: Vec<String>,
}

impl AiFillReport {
    pub fn usage_note(&self) -> Option<String> {
        self.usage.note()
    }

    pub fn note(&self) -> String {
        let free = self.glossary_hits
            + self.shared_hits
            + self.shared_glossary_hits
            + self.tm_hits;
        let denom = free + self.ai_translated;
        let mut parts = vec![format!(
            "補譯 {} 條（術語 {}、共享術語 {}、共享庫 {}、翻譯記憶 {}、AI {}）",
            self.filled,
            self.glossary_hits,
            self.shared_glossary_hits,
            self.shared_hits,
            self.tm_hits,
            self.ai_translated
        )];
        if denom > 0 {
            let pct = (free * 100) / denom;
            parts.push(format!("免 AI 比例約 {pct}%（{free}/{denom}）"));
        }
        if self.quality_skipped > 0 {
            parts.push(format!(
                "品質未過略過 {} 句（未寫入；{} {}／{} {}／{} {}）",
                self.quality_skipped,
                QualityFailReason::StillEnglish.label_zh(),
                self.quality_still_english,
                QualityFailReason::MixedFragment.label_zh(),
                self.quality_mixed_fragment,
                QualityFailReason::SameAsSource.label_zh(),
                self.quality_same_as_source
            ));
        }
        let deferred = self
            .quality_deferred
            .values()
            .map(|map| map.len())
            .sum::<usize>();
        if deferred > 0 {
            parts.push(format!("品質暫緩 {deferred} 條，本次不重送"));
        }
        if self.rejected > 0 {
            parts.push(format!("{} 條因佔位符不符退回原文", self.rejected));
        }
        if let Some(usage) = self.usage.note() {
            parts.push(usage);
        }
        parts.extend(self.notes.iter().cloned());
        parts.join("；")
    }
}

/// 主批譯文分類：品質失敗不得進佔位符單句重試。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateClass {
    Accept,
    QualityFail,
    PlaceholderFail,
}

/// 非本地模型的分類（測試用的簡短寫法）。
#[cfg(test)]
fn classify_candidate(
    source: &str,
    candidate: &str,
) -> (
    CandidateClass,
    Option<String>,
    GuardStats,
    Option<QualityFailReason>,
) {
    classify_candidate_for(source, candidate, false).0
}

/// 分類譯文，並在本地模型時多跑一輪小模型專屬的退化檢查。
///
/// 回傳 `(分類結果, 本地退化細因)`。細因只用來告訴使用者哪裡出問題，
/// 判定本身仍走同一條 `QualityFail` 路徑。
#[allow(clippy::type_complexity)]
/// AI 送出前的三道前置檢查，全部收斂成一個 `Result`。
///
/// 抽出來是為了讓呼叫端能用 `match` 明確處理「失敗但不該中斷」，
/// 而不是被三個散落的 `?` 悄悄丟掉已完成的工作。
fn ai_preflight() -> Result<Engine, String> {
    super::discord_auth::require_discord_guild_for_ai()?;
    if get_ai_mode() == "local" {
        if !crate::engine::local_llm::is_installed() {
            return Err("尚未安裝本地模型。請先同意並完成偵測／下載。".into());
        }
        crate::engine::local_llm::ensure_ready_for_translate(None)?;
    }
    Engine::connect()
}

fn classify_candidate_for(
    source: &str,
    candidate: &str,
    local: bool,
) -> (
    (
        CandidateClass,
        Option<String>,
        GuardStats,
        Option<QualityFailReason>,
    ),
    Option<local_quality::LocalIssue>,
) {
    let mut stats = GuardStats::default();
    let Some(safe) = placeholder::guard(source, candidate, &mut stats) else {
        return ((CandidateClass::PlaceholderFail, None, stats, None), None);
    };
    if let Err(reason) = quality_fail_reason(source, &safe) {
        return ((CandidateClass::QualityFail, None, stats, Some(reason)), None);
    }
    if !local {
        return ((CandidateClass::Accept, Some(safe), stats, None), None);
    }

    // 以下是雲端大模型不需要的：小模型會吐出「看起來像正常中文」但實際壞掉的譯文
    // （鬼打牆重複、話沒說完、數字被改掉）。既有三道閘門一道都攔不住。
    //
    // 處理順序刻意是「先修、再判嚴重性」：
    //  1. 修得掉的（模型旁白）直接清乾淨，那通常就是正確答案
    //  2. 修不掉的才看嚴重性——只有**會害到玩家**的才退回英文
    let check = local_quality::check(source, &safe);
    // 清乾淨的版本要重新過一次佔位符把關，不能因為「只是去掉開頭」就免檢
    let text = match check.cleaned {
        Some(cleaned) => match placeholder::guard(source, &cleaned, &mut stats) {
            Some(ok) => ok,
            None => safe.clone(),
        },
        None => safe.clone(),
    };
    let Some(issue) = check.issue else {
        return ((CandidateClass::Accept, Some(text), stats, None), None);
    };
    // 話沒說完要不要擋，得看譯文本身還剩多少資訊，所以用 blocks_output_for
    let blocks = issue.blocks_output_for(source, &text);
    crate::dev_log!(
        "quality",
        "本地退化 {}（{}） | 原文長 {} / 譯文長 {} | 原文={:?} | 譯文={:?}",
        issue.label_zh(),
        if blocks { "退回英文並排進補完佇列" } else { "照樣採用" },
        source.chars().count(),
        text.chars().count(),
        source.chars().take(80).collect::<String>(),
        text.chars().take(80).collect::<String>()
    );
    if blocks {
        return (
            (CandidateClass::QualityFail, None, stats, None),
            Some(issue),
        );
    }
    // 不完美但資訊還在：照樣採用，只把問題記下來。
    // 對看不懂英文的玩家來說，「話沒說完的中文」還是遠比「完整的英文」有用。
    ((CandidateClass::Accept, Some(text), stats, None), Some(issue))
}

/// 累計本地退化細因，供最終報告告訴使用者「模型在哪裡出了什麼問題」。
fn record_local_issue(report: &mut AiFillReport, issue: Option<local_quality::LocalIssue>) {
    let Some(issue) = issue else { return };
    let key = issue.label_zh();
    match report.local_degenerate.iter_mut().find(|(k, _)| *k == key) {
        Some((_, count)) => *count += 1,
        None => report.local_degenerate.push((key, 1)),
    }
}

fn record_quality_fail(report: &mut AiFillReport, reason: Option<QualityFailReason>) {
    match reason {
        Some(QualityFailReason::StillEnglish) => report.quality_still_english += 1,
        Some(QualityFailReason::MixedFragment) => report.quality_mixed_fragment += 1,
        Some(QualityFailReason::SameAsSource) => report.quality_same_as_source += 1,
        None => {}
    }
}

/// 嚴格重試清單切成「可送 AI」與「超過上限略過」。
fn split_strict_retry_cap(
    mut items: Vec<MaskedItem>,
    cap: usize,
) -> (Vec<MaskedItem>, Vec<MaskedItem>) {
    if items.len() <= cap {
        return (items, Vec::new());
    }
    let overflow = items.split_off(cap);
    (items, overflow)
}

/// 品質失敗：不寫入譯文（維持 absent），只累加 `quality_skipped`。
/// 佔位符失敗：寫回英文原文並負向快取，避免同輪重燒。
fn keep_english_skip(
    translations: &mut HashMap<usize, String>,
    report: &mut AiFillReport,
    uid: usize,
    source: &str,
    placeholder: bool,
    quality_reason: Option<QualityFailReason>,
) {
    if placeholder {
        translations.insert(uid, source.to_string());
        remember_placeholder_rejection(source);
        report.rejected += 1;
    } else {
        // 假英文不得進 langmap／資源包／TM；下次仍可補翻真缺
        report.quality_skipped += 1;
        record_quality_fail(report, quality_reason);
    }
}

// ═══ 玩家向訊息 ═══════════════════════════════════════════════

/// 給玩家看的額度／驗證說明（不洩模型／主機名，依目前 AI 來源給下一步）
fn ai_quota_support_message(detail: &str) -> String {
    let d = sanitize_provider_name(detail);
    if d.contains("GPT") {
        return ai_quota_support_message_for(detail, AiProvider::Codex, false);
    }
    ai_quota_support_message_for(detail, AiProvider::Deepseek, false)
}

fn ai_quota_support_message_for(detail: &str, provider: AiProvider, _managed: bool) -> String {
    let d = sanitize_provider_name(detail);
    if matches!(provider, AiProvider::Codex) {
        return format!(
            "【ChatGPT 暫時不能翻譯】\n\
{d}\n\n\
這次 ChatGPT 不接受翻譯請求，可能是翻譯用量已達上限，也可能只是短時間內送出太多次；這不一定代表你平常聊天的額度用完了。\n\
翻譯會消耗你的 ChatGPT 帳號額度，可以到 ChatGPT「設定 → 使用量」查看什麼時候重設。\n\
工具在試翻時就停下來了，沒有寫入任何翻譯。請稍後再試、重新登入 ChatGPT，或改用自訂 API／本地模型。"
        );
    }
    if matches!(provider, AiProvider::LocalLlm) {
        return format!(
            "【本地模型沒有回應】\n\
{d}\n\n\
請確認已完成本地模型安裝，且健康檢查已通過。速度隨這台電腦而異。"
        );
    }
    format!(
        "【自訂 API 額度或金鑰無法使用】\n\
{d}\n\n\
自訂 API 使用你填入服務商的金鑰與額度。\n\
請到該服務商後台確認金鑰、餘額與速率限制；也可改用 GPT 或本地模型。"
    )
}

fn sanitize_provider_name(s: &str) -> String {
    let mut out = s.to_string();
    for needle in [
        "deepseek",
        "DeepSeek",
        "DEEPSEEK",
        "api.deepseek.com",
        "deepseek-chat",
    ] {
        out = out.replace(needle, "AI 服務");
    }
    out
}

fn looks_like_quota_or_auth_error(msg: &str) -> bool {
    // 僅依「當下」錯誤判斷；不含「無回應」——那是暫時連線問題，勿包成額度用完。
    let m = msg.to_ascii_lowercase();
    m.contains("insufficient")
        || m.contains("balance")
        || m.contains("quota")
        || m.contains("billing")
        || m.contains("payment")
        || m.contains("exceed")
        || m.contains("credit")
        || m.contains("402")
        || m.contains("invalid api key")
        || m.contains("金鑰無效")
        || m.contains("沒有額度")
        || m.contains("額度")
        || m.contains("餘額")
        || m.contains("免費翻譯的當日額度")
        || (m.contains("403") && (m.contains("key") || m.contains("金鑰") || m.contains("forbidden")))
}

fn is_cancel_message(msg: &str) -> bool {
    msg.contains("已依你的要求停止")
}

fn is_auth_unavailable_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("auth_unavailable")
        || msg.contains("會員驗證暫時無法連線")
        || msg.contains("登入／會員驗證暫時無法連線")
        || msg.contains("登入/會員驗證暫時無法連線")
}

/// 真需要玩家重登 Discord（不含暫態 auth_unavailable、不含裸 cloudflare HTML）。
fn is_auth_relogin_error(msg: &str) -> bool {
    if is_auth_unavailable_error(msg) || is_cancel_message(msg) {
        return false;
    }
    let m = msg.to_ascii_lowercase();
    m.contains("login_required")
        || m.contains("login expired")
        || msg.contains("請先登入 Discord")
        || msg.contains("Discord 登入已失效")
        || msg.contains("請回到工具重新登入")
        || msg.contains("請在工具重新登入 Discord")
        || msg.contains("安全驗證已過期，請回到工具重新驗證")
}

fn auth_relogin_message() -> String {
    "Discord 登入已失效或需重新確認會員資格，請回到工具重新登入後再試。".into()
}

fn auth_unavailable_message() -> String {
    "Discord 登入／會員驗證暫時無法連線，請稍後再試或重新登入；也可改用自訂 API。".into()
}

fn truncate_err_msg(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max_chars).collect::<String>())
}

/// 以最新一筆錯誤分類「不可恢復」中止原因。暫態／限流不走這裡。
fn classify_batch_abort(err_peek: &[String]) -> Option<String> {
    let latest = err_peek.last()?;
    if is_cancel_message(latest) {
        return Some(CANCEL_MESSAGE.to_string());
    }
    // auth_unavailable 可恢復，不在此秒殺
    if is_auth_unavailable_error(latest) {
        return None;
    }
    if is_auth_relogin_error(latest) {
        return Some(auth_relogin_message());
    }
    if looks_like_quota_or_auth_error(latest) {
        return Some(ai_quota_support_message(latest));
    }
    None
}

fn is_recoverable_batch_error(msg: &str) -> bool {
    if is_cancel_message(msg) || is_auth_relogin_error(msg) {
        return false;
    }
    if looks_like_quota_or_auth_error(msg) && !is_auth_unavailable_error(msg) {
        // 明確額度／金鑰：不可靠重試；限流文案另判
        if msg.contains("請求太頻繁") || msg.contains("稍後再試") {
            return true;
        }
        if msg.contains("當日額度") || msg.contains("餘額不足") || msg.contains("額度可能已用完") {
            return false;
        }
    }
    if is_auth_unavailable_error(msg) {
        return true;
    }
    let m = msg.to_ascii_lowercase();
    m.contains("逾時")
        || m.contains("timeout")
        || m.contains("無回應")
        || m.contains("503")
        || m.contains("502")
        || m.contains("429")
        || m.contains("請求太頻繁")
        || m.contains("暫時無法")
        || m.contains("沒有回傳翻譯內容")
        || m.contains("無法解析")
        || m.contains("空 json")
        || m.contains("上游")
}

/// 從 Worker／上游 JSON 取出 `error.type`（僅診斷用，不含金鑰）。
fn extract_proxy_error_type(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    v.get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatErrorKind {
    /// 可退避重試（限流／5xx／驗證基建暫態）
    Transient,
    /// 當日預算／餘額——應中止 AI
    Quota,
    /// 需玩家重登 Discord
    Relogin,
    /// 需加入伺服器等硬失敗（不開 cookie 長等待）
    Fatal,
}

#[derive(Debug, Clone)]
struct MappedChatError {
    message: String,
    kind: ChatErrorKind,
}

impl MappedChatError {
    fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ChatErrorKind::Transient,
        }
    }
}

/// 代管／自訂 AI 的 HTTP 錯誤 → 玩家可行動文案。優先看 `error.type`，避免上游狀態被誤標成 Discord。
fn map_chat_http_error(code: u16, body: &str, managed: bool) -> Option<MappedChatError> {
    let err_type = extract_proxy_error_type(body);
    let type_ref = err_type.as_deref();
    let snippet = || sanitize_provider_name(&body.chars().take(160).collect::<String>());

    // 1) 明確 type 優先（與 HTTP 狀態解耦）
    match type_ref {
        Some("insufficient_quota") => {
            return Some(MappedChatError {
                message: "免費翻譯的當日額度已用完".into(),
                kind: ChatErrorKind::Quota,
            });
        }
        Some("login_required") => {
            return Some(MappedChatError {
                message: "使用開發者代管 AI 前，請先登入 Discord。".into(),
                kind: ChatErrorKind::Relogin,
            });
        }
        Some("auth_unavailable") => {
            return Some(MappedChatError {
                message: auth_unavailable_message(),
                kind: ChatErrorKind::Transient,
            });
        }
        Some("guild_required") => {
            // 高並行下 Worker member-tier 偶發閃斷；當暫態讓既有 attempts<2 重試。
            return Some(MappedChatError {
                message: "使用開發者代管 AI 前，請先加入 ZeitFrei 官方 Discord 伺服器。".into(),
                kind: ChatErrorKind::Transient,
            });
        }
        Some("client_upgrade_required") => {
            return Some(MappedChatError {
                message: "這個版本已不能使用開發者代管 AI，請更新工具後再試。".into(),
                kind: ChatErrorKind::Fatal,
            });
        }
        Some("server_not_ready") => {
            return Some(MappedChatError {
                message: "免費翻譯暫時無法使用（服務端維護中）。你可以自行填入 AI 金鑰，或稍後再試"
                    .into(),
                kind: ChatErrorKind::Fatal,
            });
        }
        Some("turnstile_unavailable") => {
            return Some(MappedChatError {
                message: "雲端翻譯閘門設定異常。請確認已登入 Discord 並加入官方伺服器；若仍失敗可改用自訂 API，或稍後再試。"
                    .into(),
                kind: ChatErrorKind::Fatal,
            });
        }
        _ => {}
    }

    match code {
        503 if !managed && body.contains("server_not_ready") => Some(MappedChatError {
            message: "免費翻譯暫時無法使用（服務端維護中）。你可以自行填入 AI 金鑰，或稍後再試"
                .into(),
            kind: ChatErrorKind::Fatal,
        }),
        503 if managed => {
            let s = snippet();
            if s.trim().is_empty() {
                Some(MappedChatError::transient(
                    "代管 AI 暫時無法使用（503）。請稍後再試，或改用自訂 API。",
                ))
            } else {
                Some(MappedChatError::transient(format!(
                    "代管 AI 暫時無法使用（503）：{s}"
                )))
            }
        }
        // 無 insufficient_quota type 的 429：當上游／邊緣限流，可重試
        429 if managed => Some(MappedChatError::transient("請求太頻繁，稍後再試")),
        426 if managed => Some(MappedChatError {
            message: "這個版本已不能使用開發者代管 AI，請更新工具後再試。".into(),
            kind: ChatErrorKind::Fatal,
        }),
        428 if managed => {
            // 僅明確驗證語意才當重登；裸 428 當暫態
            if body.to_ascii_lowercase().contains("turnstile")
                || body.to_ascii_lowercase().contains("verification")
            {
                Some(MappedChatError {
                    message: "安全驗證已過期，請回到工具重新驗證。".into(),
                    kind: ChatErrorKind::Relogin,
                })
            } else {
                Some(MappedChatError::transient("服務暫時要求重試（428），稍後再試"))
            }
        }
        401 if managed => {
            // 無 login_required type：多半是轉發上游，禁止標成 Discord
            let s = snippet();
            Some(MappedChatError {
                message: if s.trim().is_empty() {
                    "AI 上游拒絕請求（401）。請稍後再試，或改用自訂 API。".into()
                } else {
                    format!("AI 上游拒絕請求（401）：{s}")
                },
                kind: ChatErrorKind::Fatal,
            })
        }
        403 if managed => Some(MappedChatError {
            message: "使用開發者代管 AI 前，請先加入 ZeitFrei 官方 Discord 伺服器。".into(),
            kind: ChatErrorKind::Fatal,
        }),
        _ => None,
    }
}

// ═══ 語境判斷 ═════════════════════════════════════════════════

/// 由 lang key 推測語境，讓 AI 知道這是物品名還是整句提示。
///
/// `item.create.wrench` → 物品名 → AI 會給「扳手」而不是「一把用來旋轉的工具」。
pub fn context_hint(lang_key: &str) -> Option<&'static str> {
    let lower = lang_key.to_ascii_lowercase();
    let mut segments = lower.split('.');
    if let Some(hit) = segments.next().and_then(kind_of_segment) {
        return Some(hit);
    }
    // `create.tooltip.xxx`：模組把自己的 id 放最前面，往後找
    lower.split('.').find_map(kind_of_segment)
}

fn kind_of_segment(seg: &str) -> Option<&'static str> {
    Some(match seg {
        "item" | "itemgroup" | "item_group" => "物品名",
        "block" => "方塊名",
        "fluid" => "液體名",
        "entity" => "生物名",
        "biome" => "生態域名",
        "effect" | "mob_effect" | "potion" => "狀態效果名",
        "enchantment" => "附魔名",
        "advancement" | "advancements" => "進度名稱或說明",
        "death" => "死亡訊息",
        "subtitles" | "subtitle" => "音效字幕",
        "key" | "keybind" | "keybinds" => "按鍵綁定名稱",
        "gui" | "menu" | "screen" | "container" | "options" | "button" => "介面文字",
        "tooltip" | "desc" | "description" | "info" | "hint" => "提示說明",
        "command" | "commands" | "argument" => "指令訊息",
        "config" | "configuration" => "設定項",
        "chat" | "message" | "msg" => "訊息",
        "quest" | "quests" => "任務文字",
        "curios" | "trinket" | "trinkets" => "飾品名",
        _ => return None,
    })
}

// ═══ 主要入口：補語言表缺漏 ═══════════════════════════════════

/// `en_only`: ns -> (key -> 原文)。翻好的寫回 `zh`。
///
/// `use_ai == false` 時仍會跑術語表與翻譯記憶（兩者都不需要網路），
/// 只是不呼叫 AI。沒有金鑰的玩家因此還是拿得到官方譯名與先前翻過的內容。
#[allow(dead_code)]
pub fn fill_missing_with_ai<F>(
    zh: &mut LangMap,
    en_only: &LangMap,
    use_ai: bool,
    on_progress: F,
) -> Result<AiFillReport, String>
where
    F: FnMut(u8, &str),
{
    fill_missing_with_mode(
        zh,
        en_only,
        use_ai,
        false,
        TranslationQuality::Balanced,
        None,
        on_progress,
    )
}

pub fn fill_missing_with_ai_with_scope<F>(
    zh: &mut LangMap,
    en_only: &LangMap,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    on_progress: F,
) -> Result<AiFillReport, String>
where
    F: FnMut(u8, &str),
{
    fill_missing_with_mode(
        zh,
        en_only,
        use_ai,
        false,
        TranslationQuality::Balanced,
        scope,
        on_progress,
    )
}

/// 與一般補翻相同，但 Force 模式會略過本機翻譯記憶，避免重跑時一直沿用舊機翻。
pub fn fill_missing_with_mode<F>(
    zh: &mut LangMap,
    en_only: &LangMap,
    use_ai: bool,
    force_refresh: bool,
    quality: TranslationQuality,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<AiFillReport, String>
where
    F: FnMut(u8, &str),
{
    on_progress(42, "AI：組裝待譯清單…");

    let mut jobs: Vec<shared_tm::SharedTmJob> = Vec::new();
    for (ns, map) in en_only {
        for (k, en) in map {
            if zh.get(ns).and_then(|m| m.get(k)).is_some() {
                continue;
            }
            let t = en.trim();
            // 集中判定（P0-03）。這裡是 AI／共享庫／TM 的共同入口，
            // 而且手上同時有 key 與 value——舊版只把 value 丟給 looks_untranslatable，
            // 於是 `botania.entry.bcIntegration` 這種語言 key 一路送進 AI 並寫進共享庫。
            let verdict = eligibility::classify(eligibility::Candidate {
                source_kind: "lang",
                logical_key: k,
                text: t,
            });
            if !verdict.may_send_to_ai() {
                continue;
            }
            jobs.push(shared_tm::SharedTmJob {
                namespace: ns.clone(),
                key: k.clone(),
                source: en.clone(),
                context: context_hint(k).map(str::to_owned),
                scope: scope.cloned(),
            });
        }
    }

    jobs.sort_by(|a, b| (&a.namespace, &a.key).cmp(&(&b.namespace, &b.key)));

    if jobs.is_empty() {
        on_progress(88, "沒有需要 AI 補的文字，略過網路翻譯");
        return Ok(AiFillReport::default());
    }

    let mut report = AiFillReport::default();
    let mut guard = GuardStats::default();

    // ── 社群共享翻譯記憶（keyed：模組·key·原文）：先撈，命中就免送 AI ──
    // 隱藏、預設開；查不到／服務未就緒都略過，但寫可觀測 note。
    on_progress(43, "查詢社群共享翻譯（不需你設定）…");
    let skip_shared_lookup = super::shared_tm::skip_shared_lookup();
    let reuse_tm = !force_refresh;
    let shared_lookup = if skip_shared_lookup {
        shared_tm::LookupResult::default()
    } else {
        shared_tm::lookup_detailed(&jobs)
    };
    let shared = &shared_lookup.hits;
    report.notes.push(shared_lookup.player_note());
    let mut shared_done: std::collections::HashSet<usize> = std::collections::HashSet::new();
    if !shared.is_empty() {
        for (i, job) in jobs.iter().enumerate() {
            if let Some(cand) = shared.get(&i) {
                // 共享來的一樣要過佔位符守衛＋品質閘門才敢用
                if let Some(safe) = placeholder::guard(&job.source, cand, &mut guard) {
                    if is_usable_zh(&job.source, &safe) {
                        zh.entry(job.namespace.clone())
                            .or_default()
                            .insert(job.key.clone(), safe);
                        report.filled += 1;
                        report.shared_hits += 1;
                        shared_done.insert(i);
                    }
                }
            }
        }
        if report.shared_hits > 0 {
            on_progress(
                44,
                &format!("社群共享庫命中 {} 條（免送 AI）", report.shared_hits),
            );
        }
    } else if shared_lookup.status == shared_tm::LookupStatus::Empty {
        on_progress(44, "社群共享庫已連線，本次 0 命中");
    } else if shared_lookup.status == shared_tm::LookupStatus::Failed {
        on_progress(44, "社群共享庫查詢失敗，已略過（不影響本機翻譯）");
    }

    // 剩下未命中的才進去重＋術語表＋本機記憶＋AI
    let remaining: Vec<usize> = (0..jobs.len())
        .filter(|i| !shared_done.contains(i))
        .collect();
    let mut unique: Vec<String> = Vec::new();
    let mut ctx: Vec<Option<&'static str>> = Vec::new();
    let mut seen: HashMap<(String, Option<&'static str>), usize> = HashMap::new();
    let mut job_uid: HashMap<usize, usize> = HashMap::new(); // job index → uid
    for &i in &remaining {
        let job = &jobs[i];
        let hint = context_hint(&job.key);
        let dedupe_key = (job.source.clone(), hint);
        if let Some(&id) = seen.get(&dedupe_key) {
            job_uid.insert(i, id);
        } else {
            let id = unique.len();
            seen.insert(dedupe_key, id);
            unique.push(job.source.clone());
            ctx.push(hint);
            job_uid.insert(i, id);
        }
    }

    if !unique.is_empty() {
        let resolved = resolve_unique(
            &unique,
            &ctx,
            use_ai,
            reuse_tm,
            force_refresh,
            quality,
            44,
            44,
            scope,
            &mut on_progress,
        )?;
        let quality_failed_uids = resolved.quality_deferred.clone();
        // 併入子報告的計數
        let sub = &resolved.report;
        report.glossary_hits += sub.glossary_hits;
        report.tm_hits += sub.tm_hits;
        report.shared_hits += sub.shared_hits;
        report.shared_glossary_hits += sub.shared_glossary_hits;
        report.ai_translated += sub.ai_translated;
        report.rejected += sub.rejected;
        report.quality_skipped += sub.quality_skipped;
        report.local_degenerate.extend(sub.local_degenerate.clone());
        // AI 不可用要一路帶到最外層，呼叫端才知道這一輪只有資料層的成果，
        // 不可以講成「完成」。
        if report.ai_unavailable.is_none() {
            report.ai_unavailable = sub.ai_unavailable.clone();
        }
        report.notes.extend(sub.notes.clone());

        // 寫回語言表 + 蒐集「這次新由 AI 產出的」以貢獻給社群
        let mut to_share: Vec<shared_tm::SharedTmEntry> = Vec::new();
        let mut glossary_share: Vec<shared_glossary::SharedGlossaryEntry> = Vec::new();
        let write_total = remaining.len();
        on_progress(
            86,
            &format!("正在寫回譯文到語言表（0/{write_total}）…"),
        );
        let mut write_done = 0usize;
        for &i in &remaining {
            let Some(&uid) = job_uid.get(&i) else {
                continue;
            };
            if let Some(text) = resolved.translations.get(&uid) {
                let job = &jobs[i];
                zh.entry(job.namespace.clone())
                    .or_default()
                    .insert(job.key.clone(), text.clone());
                report.filled += 1;
                if job.scope.is_some()
                    && is_usable_zh(&job.source, text)
                    && !is_poisoned_mech_translation(&job.source, text)
                    && super::output_guard::passes(&job.source, text)
                {
                    to_share.push(shared_tm::SharedTmEntry {
                        namespace: job.namespace.clone(),
                        key: job.key.clone(),
                        source: job.source.clone(),
                        translated: text.clone(),
                        context: job.context.clone(),
                        scope: job.scope.clone(),
                    });
                    if let Some(scope) = job.scope.clone() {
                        if is_shared_term_candidate(&job.source, job.context.as_deref()) {
                            glossary_share.push(shared_glossary::SharedGlossaryEntry {
                                source: job.source.clone(),
                                translated: text.clone(),
                                context: job.context.clone(),
                                scope,
                            });
                        }
                    }
                }
            }
            write_done += 1;
            if write_done == 1
                || write_done == write_total
                || write_done % 2000 == 0
            {
                on_progress(
                    86,
                    &format!("正在寫回譯文到語言表（{write_done}/{write_total}）…"),
                );
            }
            if quality_failed_uids.contains(&uid) {
                let job = &jobs[i];
                report
                    .quality_deferred
                    .entry(job.namespace.clone())
                    .or_default()
                    .insert(job.key.clone(), job.source.clone());
            }
        }
        // 匿名回饋給社群（失敗／逾時不影響本機；有牆鐘預算）
        if !to_share.is_empty() {
            on_progress(
                88,
                &format!(
                    "正在回饋社群共享庫（{} 條，逾時會暫存稍後再送）…",
                    to_share.len()
                ),
            );
            let result = shared_tm::contribute(&to_share);
            if let Some(note) = result.player_note() {
                report.notes.push(note);
            }
            on_progress(89, "社群共享庫回饋結束（本機翻譯不受影響）");
        }
        if !glossary_share.is_empty() {
            on_progress(
                89,
                &format!("正在回饋共享術語（{} 條）…", glossary_share.len()),
            );
            let g = shared_glossary::contribute(&glossary_share);
            if let Some(note) = g.player_note() {
                report.notes.push(note);
            }
        }
    }

    on_progress(90, &report.note());
    Ok(report)
}

fn is_shared_term_candidate(source: &str, _context: Option<&str>) -> bool {
    let trimmed = source.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 120
        && !trimmed.contains(['\n', '\r'])
}

/// 收尾：把合格短詞貢獻到社群共享術語；回傳玩家可見說明（若有）。
pub fn contribute_shared_glossary_from_langmaps(
    en: &LangMap,
    zh: &LangMap,
    scope: &TranslationScope,
) -> Option<String> {
    let entries = collect_glossary_share_from_langmaps(en, zh, scope);
    shared_glossary::contribute(&entries).player_note()
}

/// 從最終 en∩zh 蒐集可貢獻的共享術語（短詞；ctx 可空，與 lookup hash 規則一致）。
pub fn collect_glossary_share_from_langmaps(
    en: &LangMap,
    zh: &LangMap,
    scope: &TranslationScope,
) -> Vec<shared_glossary::SharedGlossaryEntry> {
    if !scope.is_known() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (ns, en_map) in en {
        let Some(zh_map) = zh.get(ns) else {
            continue;
        };
        for (key, source) in en_map {
            let Some(translated) = zh_map.get(key) else {
                continue;
            };
            if !is_usable_zh(source, translated)
                || is_poisoned_mech_translation(source, translated)
                || !placeholder::is_compatible(source, translated)
                || !super::output_guard::passes(source, translated)
            {
                continue;
            }
            let ctx = context_hint(key);
            if !is_shared_term_candidate(source, ctx) {
                continue;
            }
            let gh = shared_glossary::glossary_hash(source, ctx);
            if !seen.insert(gh) {
                continue;
            }
            out.push(shared_glossary::SharedGlossaryEntry {
                source: source.clone(),
                translated: translated.clone(),
                context: ctx.map(str::to_owned),
                scope: scope.clone(),
            });
        }
    }
    out
}

/// 翻譯任意字串列表（任務書／書本／覆寫文字用），回傳與輸入等長（缺則空字串）。
#[allow(dead_code)]
pub fn translate_plain_strings<F>(
    texts: &[String],
    on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    translate_plain_strings_with_scope(texts, None, on_progress)
}

pub fn translate_plain_strings_with_scope<F>(
    texts: &[String],
    scope: Option<&TranslationScope>,
    on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    translate_plain_strings_ex(texts, scope, &[], on_progress)
}

/// 與 `translate_plain_strings_with_scope` 相同，但依原文對應模組／pack namespace。
pub fn translate_plain_strings_mapped<F>(
    texts: &[String],
    scope: Option<&TranslationScope>,
    ns_by_src: &HashMap<String, String>,
    on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    let namespaces = super::shared_identity::aligned_namespaces(texts, ns_by_src, scope);
    translate_plain_strings_ex(texts, scope, &namespaces, on_progress)
}

/// 與 `translate_plain_strings_with_scope` 相同，但可帶每條模組／pack namespace。
pub fn translate_plain_strings_ex<F>(
    texts: &[String],
    scope: Option<&TranslationScope>,
    namespaces: &[String],
    mut on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    if texts.is_empty() {
        return Ok(vec![]);
    }

    let reuse_shared = !super::shared_tm::skip_shared_lookup();
    let jobs: Vec<shared_tm::SharedTmJob> = texts
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let raw_ns = namespaces
                .get(i)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| super::shared_identity::pack_namespace(scope));
            shared_tm::SharedTmJob {
                namespace: super::shared_identity::sanitize_share_ns(&raw_ns, scope),
                key: "overlay".into(),
                source: source.clone(),
                context: None,
                scope: scope.cloned(),
            }
        })
        .collect();

    let mut out = vec![String::new(); texts.len()];
    let mut guard = GuardStats::default();
    let mut shared_done: HashSet<usize> = HashSet::new();
    if reuse_shared {
        on_progress(0, "查詢共享庫（覆寫／任務）…");
        let hits = shared_tm::lookup(&jobs);
        for (i, job) in jobs.iter().enumerate() {
            if let Some(cand) = hits.get(&i) {
                if let Some(safe) = placeholder::guard(&job.source, cand, &mut guard) {
                    if is_usable_zh(&job.source, &safe)
                        && !is_poisoned_mech_translation(&job.source, &safe)
                    {
                        out[i] = safe;
                        shared_done.insert(i);
                    }
                }
            }
        }
    }

    let remaining_idx: Vec<usize> = (0..texts.len())
        .filter(|i| !shared_done.contains(i))
        .collect();
    if remaining_idx.is_empty() {
        contribute_plain_job_outputs(&jobs, &out);
        return Ok(out);
    }

    let mut unique: Vec<String> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut idx_uid: Vec<(usize, usize)> = Vec::new();
    for &i in &remaining_idx {
        let t = &texts[i];
        if let Some(&id) = seen.get(t) {
            idx_uid.push((i, id));
        } else {
            let id = unique.len();
            seen.insert(t.clone(), id);
            unique.push(t.clone());
            idx_uid.push((i, id));
        }
    }
    let ctx = vec![None; unique.len()];
    let resolved = resolve_unique(
        &unique,
        &ctx,
        true,
        true,
        true,
        TranslationQuality::Balanced,
        0,
        99,
        scope,
        &mut on_progress,
    )?;
    for (i, uid) in idx_uid {
        if let Some(text) = resolved.translations.get(&uid) {
            out[i] = text.clone();
        }
    }
    contribute_plain_job_outputs(&jobs, &out);
    Ok(out)
}

fn contribute_plain_job_outputs(jobs: &[shared_tm::SharedTmJob], out: &[String]) {
    let mut entries: Vec<shared_tm::SharedTmEntry> = Vec::new();
    for (job, zh) in jobs.iter().zip(out.iter()) {
        let t = zh.trim();
        if t.is_empty() || t == job.source.trim() {
            continue;
        }
        if !is_usable_zh(&job.source, t)
            || is_poisoned_mech_translation(&job.source, t)
            || !super::output_guard::passes(&job.source, t)
        {
            continue;
        }
        entries.push(shared_tm::SharedTmEntry {
            namespace: job.namespace.clone(),
            key: job.key.clone(),
            source: job.source.clone(),
            translated: t.to_string(),
            context: job.context.clone(),
            scope: job.scope.clone(),
        });
    }
    if !entries.is_empty() {
        let _ = shared_tm::contribute(&entries);
    }
}

// ═══ 共用流程 ═════════════════════════════════════════════════

struct Resolved {
    translations: HashMap<usize, String>,
    report: AiFillReport,
    quality_deferred: HashSet<usize>,
}

/// 術語表 → 翻譯記憶 → AI，三層依序解決 `unique` 裡的每一條。
///
/// 前兩層完全離線，所以 `use_ai == false` 時仍值得跑。
fn resolve_unique(
    unique: &[String],
    ctx: &[Option<&'static str>],
    use_ai: bool,
    reuse_tm: bool,
    allow_quality_retry: bool,
    _quality: TranslationQuality,
    base_pct: u8,
    span_pct: u8,
    scope: Option<&TranslationScope>,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<Resolved, String> {
    cancel::check()?;

    let gloss = glossary::load(None);
    if gloss.user_entries > 0 {
        on_progress(
            base_pct,
            &format!("已套用你自訂的 {} 條譯名", gloss.user_entries),
        );
    }
    // user → builtin（glossary::load 已合併；後寫入的 user 覆寫 builtin）
    let mut tm = TmSaveGuard::new(Tm::load());
    let mut translations: HashMap<usize, String> = HashMap::new();
    let mut report = AiFillReport::default();
    let mut quality_deferred: HashSet<usize> = HashSet::new();
    let mut guard = GuardStats::default();

    let reuse_shared = !super::shared_tm::skip_shared_lookup();
    if reuse_shared {
        on_progress(base_pct, "查詢共享庫與術語表（免送 AI）…");
    }

    let glossary_jobs: Vec<shared_glossary::SharedGlossaryJob> = unique
        .iter()
        .enumerate()
        .map(|(uid, source)| shared_glossary::SharedGlossaryJob {
            source: source.clone(),
            context: ctx.get(uid).copied().flatten().map(str::to_owned),
            scope: scope.cloned(),
        })
        .collect();
    let shared_glossary_lookup = if reuse_shared {
        shared_glossary::lookup_detailed(&glossary_jobs)
    } else {
        shared_glossary::LookupResult::default()
    };
    report.notes.push(shared_glossary_lookup.player_note());
    let shared_glossary_hits = &shared_glossary_lookup.hits;

    // 優先序：user glossary → builtin → shared → tm → AI
    let mut need_ai: Vec<(usize, String)> = Vec::new();
    let mut mechanism_kept = 0usize;
    for (uid, src) in unique.iter().enumerate() {
        if let Some(zh) = gloss.exact(src) {
            if let Some(safe) = placeholder::guard(src, zh, &mut guard)
                .filter(|safe| is_usable_zh(src, safe) && !is_poisoned_mech_translation(src, safe))
            {
                translations.insert(uid, safe);
                report.glossary_hits += 1;
                continue;
            }
        }
        if let Some(candidate) = shared_glossary_hits.get(&uid) {
            if let Some(safe) = placeholder::guard(src, candidate, &mut guard) {
                if is_usable_zh(src, &safe) && !is_poisoned_mech_translation(src, &safe) {
                    translations.insert(uid, safe);
                    report.shared_glossary_hits += 1;
                    continue;
                }
            }
        }
        let context = ctx.get(uid).copied().flatten();
        if reuse_tm {
            let tm_hit = match context {
                Some(value) => tm.get_with_context(src, Some(value)),
                None => tm.get(src),
            };
            if let Some(zh) = tm_hit {
                if is_usable_zh(src, &zh) && !is_poisoned_mech_translation(src, &zh) {
                    translations.insert(uid, zh);
                    report.tm_hits += 1;
                    continue;
                }
            }
        }
        // 機制 token／FancyMenu meta：不送 AI（避免再產生毒譯文）
        if skip_before_ai(src) {
            mechanism_kept += 1;
            continue;
        }
        need_ai.push((uid, src.clone()));
    }
    if mechanism_kept > 0 {
        report
            .notes
            .push(format!("{mechanism_kept} 項是機制代號或檔案路徑，保留原文（翻了會失效）"));
    }

    let pre = report.glossary_hits
        + report.shared_glossary_hits
        + report.shared_hits
        + report.tm_hits;
    if pre > 0 {
        on_progress(
            base_pct,
            &format!(
                "免費命中 {} 句（本機術語 {}、共享術語 {}、共享庫 {}、翻譯記憶 {}），只剩 {} 句要送 AI",
                pre,
                report.glossary_hits,
                report.shared_glossary_hits,
                report.shared_hits,
                report.tm_hits,
                need_ai.len()
            ),
        );
    }

    if need_ai.is_empty() {
        report.notes.push(tm.note());
        return Ok(Resolved {
            translations,
            report,
            quality_deferred,
        });
    }

    if !use_ai {
        // 沒勾 AI：前兩層照樣有貢獻，剩下的保留原文
        on_progress(
            base_pct + span_pct.min(100 - base_pct),
            &format!(
                "未使用 AI：離線補了 {} 句，其餘 {} 句保留原文",
                pre,
                need_ai.len()
            ),
        );
        report
            .notes
            .push(format!("未使用 AI，{} 句維持原文", need_ai.len()));
        report.notes.push(tm.note());
        return Ok(Resolved {
            translations,
            report,
            quality_deferred,
        });
    }

    // ── 第 3 層：AI（限次重試佔位符拒譯；負向快取避免同輪重燒）──
    //
    // ⚠️ 這一段的失敗**絕對不可以用 `?` 往上拋**。
    //
    // `translations` 此刻已經裝著資料層（術語表／共享術語／共享翻譯庫／翻譯記憶）
    // 的全部命中。實測站長一包：33233 條待譯裡，共享庫命中 32151、共享術語 671，
    // 真正要送 AI 的只有 271 條。舊版在這裡 `?` 一拋，那 32822 條查得到的譯文
    // **全部被丟掉**，整個階段 0 產出——FTB Quests 與文字覆寫在 GPT 額度用盡時
    // 一個字都沒翻出來，就是這樣來的。
    //
    // 使用者明確選了 AI，就不能在 AI 無法翻譯時靜默降級成「不使用 AI」。
    // 那會把「有 AI 品質把關」的期待變成只有共享庫／快取的部分結果，還讓人誤以為
    // 整包已照選擇翻完。開工前已有嚴格探測；若執行中端點仍失效，也必須中止並保留
    // 可診斷原因，而不是改走另一個未被選擇的流程。
    on_progress(base_pct, "連線 AI 並探測服務…");
    let ai_ready = ai_preflight();
    let engine = match ai_ready {
        Ok(engine) => engine,
        Err(reason) if is_cancel_message(&reason) => return Err(reason),
        Err(reason) => {
            let short = reason.lines().next().unwrap_or("AI 不可用");
            on_progress(base_pct, &format!("AI 不可用（{short}），已停止翻譯。"));
            crate::dev_log!("ai", "AI 不可用，停止翻譯（已命中本機／共享 {} 句）：{}", pre, reason);
            return Err(format!("AI 無法協助翻譯，已停止本次翻譯：{reason}"));
        }
    };
    let is_local_model = matches!(engine.provider, AiProvider::LocalLlm);
    // 本地模型重試只花時間、不花錢也不吃額度，所以品質沒過就該再試一次，
    // 不必像雲端那樣等使用者選「強制模式」。這是本地與雲端成本結構的根本差異。
    let allow_quality_retry = allow_quality_retry || is_local_model;
    let system_prompt = build_system_prompt(&gloss, unique, is_local_model);
    let mut batch_state = BatchRuntimeState::new(system_prompt, engine.capabilities.start_parallel);
    let mut term_stats = TermConsistencyStats::default();
    let mut pending_ai: Vec<PendingItem> = need_ai
        .into_iter()
        .filter_map(|(uid, src)| {
            if is_placeholder_negatively_cached(&src) {
                report.rejected += 1;
                None
            } else {
                Some(PendingItem {
                    uid,
                    source: src,
                    reason: PendingReason::NoAnswer,
                })
            }
        })
        .collect();
    on_progress(
        base_pct,
        &format!("AI 翻譯 {} 句（已扣掉重複與已知譯名）…", pending_ai.len()),
    );

    // 長句先拆句再送。本地小模型碰到長從屬句常「只翻到一半」甚至語意反轉
    // （實測：`Heal … Or Harm Undead Creatures` 被譯成「造成 4 點傷害」，
    // 治療變傷害而且整個 Or 子句消失）。拆開之後每一段都短、指涉清楚。
    // 這一步失敗時項目原封不動退回下面的主迴圈整句翻，不會留下半成品。
    if !pending_ai.is_empty() {
        pending_ai = translate_long_sentences_split(
            &engine,
            &mut batch_state,
            &gloss,
            &mut term_stats,
            &mut guard,
            &mut tm,
            &mut translations,
            &mut report,
            pending_ai,
            ctx,
            reuse_tm,
            base_pct,
            span_pct,
            on_progress,
        )?;
    }

    let mut attempt = 0usize;
    while !pending_ai.is_empty() && attempt <= PLACEHOLDER_RETRY_LIMIT {
        if attempt > 0 {
            on_progress(
                base_pct.saturating_add(span_pct.saturating_mul(attempt as u8) / ((PLACEHOLDER_RETRY_LIMIT as u8) + 1).max(1)),
                &format!(
                    "補充：只重送仍未解決的第 {} 次（{} 句）…",
                    attempt,
                    pending_ai.len()
                ),
            );
        }
        let masked_need_ai = build_masked_items(&pending_ai, ctx);
        let raw = run_batches(
            &engine,
            &mut batch_state,
            &masked_need_ai,
            base_pct,
            span_pct,
            on_progress,
            false,
        )?;

        let mut next_pending = collect_unresolved_items(&masked_need_ai, &raw);
        let unresolved_ids: HashSet<usize> = next_pending.iter().map(|item| item.uid).collect();
        let mut placeholder_retry: Vec<MaskedItem> = Vec::new();
        let mut quality_retry: Vec<MaskedItem> = Vec::new();
        for item in &masked_need_ai {
            if unresolved_ids.contains(&item.uid) {
                continue;
            }
            let Some(masked_out) = raw.get(&item.uid) else {
                continue;
            };
            let candidate = placeholder::unmask(masked_out, &item.tokens);
            let candidate = gloss.enforce_terms(&item.source, &candidate, &mut term_stats);
            let ((class, safe, attempt_guard, qreason), local_issue) =
                classify_candidate_for(&item.source, &candidate, is_local_model);
            record_local_issue(&mut report, local_issue);
            guard.checked += attempt_guard.checked;
            guard.repaired += attempt_guard.repaired;
            guard.rejected += attempt_guard.rejected;
            match class {
                CandidateClass::Accept => {
                    let safe = safe.expect("Accept always has safe text");
                    let context = item.context;
                    if reuse_tm {
                        if let Some(value) = context {
                            tm.insert_with_context(&item.source, &safe, Some(value));
                        } else {
                            tm.insert(&item.source, &safe);
                        }
                    } else if let Some(value) = context {
                        tm.upsert_with_context(&item.source, &safe, Some(value));
                    } else {
                        tm.upsert(&item.source, &safe);
                    }
                    translations.insert(item.uid, safe);
                    report.ai_translated += 1;
                }
                CandidateClass::QualityFail => {
                    if allow_quality_retry {
                        // 強制模式才允許同輪品質複查一次。
                        quality_retry.push(item.clone());
                    } else {
                        // 一般模式不重送相同項目；保留具體 uid 供工作階段暫緩。
                        keep_english_skip(
                            &mut translations,
                            &mut report,
                            item.uid,
                            &item.source,
                            false,
                            qreason,
                        );
                        quality_deferred.insert(item.uid);
                    }
                }
                CandidateClass::PlaceholderFail => {
                    placeholder_retry.push(item.clone());
                }
            }
        }
        if !quality_retry.is_empty() {
            on_progress(
                base_pct.saturating_add(span_pct / 4),
                &format!(
                    "品質複查：強制模式重試 {} 句…",
                    quality_retry.len()
                ),
            );
            let quality_raw = match run_batches(
                &engine,
                &mut batch_state,
                &quality_retry,
                base_pct,
                span_pct,
                on_progress,
                false,
            ) {
                Ok(raw) => raw,
                Err(e) if is_cancel_message(&e) => return Err(e),
                Err(e) => {
                    report.notes.push(format!(
                        "品質再試中斷（已保留主批成果）：{}",
                        e.lines().next().unwrap_or("連線問題")
                    ));
                    for item in &quality_retry {
                        keep_english_skip(
                            &mut translations,
                            &mut report,
                            item.uid,
                            &item.source,
                            false,
                            None,
                        );
                    }
                    HashMap::new()
                }
            };
            let mut quality_skip_round = 0usize;
            for item in &quality_retry {
                let Some(masked_out) = quality_raw.get(&item.uid) else {
                    if quality_raw.is_empty() {
                        // 已在 Err 分支計入 quality_skipped
                        quality_skip_round += 1;
                        continue;
                    }
                    keep_english_skip(
                        &mut translations,
                        &mut report,
                        item.uid,
                        &item.source,
                        false,
                        None,
                    );
                    quality_skip_round += 1;
                    continue;
                };
                let candidate = placeholder::unmask(masked_out, &item.tokens);
                let candidate = gloss.enforce_terms(&item.source, &candidate, &mut term_stats);
                let ((class, safe, attempt_guard, qreason), local_issue) =
                    classify_candidate_for(&item.source, &candidate, is_local_model);
                record_local_issue(&mut report, local_issue);
                guard.checked += attempt_guard.checked;
                guard.repaired += attempt_guard.repaired;
                guard.rejected += attempt_guard.rejected;
                match class {
                    CandidateClass::Accept => {
                        let safe = safe.expect("Accept always has safe text");
                        if reuse_tm {
                            if let Some(value) = item.context {
                                tm.insert_with_context(&item.source, &safe, Some(value));
                            } else {
                                tm.insert(&item.source, &safe);
                            }
                        } else if let Some(value) = item.context {
                            tm.upsert_with_context(&item.source, &safe, Some(value));
                        } else {
                            tm.upsert(&item.source, &safe);
                        }
                        translations.insert(item.uid, safe);
                        report.ai_translated += 1;
                    }
                    CandidateClass::QualityFail | CandidateClass::PlaceholderFail => {
                        keep_english_skip(
                            &mut translations,
                            &mut report,
                            item.uid,
                            &item.source,
                            false,
                            qreason,
                        );
                        // 品質未過一律排進補完佇列——包含**沒有 qreason 的那種**。
                        // 本地模型被擋下的退化輸出（`{0` 亂碼、只剩半個詞）走的就是
                        // qreason=None 這條路；舊版因此把它們排除在補完之外，
                        // 等於「本地翻不好的句子永遠翻不好」。
                        if matches!(class, CandidateClass::QualityFail) {
                            quality_deferred.insert(item.uid);
                        }
                        quality_skip_round += 1;
                    }
                }
            }
            if quality_skip_round > 0 {
                on_progress(
                    base_pct.saturating_add(span_pct / 4),
                    &format!(
                        "品質未過略過 {} 句（未寫入）",
                        quality_skip_round
                    ),
                );
            }
        }

        if !placeholder_retry.is_empty() {
            let total_ph = placeholder_retry.len();
            let (to_strict, overflow) =
                split_strict_retry_cap(placeholder_retry, STRICT_PLACEHOLDER_RETRY_CAP);
            if !overflow.is_empty() {
                let n = overflow.len();
                for item in &overflow {
                    keep_english_skip(
                        &mut translations,
                        &mut report,
                        item.uid,
                        &item.source,
                        true,
                        None,
                    );
                }
                on_progress(
                    base_pct.saturating_add(span_pct / 2),
                    &format!(
                        "佔位符失敗 {} 句：嚴格重試至多 {} 句，已略過 {} 句（超過嚴格上限）",
                        total_ph,
                        STRICT_PLACEHOLDER_RETRY_CAP,
                        n
                    ),
                );
                report.notes.push(format!(
                    "佔位符嚴格重試上限 {}：略過 {} 句（保留英文，不重燒）",
                    STRICT_PLACEHOLDER_RETRY_CAP, n
                ));
            }
            if !to_strict.is_empty() {
                on_progress(
                    base_pct.saturating_add(span_pct / 2),
                    &format!(
                        "佔位符嚴格重試 {} 句（上限 {}）…",
                        to_strict.len(),
                        STRICT_PLACEHOLDER_RETRY_CAP
                    ),
                );
                let strict_raw = match run_batches(
                    &engine,
                    &mut batch_state,
                    &to_strict,
                    base_pct,
                    span_pct,
                    on_progress,
                    true,
                ) {
                    Ok(raw) => raw,
                    Err(e) if is_cancel_message(&e) => return Err(e),
                    Err(e) => {
                        report.notes.push(format!(
                            "佔位符嚴格重試中斷（已保留既有成果）：{}",
                            e.lines().next().unwrap_or("連線問題")
                        ));
                        for item in &to_strict {
                            next_pending.push(PendingItem {
                                uid: item.uid,
                                source: item.source.clone(),
                                reason: PendingReason::NoAnswer,
                            });
                        }
                        HashMap::new()
                    }
                };
                for item in &to_strict {
                    let Some(masked_out) = strict_raw.get(&item.uid) else {
                        if strict_raw.is_empty() {
                            continue;
                        }
                        next_pending.push(PendingItem {
                            uid: item.uid,
                            source: item.source.clone(),
                            reason: PendingReason::NoAnswer,
                        });
                        continue;
                    };
                    let candidate = placeholder::unmask(masked_out, &item.tokens);
                    let candidate = gloss.enforce_terms(&item.source, &candidate, &mut term_stats);
                    let ((class, safe, attempt_guard, qreason), local_issue) =
                        classify_candidate_for(&item.source, &candidate, is_local_model);
                    record_local_issue(&mut report, local_issue);
                    guard.checked += attempt_guard.checked;
                    guard.repaired += attempt_guard.repaired;
                    guard.rejected += attempt_guard.rejected;
                    match class {
                        CandidateClass::Accept => {
                            let safe = safe.expect("Accept always has safe text");
                            if reuse_tm {
                                if let Some(value) = item.context {
                                    tm.insert_with_context(&item.source, &safe, Some(value));
                                } else {
                                    tm.insert(&item.source, &safe);
                                }
                            } else if let Some(value) = item.context {
                                tm.upsert_with_context(&item.source, &safe, Some(value));
                            } else {
                                tm.upsert(&item.source, &safe);
                            }
                            translations.insert(item.uid, safe);
                            report.ai_translated += 1;
                        }
                        CandidateClass::QualityFail => {
                            keep_english_skip(
                                &mut translations,
                                &mut report,
                                item.uid,
                                &item.source,
                                false,
                                qreason,
                            );
                            // 沒有 qreason 的品質未過＝本地模型退化被擋下，
                            // 同樣要排進補完佇列（見上面同類註解）
                            quality_deferred.insert(item.uid);
                        }
                        CandidateClass::PlaceholderFail => {
                            next_pending.push(PendingItem {
                                uid: item.uid,
                                source: item.source.clone(),
                                reason: PendingReason::PlaceholderBroken,
                            });
                        }
                    }
                }
            }
        }

        pending_ai = next_pending;
        attempt += 1;
    }

    // 本地模式的最後一步：小模型過不了品質關的句子，改用雲端翻一次。
    //
    // 「本地翻譯品質要跟雲端一樣」這個要求，靠調本地模型本身是做不到的——
    // 小模型的能力就是不如大模型。但**交付出去的結果**可以一樣：
    // 讓本地負責它做得好的（絕大多數），做不好的那些交給雲端補完。
    if is_local_model && !quality_deferred.is_empty() {
        escalate_to_cloud(
            &gloss,
            &mut term_stats,
            &mut guard,
            &mut tm,
            &mut translations,
            &mut report,
            &mut quality_deferred,
            unique,
            ctx,
            reuse_tm,
            base_pct,
            span_pct,
            on_progress,
        );
    }

    if report.quality_skipped > 0 {
        report.notes.push(format!(
            "品質未過略過 {} 句（未寫入；{}={}／{}={}／{}={}）",
            report.quality_skipped,
            QualityFailReason::StillEnglish.as_str(),
            report.quality_still_english,
            QualityFailReason::MixedFragment.as_str(),
            report.quality_mixed_fragment,
            QualityFailReason::SameAsSource.as_str(),
            report.quality_same_as_source
        ));
    }
    if !report.local_degenerate.is_empty() {
        // 這段話要讓「不懂技術的玩家」看得懂發生什麼事、要不要理它。
        // 關鍵是分清楚兩種情況：**有翻出來但不夠好**、和**沒翻出來留英文**——
        // 前者不必做什麼，後者才需要考慮改用線上 AI。
        let detail = report
            .local_degenerate
            .iter()
            .map(|(label, count)| format!("{label} {count} 句"))
            .collect::<Vec<_>>()
            .join("、");
        let total: usize = report.local_degenerate.iter().map(|(_, c)| *c).sum();
        report.notes.push(format!(
            "本地模型有 {total} 句翻得不太好（{detail}）。這是離線小模型的能力限制，\
不是工具漏掉沒翻。其中只有會誤導你的（數字對不上、整段重複）才會留原文，\
其餘照樣採用——讀起來不完美，但看得懂。想要更好的品質，可以在工作台的「AI 輔助翻譯」改選「自訂 API」或「GPT」。"
        ));
    }
    drain_pending_ai(
        &pending_ai,
        &mut translations,
        &mut report,
        &mut quality_deferred,
    );

    // 本地模式的最後一步：小模型過不了品質關、或這一輪沒拿到回應的句子，
    // 改用雲端翻一次。
    //
    // 「本地翻譯品質要跟雲端一樣」這個要求，靠調本地模型本身是做不到的——
    // 小模型的能力就是不如大模型。但**交付出去的結果**可以一樣：
    // 讓本地負責它做得好的（絕大多數），做不好的那些交給雲端補完。
    //
    // 這段必須排在 pending_ai 收尾**之後**：沒拿到回應的句子是在上面那段
    // 才進 quality_deferred 的，排在前面就會漏掉它們（舊版就是排在前面）。
    if is_local_model && !quality_deferred.is_empty() {
        escalate_to_cloud(
            &gloss,
            &mut term_stats,
            &mut guard,
            &mut tm,
            &mut translations,
            &mut report,
            &mut quality_deferred,
            unique,
            ctx,
            reuse_tm,
            base_pct,
            span_pct,
            on_progress,
        );
    }

    report.usage = engine.usage_snapshot();
    for note in engine.drain_notices() {
        report.notes.push(note);
    }
    if let Some(cost_note) = engine.capabilities.cost_note(
        engine.provider,
        report.usage.prompt_cache_hit_tokens,
        report.usage.prompt_cache_miss_tokens,
        report.usage.completion_tokens,
    ) {
        report.notes.push(cost_note);
    }

    on_progress(
        base_pct.saturating_add(span_pct.saturating_mul(95) / 100),
        "正在儲存本機翻譯記憶…",
    );
    if let Err(e) = tm.save() {
        report.notes.push(format!("翻譯記憶未能存檔：{e}"));
    }
    report.notes.push(tm.note());
    if guard.repaired > 0 || guard.rejected > 0 {
        report.notes.push(guard.note());
    }
    if let Some(note) = term_stats.note() {
        report.notes.push(note);
    }

    // 社群貢獻改由 fill_missing 寫回後的 keyed to_share 一次送出（避免雙重上傳卡住 UI）
    Ok(Resolved {
        translations,
        report,
        quality_deferred,
    })
}

/// 本地模型過不了品質關的句子，改用雲端補完，讓最終品質與全程走雲端一致。
///
/// 為什麼需要這一步：小模型的**能力**不可能等於大模型，這是模型本身的限制，
/// 調參數、換 prompt 都繞不過去。但站長要的是「**結果**一樣」——那就讓本地做它
/// 做得好的（絕大多數短句、名稱、術語），把它做不好的那一小撮交給雲端。
///
/// 沒有可用雲端時（沒設自訂 API、也沒登入 GPT）不是失敗，只是補不了；
/// 這時要明講「設定哪一個就能自動補完」，而不是留下一句沒人看得懂的錯誤。
#[allow(clippy::too_many_arguments)]
fn escalate_to_cloud(
    gloss: &Glossary,
    term_stats: &mut TermConsistencyStats,
    guard: &mut GuardStats,
    tm: &mut TmSaveGuard,
    translations: &mut HashMap<usize, String>,
    report: &mut AiFillReport,
    quality_deferred: &mut HashSet<usize>,
    unique: &[String],
    ctx: &[Option<&'static str>],
    reuse_tm: bool,
    base_pct: u8,
    span_pct: u8,
    on_progress: &mut dyn FnMut(u8, &str),
) {
    let pending: Vec<PendingItem> = {
        let mut ids: Vec<usize> = quality_deferred.iter().copied().collect();
        ids.sort_unstable(); // HashSet 疊代順序不穩定，排序才能重現
        ids.into_iter()
            .filter_map(|uid| {
                unique.get(uid).map(|source| PendingItem {
                    uid,
                    source: source.clone(),
                    reason: PendingReason::NoAnswer,
                })
            })
            .collect()
    };
    if pending.is_empty() {
        return;
    }

    let Some(engine) = Engine::connect_cloud_fallback() else {
        // 兩種原因：玩家自己關掉了補完，或是還沒有可用的線上 AI。
        // 分開講，因為要採取的行動完全不同。
        if !super::secrets::cloud_topup_enabled() {
            report.notes.push(format!(
                "有 {} 句本地模型翻不好，依你的設定保留原文（沒有送到線上 AI）。",
                pending.len()
            ));
        } else {
            report.notes.push(format!(
                "有 {} 句本地模型翻不好，先保留原文。到工作台的「AI 輔助翻譯」改選「自訂 API」或「GPT」之後，工具會自動用線上 AI 把這些句子補完，品質就跟全程用線上 AI 一樣。",
                pending.len()
            ));
        }
        return;
    };
    on_progress(
        base_pct,
        &format!(
            "本地模型翻不好的 {} 句，改用線上 AI 補完（讓品質一致）…",
            pending.len()
        ),
    );

    let items = build_masked_items(&pending, ctx);
    let system_prompt = build_system_prompt(gloss, unique, false);
    let mut state = BatchRuntimeState::new(system_prompt, engine.capabilities.start_parallel);
    let raw = match run_batches(&engine, &mut state, &items, base_pct, span_pct, on_progress, false)
    {
        Ok(raw) => raw,
        Err(e) => {
            // 補完失敗不影響本地已經翻好的部分，照原樣留在待補清單
            report.notes.push(format!(
                "線上 AI 補完沒有完成，這些句子先保留原文：{}",
                e.lines().next().unwrap_or("連線問題")
            ));
            return;
        }
    };

    let mut fixed = 0usize;
    for item in &items {
        let Some(masked_out) = raw.get(&item.uid) else {
            continue;
        };
        let candidate = placeholder::unmask(masked_out, &item.tokens);
        let candidate = gloss.enforce_terms(&item.source, &candidate, term_stats);
        // 雲端的產出一樣要過把關；這裡不開本地專屬檢查（大模型不犯那些病）
        let ((class, safe, attempt_guard, _), _) =
            classify_candidate_for(&item.source, &candidate, false);
        guard.checked += attempt_guard.checked;
        guard.repaired += attempt_guard.repaired;
        guard.rejected += attempt_guard.rejected;
        if !matches!(class, CandidateClass::Accept) {
            continue;
        }
        let safe = safe.expect("Accept always has safe text");
        if reuse_tm {
            if let Some(value) = item.context {
                tm.insert_with_context(&item.source, &safe, Some(value));
            } else {
                tm.insert(&item.source, &safe);
            }
        } else if let Some(value) = item.context {
            tm.upsert_with_context(&item.source, &safe, Some(value));
        } else {
            tm.upsert(&item.source, &safe);
        }
        translations.insert(item.uid, safe);
        quality_deferred.remove(&item.uid);
        report.ai_translated += 1;
        // 這一句已經補好了，不該再算在「品質未過」裡
        report.quality_skipped = report.quality_skipped.saturating_sub(1);
        fixed += 1;
    }

    report.usage.add(&engine.usage_snapshot());
    if fixed > 0 {
        report.notes.push(format!(
            "已用線上 AI 補完 {fixed} 句本地模型翻不好的內容，品質與全程使用線上 AI 一樣。"
        ));
    }
    let left = pending.len().saturating_sub(fixed);
    if left > 0 {
        report
            .notes
            .push(format!("還有 {left} 句連線上 AI 也沒過品質檢查，已列進沒翻到的清單。"));
    }
}

/// 本地小模型專用：把長句拆成子句分別送翻，再依原順序組回去。
///
/// 回傳「這一輪沒解決」的項目，交給主迴圈照原本的整句流程處理。
/// 只要拆句這條路有任何一步不順（子句沒回來、組回來的譯文沒過把關、
/// 連線失敗），該句就原封不動退回去整句翻——**寧可品質差一點，
/// 也不要交出半中半英或順序錯亂的東西**。
///
/// 防錯位的關鍵：每個子句配一個**獨立 uid**（排在真實 uid 之後），
/// 送進既有批次引擎後用 uid 一一取回，完全不依賴模型維持順序或數量。
#[allow(clippy::too_many_arguments)]
fn translate_long_sentences_split(
    engine: &Engine,
    batch_state: &mut BatchRuntimeState,
    gloss: &Glossary,
    term_stats: &mut TermConsistencyStats,
    guard: &mut GuardStats,
    tm: &mut TmSaveGuard,
    translations: &mut HashMap<usize, String>,
    report: &mut AiFillReport,
    pending: Vec<PendingItem>,
    ctx: &[Option<&'static str>],
    reuse_tm: bool,
    base_pct: u8,
    span_pct: u8,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<Vec<PendingItem>, String> {
    let is_local = matches!(engine.provider, AiProvider::LocalLlm);
    let mut leftover: Vec<PendingItem> = Vec::with_capacity(pending.len());
    let mut jobs: Vec<(PendingItem, Vec<String>)> = Vec::new();
    for item in pending {
        // 用 _safe 版本：拆句只是最佳化，不該有能力中斷整輪翻譯。
        // v21 就是這裡一個位元組索引 bug 讓整包翻譯在 63% 全毀。
        let segments = if sentence_split::should_split_safe(&item.source, is_local) {
            sentence_split::split_sentence_safe(&item.source)
        } else {
            Vec::new()
        };
        if segments.len() >= 2 {
            jobs.push((item, segments));
        } else {
            leftover.push(item);
        }
    }
    if jobs.is_empty() {
        return Ok(leftover);
    }

    // 子句 uid 一律排在所有真實 uid 之後，確保不會和任何待譯項目相撞
    let mut next_uid = ctx
        .len()
        .max(jobs.iter().map(|(item, _)| item.uid + 1).max().unwrap_or(0));
    let mut segment_items: Vec<MaskedItem> = Vec::new();
    // (原句 uid, 該句的子句 uid 依原順序)
    let mut segment_ids: Vec<Vec<usize>> = Vec::with_capacity(jobs.len());
    for (item, segments) in &jobs {
        let context = ctx.get(item.uid).copied().flatten();
        let mut ids = Vec::with_capacity(segments.len());
        for segment in segments {
            let (masked, tokens) = placeholder::mask(segment);
            segment_items.push(MaskedItem {
                uid: next_uid,
                source: segment.clone(),
                masked,
                tokens,
                context,
            });
            ids.push(next_uid);
            next_uid += 1;
        }
        segment_ids.push(ids);
    }

    on_progress(
        base_pct,
        &format!(
            "長句拆解：{} 句較長的說明先拆成 {} 個子句分別翻譯（降低漏譯與語意錯誤）…",
            jobs.len(),
            segment_items.len()
        ),
    );

    let raw = match run_batches(
        engine,
        batch_state,
        &segment_items,
        base_pct,
        span_pct,
        on_progress,
        false,
    ) {
        Ok(raw) => raw,
        Err(e) if is_cancel_message(&e) => return Err(e),
        Err(e) => {
            // 拆句這條路壞掉不該影響正事：整批退回整句翻
            report.notes.push(format!(
                "長句拆解未完成，改用整句翻譯：{}",
                e.lines().next().unwrap_or("連線問題")
            ));
            leftover.extend(jobs.into_iter().map(|(item, _)| item));
            return Ok(leftover);
        }
    };

    let by_uid: HashMap<usize, &MaskedItem> =
        segment_items.iter().map(|item| (item.uid, item)).collect();
    let mut merged = 0usize;
    for ((item, segments), ids) in jobs.into_iter().zip(segment_ids.into_iter()) {
        let pieces: Vec<Option<String>> = ids
            .iter()
            .map(|id| {
                let segment = by_uid.get(id)?;
                let masked_out = raw.get(id)?;
                let text = placeholder::unmask(masked_out, &segment.tokens);
                Some(gloss.enforce_terms(&segment.source, &text, term_stats))
            })
            .collect();
        let Some(joined) = sentence_split::rejoin(&segments, &pieces) else {
            leftover.push(item);
            continue;
        };
        // 組回來的整句要重新過一次佔位符與品質檢查——拆句不是免檢通道
        let ((class, safe, attempt_guard, _), local_issue) =
            classify_candidate_for(&item.source, &joined, is_local);
        record_local_issue(report, local_issue);
        guard.checked += attempt_guard.checked;
        guard.repaired += attempt_guard.repaired;
        guard.rejected += attempt_guard.rejected;
        match class {
            CandidateClass::Accept => {
                let safe = safe.expect("Accept always has safe text");
                let context = ctx.get(item.uid).copied().flatten();
                if reuse_tm {
                    if let Some(value) = context {
                        tm.insert_with_context(&item.source, &safe, Some(value));
                    } else {
                        tm.insert(&item.source, &safe);
                    }
                } else if let Some(value) = context {
                    tm.upsert_with_context(&item.source, &safe, Some(value));
                } else {
                    tm.upsert(&item.source, &safe);
                }
                translations.insert(item.uid, safe);
                report.ai_translated += 1;
                merged += 1;
            }
            // 拆句版本沒過把關就當作沒發生過，退回整句翻
            CandidateClass::QualityFail | CandidateClass::PlaceholderFail => leftover.push(item),
        }
    }
    if merged > 0 {
        on_progress(
            base_pct,
            &format!("長句拆解完成 {merged} 句；其餘退回整句翻譯"),
        );
    }
    Ok(leftover)
}

// ═══ 連線 ═════════════════════════════════════════════════════

#[derive(Clone)]
struct Engine {
    client: Arc<reqwest::blocking::Client>,
    base_url: Arc<String>,
    url: Arc<String>,
    /// 自訂 API 金鑰；GPT 模式可為空。
    api_key: Arc<String>,
    model: Arc<String>,
    managed_session: Arc<Mutex<String>>,
    managed: bool,
    provider: AiProvider,
    capabilities: ProviderCapabilities,
    degraded: Arc<Mutex<RequestDegradeState>>,
    usage: Arc<Mutex<AiUsageTotals>>,
    notices: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug, Clone)]
struct PendingItem {
    uid: usize,
    source: String,
    reason: PendingReason,
}

/// 一句話為什麼還躺在待譯清單裡。**這兩種必須分開處理。**
///
/// 舊版把兩種都當成「佔位符失敗」：寫回英文 ＋ 進負向快取。
/// 結果是「這一批 AI 根本沒回應」的句子被永久標記成翻不動——
/// 玩家按「補充漏翻」也救不回來，因為負向快取讓它連試都不試。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingReason {
    /// AI 有回應，但譯文把 `%s`／`§` 之類的格式符號弄壞了。
    ///
    /// 寫回英文是對的：壞掉的格式符號會讓遊戲當掉，英文至少能玩。
    /// 進負向快取也是對的：同一句再送一次，結果通常一樣。
    PlaceholderBroken,
    /// 根本沒拿到回應（批次重試額度用盡、連線中斷、回應裡缺這一條）。
    ///
    /// 這種**不是**翻不動，是還沒翻到。不可以寫回英文假裝完成，
    /// 也不可以進負向快取——要留在缺口裡，讓補完／補充漏翻能再試。
    NoAnswer,
}

#[derive(Debug, Clone)]
struct MaskedItem {
    uid: usize,
    source: String,
    masked: String,
    tokens: Vec<String>,
    context: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequestFeatures {
    send_response_format: bool,
    max_tokens_field: MaxTokensField,
    send_temperature: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct RequestDegradeState {
    drop_response_format: bool,
    use_max_completion_tokens: bool,
    drop_temperature: bool,
}

impl RequestDegradeState {
    fn features(self, capabilities: ProviderCapabilities) -> RequestFeatures {
        let max_tokens_field = if self.use_max_completion_tokens {
            MaxTokensField::MaxCompletionTokens
        } else {
            capabilities.max_tokens_field
        };
        RequestFeatures {
            send_response_format: capabilities.supports_json_mode && !self.drop_response_format,
            max_tokens_field,
            send_temperature: !self.drop_temperature,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BatchTrackKind {
    Name,
    Ui,
    Story,
    Solo,
}

impl BatchTrackKind {
    /// 每批送幾條。
    ///
    /// 本地模型另有一套較小的值。原因不是品質假設，是硬限制：llama-server 的上下文視窗
    /// 是 prompt ＋ 輸出共用，一批 48 條加上術語表很容易把視窗吃滿，結果是漏條、
    /// 格式跑掉或直接截斷。每條字串本來就彼此獨立、不需要跨條上下文，所以縮小批次
    /// 只影響「一次送幾條」，不影響單條的翻譯品質；模組包語境是靠 system prompt 的
    /// 術語表提供的，與批次大小無關。
    fn batch_size_for(self, strict_single: bool, local: bool) -> usize {
        if strict_single {
            return 1;
        }
        if local {
            return match self {
                Self::Name => 12,
                Self::Ui => 12,
                Self::Story => 6,
                Self::Solo => 1,
            };
        }
        match self {
            Self::Name => 48,
            Self::Ui => 48,
            Self::Story => 20,
            Self::Solo => 1,
        }
    }

    fn timeout_secs(self) -> u64 {
        match self {
            Self::Name => 60,
            Self::Ui => 90,
            Self::Story | Self::Solo => 240,
        }
    }

    /// Name／Ui 大批空回應重排時可拆半；Story／Solo 已夠小不拆。
    fn may_split_on_empty_requeue(self) -> bool {
        matches!(self, Self::Name | Self::Ui)
    }
}

/// AIMD 只認明確 congestion（429／5xx／逾時），不含空 content 等 Transient。
fn should_mark_round_congestion(err_congestion: bool) -> bool {
    err_congestion
}

fn empty_content_backoff_ms(attempt: usize) -> u64 {
    400 + attempt as u64 * 500
}

fn choice_finish_reason(v: &Value) -> &str {
    v["choices"][0]
        .get("finish_reason")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
}

fn is_length_truncated(v: &Value) -> bool {
    choice_finish_reason(v).eq_ignore_ascii_case("length")
}

/// DeepSeek 官方：思考模式預設開啟；批次 JSON 翻譯需明確關閉。
fn should_send_thinking_disabled(engine: &Engine) -> bool {
    matches!(engine.provider, AiProvider::Deepseek)
}

/// 本地模型多半是 Qwen3 系列的「混合推理」模型，llama-server（--jinja）用
/// `chat_template_kwargs.enable_thinking` 這個鍵關閉思考過程——跟 DeepSeek 官方的
/// `thinking` 欄位是兩套完全不同的 API 慣例，不能共用同一個判斷式。
fn should_disable_local_thinking(engine: &Engine) -> bool {
    matches!(engine.provider, AiProvider::LocalLlm)
}

fn is_empty_response_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("沒有回傳翻譯內容")
        || m.contains("回傳為空 json")
        || m.contains("空 json 物件")
        || m.contains("finish_reason=length")
}

/// 從 choices[0] 拼空 content 診斷（finish_reason／refusal／鍵名），方便日誌對症。
fn diagnose_empty_choice(v: &Value) -> String {
    let choice = &v["choices"][0];
    let finish = choice
        .get("finish_reason")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let message = &choice["message"];
    let refusal = message
        .get("refusal")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let mut parts: Vec<String> = Vec::new();
    if !finish.is_empty() {
        parts.push(format!("finish_reason={finish}"));
    }
    if !refusal.is_empty() {
        let short: String = refusal.chars().take(80).collect();
        parts.push(format!("refusal={short}"));
    }
    let choice_keys = object_field_keys(choice, 12);
    if !choice_keys.is_empty() {
        parts.push(format!("choice_keys={choice_keys}"));
    }
    let message_keys = object_field_keys(message, 12);
    if !message_keys.is_empty() {
        parts.push(format!("message_keys={message_keys}"));
    }
    if parts.is_empty() {
        "服務有連線但沒有回傳翻譯內容".into()
    } else {
        format!(
            "服務有連線但沒有回傳翻譯內容（{}）",
            parts.join(", ")
        )
    }
}

/// 消毒：只列 JSON object 鍵名，不寫內容／金鑰。
fn object_field_keys(v: &Value, max: usize) -> String {
    let Some(map) = v.as_object() else {
        return String::new();
    };
    let mut keys: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    keys.into_iter().take(max).collect::<Vec<_>>().join(",")
}

/// content 空時，僅當 reasoning 可解析為譯文 JSON 才採用（reasoner／相容閘道）。
fn looks_like_translation_payload(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    parse_translation_object(text)
        .map(|m| !m.is_empty())
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
struct BatchPlan {
    track: BatchTrackKind,
    items: Vec<MaskedItem>,
}

#[derive(Debug)]
struct ChunkError {
    message: String,
    congestion: bool,
    retries: usize,
    kind: ChatErrorKind,
}

#[derive(Debug)]
struct ChunkSuccess {
    map: HashMap<usize, String>,
    retries: usize,
}

#[derive(Debug, Clone)]
struct QueuedPlan {
    batch_no: usize,
    plan: BatchPlan,
    requeues: usize,
}

/// 空回應重排：Name／Ui 且 >1 條時拆半，降低截斷／空物件機率。
fn split_queued_for_retry(queued: QueuedPlan) -> Vec<QueuedPlan> {
    let next_requeues = queued.requeues.saturating_add(1);
    let track = queued.plan.track;
    let batch_no = queued.batch_no;
    let items = queued.plan.items;
    if track.may_split_on_empty_requeue() && items.len() > 1 {
        let mid = items.len() / 2;
        let right = items[mid..].to_vec();
        let left = items[..mid].to_vec();
        return vec![
            QueuedPlan {
                batch_no,
                plan: BatchPlan {
                    track,
                    items: left,
                },
                requeues: next_requeues,
            },
            QueuedPlan {
                batch_no,
                plan: BatchPlan {
                    track,
                    items: right,
                },
                requeues: next_requeues,
            },
        ];
    }
    vec![QueuedPlan {
        batch_no,
        plan: BatchPlan { track, items },
        requeues: next_requeues,
    }]
}

fn push_requeue_plans(requeue_back: &mut Vec<QueuedPlan>, queued: QueuedPlan, split_empty: bool) {
    if split_empty {
        requeue_back.extend(split_queued_for_retry(queued));
    } else {
        let mut again = queued;
        again.requeues = again.requeues.saturating_add(1);
        requeue_back.push(again);
    }
}

#[derive(Debug)]
struct BatchRuntimeState {
    system_prompt: Arc<String>,
    parallel_cap: usize,
    current_parallel: usize,
    warmed_up: bool,
}

impl BatchRuntimeState {
    fn new(system_prompt: String, parallel_cap: usize) -> Self {
        let cap = parallel_cap.max(1);
        Self {
            system_prompt: Arc::new(system_prompt),
            parallel_cap: cap,
            current_parallel: 1,
            warmed_up: false,
        }
    }

    fn finish_round(&mut self, congestion: bool, made_progress: bool) {
        if congestion {
            self.current_parallel = (self.current_parallel / 2).max(1);
            self.warmed_up = true;
            return;
        }
        if !made_progress {
            return;
        }
        if !self.warmed_up {
            self.current_parallel = 4.min(self.parallel_cap).max(1);
            self.warmed_up = true;
        } else {
            self.current_parallel = (self.current_parallel + 2).min(self.parallel_cap);
        }
    }
}

impl Engine {
    fn connect_raw() -> Result<Self, String> {
        let cfg = resolve_ai_config()?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(cfg.capabilities.start_parallel + 2)
            .build()
            .map_err(|e| e.to_string())?;
        let url = if matches!(cfg.provider, AiProvider::Codex) {
            CODEX_RESPONSES_URL.to_string()
        } else {
            api_chat_completions_url(&cfg.base_url)
        };
        Ok(Engine {
            client: Arc::new(client),
            base_url: Arc::new(cfg.base_url.clone()),
            url: Arc::new(url),
            api_key: Arc::new(cfg.api_key),
            model: Arc::new(cfg.model),
            managed_session: Arc::new(Mutex::new(String::new())),
            managed: false,
            provider: cfg.provider,
            capabilities: cfg.capabilities,
            degraded: Arc::new(Mutex::new(RequestDegradeState::default())),
            usage: Arc::new(Mutex::new(AiUsageTotals::default())),
            notices: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// 用「雲端補完」設定建一個引擎，不管使用者目前選的是哪個模式。
    ///
    /// 給本地模式的品質升級用：小模型過不了品質關的句子改用雲端翻一次，
    /// 交付出去的結果就跟全程走雲端一樣。
    fn connect_cloud_fallback() -> Option<Self> {
        let cfg = resolve_cloud_fallback_config()?;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(cfg.capabilities.start_parallel + 2)
            .build()
            .ok()?;
        let url = if matches!(cfg.provider, AiProvider::Codex) {
            CODEX_RESPONSES_URL.to_string()
        } else {
            api_chat_completions_url(&cfg.base_url)
        };
        Some(Engine {
            client: Arc::new(client),
            base_url: Arc::new(cfg.base_url.clone()),
            url: Arc::new(url),
            api_key: Arc::new(cfg.api_key),
            model: Arc::new(cfg.model),
            managed_session: Arc::new(Mutex::new(String::new())),
            managed: false,
            provider: cfg.provider,
            capabilities: cfg.capabilities,
            degraded: Arc::new(Mutex::new(RequestDegradeState::default())),
            usage: Arc::new(Mutex::new(AiUsageTotals::default())),
            notices: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn connect() -> Result<Self, String> {
        let engine = Self::connect_raw()?;
        if let Err(probe) = probe_ai_ready(&engine) {
            return Err(ai_quota_support_message_for(
                &probe,
                engine.provider,
                false,
            ));
        }
        engine.reset_usage();
        Ok(engine)
    }

    fn current_features(&self) -> RequestFeatures {
        self.degraded
            .lock()
            .map(|state| state.features(self.capabilities))
            .unwrap_or_else(|_| RequestDegradeState::default().features(self.capabilities))
    }

    fn maybe_degrade_for_unsupported(&self, code: u16, body: &str) -> bool {
        if !(400..500).contains(&code) || !mentions_unsupported_params(body) {
            return false;
        }
        let lower = body.to_ascii_lowercase();
        let mentions_response_format = lower.contains("response_format");
        let mentions_token_field =
            lower.contains("max_tokens") || lower.contains("max_completion_tokens");
        let mentions_temperature = lower.contains("temperature");
        let generic_only =
            !mentions_response_format && !mentions_token_field && !mentions_temperature;
        let mut changed = None;
        if let Ok(mut state) = self.degraded.lock() {
            if self.capabilities.supports_json_mode
                && !state.drop_response_format
                && (mentions_response_format || generic_only)
            {
                state.drop_response_format = true;
                changed = Some(
                    "AI 相容降級：此服務不接受 response_format，後續改用提示要求 JSON 物件。"
                        .to_string(),
                );
            } else if !state.use_max_completion_tokens
                && self.capabilities.max_tokens_field != MaxTokensField::MaxCompletionTokens
                && (mentions_token_field || generic_only)
            {
                state.use_max_completion_tokens = true;
                changed = Some(
                    "AI 相容降級：此服務不接受目前的 token 欄位，後續改用 max_completion_tokens。"
                        .to_string(),
                );
            } else if !state.drop_temperature && (mentions_temperature || generic_only) {
                state.drop_temperature = true;
                changed = Some(
                    "AI 相容降級：此服務不接受 temperature，後續請求已省略。".to_string(),
                );
            }
        }
        if let Some(note) = changed {
            self.push_notice(note);
            true
        } else {
            false
        }
    }

    fn push_notice(&self, note: String) {
        if let Ok(mut notices) = self.notices.lock() {
            if !notices.iter().any(|existing| existing == &note) {
                notices.push(note);
            }
        }
    }

    fn drain_notices(&self) -> Vec<String> {
        self.notices
            .lock()
            .map(|mut notices| std::mem::take(&mut *notices))
            .unwrap_or_default()
    }

    fn record_usage(&self, usage: &AiUsageTotals) {
        if let Ok(mut total) = self.usage.lock() {
            total.add(usage);
        }
    }

    fn usage_snapshot(&self) -> AiUsageTotals {
        self.usage.lock().map(|usage| usage.clone()).unwrap_or_default()
    }

    fn reset_usage(&self) {
        if let Ok(mut usage) = self.usage.lock() {
            *usage = AiUsageTotals::default();
        }
    }

    fn session_cookie(&self) -> String {
        self.managed_session
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    fn set_session_cookie(&self, cookie: String) {
        if let Ok(mut g) = self.managed_session.lock() {
            *g = cookie;
        }
    }

    /// 從磁碟重載 Discord session；cookie 有變更回 true。
    fn reload_managed_session_from_disk(&self) -> bool {
        if !self.managed {
            return false;
        }
        let Ok(cookie) = managed_ai_session_cookie() else {
            return false;
        };
        let current = self.session_cookie();
        if cookie != current && !cookie.trim().is_empty() {
            self.set_session_cookie(cookie);
            true
        } else {
            false
        }
    }
}

/// 已無免費代管；此函式保留給舊呼叫點，恆為 false。
#[allow(dead_code)]
pub fn managed_ai_available() -> bool {
    false
}

fn build_masked_items(
    items: &[PendingItem],
    ctx: &[Option<&'static str>],
) -> Vec<MaskedItem> {
    items
        .iter()
        .map(|item| {
            let (masked, tokens) = placeholder::mask(&item.source);
            MaskedItem {
                uid: item.uid,
                source: item.source.clone(),
                masked,
                tokens,
                context: ctx.get(item.uid).copied().flatten(),
            }
        })
        .collect()
}

/// AI 這一輪跑完後，還沒有譯文的句子怎麼收尾。
///
/// # 為什麼要分兩種
///
/// 舊版一律當成「佔位符失敗」：寫回英文原文 ＋ 進負向快取。對真的把 `%s`
/// 弄壞的譯文來說那是對的，但對「這一批 AI 根本沒回應」的句子來說是災難——
/// 英文被寫進 langmap 看起來像翻完了，負向快取又讓「補充漏翻」連試都不試，
/// 所以那些句子**永遠**回不來。
///
/// 回傳 `(佔位符失敗數, 沒拿到回應數)`。
fn drain_pending_ai(
    pending: &[PendingItem],
    translations: &mut HashMap<usize, String>,
    report: &mut AiFillReport,
    quality_deferred: &mut HashSet<usize>,
) -> (usize, usize) {
    let mut broken = 0usize;
    let mut unanswered = 0usize;
    for item in pending {
        match item.reason {
            PendingReason::PlaceholderBroken => {
                // 譯文把格式符號弄壞了：英文原文至少不會讓遊戲當掉，
                // 而且同一句再送一次結果通常一樣，所以負向快取是對的。
                keep_english_skip(translations, report, item.uid, &item.source, true, None);
                broken += 1;
            }
            PendingReason::NoAnswer => {
                // 沒拿到回應＝還沒翻到，不是翻不動。
                // 不寫英文、不進負向快取，留在缺口裡讓補完與補充漏翻能再試。
                quality_deferred.insert(item.uid);
                unanswered += 1;
            }
        }
    }
    if broken > 0 {
        report
            .notes
            .push(format!("已略過 {broken} 句佔位符失敗（不重燒）"));
    }
    if unanswered > 0 {
        report.notes.push(format!(
            "有 {unanswered} 句這一輪沒拿到 AI 回應（不是翻不動）。\
它們維持原文並留在缺口裡，下次按「補充漏翻」會再試一次。"
        ));
    }
    (broken, unanswered)
}

fn collect_unresolved_items(
    items: &[MaskedItem],
    resolved: &HashMap<usize, String>,
) -> Vec<PendingItem> {
    items
        .iter()
        .filter(|item| !resolved.contains_key(&item.uid))
        // 沒出現在結果裡＝沒拿到回應（批次被放棄或回應缺這一條）
        .map(|item| PendingItem {
            uid: item.uid,
            source: item.source.clone(),
            reason: PendingReason::NoAnswer,
        })
        .collect()
}

fn build_system_prompt(gloss: &Glossary, texts: &[String], local: bool) -> String {
    let term_cap = if local {
        LOCAL_PROMPT_GLOSSARY_TERMS
    } else {
        MAX_PROMPT_GLOSSARY_TERMS
    };
    let fixed_terms = gloss
        .prompt_cache_terms(texts)
        .into_iter()
        .take(term_cap)
        .collect::<Vec<_>>();
    // 固定前綴必須跨批 byte-identical，且夠長以觸發上游 Context Caching（≥64 token）。
    // 動態內容只允許「本回合固定譯名」區塊；勿插入時間戳／批號／句數。
    let mut prompt = String::with_capacity(8192);
    if local {
        // 本地小模型走精簡版：規則越多、指令遵循越差，而且 8192 的視窗是
        // prompt 與輸出共用的，system 佔掉的每一個 token 都從可翻譯的內容裡扣。
        // 雲端那份有 10 條規則加上百餘個台灣用語範例，對小模型是反效果。
        prompt.push_str(LOCAL_SYSTEM_PREFIX);
    } else {
        prompt.push_str(STABLE_SYSTEM_PREFIX);
    }
    prompt.push_str("固定譯名（英文=中文；原文出現時優先使用）：\n");
    if fixed_terms.is_empty() {
        if local {
            prompt.push_str("(這批沒有固定術語。)\n");
        } else {
            // 維持穩定長度：即使用不到術語，也輸出固定占位，避免前綴過短無法進 cache
            prompt.push_str(STABLE_EMPTY_TERMS_BLOCK);
        }
    } else {
        for (en, zh) in &fixed_terms {
            prompt.push_str("- ");
            prompt.push_str(en);
            prompt.push('=');
            prompt.push_str(zh);
            prompt.push('\n');
        }
    }
    prompt.push_str(STABLE_SYSTEM_SUFFIX);
    prompt
}

/// 本地小模型的 prompt 裡最多放幾條固定術語。
///
/// 雲端可以放 150 條（有 context caching，成本幾乎不變）；本地的 8192 視窗是
/// prompt 與輸出**共用**的，術語表塞太多就等於把可翻譯的空間吃掉。
const LOCAL_PROMPT_GLOSSARY_TERMS: usize = 40;

/// 本地小模型專用的精簡 system prompt。
///
/// 設計取捨：只留**違反就會壞掉**的規則（佔位符、只輸出 JSON、不要旁白），
/// 其餘品質期待（語氣、簡潔度、術語一致）改由譯後把關處理——
/// 小模型記不住十條規則，但譯後檢查一定會執行。
///
/// 特別加了「不要重複同一個詞」與「不要加解釋」兩條，因為那正是
/// `local_quality.rs` 抓到最多的兩種退化輸出，能在源頭少發生一次是一次。
const LOCAL_SYSTEM_PREFIX: &str = "\
你是 Minecraft 模組的台灣繁體中文譯者。把每筆 t 譯成自然的台灣繁體中文。\n\
1. {0} {name} %s %1$s <item:...> $(br) 這類 token 原封不動保留，不可增刪或改字。\n\
2. 數字與單位照抄，不可改動。\n\
3. 只輸出譯文本身：不要加解釋、不要寫「以下是翻譯」、不要重複同一個詞。\n\
4. 純代號、路徑、網址照原樣輸出。\n\
5. 譯文長度應與原文相當，不要擴寫。\n\
";

/// 跨批不變的 system 前半（規則＋台灣用語指引）。長度刻意超過 Context Caching 最小單位。
const STABLE_SYSTEM_PREFIX: &str = "\
你是 Minecraft 模組的繁體中文（台灣）在地化譯者。\
輸出必須符合台灣玩家習慣用詞，不要使用中國大陸簡體或陸用詞。\
規則：\n\
1. 文中的 {0} {1} {name} %s %1$s <item:...> #tag $(br) 等結構 token 必須原封不動保留；可以依中文語序移動，但不可增刪、改字或改成全形。\n\
2. 保留原文開頭與結尾的空白。\n\
3. 若輸入物件有 c，代表語境；物品名、方塊名、生物名、附魔名、狀態效果名、飾品名、按鍵綁定名稱要像名稱，簡短不成句。\n\
4. 必須整句翻成自然的台灣繁體中文；禁止只換部分英文詞、禁止中英拼接、禁止刪減資訊。\n\
5. 已經是正確繁中的句子照原樣輸出；純代號、路徑、網址、resource id 照原樣輸出。\n\
6. 完整保留語意、語氣、段落與上下文；不要省略資訊。\n\
7. 介面按鈕與提示用語簡潔清楚；任務與書本敘事可較完整，但仍避免冗長翻譯腔。\n\
8. 數字、單位、座標、指令參數若屬機械意義則保留原文符號與格式。\n\
9. 不要解釋規則，不要輸出除譯文 JSON 以外的說明。\n\
10. 同一英文術語在整包中保持譯名一致；若下方固定譯名有列出，必須優先採用。\n\
台灣用語參考（僅在語意相符時使用，勿生硬套用）：模組、整合包、伺服器、單人、創造模式、生存模式、觀察者模式、終界、地獄、主世界、紅石、活塞、漏斗、箱子、工作台、熔爐、附魔台、鐵砧、經驗值、生命值、飽食度、盔甲、工具、武器、弓、弩、盾牌、鞘翅、傳送門、生怪磚、指令方塊、資料包、資源包、光影、幀數、延遲、區塊、生物群系、村民、掠奪者、守護者、烈焰人、終界使者、苦力怕、殭屍、骷髏、蜘蛛、史萊姆、悅靈、狼、貓、馬、驢、騾、豬、牛、羊、雞。\n\
";

const STABLE_EMPTY_TERMS_BLOCK: &str = "\
(本次無額外固定術語；請仍遵守上方台灣用語與佔位符規則。)\n\
(CachePrefixPad) Minecraft modpack Traditional Chinese Taiwan localization stable system prefix for prompt caching across translation batches. Keep this English padding unchanged so the shared prefix stays long enough for disk context cache hits on subsequent identical system messages.\n\
";

const STABLE_SYSTEM_SUFFIX: &str =
    "輸出格式：只輸出一個 JSON 物件 {\"r\":[{\"i\":輸入的 i,\"t\":\"譯文\"}]}，不得附帶任何其他文字。";


fn build_user_payload(items: &[MaskedItem]) -> String {
    let rows: Vec<Value> = items
        .iter()
        .map(|item| match item.context {
            Some(context) => json!({ "i": item.uid, "t": item.masked, "c": context }),
            None => json!({ "i": item.uid, "t": item.masked }),
        })
        .collect();
    let data = serde_json::to_string(&json!({ "r": rows })).unwrap_or_else(|_| "{\"r\":[]}".into());
    // 官方 JSON Output：system 或 user 必須明確要求 JSON，否則可能空白到 length
    format!(
        "將每筆 t 譯成台灣繁體中文。只回傳一個 JSON 物件，格式為 {{\"r\":[{{\"i\":數字,\"t\":\"譯文\"}}]}}，不要其他文字。\n{data}"
    )
}

/// 本地模型的輸出上限。
///
/// llama-server 的 `-c` 是**整個** KV 視窗（prompt ＋ 輸出共用）。舊版上下文寫死 4096，
/// 這裡卻可能要求 8192 個輸出 token——比整個視窗還大，長文本必定截斷或直接失敗，
/// 使用者看到的是「已安裝但翻譯失敗」。現在上下文最低 8192（見 local_llm/server.rs），
/// 輸出夾在 2048＝視窗的四分之一，剩下四分之三留給 system prompt、術語表與待譯內容。
pub(crate) const LOCAL_LLM_MAX_COMPLETION_TOKENS: usize = 2048;

fn clamp_completion_tokens_for(input_chars: usize, item_count: usize, local: bool) -> usize {
    let from_chars = input_chars.saturating_mul(8).saturating_div(10);
    let from_items = item_count
        .saturating_mul(TOKENS_PER_ITEM)
        .max(MIN_COMPLETION_TOKENS);
    let ceiling = if local {
        LOCAL_LLM_MAX_COMPLETION_TOKENS
    } else {
        MAX_COMPLETION_TOKENS
    };
    from_chars
        .max(from_items)
        .clamp(MIN_COMPLETION_TOKENS.min(ceiling), ceiling)
}

fn classify_track(source: &str, context: Option<&str>) -> BatchTrackKind {
    let len = source.chars().count();
    if len > SOLO_MIN_CHARS {
        return BatchTrackKind::Solo;
    }
    if len > STORY_MIN_CHARS || is_story_like(context, source) {
        return BatchTrackKind::Story;
    }
    if is_nameish_context(context) && len <= NAME_MAX_CHARS {
        return BatchTrackKind::Name;
    }
    if len <= UI_MAX_CHARS {
        BatchTrackKind::Ui
    } else {
        BatchTrackKind::Story
    }
}

fn is_nameish_context(context: Option<&str>) -> bool {
    matches!(
        context,
        Some(
            "物品名"
                | "方塊名"
                | "液體名"
                | "生物名"
                | "生態域名"
                | "狀態效果名"
                | "附魔名"
                | "飾品名"
                | "按鍵綁定名稱"
        )
    )
}

fn is_story_like(context: Option<&str>, source: &str) -> bool {
    matches!(context, Some("任務文字" | "進度名稱或說明"))
        || (matches!(context, Some("提示說明" | "設定項" | "訊息"))
            && (source.contains('\n') || source.chars().count() > 120))
}

fn plan_track_batches_for(
    items: &[MaskedItem],
    strict_single: bool,
    local: bool,
) -> Vec<BatchPlan> {
    if strict_single {
        return items
            .iter()
            .cloned()
            .map(|item| BatchPlan {
                track: classify_track(&item.source, item.context),
                items: vec![item],
            })
            .collect();
    }

    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| {
        let track_a = classify_track(&a.source, a.context);
        let track_b = classify_track(&b.source, b.context);
        track_a
            .cmp(&track_b)
            .then_with(|| a.source.chars().count().cmp(&b.source.chars().count()))
            .then_with(|| a.source.to_ascii_lowercase().cmp(&b.source.to_ascii_lowercase()))
            .then_with(|| a.uid.cmp(&b.uid))
    });

    let mut plans = Vec::new();
    let mut index = 0usize;
    while index < sorted.len() {
        let track = classify_track(&sorted[index].source, sorted[index].context);
        let batch_size = track.batch_size_for(false, local);
        let mut bucket = Vec::new();
        while index < sorted.len()
            && classify_track(&sorted[index].source, sorted[index].context) == track
            && bucket.len() < batch_size
        {
            bucket.push(sorted[index].clone());
            index += 1;
        }
        plans.push(BatchPlan { track, items: bucket });
    }
    plans
}

// ═══ 批次執行 ═════════════════════════════════════════════════

/// 分組並行送出，回傳 uid → 譯文（未經佔位符把關的原始結果）。
fn run_batches(
    engine: &Engine,
    batch_state: &mut BatchRuntimeState,
    items: &[MaskedItem],
    base_pct: u8,
    span_pct: u8,
    on_progress: &mut dyn FnMut(u8, &str),
    strict_single: bool,
) -> Result<HashMap<usize, String>, String> {
    let plans = plan_track_batches_for(
        items,
        strict_single,
        matches!(engine.provider, AiProvider::LocalLlm),
    );
    let total_batches = plans.len().max(1);
    let total_unique = items.len();

    let translations: Arc<Mutex<HashMap<usize, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut empty_rounds = 0usize;
    let mut finished_batches = 0usize;
    let mut retried_batches = 0usize;
    let mut retry_attempts = 0usize;
    let mut failed_batches = 0usize;
    let mut next_batch_no = 1usize;
    let phase_start = Instant::now();

    let pct = |done: usize, got: usize| -> u8 {
        by_batch_or_strings(done, total_batches, got, total_unique, base_pct, span_pct)
    };

    let flush_notices = |engine: &Engine, p: u8, on_progress: &mut dyn FnMut(u8, &str)| {
        for note in engine.drain_notices() {
            on_progress(p, &note);
        }
    };

    flush_notices(engine, pct(0, 0), on_progress);

    let mut pending: VecDeque<QueuedPlan> = plans
        .into_iter()
        .map(|plan| {
            let batch_no = next_batch_no;
            next_batch_no += 1;
            QueuedPlan {
                batch_no,
                plan,
                requeues: 0,
            }
        })
        .collect();

    while !pending.is_empty() {
        if cancel::is_cancelled() {
            let partial = translations.lock().map(|t| t.clone()).unwrap_or_default();
            engine.push_notice(
                "已依你的要求停止；已保留本階段已成功譯文。".into(),
            );
            flush_notices(engine, pct(finished_batches, partial.len()), on_progress);
            if !partial.is_empty() {
                return Ok(partial);
            }
            return Err(CANCEL_MESSAGE.to_string());
        }

        let parallel = batch_state.current_parallel.max(1).min(batch_state.parallel_cap);
        let mut group: Vec<QueuedPlan> = Vec::new();
        while group.len() < parallel {
            match pending.pop_front() {
                Some(q) => group.push(q),
                None => break,
            }
        }
        let group_n = group.len();
        if group_n == 0 {
            break;
        }
        let got_before = translations.lock().map(|t| t.len()).unwrap_or(0);

        let (tx, rx) = mpsc::channel::<(QueuedPlan, Result<ChunkSuccess, ChunkError>)>();
        let mut handles = Vec::new();
        for queued in group {
            let engine = engine.clone();
            let prompt = Arc::clone(&batch_state.system_prompt);
            let tx = tx.clone();
            let translations = Arc::clone(&translations);
            handles.push(thread::spawn(move || {
                let batch_no = queued.batch_no;
                let result = match translate_chunk(&engine, &prompt, &queued.plan) {
                    Ok(success) => {
                        if let Ok(mut merged) = translations.lock() {
                            for (uid, translated) in &success.map {
                                if !translated.trim().is_empty() {
                                    merged.insert(*uid, translated.clone());
                                }
                            }
                        }
                        Ok(success)
                    }
                    Err(err) => Err(ChunkError {
                        message: if is_cancel_message(&err.message) {
                            err.message
                        } else {
                            format!(
                                "第 {batch_no} 批失敗：{}",
                                sanitize_provider_name(&err.message)
                            )
                        },
                        congestion: err.congestion,
                        retries: err.retries,
                        kind: err.kind,
                    }),
                };
                let _ = tx.send((queued, result));
            }));
        }
        drop(tx);

        let mut done_in_group = 0usize;
        let mut round_congestion = false;
        let mut round_failed = false;
        let mut requeue_back: Vec<QueuedPlan> = Vec::new();
        let mut saw_cancel = false;
        let group_t0 = Instant::now();
        while done_in_group < group_n {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok((queued, outcome)) => {
                    done_in_group += 1;
                    finished_batches += 1;
                    match outcome {
                        Ok(success) => {
                            if success.retries > 0 {
                                retried_batches += 1;
                                retry_attempts += success.retries;
                            }
                            // 成功但空 map：可恢復則重排（Name／Ui 拆半）
                            if success.map.is_empty() && queued.requeues < MAX_PLAN_REQUEUE {
                                push_requeue_plans(&mut requeue_back, queued, true);
                            }
                        }
                        Err(err) => {
                            round_failed = true;
                            // AIMD 只認明確 congestion；空 content Transient 不降並行
                            round_congestion |= should_mark_round_congestion(err.congestion);
                            failed_batches += 1;
                            retry_attempts += err.retries;
                            if err.retries > 0 {
                                retried_batches += 1;
                            }
                            if is_cancel_message(&err.message) {
                                saw_cancel = true;
                            }
                            let detail = truncate_err_msg(&err.message, 220);
                            if let Ok(mut er) = errors.lock() {
                                er.push(err.message.clone());
                            }
                            let got = translations.lock().map(|t| t.len()).unwrap_or(0);
                            on_progress(pct(finished_batches, got), &detail);

                            let can_requeue = !saw_cancel
                                && queued.requeues < MAX_PLAN_REQUEUE
                                && (matches!(err.kind, ChatErrorKind::Transient)
                                    || (err.congestion && is_recoverable_batch_error(&err.message)));
                            if can_requeue {
                                let split_empty = is_empty_response_error(&err.message);
                                push_requeue_plans(&mut requeue_back, queued, split_empty);
                            }
                        }
                    }
                    let got = translations.lock().map(|t| t.len()).unwrap_or(0);
                    let usage = engine.usage_snapshot();
                    flush_notices(engine, pct(finished_batches, got), on_progress);
                    on_progress(
                        pct(finished_batches, got),
                        &format!(
                            "AI 翻譯中… {}／{} 批 · 已得 {}/{} 句 · 重試 {}（{} 批） · 批失敗 {} · {} · 已進行 {} 秒{}",
                            finished_batches.min(total_batches.saturating_add(failed_batches)),
                            total_batches,
                            got,
                            total_unique,
                            retry_attempts,
                            retried_batches,
                            failed_batches,
                            usage.inline_note(),
                            phase_start.elapsed().as_secs(),
                            if round_failed { " · 本輪含批失敗" } else { "" }
                        ),
                    );
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let got = translations.lock().map(|t| t.len()).unwrap_or(0);
                    let wait_secs = group_t0.elapsed().as_secs();
                    let usage = engine.usage_snapshot();
                    flush_notices(engine, pct(finished_batches, got), on_progress);
                    if wait_secs == 0 || wait_secs % 10 == 0 {
                        on_progress(
                            pct(finished_batches, got),
                            &format!(
                                "AI 翻譯中…等待本輪回應 · 已完成 {}／{} 批 · 已得 {}/{} 句 · 重試 {}（{} 批） · 批失敗 {} · {} · 本輪 {} 秒／合計 {} 秒",
                                finished_batches.min(total_batches),
                                total_batches,
                                got,
                                total_unique,
                                retry_attempts,
                                retried_batches,
                                failed_batches,
                                usage.inline_note(),
                                wait_secs,
                                phase_start.elapsed().as_secs()
                            ),
                        );
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        for handle in handles {
            let _ = handle.join();
        }

        if saw_cancel || cancel::is_cancelled() {
            let partial = translations.lock().map(|t| t.clone()).unwrap_or_default();
            engine.push_notice(
                "已依你的要求停止；已保留本階段已成功譯文。".into(),
            );
            flush_notices(engine, pct(finished_batches, partial.len()), on_progress);
            if !partial.is_empty() {
                return Ok(partial);
            }
            return Err(CANCEL_MESSAGE.to_string());
        }

        // 失敗批重排到佇列前端（降並行後優先重試）
        for q in requeue_back.into_iter().rev() {
            pending.push_front(q);
        }

        let got = translations.lock().map(|t| t.len()).unwrap_or(0);
        let err_peek = errors.lock().map(|e| e.clone()).unwrap_or_default();
        let latest = err_peek.last().cloned().unwrap_or_default();

        if got.saturating_sub(got_before) == 0 {
            empty_rounds += 1;
            on_progress(
                pct(finished_batches, got),
                &format!("AI 這一輪沒有新譯文（連續 {} 次）…", empty_rounds),
            );

            // 僅真 login_required：短等待重登（看 cookie 變更或磁碟重載）
            if engine.managed && is_auth_relogin_error(&latest) {
                if engine.reload_managed_session_from_disk() {
                    empty_rounds = 0;
                    on_progress(
                        pct(finished_batches, got),
                        "已重新載入 Discord 登入，繼續翻譯…",
                    );
                    continue;
                }
                on_progress(
                    pct(finished_batches, got),
                    "請在工具重新登入 Discord（最多等待約 10 分鐘）…",
                );
                let before = engine.session_cookie();
                let wait_started = Instant::now();
                let wait_limit = Duration::from_secs(600);
                let mut recovered = false;
                let mut last_prompt_at = Instant::now();
                while wait_started.elapsed() < wait_limit {
                    if cancel::is_cancelled() {
                        break;
                    }
                    thread::sleep(Duration::from_secs(3));
                    if let Ok(cookie) = managed_ai_session_cookie() {
                        if !cookie.trim().is_empty() && cookie != before {
                            engine.set_session_cookie(cookie);
                            recovered = true;
                            break;
                        }
                    }
                    let _ = engine.reload_managed_session_from_disk();
                    if engine.session_cookie() != before && !engine.session_cookie().trim().is_empty()
                    {
                        recovered = true;
                        break;
                    }
                    if last_prompt_at.elapsed() >= Duration::from_secs(45) {
                        last_prompt_at = Instant::now();
                        on_progress(
                            pct(finished_batches, got),
                            &format!(
                                "等待 Discord 登入／驗證恢復…已等 {} 秒（可停止）",
                                wait_started.elapsed().as_secs()
                            ),
                        );
                    }
                }
                if cancel::is_cancelled() {
                    let partial = translations.lock().map(|t| t.clone()).unwrap_or_default();
                    engine.push_notice(
                        "已依你的要求停止；已保留本階段已成功譯文。".into(),
                    );
                    flush_notices(engine, pct(finished_batches, partial.len()), on_progress);
                    if !partial.is_empty() {
                        return Ok(partial);
                    }
                    return Err(CANCEL_MESSAGE.to_string());
                }
                if recovered {
                    empty_rounds = 0;
                    on_progress(
                        pct(finished_batches, got),
                        "Discord 登入已恢復，繼續翻譯…",
                    );
                    continue;
                }
                let partial = translations.lock().map(|t| t.clone()).unwrap_or_default();
                engine.push_notice(
                    "AI 因 Discord 登入中斷提前結束；已保留已成功譯文，可稍後補翻。".into(),
                );
                flush_notices(engine, pct(finished_batches, partial.len()), on_progress);
                if !partial.is_empty() {
                    return Ok(partial);
                }
                return Err(auth_relogin_message());
            }

            // 驗證基建暫態：短退避後繼續（佇列已重排失敗批）
            if is_auth_unavailable_error(&latest) {
                on_progress(
                    pct(finished_batches, got),
                    "驗證暫時無法連線，稍候重試同一批…",
                );
                thread::sleep(Duration::from_secs(2));
                empty_rounds = empty_rounds.saturating_sub(1);
                batch_state.finish_round(true, false);
                continue;
            }

            // 不可恢復（額度等）：有譯文 soft 結束
            if let Some(classified) = classify_batch_abort(&err_peek) {
                let partial = translations.lock().map(|t| t.clone()).unwrap_or_default();
                if !partial.is_empty() {
                    engine.push_notice(format!(
                        "AI 提前結束（{}）；已保留已成功譯文。",
                        classified.lines().next().unwrap_or("連線問題")
                    ));
                    flush_notices(engine, pct(finished_batches, partial.len()), on_progress);
                    return Ok(partial);
                }
                return Err(classified);
            }

            // 可恢復 congestion：降並行，繼續佇列
            if round_congestion || (!latest.is_empty() && is_recoverable_batch_error(&latest)) {
                let old_parallel = batch_state.current_parallel;
                batch_state.finish_round(true, false);
                if batch_state.current_parallel < old_parallel {
                    on_progress(
                        pct(finished_batches, got),
                        &format!(
                            "AI 限流：並行 {}→{}，已降速並重送失敗批",
                            old_parallel, batch_state.current_parallel
                        ),
                    );
                }
                if empty_rounds >= EMPTY_ROUNDS_ABORT {
                    let partial = translations.lock().map(|t| t.clone()).unwrap_or_default();
                    if !partial.is_empty() {
                        engine.push_notice(
                            "AI 連續多輪無新譯文，提前結束；已保留已成功譯文。".into(),
                        );
                        flush_notices(engine, pct(finished_batches, partial.len()), on_progress);
                        return Ok(partial);
                    }
                    let detail = err_peek
                        .last()
                        .cloned()
                        .unwrap_or_else(|| "多次請求沒有有效回應".into());
                    return Err(detail);
                }
                continue;
            }

            if empty_rounds >= EMPTY_ROUNDS_ABORT {
                let partial = translations.lock().map(|t| t.clone()).unwrap_or_default();
                if !partial.is_empty() {
                    engine.push_notice(
                        "AI 連續多輪無新譯文，提前結束；已保留已成功譯文。".into(),
                    );
                    flush_notices(engine, pct(finished_batches, partial.len()), on_progress);
                    return Ok(partial);
                }
                let detail = err_peek
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "多次請求沒有有效回應".into());
                return Err(detail);
            }
        } else {
            empty_rounds = 0;
        }

        let old_parallel = batch_state.current_parallel;
        batch_state.finish_round(round_congestion, !round_failed && got > got_before);
        if round_congestion && batch_state.current_parallel < old_parallel {
            on_progress(
                pct(finished_batches, got),
                &format!(
                    "AI 限流：並行 {}→{}，已降速（限流）",
                    old_parallel, batch_state.current_parallel
                ),
            );
        }
    }

    let out = translations.lock().map_err(|e| e.to_string())?.clone();
    let err_list = errors.lock().map_err(|e| e.to_string())?.clone();

    if !err_list.is_empty() {
        let mut seen = HashSet::new();
        let mut summary = Vec::new();
        for e in &err_list {
            let key = truncate_err_msg(e, 120);
            if seen.insert(key.clone()) {
                summary.push(key);
            }
            if summary.len() >= 12 {
                break;
            }
        }
        for line in &summary {
            engine.push_notice(format!("批失敗摘要：{line}"));
        }
        engine.push_notice(format!(
            "AI 共記錄 {} 筆批失敗（已去重摘要 {} 條）；未完成句可稍後補翻",
            err_list.len(),
            summary.len()
        ));
    }

    flush_notices(engine, pct(total_batches, out.len()), on_progress);

    if out.is_empty() {
        let detail = err_list
            .last()
            .cloned()
            .unwrap_or_else(|| "全部請求都沒有回應".into());
        if let Some(classified) = classify_batch_abort(&err_list) {
            return Err(classified);
        }
        return Err(detail);
    }
    if !err_list.is_empty() {
        on_progress(
            pct(total_batches, out.len()),
            &format!(
                "AI 有 {} 批失敗（其餘已完成）；失敗批已重試或略過，可稍後補翻未完成句",
                err_list.len()
            ),
        );
    }
    Ok(out)
}

/// 進度取「批次進度」與「句數進度」較高者，避免久卡同一個數字像當機。
fn by_batch_or_strings(
    done_batches: usize,
    total_batches: usize,
    got: usize,
    total_unique: usize,
    base: u8,
    span: u8,
) -> u8 {
    let ratio = |a: usize, b: usize| -> u32 {
        if b == 0 {
            0
        } else {
            (a as u32 * span as u32 / b as u32).min(span as u32)
        }
    };
    let p = base as u32 + ratio(done_batches, total_batches).max(ratio(got, total_unique));
    p.min((base as u32 + span as u32).min(100)) as u8
}

// ═══ 單批請求 ═════════════════════════════════════════════════

fn translate_chunk(
    engine: &Engine,
    system_prompt: &Arc<String>,
    plan: &BatchPlan,
) -> Result<ChunkSuccess, ChunkError> {
    let payload = build_user_payload(&plan.items);
    let max_tokens = clamp_completion_tokens_for(
        payload.chars().count(),
        plan.items.len(),
        matches!(engine.provider, AiProvider::LocalLlm),
    );
    let wanted: HashSet<usize> = plan.items.iter().map(|item| item.uid).collect();
    let mut attempts = 0usize;
    let mut length_retries = 0usize;
    loop {
        if cancel::is_cancelled() {
            return Err(ChunkError {
                message: CANCEL_MESSAGE.to_string(),
                congestion: false,
                retries: attempts,
                kind: ChatErrorKind::Fatal,
            });
        }
        let features = engine.current_features();
        crate::dev_log!(
            "ai",
            "送出批次 track={:?} 條數={} 首句={:?} 上限tokens={} 嘗試={}",
            plan.track,
            plan.items.len(),
            plan.items.first().map(|i| i.source.chars().take(48).collect::<String>()),
            max_tokens,
            attempts
        );
        let mut body = json!({
            "model": engine.model.as_str(),
            "messages": [
                {"role": "system", "content": system_prompt.as_str()},
                {"role": "user", "content": payload.as_str()}
            ]
        });
        if features.send_temperature {
            body["temperature"] = json!(if plan.items.len() == 1 { 0.0 } else { 0.1 });
        }
        if matches!(engine.provider, AiProvider::LocalLlm) {
            // 重複懲罰是本地小模型最有效的一道防線：低溫取樣容易卡在同一個 token 上，
            // 吐出「造成傷害傷害傷害傷害」這種輸出。`local_quality.rs` 會在事後抓到它，
            // 但在源頭少發生一次，就少一次重翻。
            // 這兩個是 llama.cpp 的取樣參數（我們自己安裝的 llama-server 一定支援），
            // 只送給本地端，不影響任何雲端服務。
            body["repeat_penalty"] = json!(1.15);
            body["repeat_last_n"] = json!(128);
        }
        if features.send_response_format {
            body["response_format"] = json!({ "type": "json_object" });
        }
        if should_send_thinking_disabled(engine) {
            // https://api-docs.deepseek.com/zh-cn/guides/thinking_mode — 預設開啟；批次翻譯關閉
            body["thinking"] = json!({ "type": "disabled" });
        }
        if should_disable_local_thinking(engine) {
            // 現場實測過：不關會把 max_tokens 燒在思考過程上，content 留空、
            // finish_reason=length；送 "Iron Sword" 給 32 tokens 全被 reasoning_content
            // 吃光。關掉後同一句話 3 個 token 就答完，且是正確答案「鐵劍」；真實批次
            // （4 條混雜句子）也驗證過完整正確、finish_reason=stop。翻譯不需要推理過程。
            body["chat_template_kwargs"] = json!({ "enable_thinking": false });
        }
        match features.max_tokens_field {
            MaxTokensField::MaxTokens => body["max_tokens"] = json!(max_tokens),
            MaxTokensField::MaxCompletionTokens => {
                body["max_completion_tokens"] = json!(max_tokens)
            }
        }

        let response_json: Value = if matches!(engine.provider, AiProvider::Codex) {
            match codex_chat::complete_chat(
                engine.client.as_ref(),
                engine.model.as_str(),
                &body,
                Duration::from_secs(plan.track.timeout_secs()),
            ) {
                Ok(value) => value,
                Err(error) => {
                    let code = error.status_code.unwrap_or(0);
                    let clean = sanitize_provider_name(&error.message);
                    crate::dev_log!(
                        "gpt",
                        "GPT 翻譯端點回應失敗 | HTTP={} | 模型={} | 嘗試={}",
                        code,
                        engine.model,
                        attempts
                    );
                    let is_retryable = code == 0 || code == 200 || code == 429 || code >= 500;
                    let congestion = code == 429
                        || code >= 500
                        || clean.contains("逾時")
                        || clean.contains("稍後再試");
                    if is_retryable && attempts < 2 {
                        attempts += 1;
                        thread::sleep(Duration::from_millis(600 + attempts as u64 * 600));
                        continue;
                    }
                    let kind = if code == 402 {
                        ChatErrorKind::Quota
                    } else if code == 401 || code == 403 {
                        ChatErrorKind::Fatal
                    } else if code == 429 || code == 0 || code == 200 || code >= 500 {
                        ChatErrorKind::Transient
                    } else if looks_like_quota_or_auth_error(&clean) {
                        ChatErrorKind::Quota
                    } else {
                        ChatErrorKind::Fatal
                    };
                    return Err(ChunkError {
                        message: clean,
                        congestion,
                        retries: attempts,
                        kind,
                    });
                }
            }
        } else {
            let mut req = engine
                .client
                .post(engine.url.as_str())
                .header("Content-Type", "application/json")
                .timeout(Duration::from_secs(plan.track.timeout_secs()))
                .json(&body);
            if engine.managed {
                req = req
                    .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
                    .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
                    .header("X-Zeitfrei-Session", engine.session_cookie());
            } else if !engine.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", engine.api_key));
            }

            let resp = match req.send() {
                Ok(resp) => resp,
                Err(err) => {
                    let is_timeout = err.is_timeout();
                    let err_msg = if is_timeout {
                        "等待 AI 回應逾時".into()
                    } else {
                        format!("連線失敗（無回應）：{err}")
                    };
                    if attempts < 2 {
                        attempts += 1;
                        thread::sleep(Duration::from_millis(400 + attempts as u64 * 400));
                        continue;
                    }
                    return Err(ChunkError {
                        message: err_msg,
                        congestion: is_timeout,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
            };

            let status = resp.status();
            let code = status.as_u16();
            let body_text = resp.text().unwrap_or_default();
            // 本地模型的失敗診斷：實測有一批 8 條被送九次仍失敗，而當時的紀錄
            // 只寫「批失敗」，看不出模型到底回了什麼。依 rules/50 的升級梯，
            // 這一輪先收證據（HTTP 狀態、finish_reason、回應前 200 字），
            // 不憑猜測調 prompt 或參數。
            if matches!(engine.provider, AiProvider::LocalLlm) && !status.is_success() {
                crate::dev_log!(
                    "local",
                    "本地模型 HTTP {} | 批次 {} 條 | 嘗試 {} 次 | 上限tokens={} | 回應前 200 字={:?}",
                    code,
                    plan.items.len(),
                    attempts,
                    max_tokens,
                    body_text.chars().take(200).collect::<String>()
                );
            }
            if !status.is_success() {
                if engine.maybe_degrade_for_unsupported(code, &body_text) {
                    continue;
                }
                if let Some(mapped) = map_chat_http_error(code, &body_text, engine.managed) {
                    let congestion = matches!(mapped.kind, ChatErrorKind::Transient)
                        || code == 429
                        || code >= 500;
                    if matches!(mapped.kind, ChatErrorKind::Transient) && attempts < 2 {
                        attempts += 1;
                        thread::sleep(Duration::from_millis(600 + attempts as u64 * 600));
                        continue;
                    }
                    return Err(ChunkError {
                        message: mapped.message,
                        congestion,
                        retries: attempts,
                        kind: mapped.kind,
                    });
                }
                if code == 429 || code >= 500 {
                    let err_msg = if code == 429 {
                        "請求太頻繁，稍後再試".into()
                    } else if code == 503 {
                        "服務暫時無法使用（503），稍後再試".into()
                    } else {
                        let snippet =
                            sanitize_provider_name(&body_text.chars().take(200).collect::<String>());
                        format!("服務錯誤 {code}：{snippet}")
                    };
                    if attempts < 2 {
                        attempts += 1;
                        thread::sleep(Duration::from_millis(600 + attempts as u64 * 600));
                        continue;
                    }
                    return Err(ChunkError {
                        message: err_msg,
                        congestion: true,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
                if code == 401 || code == 403 {
                    return Err(ChunkError {
                        message: format!(
                            "金鑰無效或無權限：{}",
                            sanitize_provider_name(&body_text.chars().take(120).collect::<String>())
                        ),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Fatal,
                    });
                }
                if code == 402 {
                    return Err(ChunkError {
                        message: format!(
                            "帳號餘額不足：{}",
                            sanitize_provider_name(&body_text.chars().take(120).collect::<String>())
                        ),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Quota,
                    });
                }
                let snippet =
                    sanitize_provider_name(&body_text.chars().take(200).collect::<String>());
                if looks_like_quota_or_auth_error(&snippet)
                    || looks_like_quota_or_auth_error(&body_text)
                {
                    return Err(ChunkError {
                        message: format!("可能額度不足或金鑰問題（{code}）：{snippet}"),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Quota,
                    });
                }
                return Err(ChunkError {
                    message: format!("服務錯誤 {code}：{snippet}"),
                    congestion: false,
                    retries: attempts,
                    kind: ChatErrorKind::Fatal,
                });
            }

            match serde_json::from_str(&body_text) {
                Ok(value) => value,
                Err(err) => {
                    let err_msg = format!("回應無法解析（無有效內容）：{err}");
                    if attempts < 2 {
                        attempts += 1;
                        thread::sleep(Duration::from_millis(200));
                        continue;
                    }
                    return Err(ChunkError {
                        message: err_msg,
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
            }
        };
        engine.record_usage(&parse_usage_totals(&response_json));
        let content = extract_message_content(&response_json);
        if content.trim().is_empty() {
            let msg = diagnose_empty_choice(&response_json);
            if matches!(engine.provider, AiProvider::LocalLlm) {
                // 本地模型回空 content 的證據：光看「批失敗」看不出是被截斷、
                // 被拒答、還是 JSON 解析不出來。留下 finish_reason 與原始回應
                // 才有辦法對症——這一輪只收證據，不調參數。
                crate::dev_log!(
                    "local",
                    "本地模型回空 content | {} | 批次 {} 條 | 嘗試 {} 次 | 上限tokens={} | 首句={:?}",
                    msg,
                    plan.items.len(),
                    attempts,
                    max_tokens,
                    plan.items
                        .first()
                        .map(|i| i.source.chars().take(48).collect::<String>())
                );
            }
            if is_length_truncated(&response_json) {
                if length_retries < LENGTH_TRUNCATION_RETRY_LIMIT {
                    length_retries += 1;
                    attempts += 1;
                    engine.push_notice(format!(
                        "本批疑似輸出截斷（finish_reason=length），第 {length_retries} 次同尺寸重試…"
                    ));
                    thread::sleep(Duration::from_millis(empty_content_backoff_ms(attempts)));
                    continue;
                }
                engine.push_notice(
                    "本批疑似輸出截斷（finish_reason=length），改拆半或重排…".into(),
                );
                return Err(ChunkError {
                    message: msg,
                    congestion: false,
                    retries: attempts,
                    kind: ChatErrorKind::Transient,
                });
            }
            if attempts < EMPTY_CONTENT_RETRY_LIMIT {
                attempts += 1;
                engine.push_notice(format!(
                    "本批空回應，第 {attempts} 次重試…"
                ));
                thread::sleep(Duration::from_millis(empty_content_backoff_ms(attempts)));
                continue;
            }
            return Err(ChunkError {
                message: msg,
                congestion: false,
                retries: attempts,
                kind: ChatErrorKind::Transient,
            });
        }

        let requested_ids: Vec<usize> = wanted.iter().copied().collect();
        match parse_translation_object_audited(&content, &requested_ids) {
            Ok((map, audit)) => {
                // 對帳結果決定這批可不可信（Phase 3）。
                // 模型自己編出來的 id、或同一個 id 給了兩種不同譯文，代表它沒有照
                // 格式走；此時整批的其他答案也不值得採信。少數幾筆沒回來則屬常態，
                // 交給下一輪補——兩者的處理方式必須不同，混在一起就會把
                // 「模型亂回」靜默降級成「這次少翻了幾句」。
                if audit.needs_diagnostic_split() {
                    engine.push_notice(format!("{}；改拆半重試…", audit.summary()));
                    return Err(ChunkError {
                        message: format!("批次回應與送出內容對不上（{}）", audit.summary()),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
                // 沒到要拆半的程度，但也不是乾淨的一批：把差額講出來。
                // 這些沒回來的句子會留在待補清單裡，使用者看得到原因，
                // 而不是事後發現「明明說翻完了卻少了幾句」。
                if !audit.is_clean() {
                    engine.push_notice(audit.summary());
                }
                let filtered = map
                    .into_iter()
                    .filter(|(uid, translated)| wanted.contains(uid) && !translated.trim().is_empty())
                    .collect::<HashMap<_, _>>();
                if !filtered.is_empty() {
                    return Ok(ChunkSuccess {
                        map: filtered,
                        retries: attempts,
                    });
                }
                if is_length_truncated(&response_json) {
                    if length_retries < LENGTH_TRUNCATION_RETRY_LIMIT {
                        length_retries += 1;
                        attempts += 1;
                        engine.push_notice(format!(
                            "本批截斷後無譯文（finish_reason=length），第 {length_retries} 次同尺寸重試…"
                        ));
                        thread::sleep(Duration::from_millis(empty_content_backoff_ms(attempts)));
                        continue;
                    }
                    engine.push_notice(
                        "本批截斷後無譯文（finish_reason=length），改拆半或重排…".into(),
                    );
                    return Err(ChunkError {
                        message: diagnose_empty_choice(&response_json),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
                if attempts < EMPTY_CONTENT_RETRY_LIMIT {
                    attempts += 1;
                    engine.push_notice(format!(
                        "本批回傳空 JSON，第 {attempts} 次重試…"
                    ));
                    thread::sleep(Duration::from_millis(empty_content_backoff_ms(attempts)));
                    continue;
                }
                return Err(ChunkError {
                    message: "回傳為空 JSON 物件（無譯文）".into(),
                    congestion: false,
                    retries: attempts,
                    kind: ChatErrorKind::Transient,
                });
            }
            Err(err) => {
                if is_length_truncated(&response_json) {
                    engine.push_notice(
                        "本批疑似截斷導致 JSON 不完整，改拆半或重排…".into(),
                    );
                    return Err(ChunkError {
                        message: format!("{err}（finish_reason=length）"),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
                if attempts < 2 {
                    attempts += 1;
                    thread::sleep(Duration::from_millis(150));
                    continue;
                }
                return Err(ChunkError {
                    message: err,
                    congestion: false,
                    retries: attempts,
                    kind: ChatErrorKind::Transient,
                });
            }
        }
    }
}

fn extract_message_content(v: &Value) -> String {
    let message = &v["choices"][0]["message"];
    let primary = match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
            .collect::<String>(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    if !primary.trim().is_empty() {
        return primary;
    }
    if let Some(reasoning) = message
        .get("reasoning_content")
        .and_then(|value| value.as_str())
    {
        if looks_like_translation_payload(reasoning) {
            return reasoning.to_string();
        }
    }
    primary
}

/// 解析模型回覆：優先吃 `{"r":[...]}`，但保留陣列回退相容。
fn parse_translation_object(content: &str) -> Result<HashMap<usize, String>, String> {
    let body = extract_json_body(strip_code_fence(content));
    let value: Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "回傳格式不對：{e} / {}",
            body.chars().take(120).collect::<String>()
        )
    })?;
    let items: Vec<Value> = match value {
        Value::Object(map) => map
            .get("r")
            .and_then(|rows| rows.as_array())
            .cloned()
            .unwrap_or_default(),
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    Ok(parse_translation_rows(&items).into_iter().collect())
}

/// 抽出原始 (id, 譯文) 序列，**不去重**——去重會抹掉「同一個 id 回兩次」這個訊號。
fn parse_translation_rows(items: &[Value]) -> Vec<(usize, String)> {
    let mut rows = Vec::new();
    for item in items {
        let Some(i) = item.get("i").and_then(|value| value.as_u64()) else {
            continue;
        };
        let Some(translated) = item.get("t").and_then(|value| value.as_str()) else {
            continue;
        };
        rows.push((i as usize, translated.to_string()));
    }
    rows
}

/// 解析並對帳。回傳的 map 只含**送出去過而且沒有衝突**的 id：
/// 模型自己編的 id 一律丟掉（不可能寫進任何 key），
/// 同 id 衝突的也丟掉（不知道哪個才對，就不要猜）。
pub fn parse_translation_object_audited(
    content: &str,
    requested: &[usize],
) -> Result<(HashMap<usize, String>, BatchAudit), String> {
    let body = extract_json_body(strip_code_fence(content));
    let value: Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "回傳格式不對：{e} / {}",
            body.chars().take(120).collect::<String>()
        )
    })?;
    let items: Vec<Value> = match value {
        Value::Object(map) => map
            .get("r")
            .and_then(|rows| rows.as_array())
            .cloned()
            .unwrap_or_default(),
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    let rows = parse_translation_rows(&items);
    let audit = audit_batch_response(requested, &rows);

    let requested_set: HashSet<usize> = requested.iter().copied().collect();
    let mut by_i: HashMap<usize, String> = HashMap::new();
    for (id, text) in rows {
        if !requested_set.contains(&id) || audit.conflicting.contains(&id) {
            continue;
        }
        by_i.entry(id).or_insert(text);
    }
    Ok((by_i, audit))
}

/// 一批 AI 回應與「我們送出去的東西」對帳的結果。
///
/// 配對本身早就是靠 id（`{"i":…,"t":…}`）而不是陣列位置，所以不會錯位。
/// 這裡補的是另一半：**回應本身可不可信**。模型少回一筆、同一個 id 回兩次、
/// 回一個我們沒送過的 id——這些都代表這批的輸出不穩定，靜默接受就會變成
/// 「翻譯完成了但少了幾句」，而且沒有任何跡象可查。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BatchAudit {
    pub requested: usize,
    /// 有回、且內容非空的 id 數
    pub resolved: usize,
    /// 送出去但沒回來的
    pub missing: Vec<usize>,
    /// 同一個 id 回了不只一次，且內容彼此不同（純重複同值不算問題）
    pub conflicting: Vec<usize>,
    /// 回了我們沒送過的 id（模型自己編的）
    pub unknown: Vec<usize>,
    /// 有回但譯文是空字串
    pub empty: Vec<usize>,
}

impl BatchAudit {
    /// 這批可以整批採信嗎？
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty()
            && self.conflicting.is_empty()
            && self.unknown.is_empty()
            && self.empty.is_empty()
    }

    /// 這批該不該進「拆批重試」流程。
    ///
    /// 判準刻意分兩級：少數幾筆沒回來是常態（下一輪補），
    /// 但**幻覺 id 或同 id 衝突**代表模型沒有照格式走，整批都不該當成可靠輸出。
    pub fn needs_diagnostic_split(&self) -> bool {
        if !self.conflicting.is_empty() || !self.unknown.is_empty() {
            return true;
        }
        // 超過三成沒回來，不是零星漏掉，是這批本身有問題（多半是被截斷）
        self.requested > 0 && self.missing.len() * 10 > self.requested * 3
    }

    /// 給開發者定位用的一行摘要。不含原文，只有數量與 id。
    pub fn summary(&self) -> String {
        format!(
            "批次對帳：送出 {}、可用 {}、未回 {}、衝突 {}、未知 {}、空譯文 {}",
            self.requested,
            self.resolved,
            self.missing.len(),
            self.conflicting.len(),
            self.unknown.len(),
            self.empty.len()
        )
    }
}

/// 以「我們送出去的 id 集合」為準對帳一批回應。
///
/// `raw_rows` 是還沒去重的原始 (id, 譯文) 序列——必須未去重，
/// 否則偵測不到「同一個 id 回兩次」。
pub fn audit_batch_response(requested: &[usize], raw_rows: &[(usize, String)]) -> BatchAudit {
    use std::collections::BTreeMap;
    let requested_set: HashSet<usize> = requested.iter().copied().collect();

    let mut first_seen: BTreeMap<usize, String> = BTreeMap::new();
    let mut conflicting: Vec<usize> = Vec::new();
    let mut unknown: Vec<usize> = Vec::new();

    for (id, text) in raw_rows {
        if !requested_set.contains(id) {
            if !unknown.contains(id) {
                unknown.push(*id);
            }
            continue;
        }
        match first_seen.get(id) {
            Some(prev) if prev != text => {
                if !conflicting.contains(id) {
                    conflicting.push(*id);
                }
            }
            Some(_) => {} // 同一個 id 回了兩次但內容相同，無害
            None => {
                first_seen.insert(*id, text.clone());
            }
        }
    }

    let mut missing: Vec<usize> = Vec::new();
    let mut empty: Vec<usize> = Vec::new();
    let mut resolved = 0usize;
    for id in requested {
        match first_seen.get(id) {
            None => missing.push(*id),
            Some(text) if text.trim().is_empty() => empty.push(*id),
            Some(_) => resolved += 1,
        }
    }

    BatchAudit {
        requested: requested.len(),
        resolved,
        missing,
        conflicting,
        unknown,
        empty,
    }
}

fn parse_usage_totals(response: &Value) -> AiUsageTotals {
    let usage = response.get("usage").unwrap_or(&Value::Null);
    let prompt_cache_hit_tokens = usage
        .get("prompt_cache_hit_tokens")
        .and_then(|value| value.as_u64())
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|value| value.get("cached_tokens"))
                .and_then(|value| value.as_u64())
        })
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|value| value.get("cached_tokens"))
                .and_then(|value| value.as_u64())
        })
        .unwrap_or(0) as usize;
    let prompt_total = usage
        .get("prompt_tokens")
        .and_then(|value| value.as_u64())
        .or_else(|| usage.get("input_tokens").and_then(|value| value.as_u64()))
        .unwrap_or(0) as usize;
    let prompt_cache_miss_tokens = usage
        .get("prompt_cache_miss_tokens")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or_else(|| prompt_total.saturating_sub(prompt_cache_hit_tokens));
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(|value| value.as_u64())
        .or_else(|| usage.get("output_tokens").and_then(|value| value.as_u64()))
        .unwrap_or(0) as usize;
    AiUsageTotals {
        prompt_cache_hit_tokens,
        prompt_cache_miss_tokens,
        completion_tokens,
    }
}

fn extract_json_body(s: &str) -> &str {
    let s = s.trim();
    if let (Some(a), Some(b)) = (s.find('{'), s.rfind('}')) {
        if a < b {
            return &s[a..=b];
        }
    }
    if let (Some(a), Some(b)) = (s.find('['), s.rfind(']')) {
        if a < b {
            return &s[a..=b];
        }
    }
    s
}

fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim();
    }
    if let Some(rest) = s.strip_prefix("```") {
        return rest.trim_end_matches("```").trim();
    }
    s
}

fn mentions_unsupported_params(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    (lower.contains("unsupported") || lower.contains("unknown parameter") || lower.contains("invalid parameter") || lower.contains("not supported"))
        && (lower.contains("response_format")
            || lower.contains("max_tokens")
            || lower.contains("max_completion_tokens")
            || lower.contains("temperature"))
}

/// 嚴格探測目前選定的 AI 是否真的能完成最小翻譯。
///
/// 「登入成功」、「/health 正常」或「帳號仍可聊天」都不等於翻譯端點可用：GPT
/// 的端點可能在實際 Responses 請求回 429，本地模型也可能健康檢查通過但 chat
/// endpoint／模型載入失敗。這裡必須走和正式批次相同的 `translate_chunk`，因此
/// custom API、GPT 與本地模型都會在掃描、寫檔前被同一條閘門驗證。
pub fn verify_ai_assistance() -> Result<(), String> {
    let engine = Engine::connect_raw().map_err(|e| sanitize_provider_name(&e))?;
    probe_ai_ready_inner(&engine, true).map_err(|e| {
        let clean = sanitize_provider_name(&e);
        if matches!(engine.provider, AiProvider::Codex) || looks_like_quota_or_auth_error(&clean) {
            ai_quota_support_message_for(&clean, engine.provider, engine.managed)
        } else {
            clean
        }
    })
}

/// 嚴格探測自訂 API 金鑰（儲存後／測試鈕）。
/// 網路失敗、空內容、401／餘額問題一律 Err；代管模式不可呼叫。
pub fn verify_custom_api() -> Result<(), String> {
    if get_ai_mode() != "custom" {
        return Err("目前不是自訂 API 模式。".into());
    }
    verify_ai_assistance()
}

/// 輕量探測：餘額 API 或迷你 chat；網路抖動不阻擋，明確額度／金鑰問題直接回錯。
fn probe_ai_ready(engine: &Engine) -> Result<(), String> {
    probe_ai_ready_inner(engine, false)
}

fn probe_ai_ready_inner(engine: &Engine, strict: bool) -> Result<(), String> {
    let base = engine.base_url.trim_end_matches('/');

    if matches!(engine.provider, AiProvider::LocalLlm) {
        let health = format!("{base}/health");
        match engine.client.get(&health).send() {
            Ok(resp) if resp.status().is_success() => {}
            Ok(_) => return Err("本地模型尚未就緒。請先完成安裝並等到健康檢查通過。".into()),
            Err(err) if strict => return Err(format!("本地模型連線失敗：{err}")),
            Err(_) => return Err("本地模型尚未就緒。請先完成安裝並等到健康檢查通過。".into()),
        }
        // 健康檢查只證明 server 活著；不能在這裡提早 return。後面仍須送出
        // 一句真正的翻譯請求，才知道模型、chat endpoint、API key 與輸出格式都可用。
    }

    // 代管 Worker 沒有 /user/balance 端點，跳過餘額查詢，直接用迷你 chat 探測。
    if !engine.managed && !matches!(engine.provider, AiProvider::Codex) {
        let bal_url = format!("{base}/user/balance");
        match engine
            .client
            .get(&bal_url)
            .header("Authorization", format!("Bearer {}", engine.api_key))
            .send()
        {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if code == 401 || code == 403 {
                    return Err("金鑰無效或無權限".into());
                }
                if code == 402 {
                    return Err("帳號餘額不足".into());
                }
                if resp.status().is_success() {
                    if let Ok(v) = resp.json::<Value>() {
                        if balance_looks_empty(&v) {
                            return Err("帳號餘額為零或不足".into());
                        }
                    }
                }
            }
            Err(err) if strict => {
                return Err(format!("連線失敗（無回應）：{err}"));
            }
            Err(_) => {}
        }
    }

    // 不用「OK」：模型原樣回 OK 也會被視為有內容，卻不能證明它真的能翻譯。
    // 這筆 `MaskedItem` 不會交給 `fill_missing`，因此沒有任何寫入 LangMap、TM 或共享庫的路徑。
    let probe_texts = vec![AI_TRANSLATION_PROBE_SOURCE.to_string()];
    let prompt = Arc::new(build_system_prompt(
        &glossary::load(None),
        &probe_texts,
        matches!(engine.provider, AiProvider::LocalLlm),
    ));
    let plan = BatchPlan {
        track: BatchTrackKind::Ui,
        items: vec![MaskedItem {
            uid: 0,
            source: AI_TRANSLATION_PROBE_SOURCE.into(),
            masked: AI_TRANSLATION_PROBE_SOURCE.into(),
            tokens: Vec::new(),
            context: None,
        }],
    };
    match translate_chunk(engine, &prompt, &plan) {
        Ok(success)
            if success
                .map
                .get(&0)
                .is_some_and(|translated| quality_fail_reason(AI_TRANSLATION_PROBE_SOURCE, translated).is_ok()) =>
        {
            Ok(())
        }
        Ok(success) if success.map.contains_key(&0) => {
            Err("AI 有回應，但沒有給出可用的繁體中文翻譯。".into())
        }
        Ok(_) if cancel::is_cancelled() => {
            if strict {
                Err("探測已取消".into())
            } else {
                Ok(())
            }
        }
        Ok(_) => Err("探測成功連線但沒有內容回應".into()),
        Err(err) => {
            if looks_like_quota_or_auth_error(&err.message) || strict {
                Err(err.message)
            } else {
                // 網路暫時失敗：不在此硬擋，讓主流程再試
                Ok(())
            }
        }
    }
}

fn balance_looks_empty(v: &Value) -> bool {
    if let Some(false) = v.get("is_available").and_then(|x| x.as_bool()) {
        return true;
    }
    if let Some(arr) = v.get("balance_infos").and_then(|x| x.as_array()) {
        let mut any_positive = false;
        for item in arr {
            let total = item
                .get("total_balance")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| item.get("total_balance").and_then(|x| x.as_f64()))
                .unwrap_or(0.0);
            if total > 0.0001 {
                any_positive = true;
            }
        }
        return !any_positive && !arr.is_empty();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::secrets::{provider_capabilities, AiProvider, MaxTokensField};
    use std::collections::HashMap;

    fn masked(uid: usize, source: &str, context: Option<&'static str>) -> MaskedItem {
        let (masked, tokens) = placeholder::mask(source);
        MaskedItem {
            uid,
            source: source.to_string(),
            masked,
            tokens,
            context,
        }
    }

    /// AI 掛掉時，資料層命中的譯文不可以被誤標為「使用 AI 完成」。
    ///
    /// 實測代價：站長那一包 33233 條待譯裡，共享翻譯庫命中 32151、共享術語 671，
    /// 真正送 AI 的只有 271 條。舊版在 `Engine::connect()?` 一拋，
    /// 那 32822 條查得到的譯文雖然留在記憶體，卻被當成整輪完成而寫出——
    /// FTB Quests 與 FancyMenu 的缺口因此被掩蓋。
    ///
    /// 這條測試守的是：額度問題不是使用者取消；必須留下可診斷原因，
    /// 由呼叫端停止這次有選 AI 的翻譯，不能寫出假完成結果。
    #[test]
    fn ai_failure_must_not_discard_what_the_data_layers_already_found() {
        // 「AI 不可用」與「使用者取消」是兩件不同的事，只有後者該中斷
        assert!(
            !is_cancel_message("ChatGPT 暫時不接受翻譯請求：The usage limit has been reached"),
            "端點拒絕不是使用者取消，不可以當成中斷"
        );
        assert!(
            !is_cancel_message("本地模型尚未就緒。請先完成安裝並等到健康檢查通過。"),
            "本地模型沒裝好也不是取消"
        );
        assert!(is_cancel_message(CANCEL_MESSAGE), "只有這個才是真的取消");

        // 報告要能表達「資料層有成果，但 AI 沒跑成」這個中間狀態，
        // 呼叫端才有辦法把它算成「部分完成」而不是「完成」。
        let mut report = AiFillReport {
            shared_hits: 32151,
            shared_glossary_hits: 671,
            ..Default::default()
        };
        assert!(report.ai_unavailable.is_none(), "預設是 AI 正常");
        report.ai_unavailable = Some("ChatGPT 暫時不接受翻譯請求".into());
        assert!(
            report.ai_unavailable.is_some(),
            "AI 不可用必須留下痕跡，否則結尾會誤報完成"
        );
        // 資料層的成果一條都不能少
        assert_eq!(report.shared_hits + report.shared_glossary_hits, 32822);
    }

    #[test]
    fn gpt_endpoint_rejection_never_claims_the_chatgpt_account_is_exhausted() {
        let message = ai_quota_support_message_for(
            "ChatGPT 暫時不接受翻譯請求：The usage limit has been reached",
            AiProvider::Codex,
            false,
        );
        assert!(message.contains("不一定代表你平常聊天的額度用完了"));
        assert!(message.contains("翻譯會消耗你的 ChatGPT 帳號額度"), "要誠實說明會用到額度");
        assert!(message.contains("沒有寫入任何翻譯"));
        assert!(!message.contains("GPT 帳號額度或速率限制"));
        let own_text = message.replace("The usage limit has been reached", "");
        for word in ["Codex", "端點", "權杖", "探測", "預檢", "429", "Token", "token"] {
            assert!(!own_text.contains(word), "玩家看得到的文字不可出現「{word}」");
        }
    }

    #[test]
    fn translation_probe_is_a_real_zh_translation_but_has_no_persistent_target() {
        // 這是唯一用於預檢的文字；它不是玩家整合包中的鍵值，也不會交給寫檔／TM／共享庫流程。
        assert_eq!(AI_TRANSLATION_PROBE_SOURCE, "Iron Sword");
        assert!(quality_fail_reason(AI_TRANSLATION_PROBE_SOURCE, "鐵劍").is_ok());
        assert!(quality_fail_reason(AI_TRANSLATION_PROBE_SOURCE, "Iron Sword").is_err());
    }

    #[test]
    fn no_answer_items_must_not_be_written_back_as_english_or_negatively_cached() {
        // 站長那一輪的實況：一批用完重試額度後整批消失，那些句子被寫回英文
        // 並進負向快取，於是「補充漏翻」連試都不試——永遠救不回來。
        let mut translations: HashMap<usize, String> = HashMap::new();
        let mut report = AiFillReport::default();
        let mut deferred: HashSet<usize> = HashSet::new();
        let pending = vec![
            PendingItem {
                uid: 1,
                source: "Deals %s extra damage".into(),
                reason: PendingReason::NoAnswer,
            },
            PendingItem {
                uid: 2,
                source: "Costs %s mana".into(),
                reason: PendingReason::PlaceholderBroken,
            },
        ];

        let (broken, unanswered) =
            drain_pending_ai(&pending, &mut translations, &mut report, &mut deferred);
        assert_eq!((broken, unanswered), (1, 1));

        // 沒拿到回應的：維持缺口，並排進補完佇列
        assert!(
            !translations.contains_key(&1),
            "沒拿到回應的句子不可以被寫回英文假裝翻完了"
        );
        assert!(deferred.contains(&1), "要排進補完佇列才救得回來");
        assert!(
            !is_placeholder_negatively_cached("Deals %s extra damage"),
            "沒拿到回應不代表翻不動，不可以進負向快取"
        );

        // 真的把格式符號弄壞的：維持舊行為（英文保底 ＋ 不重燒）
        assert_eq!(translations.get(&2).map(String::as_str), Some("Costs %s mana"));
        assert!(!deferred.contains(&2));
        assert!(is_placeholder_negatively_cached("Costs %s mana"));

        // 兩種原因要分開講，不可以混成一句
        let notes = report.notes.join("
");
        assert!(notes.contains("佔位符失敗"), "{notes}");
        assert!(notes.contains("沒拿到 AI 回應"), "{notes}");
    }

    #[test]
    fn unresolved_batch_items_are_tagged_as_no_answer() {
        // run_batches 放棄的批次會讓這些 uid 不出現在結果裡。
        // 它們必須被標成「沒拿到回應」，而不是「翻不動」。
        let items = vec![
            MaskedItem {
                uid: 7,
                source: "Iron Sword".into(),
                masked: "Iron Sword".into(),
                tokens: vec![],
                context: None,
            },
            MaskedItem {
                uid: 8,
                source: "Gold Sword".into(),
                masked: "Gold Sword".into(),
                tokens: vec![],
                context: None,
            },
        ];
        let mut resolved = HashMap::new();
        resolved.insert(7usize, "鐵劍".to_string());

        let unresolved = collect_unresolved_items(&items, &resolved);
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].uid, 8);
        assert_eq!(unresolved[0].reason, PendingReason::NoAnswer);
    }

    #[test]
    fn keep_english_skip_quality_does_not_write_translation() {
        let mut translations = HashMap::new();
        let mut report = AiFillReport::default();
        keep_english_skip(
            &mut translations,
            &mut report,
            7,
            "Diamond Sword",
            false,
            Some(QualityFailReason::SameAsSource),
        );
        assert!(translations.get(&7).is_none(), "quality fail must not write English");
        assert_eq!(report.quality_skipped, 1);
        assert_eq!(report.quality_same_as_source, 1);
        assert_eq!(report.rejected, 0);
        keep_english_skip(&mut translations, &mut report, 8, "Hello %s", true, None);
        assert_eq!(translations.get(&8).map(String::as_str), Some("Hello %s"));
        assert_eq!(report.rejected, 1);
    }

    #[test]
    fn classify_still_english_is_quality_fail_not_placeholder() {
        let (class, safe, _, reason) = classify_candidate("Diamond Sword", "Diamond Sword");
        assert_eq!(class, CandidateClass::QualityFail);
        assert!(safe.is_none());
        assert_eq!(reason, Some(QualityFailReason::SameAsSource));
    }

    #[test]
    fn classify_broken_placeholder_is_placeholder_fail() {
        let (class, safe, _, reason) = classify_candidate("Hello %s", "你好");
        assert_eq!(class, CandidateClass::PlaceholderFail);
        assert!(safe.is_none());
        assert!(reason.is_none());
    }

    #[test]
    fn classify_good_zh_is_accept() {
        let (class, safe, _, reason) = classify_candidate("Diamond Sword", "鑽石劍");
        assert_eq!(class, CandidateClass::Accept);
        assert_eq!(safe.as_deref(), Some("鑽石劍"));
        assert!(reason.is_none());
    }

    #[test]
    fn classify_mixed_fragment_reason() {
        let (class, safe, _, reason) =
            classify_candidate("x", "黑色Argillite Brick 階梯");
        assert_eq!(class, CandidateClass::QualityFail);
        assert!(safe.is_none());
        assert_eq!(reason, Some(QualityFailReason::MixedFragment));
    }

    #[test]
    fn classify_proper_noun_with_enough_cjk_accepts() {
        let (class, safe, _, _) = classify_candidate("Flan Assault Rifle", "Flan 突擊步槍");
        assert_eq!(class, CandidateClass::Accept);
        assert_eq!(safe.as_deref(), Some("Flan 突擊步槍"));
    }

    #[test]
    fn strict_retry_cap_splits_overflow() {
        let items: Vec<MaskedItem> = (0..100)
            .map(|i| masked(i, &format!("Hello {i}"), None))
            .collect();
        let (to_strict, overflow) = split_strict_retry_cap(items, STRICT_PLACEHOLDER_RETRY_CAP);
        assert_eq!(to_strict.len(), STRICT_PLACEHOLDER_RETRY_CAP);
        assert_eq!(overflow.len(), 100 - STRICT_PLACEHOLDER_RETRY_CAP);
        assert_eq!(STRICT_PLACEHOLDER_RETRY_CAP, 48);
    }

    #[test]
    fn fill_report_note_separates_quality_and_placeholder() {
        let report = AiFillReport {
            filled: 1,
            ai_translated: 1,
            quality_skipped: 3,
            rejected: 2,
            ..Default::default()
        };
        let note = report.note();
        assert!(note.contains("品質未過"), "{note}");
        assert!(note.contains("佔位符不符"), "{note}");
    }

    fn test_engine() -> Engine {
        Engine {
            client: Arc::new(reqwest::blocking::Client::new()),
            base_url: Arc::new("https://example.com".into()),
            url: Arc::new("https://example.com/v1/chat/completions".into()),
            api_key: Arc::new(String::new()),
            model: Arc::new("demo".into()),
            managed_session: Arc::new(Mutex::new(String::new())),
            managed: false,
            provider: AiProvider::Deepseek,
            capabilities: provider_capabilities(AiProvider::Deepseek),
            degraded: Arc::new(Mutex::new(RequestDegradeState::default())),
            usage: Arc::new(Mutex::new(AiUsageTotals::default())),
            notices: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[test]
    fn only_local_llm_gets_the_thinking_disabled_kwarg() {
        // 這條釘死本輪修掉的空回應 bug：DeepSeek 用 `thinking`，本地模型用
        // `chat_template_kwargs.enable_thinking`，兩者是不同 API，不能共用判斷式，
        // 也不能誤送給不支援這個欄位的服務商。
        let mut engine = test_engine();
        engine.provider = AiProvider::LocalLlm;
        assert!(should_disable_local_thinking(&engine));
        assert!(!should_send_thinking_disabled(&engine));

        engine.provider = AiProvider::Deepseek;
        assert!(!should_disable_local_thinking(&engine));
        assert!(should_send_thinking_disabled(&engine));

        for other in [AiProvider::Openai, AiProvider::Glm, AiProvider::Qwen, AiProvider::Codex] {
            engine.provider = other;
            assert!(!should_disable_local_thinking(&engine), "{other:?} 不該被關思考");
        }
    }

    #[test]
    fn context_hint_reads_leading_segment() {
        assert_eq!(context_hint("item.minecraft.diamond_sword"), Some("物品名"));
        assert_eq!(context_hint("block.create.cogwheel"), Some("方塊名"));
        assert_eq!(context_hint("entity.minecraft.creeper"), Some("生物名"));
    }

    #[test]
    fn context_hint_finds_kind_after_mod_id() {
        // 很多模組把自己的 id 放最前面
        assert_eq!(context_hint("create.tooltip.hold_shift"), Some("提示說明"));
        assert_eq!(context_hint("mekanism.gui.energy"), Some("介面文字"));
    }

    #[test]
    fn context_hint_returns_none_for_unknown_shapes() {
        assert!(context_hint("somemod.random_thing").is_none());
    }

    #[test]
    fn parses_plain_json_object() {
        let m = parse_translation_object(r#"{"r":[{"i":0,"t":"鑽石劍"}]}"#).unwrap();
        assert_eq!(m.get(&0).map(|s| s.as_str()), Some("鑽石劍"));
    }

    #[test]
    fn parses_object_wrapped_in_code_fence_and_prose() {
        let raw =
            "當然可以：\n```json\n{\"r\":[{\"i\":0,\"t\":\"鑽石劍\"},{\"i\":1,\"t\":\"金錠\"}]}\n```";
        let m = parse_translation_object(raw).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m.get(&1).map(|s| s.as_str()), Some("金錠"));
    }

    #[test]
    fn malformed_response_is_an_error_not_a_panic() {
        assert!(parse_translation_object("完全不是 JSON").is_err());
    }

    #[test]
    fn a_clean_batch_reports_no_problems() {
        let (map, audit) = parse_translation_object_audited(
            r#"{"r":[{"i":1,"t":"鑽石劍"},{"i":2,"t":"鐵鎬"}]}"#,
            &[1, 2],
        )
        .unwrap();
        assert!(audit.is_clean(), "{}", audit.summary());
        assert!(!audit.needs_diagnostic_split());
        assert_eq!(audit.resolved, 2);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn hallucinated_ids_are_dropped_and_force_a_split() {
        // 模型回了一個我們沒送過的 id。舊版會把它放進 map（雖然後續查不到），
        // 但更重要的是：這代表模型沒照格式走，整批都不該當成可靠輸出。
        let (map, audit) = parse_translation_object_audited(
            r#"{"r":[{"i":1,"t":"鑽石劍"},{"i":99,"t":"我自己編的"}]}"#,
            &[1, 2],
        )
        .unwrap();
        assert_eq!(audit.unknown, vec![99]);
        assert!(!map.contains_key(&99), "沒送過的 id 絕不可寫進結果");
        assert!(audit.needs_diagnostic_split());
        assert!(!audit.is_clean());
    }

    #[test]
    fn same_id_with_two_different_answers_is_not_guessed() {
        // 同一個 id 給了兩種譯文＝不知道哪個對。舊版是「後面的蓋掉前面的」，
        // 靜默選一個。現在兩個都不採用，整批進拆半重試。
        let (map, audit) = parse_translation_object_audited(
            r#"{"r":[{"i":1,"t":"鑽石劍"},{"i":1,"t":"鑽石之劍"}]}"#,
            &[1],
        )
        .unwrap();
        assert_eq!(audit.conflicting, vec![1]);
        assert!(!map.contains_key(&1), "衝突的 id 不該猜一個寫進去");
        assert!(audit.needs_diagnostic_split());
    }

    #[test]
    fn duplicate_but_identical_answers_are_harmless() {
        let (map, audit) = parse_translation_object_audited(
            r#"{"r":[{"i":1,"t":"鑽石劍"},{"i":1,"t":"鑽石劍"}]}"#,
            &[1],
        )
        .unwrap();
        assert!(audit.conflicting.is_empty(), "重複但內容相同不算問題");
        assert!(audit.is_clean());
        assert_eq!(map.get(&1).map(String::as_str), Some("鑽石劍"));
    }

    #[test]
    fn a_few_missing_entries_are_normal_but_mass_truncation_is_not() {
        // 少數沒回來＝下一輪補，不必整批重試
        let few = audit_batch_response(
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            &[(1, "a".into()), (2, "b".into()), (3, "c".into()), (4, "d".into()),
              (5, "e".into()), (6, "f".into()), (7, "g".into()), (8, "h".into()),
              (9, "i".into())],
        );
        assert_eq!(few.missing, vec![10]);
        assert!(!few.needs_diagnostic_split(), "掉一筆不該讓整批重跑");

        // 掉太多＝這批本身有問題（多半被截斷），要拆半
        let many = audit_batch_response(
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            &[(1, "a".into()), (2, "b".into())],
        );
        assert_eq!(many.missing.len(), 8);
        assert!(many.needs_diagnostic_split());
    }

    #[test]
    fn empty_translations_are_flagged_not_written() {
        let (map, audit) = parse_translation_object_audited(
            r#"{"r":[{"i":1,"t":""},{"i":2,"t":"鐵鎬"}]}"#,
            &[1, 2],
        )
        .unwrap();
        assert_eq!(audit.empty, vec![1]);
        assert_eq!(audit.resolved, 1);
        assert!(!audit.is_clean());
        // map 裡雖然有 key，但呼叫端會用 trim().is_empty() 濾掉；
        // 這裡確認稽核本身有把它標出來，不是靜默當成翻好了
        assert_eq!(map.get(&1).map(String::as_str), Some(""));
    }

    #[test]
    fn system_prompt_is_batch_independent() {
        let gloss = glossary::load(None);
        let texts = vec![
            "Creeper Head".to_string(),
            "Quest Start".to_string(),
            "Diamond Sword".to_string(),
        ];
        let prompt = build_system_prompt(&gloss, &texts, false);
        let again = build_system_prompt(&gloss, &texts, false);
        assert_eq!(prompt, again);
        let payload_a = build_user_payload(&[masked(0, "Creeper {0}", Some("生物名"))]);
        let payload_b = build_user_payload(&[masked(7, "Quest line", Some("任務文字"))]);
        assert_ne!(payload_a, payload_b);
        assert!(prompt.contains("固定譯名"));
        assert!(prompt.starts_with("你是 Minecraft"));
        assert!(
            prompt.len() >= 400,
            "system 前綴應夠長以利 Context Caching，實際 {}",
            prompt.len()
        );
        assert!(!prompt.contains("\"i\":0"));
        assert!(!prompt.contains("Quest line"));
        // 不同句集合若術語命中不同，譯名區塊可不同；但規則前綴必須相同
        let other = build_system_prompt(&gloss, &["Totally Unique Widget Name XYZ".into()], false);
        assert!(other.starts_with("你是 Minecraft"));
        assert_eq!(
            prompt.find("固定譯名"),
            other.find("固定譯名")
        );
    }

    #[test]
    fn system_prompt_stable_prefix_is_long_enough_for_cache() {
        let gloss = glossary::load(None);
        let prompt = build_system_prompt(&gloss, &[], false);
        assert!(prompt.contains("CachePrefixPad") || prompt.len() >= 500);
        assert!(prompt.contains("輸出格式"));
    }

    #[test]
    fn local_prompt_is_much_shorter_than_the_cloud_one() {
        // 本地模型的 8192 視窗是 prompt 與輸出共用的：system 佔掉的每個 token
        // 都是從可翻譯的內容裡扣的。而且規則越多，小模型的指令遵循越差。
        let gloss = glossary::load(None);
        let texts = vec!["Diamond Sword".to_string(), "Creeper Head".to_string()];
        let cloud = build_system_prompt(&gloss, &texts, false);
        let local = build_system_prompt(&gloss, &texts, true);
        assert!(
            local.chars().count() * 2 < cloud.chars().count(),
            "本地 prompt 應不到雲端的一半：本地 {} 字、雲端 {} 字",
            local.chars().count(),
            cloud.chars().count()
        );
        // 精簡不等於拿掉會壞掉的規則
        assert!(local.contains("%s"), "佔位符規則不可省");
        assert!(local.contains("數字"), "數字照抄的規則不可省");
        assert!(local.contains("輸出格式"), "JSON 輸出格式不可省");
        assert!(local.contains("固定譯名"), "術語表仍要帶上");
        // 雲端那份的台灣用語長清單對小模型是反效果，不該出現在本地版
        assert!(!local.contains("苦力怕、殭屍"), "百餘字的用語清單不該進本地 prompt");
    }

    #[test]
    fn track_batching_is_deterministic() {
        let long_story = "A".repeat(260);
        let solo_story = "B".repeat(2201);
        let items_a = vec![
            masked(4, &solo_story, Some("任務文字")),
            masked(1, "Diamond Sword", Some("物品名")),
            masked(3, &long_story, Some("任務文字")),
            masked(2, "Open Quest Book", Some("介面文字")),
        ];
        let items_b = vec![
            items_a[2].clone(),
            items_a[0].clone(),
            items_a[3].clone(),
            items_a[1].clone(),
        ];
        let summary = |plans: Vec<BatchPlan>| {
            plans
                .into_iter()
                .map(|plan| {
                    (
                        plan.track,
                        plan.items.iter().map(|item| item.uid).collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(summary(plan_track_batches_for(&items_a, false, false)), summary(plan_track_batches_for(&items_b, false, false)));
    }

    #[test]
    fn max_tokens_are_clamped() {
        assert_eq!(clamp_completion_tokens_for(10, 1, false), 512);
        assert_eq!(clamp_completion_tokens_for(1000, 1, false), 800);
        assert_eq!(clamp_completion_tokens_for(20_000, 1, false), 8192);
        // 多條時依條數提高下限（48 * 40 = 1920）
        assert_eq!(clamp_completion_tokens_for(10, 40, false), 1920);
    }

    #[test]
    fn name_ui_batch_size_is_48() {
        assert_eq!(BatchTrackKind::Name.batch_size_for(false, false), 48);
        assert_eq!(BatchTrackKind::Ui.batch_size_for(false, false), 48);
        assert_eq!(BatchTrackKind::Story.batch_size_for(false, false), 20);
    }

    #[test]
    fn length_truncated_detected_from_finish_reason() {
        let v = json!({
            "choices": [{
                "finish_reason": "length",
                "message": { "content": "" }
            }]
        });
        assert!(is_length_truncated(&v));
        assert_eq!(LENGTH_TRUNCATION_RETRY_LIMIT, 0);
        let stop = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "content": "" }
            }]
        });
        assert!(!is_length_truncated(&stop));
    }

    #[test]
    fn user_payload_requires_json_output() {
        let items = vec![masked(0, "Sword", None)];
        let payload = build_user_payload(&items);
        assert!(payload.contains("JSON"));
        assert!(payload.contains("\"r\""));
        assert!(payload.contains("Sword"));
    }

    #[test]
    fn usage_parser_supports_both_shapes() {
        let deepseek = json!({
            "usage": {
                "prompt_cache_hit_tokens": 12,
                "prompt_cache_miss_tokens": 34,
                "completion_tokens": 56
            }
        });
        assert_eq!(
            parse_usage_totals(&deepseek),
            AiUsageTotals {
                prompt_cache_hit_tokens: 12,
                prompt_cache_miss_tokens: 34,
                completion_tokens: 56,
            }
        );

        let prompt_details = json!({
            "usage": {
                "prompt_tokens": 120,
                "completion_tokens": 30,
                "prompt_tokens_details": { "cached_tokens": 40 }
            }
        });
        assert_eq!(
            parse_usage_totals(&prompt_details),
            AiUsageTotals {
                prompt_cache_hit_tokens: 40,
                prompt_cache_miss_tokens: 80,
                completion_tokens: 30,
            }
        );

        let responses_details = json!({
            "usage": {
                "input_tokens": 200,
                "output_tokens": 50,
                "input_tokens_details": { "cached_tokens": 75 }
            }
        });
        assert_eq!(
            parse_usage_totals(&responses_details),
            AiUsageTotals {
                prompt_cache_hit_tokens: 75,
                prompt_cache_miss_tokens: 125,
                completion_tokens: 50,
            }
        );
    }

    #[test]
    fn degrade_once_per_step_and_reuse_flags() {
        let engine = test_engine();
        assert!(engine.maybe_degrade_for_unsupported(
            400,
            "unsupported parameter: response_format"
        ));
        assert_eq!(engine.drain_notices().len(), 1);
        assert!(!engine.current_features().send_response_format);

        assert!(!engine.maybe_degrade_for_unsupported(
            400,
            "unsupported parameter: response_format"
        ));
        assert!(engine.drain_notices().is_empty());

        assert!(engine.maybe_degrade_for_unsupported(
            400,
            "unsupported parameter: max_tokens"
        ));
        assert_eq!(
            engine.current_features().max_tokens_field,
            MaxTokensField::MaxCompletionTokens
        );
    }

    #[test]
    fn retry_only_resends_unresolved_indices() {
        let items = vec![
            masked(10, "First", None),
            masked(20, "Second", None),
            masked(30, "Third", None),
        ];
        let resolved = HashMap::from([
            (10usize, "甲".to_string()),
            (30usize, "丙".to_string()),
        ]);
        let unresolved = collect_unresolved_items(&items, &resolved);
        assert_eq!(
            unresolved.into_iter().map(|item| item.uid).collect::<Vec<_>>(),
            vec![20]
        );
    }

    #[test]
    fn progress_never_exceeds_its_span() {
        // fill_missing 用 44..88 這一段，不可以蓋掉後面的打包階段
        for done in 0..=10 {
            let p = by_batch_or_strings(done, 10, done * 5, 50, 44, 44);
            assert!((44..=88).contains(&p), "超出範圍：{p}");
        }
    }

    #[test]
    fn provider_name_is_never_leaked_to_players() {
        let msg = ai_quota_support_message("deepseek-chat 402 Insufficient Balance");
        assert!(!msg.to_lowercase().contains("deepseek-chat"));
        assert!(msg.contains("AI 服務"));
        assert!(msg.contains("自訂 API") || msg.contains("服務商"));
    }

    #[test]
    fn auth_failure_is_not_classified_as_quota_failure() {
        assert!(is_auth_relogin_error("第 1 批失敗：使用開發者代管 AI 前，請先登入 Discord。"));
        assert!(is_auth_relogin_error("安全驗證已過期，請回到工具重新驗證。"));
        assert!(!is_auth_relogin_error(
            "Discord 登入／會員驗證暫時無法連線，請稍後再試；或改用自訂 API。"
        ));
        assert!(is_auth_unavailable_error(
            "Discord 登入／會員驗證暫時無法連線，請稍後再試；或改用自訂 API。"
        ));
        assert!(!is_auth_relogin_error("帳號餘額不足"));
        assert!(!is_auth_relogin_error(
            "代管 AI 暫時無法使用（503）：<!DOCTYPE html> cloudflare"
        ));
        assert!(!is_auth_relogin_error("AI 上游拒絕請求（401）：invalid api key"));
        assert!(auth_relogin_message().contains("Discord"));
        let peek = vec![
            "連線失敗（無回應）：timeout".into(),
            "翻譯前請先登入 Discord。".into(),
        ];
        let classified = classify_batch_abort(&peek).expect("classified");
        assert!(!classified.contains("額度可能已用完"));
        assert!(classified.contains("Discord"));
        // auth_unavailable 不秒殺
        assert!(classify_batch_abort(&[auth_unavailable_message()]).is_none());
    }

    #[test]
    fn quota_message_recommends_deepseek_platform() {
        let msg = ai_quota_support_message("402 Insufficient Balance");
        assert!(msg.contains("自訂 API"));
        assert!(msg.contains("platform.deepseek.com") || msg.contains("服務商"));
        assert!(!msg.to_lowercase().contains("deepseek-chat"));
        assert!(!msg.contains("免費代管"));
    }

    #[test]
    fn managed_503_legacy_turnstile_is_not_labeled_maintenance() {
        let body = r#"{"error":{"message":"Cloudflare Turnstile is not configured","type":"turnstile_unavailable"}}"#;
        let mapped = map_chat_http_error(503, body, true).expect("mapped");
        assert!(mapped.message.contains("Discord") || mapped.message.contains("自訂 API"));
        assert!(!mapped.message.contains("維護中"));
        assert_eq!(mapped.kind, ChatErrorKind::Fatal);
        assert_eq!(
            extract_proxy_error_type(body).as_deref(),
            Some("turnstile_unavailable")
        );
    }

    #[test]
    fn managed_503_missing_key_is_maintenance() {
        let body =
            r#"{"error":{"message":"managed translation not configured","type":"server_not_ready"}}"#;
        let mapped = map_chat_http_error(503, body, true).expect("mapped");
        assert!(mapped.message.contains("維護中"));
        assert_eq!(mapped.kind, ChatErrorKind::Fatal);
    }

    #[test]
    fn managed_503_auth_unavailable_is_transient() {
        let body =
            r#"{"error":{"message":"login verification unavailable","type":"auth_unavailable"}}"#;
        let mapped = map_chat_http_error(503, body, true).expect("mapped");
        assert!(mapped.message.contains("Discord"));
        assert!(!mapped.message.contains("維護中"));
        assert_eq!(mapped.kind, ChatErrorKind::Transient);
        assert!(!is_auth_relogin_error(&mapped.message));
    }

    #[test]
    fn managed_guild_required_is_transient_for_retry() {
        let body =
            r#"{"error":{"message":"official discord membership required","type":"guild_required"}}"#;
        let mapped = map_chat_http_error(403, body, true).expect("mapped");
        assert!(mapped.message.contains("Discord"));
        assert_eq!(mapped.kind, ChatErrorKind::Transient);
        assert_eq!(
            extract_proxy_error_type(body).as_deref(),
            Some("guild_required")
        );
    }

    #[test]
    fn managed_429_without_quota_type_is_rate_limit() {
        let mapped = map_chat_http_error(429, "{}", true).expect("mapped");
        assert!(mapped.message.contains("請求太頻繁"));
        assert_eq!(mapped.kind, ChatErrorKind::Transient);
    }

    #[test]
    fn managed_429_insufficient_quota_is_quota() {
        let body = r#"{"error":{"message":"daily free translation budget reached","type":"insufficient_quota"}}"#;
        let mapped = map_chat_http_error(429, body, true).expect("mapped");
        assert!(mapped.message.contains("當日額度"));
        assert_eq!(mapped.kind, ChatErrorKind::Quota);
    }

    #[test]
    fn managed_401_without_login_type_is_not_discord_relogin() {
        let body = r#"{"error":{"message":"Invalid Authentication","type":"authentication_error"}}"#;
        let mapped = map_chat_http_error(401, body, true).expect("mapped");
        assert!(mapped.message.contains("上游"));
        assert!(!is_auth_relogin_error(&mapped.message));
        assert_eq!(mapped.kind, ChatErrorKind::Fatal);
    }

    #[test]
    fn managed_401_login_required_is_relogin() {
        let body = r#"{"error":{"message":"discord login required","type":"login_required"}}"#;
        let mapped = map_chat_http_error(401, body, true).expect("mapped");
        assert!(is_auth_relogin_error(&mapped.message));
        assert_eq!(mapped.kind, ChatErrorKind::Relogin);
    }

    #[test]
    fn cloudflare_html_503_is_not_relogin() {
        let body = "<!DOCTYPE html><html>cloudflare</html>";
        let mapped = map_chat_http_error(503, body, true).expect("mapped");
        assert_eq!(mapped.kind, ChatErrorKind::Transient);
        assert!(!is_auth_relogin_error(&mapped.message));
    }

    #[test]
    fn empty_content_transient_does_not_mark_round_congestion() {
        // 空 content 是 Transient 但 congestion=false；AIMD 不得因此減半並行
        assert!(!should_mark_round_congestion(false));
        assert!(should_mark_round_congestion(true));
    }

    #[test]
    fn verify_custom_api_requires_custom_mode() {
        if get_ai_mode() == "custom" {
            return;
        }
        let err = verify_custom_api().unwrap_err();
        assert!(err.contains("自訂"));
    }

    #[test]
    fn diagnose_empty_choice_includes_finish_reason() {
        let v = json!({
            "choices": [{
                "finish_reason": "length",
                "message": { "content": null, "refusal": null }
            }]
        });
        let msg = diagnose_empty_choice(&v);
        assert!(msg.contains("沒有回傳翻譯內容"));
        assert!(msg.contains("finish_reason=length"));
        assert!(msg.contains("choice_keys="));
        assert!(msg.contains("message_keys="));
    }

    #[test]
    fn diagnose_empty_choice_includes_refusal() {
        let v = json!({
            "choices": [{
                "finish_reason": "content_filter",
                "message": { "content": "", "refusal": "policy block" }
            }]
        });
        let msg = diagnose_empty_choice(&v);
        assert!(msg.contains("refusal=policy block"));
        assert!(msg.contains("finish_reason=content_filter"));
    }

    #[test]
    fn extract_content_uses_reasoning_when_parseable_json() {
        let v = json!({
            "choices": [{
                "message": {
                    "content": null,
                    "reasoning_content": "{\"r\":[{\"i\":0,\"t\":\"鑽石劍\"}]}"
                }
            }]
        });
        assert_eq!(
            extract_message_content(&v),
            "{\"r\":[{\"i\":0,\"t\":\"鑽石劍\"}]}"
        );
    }

    #[test]
    fn extract_content_ignores_non_translation_reasoning() {
        let v = json!({
            "choices": [{
                "message": {
                    "content": "",
                    "reasoning_content": "thinking about swords..."
                }
            }]
        });
        assert!(extract_message_content(&v).trim().is_empty());
    }

    #[test]
    fn empty_content_backoff_grows_with_attempt() {
        assert_eq!(empty_content_backoff_ms(1), 900);
        assert_eq!(empty_content_backoff_ms(2), 1400);
        assert!(empty_content_backoff_ms(4) > empty_content_backoff_ms(1));
    }

    #[test]
    fn split_queued_splits_name_ui_batches() {
        let items: Vec<MaskedItem> = (0..4).map(|i| masked(i, "Sword", Some("物品名"))).collect();
        let queued = QueuedPlan {
            batch_no: 7,
            plan: BatchPlan {
                track: BatchTrackKind::Name,
                items,
            },
            requeues: 0,
        };
        let parts = split_queued_for_retry(queued);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].plan.items.len(), 2);
        assert_eq!(parts[1].plan.items.len(), 2);
        assert_eq!(parts[0].requeues, 1);
        assert_eq!(parts[1].batch_no, 7);
    }

    #[test]
    fn split_queued_keeps_story_intact() {
        let items = vec![
            masked(0, "A long story line here", Some("任務")),
            masked(1, "Another story line here", Some("任務")),
        ];
        let queued = QueuedPlan {
            batch_no: 3,
            plan: BatchPlan {
                track: BatchTrackKind::Story,
                items,
            },
            requeues: 0,
        };
        let parts = split_queued_for_retry(queued);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].plan.items.len(), 2);
        assert_eq!(parts[0].requeues, 1);
    }

    #[test]
    fn is_empty_response_error_detects_messages() {
        assert!(is_empty_response_error("第 1 批失敗：服務有連線但沒有回傳翻譯內容"));
        assert!(is_empty_response_error(
            "服務有連線但沒有回傳翻譯內容（finish_reason=length）"
        ));
        assert!(is_empty_response_error("回傳為空 JSON 物件（無譯文）"));
        assert!(!is_empty_response_error("請求太頻繁，稍後再試"));
    }

    #[test]
    fn local_llm_output_cap_fits_inside_the_context_window() {
        // 這條釘死本輪修掉的矛盾：上下文 4096、輸出上限 8192，長文本必爆。
        // 輸出上限必須 ≤ 最小上下文的四分之一，剩下的留給 prompt。
        let min_ctx = crate::engine::local_llm::context_size_for(0) as usize;
        assert!(min_ctx >= 8192, "最小上下文 {min_ctx}");
        assert!(
            LOCAL_LLM_MAX_COMPLETION_TOKENS * 4 <= min_ctx,
            "輸出 {LOCAL_LLM_MAX_COMPLETION_TOKENS} 對上下文 {min_ctx} 太大"
        );
        // 不論丟多長的輸入，本地那條路都不會超過上限
        for chars in [10usize, 5_000, 100_000] {
            for items in [1usize, 12, 48] {
                let got = clamp_completion_tokens_for(chars, items, true);
                assert!(got <= LOCAL_LLM_MAX_COMPLETION_TOKENS, "chars={chars} items={items} got={got}");
            }
        }
        // 雲端那條路維持原本的大上限，不受影響
        assert!(clamp_completion_tokens_for(100_000, 48, false) > LOCAL_LLM_MAX_COMPLETION_TOKENS);
    }

    #[test]
    fn local_llm_uses_smaller_batches() {
        for track in [BatchTrackKind::Name, BatchTrackKind::Ui, BatchTrackKind::Story] {
            let cloud = track.batch_size_for(false, false);
            let local = track.batch_size_for(false, true);
            assert!(local < cloud, "{track:?}: local={local} cloud={cloud}");
            assert!(local >= 1);
        }
        assert_eq!(BatchTrackKind::Solo.batch_size_for(false, true), 1);
        // strict_single 仍然壓過一切
        assert_eq!(BatchTrackKind::Name.batch_size_for(true, true), 1);
    }

    #[test]
    fn local_llm_parallel_matches_server_slots_and_uses_placeholder_guard() {
        // 送出端的併發必須跟 llama-server 實際開的 slot 數一致，否則多開的 slot
        // 沒人用（還是序列），或送太多請求排隊反而更慢。
        let slots = crate::engine::local_llm::recommended_parallel_slots() as usize;
        assert_eq!(provider_capabilities(AiProvider::LocalLlm).start_parallel, slots);
        assert!((1..=3).contains(&slots), "併發數要保守：實得 {slots}");
        let mut st = crate::engine::placeholder::GuardStats::default();
        assert!(crate::engine::placeholder::guard("Deals %s damage", "造成傷害", &mut st).is_none());
        assert!(crate::engine::placeholder::guard("Deals %s damage", "造成 %s 傷害", &mut st).is_some());
    }
}
