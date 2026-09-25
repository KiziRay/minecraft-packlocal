//! 每一筆候選的可追溯帳本：`Candidate → Eligibility → Resolution → Outcome`。
//!
//! 既有的 `CoverageLedger` 是**階段**計數（掃描／翻譯／寫出各成功幾件），
//! 回答得了「哪個階段整段失敗」，回答不了「這一句為什麼沒翻」。
//! 沒有 per-candidate 帳本，以下全部做不到：
//! completeness 對帳、產出後反向掃描、漏翻逐筆定位、review queue、翻譯明細。
//!
//! ## 兩條互不覆蓋的軸（總規劃 §5.7）
//!
//! `translation_outcome` 決定本次結果與補翻資格；
//! `shared_sync_state` 只決定「有沒有分享給未來的其他整合包」。
//!
//! **共享同步失敗不是翻譯失敗。** 使用者沒同意共享、離線、Worker 掛掉、
//! 佇列還沒送出——這些都不該讓一句「已經安全寫進本機結果」的譯文
//! 變回「待補翻」，更不該因此重新送 AI 再花一次錢。
//! 舊版沒有這條軸，所以「未納入共享庫」會被誤算成「未翻譯」。

use serde::{Deserialize, Serialize};

use super::eligibility::Eligibility;

/// 本次翻譯的最終結果。這是**唯一**決定覆蓋率與補翻資格的東西。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationOutcome {
    /// 這次新翻並已安全寫入
    Written,
    /// 沿用既有繁中（原生或參考包），不需重翻
    ReusedExistingZh,
    /// 刻意保留原文。**不是缺口**——語言代號、資源 id、單位符號等。
    IntentionallyUnchanged,
    /// 需要人工確認，不確定的東西不擅自翻
    ReviewNeeded,
    /// 可重試（配額用盡、網路中斷、供應者暫時失敗）
    RetryableFailure,
    /// 品質檢查未過，已保留原文
    Rejected,
    /// 明確失敗且不可重試（解析失敗、簽章 JAR 無法寫回）
    Failed,
}

impl TranslationOutcome {
    /// 這一筆算不算「還沒翻好」。
    ///
    /// `IntentionallyUnchanged` 刻意不算——把它算進去就是製造假缺口，
    /// 使用者會以為工具漏翻了 900 條，其實那些翻了反而會壞。
    pub fn counts_as_gap(self) -> bool {
        matches!(
            self,
            TranslationOutcome::ReviewNeeded
                | TranslationOutcome::RetryableFailure
                | TranslationOutcome::Rejected
                | TranslationOutcome::Failed
        )
    }

    /// 「補充漏翻」該不該處理這一筆。
    ///
    /// 只有真正可以靠再跑一次解決的才算：待人工確認、可重試失敗。
    /// 品質拒絕與明確失敗要人介入，重跑只會再拒絕一次、再花一次錢。
    pub fn eligible_for_supplement(self) -> bool {
        matches!(
            self,
            TranslationOutcome::ReviewNeeded | TranslationOutcome::RetryableFailure
        )
    }

    /// 這一筆有沒有可用的繁中內容（決定「可套用」）。
    pub fn has_usable_translation(self) -> bool {
        matches!(
            self,
            TranslationOutcome::Written | TranslationOutcome::ReusedExistingZh
        )
    }

    pub fn player_label(self) -> &'static str {
        match self {
            TranslationOutcome::Written => "本次新翻",
            TranslationOutcome::ReusedExistingZh => "沿用既有繁中",
            TranslationOutcome::IntentionallyUnchanged => "刻意保留原文",
            TranslationOutcome::ReviewNeeded => "待人工確認",
            TranslationOutcome::RetryableFailure => "可重試",
            TranslationOutcome::Rejected => "品質未過，已保留原文",
            TranslationOutcome::Failed => "失敗",
        }
    }
}

/// 有沒有分享給未來的其他整合包。**完全不影響本次結果。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedSyncState {
    /// 沒有要送（例如刻意保留的項目本來就不該進共享庫）
    NotRequested,
    /// 使用者關閉共享
    OptedOut,
    /// 已排入本機佇列，等待上傳
    Queued,
    Uploading,
    Synced,
    /// 上傳失敗，之後會再試
    RetryableSyncFailure,
    /// 共享庫規則拒收
    RejectedByLibrary,
    /// 舊 session 沒有這個欄位。**不可倒推為未翻譯。**
    UnknownLegacy,
}

