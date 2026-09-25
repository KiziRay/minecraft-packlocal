//! 「還缺幾條」到底該怎麼算。
//!
//! # 問題
//!
//! 站長跑完一次之後重開工具，卡片說「還有三萬多條缺漏」。
//! 但同一次的執行紀錄顯示：共享庫命中 32151、術語表 672、真正送 AI 只有 271 句。
//!
//! 兩個原因疊在一起：
//!
//! 1. **計數是過期的**——`pending_count` 是「本地整理完成當下」的快照，
//!    之後被共享庫／術語表補掉的三萬多條不會回寫。那一次還在 63% 崩潰，
//!    連更新計數的程式碼都沒跑到。這一項由 [`super::session::RunOutcome`] 處理：
//!    只有 `Completed` 的計數可以拿來對使用者講缺漏。
//!
//! 2. **不是每一條「沒中文」都算缺漏**——羅馬數字（`IX`）、圖示字元（`§f`）、
//!    單位（`%s FPS`）、品牌名（`OptiFine`）本來就**不該**翻譯，原樣保留才是
//!    正確答案。把它們算進缺漏，使用者永遠看到一個補不完的數字，
//!    而且每次「接續補完」都會把它們重送一次 AI，燒錢又不會變少。
//!
//! # 做法
//!
//! 把待處理項目分成**補得動**與**補不動**兩類，對使用者只講補得動的那個數字。
//! 判斷沿用既有的 [`super::translation_quality::source_stays_unchanged`]，
//! 不另外發明一套規則。

use super::jar_scan::LangMap;
use super::translation_quality::source_stays_unchanged;

/// 缺口盤點結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GapCounts {
    /// 真的還沒翻、而且原文可翻——**只有這個數字該給使用者看**
    pub actionable: usize,
    /// 本來就不該翻的（羅馬數字／圖示／單位／品牌名）
    pub untranslatable: usize,
}

impl GapCounts {
    /// 沒有補得動的東西了＝對使用者來說就是「翻完了」。
    pub fn is_complete(self) -> bool {
        self.actionable == 0
    }
}

/// 盤點一份 pending 清單裡有多少是真的補得動的。
pub fn count_gaps(pending: &LangMap) -> GapCounts {
    let mut out = GapCounts::default();
    for map in pending.values() {
        for source in map.values() {
            if source.trim().is_empty() {
                continue;
            }
            if source_stays_unchanged(source) {
                out.untranslatable += 1;
            } else {
                out.actionable += 1;
            }
        }
    }
    out
}

/// 給使用者看的一句話。
///
/// 刻意不在「已完成」的情況下提任何數字——補不動的那些講了只會讓人以為工具沒做完，
/// 而他其實什麼都不必做。
pub fn describe(counts: GapCounts) -> String {
    if counts.is_complete() {
        if counts.untranslatable > 0 {
            return format!(
                "已完成。另有 {} 條是圖示、單位、羅馬數字或專有名詞，保留原文才正確。",
                counts.untranslatable
            );
        }
        return "已完成。".into();
    }
    format!("還有 {} 條可以再補翻。", counts.actionable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn pending(items: &[(&str, &str)]) -> LangMap {
        let mut ns: HashMap<String, String> = HashMap::new();
        for (k, v) in items {
            ns.insert((*k).to_string(), (*v).to_string());
        }
        let mut out: LangMap = HashMap::new();
        out.insert("test".into(), ns);
        out
    }

    #[test]
    fn things_that_should_never_be_translated_are_not_gaps() {
        // 實測：某次真實翻譯的 663 條「品質未過」裡有 501 條是這種東西。
        // 算進缺漏的話，使用者永遠看到一個補不完的數字。
        let counts = count_gaps(&pending(&[
            ("enchantment.level.9", "IX"),   // 羅馬數字
            ("icon.star", "§f"),             // 圖示
            ("hud.fps", "%s FPS"),           // 單位
            ("mod.optifine", "OptiFine"),    // 品牌名
        ]));
        assert_eq!(counts.actionable, 0, "這些本來就不該翻，不是缺漏");
        assert_eq!(counts.untranslatable, 4);
        assert!(counts.is_complete());
    }

    #[test]
    fn real_untranslated_text_is_a_gap() {
        let counts = count_gaps(&pending(&[
            ("item.sword", "Iron Sword"),
            ("quest.intro", "Welcome to the world of magic."),
        ]));
        assert_eq!(counts.actionable, 2);
        assert_eq!(counts.untranslatable, 0);
        assert!(!counts.is_complete());
    }

    #[test]
    fn mixed_only_reports_what_the_user_can_act_on() {
        let counts = count_gaps(&pending(&[
            ("item.sword", "Iron Sword"),
            ("enchantment.level.9", "IX"),
            ("icon.star", "§f"),
        ]));
        assert_eq!(counts.actionable, 1);
        assert_eq!(counts.untranslatable, 2);
        assert_eq!(counts.actionable + counts.untranslatable, 3);
        assert!(describe(counts).contains('1'), "只講補得動的那個數字");
        assert!(!describe(counts).contains('3'), "不要把補不動的算進去嚇人");
    }

    #[test]
    fn completed_message_never_leads_with_a_scary_number() {
        let clean = count_gaps(&pending(&[("enchantment.level.9", "IX")]));
        let text = describe(clean);
        assert!(text.starts_with("已完成"), "先講結論：翻完了");
        assert!(text.contains("保留原文才正確"), "再解釋那 1 條為什麼不用管");

        let nothing = count_gaps(&pending(&[]));
        assert_eq!(describe(nothing), "已完成。");
    }

    #[test]
    fn blank_sources_are_ignored() {
        let counts = count_gaps(&pending(&[("a", ""), ("b", "   ")]));
        assert_eq!(counts.actionable + counts.untranslatable, 0);
    }
}
