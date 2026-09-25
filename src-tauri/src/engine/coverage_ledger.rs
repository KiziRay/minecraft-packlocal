//! 階段層級的完整性帳本：**沒做到的不准說成做到**。
//!
//! # 為什麼需要這個
//!
//! 站長實測（2026-09-02）：GPT 額度在 21:52:40 用盡，執行紀錄寫著
//!
//! ```text
//! 【錯誤】FTB Quests 略過／失敗：【GPT 額度或驗證暫時不可用】
//! 【錯誤】文字覆寫 略過／失敗：【GPT 額度或驗證暫時不可用】
//! ```
//!
//! 八分鐘後，同一份紀錄宣布：
//!
//! ```text
//! 可以直接開遊戲了，主要遊戲文字都已是繁體中文。
//! 完成！  • 還有 525 條可以再補翻。
//! ```
//!
//! 實際上那個整合包有 **46 個任務檔**與完整的 FancyMenu 設定，
//! 輸出資料夾裡任務檔**一個都沒有**。而「525 條」只算語言表那一層，
//! 完全不含任務書與選單——所以那個數字讓人以為只差一點。
//!
//! # 這個帳本要回答的三個問題
//!
//! 1. 每個階段**本來要處理幾個**、**實際處理了幾個**？
//! 2. 有沒有哪個階段「找到了東西但一個都沒做成」？（那就不准說完成）
//! 3. 加總對不對得起來？（對不上代表有東西在中途悄悄消失）
//!
//! 第 3 點是刻意設計的自我檢查：站長提醒過「你認為的來源可能不是來源，
//! 也可能有多重來源」。帳本對不平的時候，就是還有第五個漏翻來源的訊號。

use std::fmt::Write as _;

/// 一個翻譯階段的成果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageEntry {
    /// 給玩家看的階段名稱（「任務書」而不是 `ftbquests`）
    pub stage: String,
    /// 掃到幾個可處理的單位（檔案／字串／書本，各階段自己定義）
    pub found: usize,
    /// 實際處理成功幾個
    pub done: usize,
    /// 失敗幾個
    pub failed: usize,
    /// 失敗原因（給玩家看的一句話）
    pub reason: Option<String>,
    /// `found` 是不是精確數字。
    ///
    /// 階段整段失敗時，外層往往只知道「它失敗了」而不知道原本有幾個單位。
    /// 這種情況寧可不講數字，也不要編一個出來——編的數字會變成下一個誤導。
    pub count_known: bool,
}

impl StageEntry {
    pub fn ok(stage: impl Into<String>, found: usize, done: usize) -> Self {
        Self {
            stage: stage.into(),
            found,
            done,
            failed: found.saturating_sub(done),
            reason: None,
            count_known: true,
        }
    }

    /// 整段失敗，而且不知道原本有幾個單位。
    ///
    /// 判斷「不准說完成」只需要知道「有東西要做但一個都沒做成」，
    /// 精確數量不是必要條件——不知道就別編一個出來。
    pub fn failed_unknown_count(stage: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            found: 1,
            done: 0,
            failed: 1,
            reason: Some(reason.into()),
            count_known: false,
        }
    }

    /// 找到東西卻一個都沒做成＝這個階段整段失敗。
    ///
    /// 這是「不准說完成」的判準。`found == 0` 不算失敗——
    /// 那只是這個整合包沒有這種內容（例如沒裝 FTB Quests）。
    pub fn totally_failed(&self) -> bool {
        self.found > 0 && self.done == 0
    }

    /// 有做到一些但沒做完。
    pub fn partial(&self) -> bool {
        self.done > 0 && self.done < self.found
    }
}

/// 整輪翻譯的階段帳本。
#[derive(Debug, Clone, Default)]
pub struct CoverageLedger {
    entries: Vec<StageEntry>,
}

impl CoverageLedger {
    pub fn record(&mut self, entry: StageEntry) {
        self.entries.push(entry);
    }

    /// 有沒有任何一個階段整段失敗。**有的話就不准說「完成」。**
    pub fn has_total_failure(&self) -> bool {
        self.entries.iter().any(StageEntry::totally_failed)
    }

    /// 整段失敗的階段清單。
    pub fn total_failures(&self) -> Vec<&StageEntry> {
        self.entries.iter().filter(|e| e.totally_failed()).collect()
    }

    /// 所有階段加起來還缺幾個單位。
    ///
    /// 舊版的缺口數只算語言表，任務書整段沒翻也不會反映出來。
    pub fn outstanding(&self) -> usize {
        self.entries.iter().map(|e| e.failed).sum()
    }