impl SharedSyncState {
    pub fn player_label(self) -> &'static str {
        match self {
            SharedSyncState::NotRequested => "不需同步",
            SharedSyncState::OptedOut => "你已關閉共享",
            SharedSyncState::Queued => "待同步",
            SharedSyncState::Uploading => "同步中",
            SharedSyncState::Synced => "已同步",
            SharedSyncState::RetryableSyncFailure => "同步失敗，稍後重試",
            SharedSyncState::RejectedByLibrary => "共享庫未收錄",
            SharedSyncState::UnknownLegacy => "舊紀錄，同步狀態不明",
        }
    }
}

/// 譯文從哪裡來。用於報告與品質追查。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionSource {
    Glossary,
    LocalTm,
    SharedTm,
    Provider,
    /// 保留原文
    Original,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateOutcome {
    pub candidate_id: String,
    pub source_kind: String,
    pub namespace: String,
    pub logical_key: String,
    pub outcome: TranslationOutcome,
    /// 給玩家看的原因（保留、待確認、失敗時必填）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ResolutionSource>,
    pub shared_sync: SharedSyncState,
}

impl CandidateOutcome {
    pub fn new(
        namespace: &str,
        logical_key: &str,
        source_kind: &str,
        outcome: TranslationOutcome,
    ) -> Self {
        Self {
            candidate_id: candidate_id(namespace, logical_key),
            source_kind: source_kind.to_string(),
            namespace: namespace.to_string(),
            logical_key: logical_key.to_string(),
            outcome,
            reason: None,
            resolution: None,
            // 預設不需同步；真的要送才由共享流程改寫。
            shared_sync: SharedSyncState::NotRequested,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_resolution(mut self, source: ResolutionSource) -> Self {
        self.resolution = Some(source);
        self
    }

    /// 只改同步軸。**刻意不提供任何會連帶改到 outcome 的 API**——
    /// 兩條軸互相污染就是這個模組要防的事。
    pub fn set_shared_sync(&mut self, state: SharedSyncState) {
        self.shared_sync = state;
    }
}

/// `(namespace, key)` 的穩定識別。不含語意，可安全寫進日誌與回報。
pub fn candidate_id(namespace: &str, logical_key: &str) -> String {
    let raw = format!("{}\u{0}{}", namespace.trim(), logical_key.trim());
    super::hashutil::sha256_hex(raw.as_bytes())[..16].to_string()
}

/// 由 eligibility 判定直接得出 outcome。
///
/// 這確保「不該翻」與「不確定」在帳本裡是**兩種不同的終局**，
/// 而不是都被塞進 pending。
pub fn outcome_for_eligibility(verdict: &Eligibility) -> TranslationOutcome {
    match verdict {
        Eligibility::Translate => TranslationOutcome::RetryableFailure, // 尚未解析，先記為可重試
        Eligibility::Keep(_) => TranslationOutcome::IntentionallyUnchanged,
        Eligibility::Review(_) => TranslationOutcome::ReviewNeeded,
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeTotals {
    pub written: usize,
    pub reused_existing_zh: usize,
    pub intentionally_unchanged: usize,
    pub review_needed: usize,
    pub retryable_failure: usize,
    pub rejected: usize,
    pub failed: usize,
}

impl OutcomeTotals {
    pub fn sum(&self) -> usize {
        self.written
            + self.reused_existing_zh
            + self.intentionally_unchanged
            + self.review_needed
            + self.retryable_failure
            + self.rejected
            + self.failed
    }
}

/// 對帳結果。`balanced == false` 時 run **不得**顯示 completed。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reconciliation {
    pub total_candidates: usize,
    pub totals: OutcomeTotals,
    pub balanced: bool,
    /// 可套用（已有繁中）
    pub usable: usize,
    /// 真正的缺口（不含刻意保留）
    pub gaps: usize,
    /// 補翻該處理的筆數
    pub supplement_eligible: usize,
}

impl Reconciliation {
    /// 給玩家看的一段話。刻意把「保留」與「缺口」分開講。
    pub fn player_summary(&self) -> String {
        format!(
            "候選 {} 筆：可套用 {}（新翻 {}、沿用既有 {}）；\
             刻意保留 {}（翻了反而會出錯，不算漏翻）；\
             仍有缺口 {}（待人工 {}、可重試 {}、品質未過 {}、失敗 {}）",
            self.total_candidates,
            self.usable,
            self.totals.written,
            self.totals.reused_existing_zh,
            self.totals.intentionally_unchanged,
            self.gaps,
            self.totals.review_needed,
            self.totals.retryable_failure,
            self.totals.rejected,
            self.totals.failed
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct OutcomeLedger {
    entries: Vec<CandidateOutcome>,
}

impl OutcomeLedger {
    pub fn record(&mut self, entry: CandidateOutcome) {
        self.entries.push(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 補翻該處理的候選。**只看 translation_outcome，完全不看 shared_sync。**
    ///
    /// 這是總規劃 §5.7 的硬規則：共享同步失敗不得觸發重新翻譯或重新送 AI。
    pub fn supplement_targets(&self) -> Vec<&CandidateOutcome> {
        self.entries
            .iter()
            .filter(|e| e.outcome.eligible_for_supplement())
            .collect()
    }

    /// 待人工檢視的佇列（review queue 的資料來源）。
    pub fn review_queue(&self) -> Vec<&CandidateOutcome> {
        self.entries
            .iter()
            .filter(|e| e.outcome == TranslationOutcome::ReviewNeeded)
            .collect()
    }

    /// 尚未同步給共享庫的筆數。**與缺口是兩回事**，報告要分開列。
    pub fn pending_sync(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| {
                matches!(
                    e.shared_sync,
                    SharedSyncState::Queued
                        | SharedSyncState::Uploading
                        | SharedSyncState::RetryableSyncFailure
                )
            })
            .count()
    }

    pub fn reconcile(&self) -> Reconciliation {
        let mut totals = OutcomeTotals::default();
        for e in &self.entries {
            match e.outcome {
                TranslationOutcome::Written => totals.written += 1,
                TranslationOutcome::ReusedExistingZh => totals.reused_existing_zh += 1,
                TranslationOutcome::IntentionallyUnchanged => totals.intentionally_unchanged += 1,
                TranslationOutcome::ReviewNeeded => totals.review_needed += 1,
                TranslationOutcome::RetryableFailure => totals.retryable_failure += 1,
                TranslationOutcome::Rejected => totals.rejected += 1,
                TranslationOutcome::Failed => totals.failed += 1,
            }
        }
        let total_candidates = self.entries.len();
        Reconciliation {
            balanced: totals.sum() == total_candidates,
            usable: self.entries.iter().filter(|e| e.outcome.has_usable_translation()).count(),
            gaps: self.entries.iter().filter(|e| e.outcome.counts_as_gap()).count(),
            supplement_eligible: self
                .entries
                .iter()
                .filter(|e| e.outcome.eligible_for_supplement())
                .count(),
            totals,
            total_candidates,
        }
    }

    /// 標記哪些項目要送共享庫。
    ///
    /// 只有「本次真的新翻出來」的才需要分享；沿用既有繁中不是我們的產出，
    /// 刻意保留的更不該進共享庫（那正是 P0-03 的污染來源）。
    /// **這個方法只寫 `shared_sync`，一個字都不碰 `outcome`。**
    pub fn mark_sharing(&mut self, sharing_enabled: bool) {
        for e in self.entries.iter_mut() {
            if e.outcome != TranslationOutcome::Written {
                continue;
            }
            e.set_shared_sync(if sharing_enabled {
                SharedSyncState::Queued
            } else {
                SharedSyncState::OptedOut
            });
        }
    }

    /// 人類可讀的明細報告（總規劃 §5.3）。
    ///
    /// 玩家要能回答三件事：這次翻了什麼、哪些沒翻、為什麼。
    /// 共享同步另列一段，**刻意與缺口分開**，不讓人誤以為待同步＝待補翻。
    pub fn to_player_report(&self) -> String {
        let r = self.reconcile();
        let mut out = String::from("【翻譯結果明細】\n\n");
        out.push_str(&r.player_summary());
        out.push_str("\n\n");

        if self.is_empty() {
            out.push_str("（這次沒有任何候選項目）\n");
            return out;
        }

        // 各終局的數量，用玩家看得懂的說法
        out.push_str("── 逐項結果 ──\n");
        for (outcome, count) in [
            (TranslationOutcome::Written, r.totals.written),
            (TranslationOutcome::ReusedExistingZh, r.totals.reused_existing_zh),
            (TranslationOutcome::IntentionallyUnchanged, r.totals.intentionally_unchanged),
            (TranslationOutcome::ReviewNeeded, r.totals.review_needed),
            (TranslationOutcome::RetryableFailure, r.totals.retryable_failure),
            (TranslationOutcome::Rejected, r.totals.rejected),
            (TranslationOutcome::Failed, r.totals.failed),
        ] {
            if count > 0 {
                out.push_str(&format!("  {}：{} 項\n", outcome.player_label(), count));
            }
        }

        // 共享同步：獨立一段，數字與缺口無關
        let pending_sync = self.pending_sync();
        out.push_str("\n── 共享庫同步（與本次翻譯結果無關）──\n");
        if pending_sync == 0 {
            out.push_str("  沒有待同步項目。\n");
        } else {
            out.push_str(&format!(
                "  待同步 {pending_sync} 項。這些譯文**已經寫進你的結果**，\n  \
                 同步只影響未來其他整合包能不能重用，不影響這次的遊戲。\n"
            ));
        }
        let states = [
            SharedSyncState::Queued,
            SharedSyncState::Synced,
            SharedSyncState::OptedOut,
            SharedSyncState::RetryableSyncFailure,
            SharedSyncState::UnknownLegacy,
        ];
        for state in states {
            let n = self.entries.iter().filter(|e| e.shared_sync == state).count();
            if n > 0 {
                out.push_str(&format!("  {}：{} 項\n", state.player_label(), n));
            }
        }

        // 補翻與待人工：說清楚「按補充漏翻會處理哪些」
        out.push_str("\n── 下一步 ──\n");
        let targets = self.supplement_targets();
        let review = self.review_queue();
        if targets.is_empty() {
            out.push_str("  沒有可以靠再跑一次解決的項目。\n");
        } else {
            out.push_str(&format!(
                "  按「補充漏翻」會處理 {} 項（其中 {} 項需要你確認）。\n",
                targets.len(),
                review.len()
            ));
        }
        if r.totals.rejected > 0 || r.totals.failed > 0 {
            out.push_str(&format!(
                "  另有 {} 項品質未過或失敗，重跑不會改變結果，需要人工檢視。\n",
                r.totals.rejected + r.totals.failed
            ));
        }
        out.push_str(&format!("\n（共 {} 筆候選，明細見同目錄的 .jsonl）\n", self.len()));
        out
    }

    /// 序列化成 JSONL（一行一筆），供翻譯明細與診斷使用。
    ///
    /// 用 JSONL 而非單一 JSON 陣列：三萬多筆時可以逐行串流讀取，
    /// 而且單行壞掉不會讓整份帳本無法解析。
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            if let Ok(line) = serde_json::to_string(e) {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::eligibility::{classify, Candidate};

    fn entry(key: &str, outcome: TranslationOutcome) -> CandidateOutcome {
        CandidateOutcome::new("examplemod", key, "lang", outcome)
    }

    #[test]
    fn shared_sync_failure_never_turns_a_written_line_back_into_a_gap() {
        // 總規劃 §5.7 的核心：共享同步失敗不是翻譯失敗。
        // 舊版沒有這條軸，「未納入共享庫」會被誤算成「未翻譯」，
        // 於是使用者會被要求補翻一堆其實已經翻好的東西，再花一次 AI 的錢。
        let mut ledger = OutcomeLedger::default();
        for i in 0..100 {
            let mut e = entry(&format!("item.k{i}"), TranslationOutcome::Written);
            e.set_shared_sync(SharedSyncState::RetryableSyncFailure);
            ledger.record(e);
        }

        let r = ledger.reconcile();
        assert_eq!(r.usable, 100, "已寫入的譯文不因同步失敗而消失");
        assert_eq!(r.gaps, 0, "同步失敗不得算成缺口");
        assert_eq!(r.supplement_eligible, 0, "不得因同步失敗而觸發補翻");
        assert!(r.balanced);

        // 同步狀態要另外看得到，但走的是不同數字
        assert_eq!(ledger.pending_sync(), 100);
    }

    #[test]
    fn every_shared_sync_state_leaves_the_outcome_untouched() {
        // 逐一走過所有同步狀態，翻譯結果必須紋風不動。
        for state in [
            SharedSyncState::NotRequested,
            SharedSyncState::OptedOut,
            SharedSyncState::Queued,
            SharedSyncState::Uploading,
            SharedSyncState::Synced,
            SharedSyncState::RetryableSyncFailure,
            SharedSyncState::RejectedByLibrary,
            SharedSyncState::UnknownLegacy,
        ] {
            let mut e = entry("item.x", TranslationOutcome::Written);
            e.set_shared_sync(state);
            assert_eq!(e.outcome, TranslationOutcome::Written, "{state:?} 改到了翻譯結果");
            assert!(e.outcome.has_usable_translation());
            assert!(!e.outcome.counts_as_gap());
        }
    }

    #[test]
    fn intentionally_unchanged_is_not_a_gap() {
        // Prominence 那次的 908 條「待補」裡混著大量語言代號。
        // 把它們算成缺口，就是製造假缺口。
        let mut ledger = OutcomeLedger::default();
        for i in 0..900 {
            ledger.record(entry(&format!("k{i}"), TranslationOutcome::IntentionallyUnchanged));
        }
        ledger.record(entry("real.gap", TranslationOutcome::RetryableFailure));

        let r = ledger.reconcile();
        assert_eq!(r.gaps, 1, "只有真的缺口才算，實得 {}", r.gaps);
        assert_eq!(r.totals.intentionally_unchanged, 900);
        assert!(r.player_summary().contains("不算漏翻"));
    }

    #[test]
    fn supplement_only_targets_what_rerunning_can_actually_fix() {
        let mut ledger = OutcomeLedger::default();
        ledger.record(entry("a", TranslationOutcome::ReviewNeeded));
        ledger.record(entry("b", TranslationOutcome::RetryableFailure));
        ledger.record(entry("c", TranslationOutcome::Rejected));
        ledger.record(entry("d", TranslationOutcome::Failed));
        ledger.record(entry("e", TranslationOutcome::Written));
        ledger.record(entry("f", TranslationOutcome::IntentionallyUnchanged));

        let targets: Vec<&str> = ledger
            .supplement_targets()
            .iter()
            .map(|e| e.logical_key.as_str())
            .collect();
        // 品質拒絕與明確失敗要人介入，重跑只會再拒絕一次、再花一次錢
        assert_eq!(targets, vec!["a", "b"]);
        assert_eq!(ledger.review_queue().len(), 1);
    }

    #[test]
    fn reconciliation_equation_must_balance() {
        let mut ledger = OutcomeLedger::default();
        for (i, outcome) in [
            TranslationOutcome::Written,
            TranslationOutcome::ReusedExistingZh,
            TranslationOutcome::IntentionallyUnchanged,
            TranslationOutcome::ReviewNeeded,
            TranslationOutcome::RetryableFailure,
            TranslationOutcome::Rejected,
            TranslationOutcome::Failed,
        ]
        .into_iter()
        .enumerate()
        {
            ledger.record(entry(&format!("k{i}"), outcome));
        }
        let r = ledger.reconcile();
        assert!(r.balanced, "每個終局都要被算到一次");
        assert_eq!(r.totals.sum(), r.total_candidates);
        assert_eq!(r.total_candidates, 7);
        // 七個終局各一：可套用 2、保留 1、缺口 4
        assert_eq!(r.usable, 2);
        assert_eq!(r.totals.intentionally_unchanged, 1);
        assert_eq!(r.gaps, 4);
    }

    #[test]
    fn empty_ledger_is_balanced_not_broken() {
        let r = OutcomeLedger::default().reconcile();
        assert!(r.balanced);
        assert_eq!(r.total_candidates, 0);
        assert_eq!(r.gaps, 0);
    }

    #[test]
    fn eligibility_verdicts_map_to_distinct_terminal_outcomes() {
        // Keep 與 Review 必須是兩種不同終局，不能都塞進 pending。
        let keep = classify(Candidate {
            source_kind: "lang",
            logical_key: "botania.entry.x",
            text: "botania.entry.bcIntegration",
        });
        assert_eq!(
            outcome_for_eligibility(&keep),
            TranslationOutcome::IntentionallyUnchanged
        );

        let review = classify(Candidate {
            source_kind: "lang",
            logical_key: "__raw__",
            text: "{}",
        });
        assert_eq!(outcome_for_eligibility(&review), TranslationOutcome::ReviewNeeded);

        let translate = classify(Candidate {
            source_kind: "lang",
            logical_key: "item.x",
            text: "Iron Sword",
        });
        // 尚未解析前先記可重試，解析成功後由呼叫端改寫為 Written
        assert_eq!(
            outcome_for_eligibility(&translate),
            TranslationOutcome::RetryableFailure
        );
    }

    #[test]
    fn candidate_id_is_stable_and_carries_no_text() {
        let a = candidate_id("botania", "botania.entry.x");
        assert_eq!(a, candidate_id("botania", "botania.entry.x"), "同輸入必須同 id");
        assert_eq!(a, candidate_id(" botania ", " botania.entry.x "), "前後空白不影響");
        assert_ne!(a, candidate_id("botania", "botania.entry.y"));
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // 命名空間與 key 之間有分隔，不會因為拼接而碰撞
        assert_ne!(candidate_id("ab", "cd"), candidate_id("a", "bcd"));
    }

    #[test]
    fn jsonl_round_trips_and_survives_a_broken_line() {
        let mut ledger = OutcomeLedger::default();
        let mut written = entry("item.sword", TranslationOutcome::Written);
        written.set_shared_sync(SharedSyncState::Queued);
        ledger.record(written.with_resolution(ResolutionSource::SharedTm));
        ledger.record(
            entry("item.key", TranslationOutcome::IntentionallyUnchanged)
                .with_reason("這是模組內部的語言代號，不是玩家看得到的文字"),
        );

        let body = ledger.to_jsonl();
        let parsed: Vec<CandidateOutcome> = body
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].outcome, TranslationOutcome::Written);
        assert_eq!(parsed[0].resolution, Some(ResolutionSource::SharedTm));
        assert_eq!(parsed[1].reason.as_deref().unwrap(), "這是模組內部的語言代號，不是玩家看得到的文字");

        let with_garbage = format!("{{壞掉的行\n{body}");
        let recovered: Vec<CandidateOutcome> = with_garbage
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        assert_eq!(recovered.len(), 2, "一行壞掉不該讓整份帳本無法解析");
    }

    #[test]
    fn every_outcome_and_sync_state_has_player_facing_text() {
        for o in [
            TranslationOutcome::Written,
            TranslationOutcome::ReusedExistingZh,
            TranslationOutcome::IntentionallyUnchanged,
            TranslationOutcome::ReviewNeeded,
            TranslationOutcome::RetryableFailure,
            TranslationOutcome::Rejected,
            TranslationOutcome::Failed,
        ] {
            let label = o.player_label();
            assert!(!label.is_empty());
            assert!(!label.contains("Outcome"), "{o:?} 的說法像內部代號");
        }
        for s in [
            SharedSyncState::NotRequested,
            SharedSyncState::OptedOut,
            SharedSyncState::Queued,
            SharedSyncState::Uploading,
            SharedSyncState::Synced,
            SharedSyncState::RetryableSyncFailure,
            SharedSyncState::RejectedByLibrary,
            SharedSyncState::UnknownLegacy,
        ] {
            assert!(!s.player_label().is_empty());
        }
    }

    #[test]
    fn legacy_sessions_are_unknown_not_untranslated() {
        // 舊 session 沒有同步欄位。遷移時只能標 UnknownLegacy，
        // 絕不可倒推成「這些還沒翻」。
        let mut e = entry("item.old", TranslationOutcome::Written);
        e.set_shared_sync(SharedSyncState::UnknownLegacy);
        assert!(e.outcome.has_usable_translation());
        assert!(!e.outcome.counts_as_gap());
        assert!(!e.outcome.eligible_for_supplement());
    }
}
