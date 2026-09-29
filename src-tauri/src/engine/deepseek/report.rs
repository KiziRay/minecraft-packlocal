//! 翻譯結果報告、候選分類與負向快取。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AiUsageTotals {
    pub prompt_cache_hit_tokens: usize,
    pub prompt_cache_miss_tokens: usize,
    pub completion_tokens: usize,
}

impl AiUsageTotals {
    pub(super) fn add(&mut self, other: &Self) {
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

    pub(super) fn inline_note(&self) -> String {
        format!(
            "用量命中 {}／未命中 {}／輸出 {}",
            self.prompt_cache_hit_tokens, self.prompt_cache_miss_tokens, self.completion_tokens
        )
    }
}

pub(super) fn placeholder_negative_cache() -> &'static Mutex<HashSet<String>> {
    static CACHE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn is_placeholder_negatively_cached(source: &str) -> bool {
    retry_policy::lock_or_recover(placeholder_negative_cache()).contains(source)
}

pub(super) fn remember_placeholder_rejection(source: &str) {
    retry_policy::lock_or_recover(placeholder_negative_cache()).insert(source.to_string());
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
    /// B4：AI 沒回應（連線中斷、批次放棄、回應漏掉這一條、AI 中途停下）的項目。
    /// 跟「品質沒過」分開：這些不是翻不好，是還沒翻到——補充漏翻／接續補完一定會再送。
    pub no_answer: LangMap,
    /// B4：這一輪是使用者按停止才結束的（`ai_unavailable` 同時會有值）
    pub stopped_by_user: bool,
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
        let no_answer = self.no_answer.values().map(|map| map.len()).sum::<usize>();
        if no_answer > 0 {
            parts.push(format!("AI 沒回應 {no_answer} 條（不是翻不好，接續補完會再翻）"));
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
pub(super) enum CandidateClass {
    Accept,
    QualityFail,
    PlaceholderFail,
}

/// 非本地模型的分類（測試用的簡短寫法）。
#[cfg(test)]
pub(super) fn classify_candidate(
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
pub(super) fn ai_preflight() -> Result<Engine, String> {
    // 測試一律注入假引擎（假伺服器），絕不碰真實登入狀態與真實 AI
    #[cfg(test)]
    if let Some(result) = test_hooks::preflight() {
        return result;
    }
    super::super::discord_auth::require_discord_guild_for_ai()?;
    if get_ai_mode() == "local" {
        if !crate::engine::local_llm::is_installed() {
            return Err("尚未安裝本地模型。請先同意並完成偵測／下載。".into());
        }
        crate::engine::local_llm::ensure_ready_for_translate(None)?;
    }
    Engine::connect()
}

pub(super) fn classify_candidate_for(
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
        // B4 #7：雲端譯文也要做截斷檢查——被截到只剩開頭幾個字（或根本沒有中文）就不算翻好。
        // 只看「截斷」這一種：其他本地模型專屬的退化雲端大模型不會犯。
        if cloud_looks_truncated(source, &safe) {
            crate::dev_log!(
                "quality",
                "雲端譯文疑似截斷，退回並排進補完 | 原文長 {} / 譯文長 {}",
                source.chars().count(),
                safe.chars().count()
            );
            return ((CandidateClass::QualityFail, None, stats, None), None);
        }
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

/// B4：雲端譯文的截斷檢查。
///
/// 兩種情況算截斷：
/// - 長句（原文 ≥ 40 字元）的譯文不到原文長度的一成——中文譯文通常是英文字元數的三到五成，
///   不到一成等於只剩開頭幾個字（輸出上限被吃光、串流中斷時會這樣）；
/// - 本地模型那套「話沒說完」判斷（括號沒收、以「的／把／在」等懸空字結尾）而且資訊已經不夠用。
pub(super) fn cloud_looks_truncated(source: &str, text: &str) -> bool {
    let src = source.chars().count();
    let out = text.chars().count();
    if src >= 40 && out * 10 < src {
        return true;
    }
    let check = local_quality::check(source, text);
    check.issue == Some(local_quality::LocalIssue::Truncated)
        && local_quality::LocalIssue::Truncated.blocks_output_for(source, text)
}

/// 累計本地退化細因，供最終報告告訴使用者「模型在哪裡出了什麼問題」。
pub(super) fn record_local_issue(report: &mut AiFillReport, issue: Option<local_quality::LocalIssue>) {
    let Some(issue) = issue else { return };
    let key = issue.label_zh();
    match report.local_degenerate.iter_mut().find(|(k, _)| *k == key) {
        Some((_, count)) => *count += 1,
        None => report.local_degenerate.push((key, 1)),
    }
}

pub(super) fn record_quality_fail(report: &mut AiFillReport, reason: Option<QualityFailReason>) {
    match reason {
        Some(QualityFailReason::StillEnglish) => report.quality_still_english += 1,
        Some(QualityFailReason::MixedFragment) => report.quality_mixed_fragment += 1,
        Some(QualityFailReason::SameAsSource) => report.quality_same_as_source += 1,
        None => {}
    }
}

/// 嚴格重試清單切成「可送 AI」與「超過上限略過」。
pub(super) fn split_strict_retry_cap(
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
pub(super) fn keep_english_skip(
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