    /// 帳有沒有平：每個階段的 `found` 必須等於 `done + failed`。
    ///
    /// 對不上代表有單位在中途消失了——那就是還沒被發現的漏翻來源。
    pub fn imbalances(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.found != e.done + e.failed)
            .map(|e| {
                format!(
                    "{}：找到 {} 個，但只交代了 {}（完成 {}／失敗 {}），少了 {}",
                    e.stage,
                    e.found,
                    e.done + e.failed,
                    e.done,
                    e.failed,
                    e.found as i64 - (e.done + e.failed) as i64
                )
            })
            .collect()
    }

    /// 給玩家看的結尾。**這是「不准謊報完成」的執行點。**
    ///
    /// 有階段整段失敗時，第一句就要講清楚哪一塊完全沒翻到、為什麼，
    /// 而不是把它藏在二十個子句組成的長句裡（舊版就是那樣）。
    pub fn player_summary(&self) -> String {
        let failures = self.total_failures();
        if failures.is_empty() {
            return String::new();
        }
        let mut out = String::from("這次有內容完全沒有翻到：\n");
        for entry in &failures {
            if entry.count_known {
                let _ = write!(out, "• {}（{} 個）", entry.stage, entry.found);
            } else {
                // 不知道確切數量就別編——編出來的數字會變成下一個誤導
                let _ = write!(out, "• {}", entry.stage);
            }
            if let Some(reason) = &entry.reason {
                let _ = write!(out, "——{reason}");
            }
            out.push('\n');
        }
        out.push_str("這些不是漏掉沒處理，是這一輪真的沒做成；排除原因後再跑一次就會補上。");
        out
    }

    /// 開發人員模式用的完整帳本（含驗算）。
    pub fn dev_report(&self) -> String {
        let mut out = String::from("階段帳本：\n");
        for e in &self.entries {
            let _ = writeln!(
                out,
                "  {:<12} found={:<6} done={:<6} failed={:<6}{}{}",
                e.stage,
                e.found,
                e.done,
                e.failed,
                if e.totally_failed() {
                    " [整段失敗]"
                } else if e.partial() {
                    " [部分完成]"
                } else {
                    ""
                },
                e.reason
                    .as_ref()
                    .map(|r| format!(" reason={r}"))
                    .unwrap_or_default()
            );
        }
        let imbalances = self.imbalances();
        if imbalances.is_empty() {
            out.push_str("  帳目平衡：每個階段的 found 都等於 done + failed\n");
        } else {
            out.push_str("  ⚠️ 帳不平（代表有單位中途消失，可能還有沒發現的漏翻來源）：\n");
            for line in imbalances {
                let _ = writeln!(out, "    - {line}");
            }
        }
        out
    }
}

#[cfg(test)]
impl StageEntry {
    /// 整段失敗且知道原本有幾個單位。
    ///
    /// 目前只有測試用得到：正式程式碼裡，階段整段失敗時外層拿到的是
    /// `Err(String)`，本來就不知道原本有幾個單位（所以用
    /// [`StageEntry::failed_unknown_count`]）。等哪個階段改成「先數再做」，
    /// 把這支移出 `cfg(test)` 就能用。
    pub fn failed(stage: impl Into<String>, found: usize, reason: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            found,
            done: 0,
            failed: found,
            reason: Some(reason.into()),
            count_known: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stage_that_found_work_but_did_none_blocks_the_completion_message() {
        // 站長那次的實況：任務書找到 46 個檔案，一個都沒翻成。
        let mut ledger = CoverageLedger::default();
        ledger.record(StageEntry::ok("語言表", 33233, 32859));
        ledger.record(StageEntry::failed("任務書", 46, "GPT 額度已用盡"));
        ledger.record(StageEntry::failed("選單文字", 120, "GPT 額度已用盡"));

        assert!(ledger.has_total_failure(), "有階段整段失敗就不准說完成");
        let summary = ledger.player_summary();
        assert!(summary.contains("任務書"), "要點名是哪一塊：{summary}");
        assert!(summary.contains("46"), "要講清楚有多少沒翻到");
        assert!(summary.contains("選單文字"));
        assert!(summary.contains("GPT 額度已用盡"), "要講原因");
        assert!(!summary.contains("完成"), "不可以出現完成字樣：{summary}");
    }

    #[test]
    fn outstanding_counts_every_stage_not_just_the_language_table() {
        // 舊版只報語言表的 374，讓人以為只差一點——
        // 實際上任務書 46 個與選單 120 個完全沒翻。
        let mut ledger = CoverageLedger::default();
        ledger.record(StageEntry::ok("語言表", 33233, 32859));
        ledger.record(StageEntry::failed("任務書", 46, "GPT 額度已用盡"));
        ledger.record(StageEntry::failed("選單文字", 120, "GPT 額度已用盡"));
        assert_eq!(ledger.outstanding(), 374 + 46 + 120);
    }

    #[test]
    fn nothing_found_is_not_a_failure() {
        // 沒裝 FTB Quests 的整合包，任務書階段 found=0——那是正常的，
        // 不可以因此說「有東西沒翻到」嚇人。
        let mut ledger = CoverageLedger::default();
        ledger.record(StageEntry::ok("任務書", 0, 0));
        ledger.record(StageEntry::ok("語言表", 100, 100));
        assert!(!ledger.has_total_failure());
        assert_eq!(ledger.player_summary(), "");
        assert_eq!(ledger.outstanding(), 0);
    }

    #[test]
    fn the_books_must_balance_or_something_vanished_silently() {
        // 這條是自我檢查：對不上就代表有單位中途消失，
        // 也就是還有一個我們還沒發現的漏翻來源。
        let mut ledger = CoverageLedger::default();
        ledger.record(StageEntry {
            stage: "語言表".into(),
            found: 100,
            done: 40,
            failed: 20, // 40 + 20 != 100 → 少了 40
            reason: None,
            count_known: true,
        });
        let imbalances = ledger.imbalances();
        assert_eq!(imbalances.len(), 1);
        assert!(imbalances[0].contains("少了 40"), "{}", imbalances[0]);
        assert!(ledger.dev_report().contains("帳不平"));
    }

    #[test]
    fn a_balanced_run_reports_balanced() {
        let mut ledger = CoverageLedger::default();
        ledger.record(StageEntry::ok("語言表", 100, 100));
        ledger.record(StageEntry::ok("任務書", 46, 46));
        assert!(ledger.imbalances().is_empty());
        assert!(ledger.dev_report().contains("帳目平衡"));
        assert!(!ledger.has_total_failure());
    }

    #[test]
    fn partial_progress_is_not_a_total_failure() {
        // 翻了一半也是有價值的，不該跟「一個都沒翻」講同樣的話
        let entry = StageEntry::ok("任務書", 46, 20);
        assert!(!entry.totally_failed());
        assert!(entry.partial());
        assert_eq!(entry.failed, 26);
    }
}
