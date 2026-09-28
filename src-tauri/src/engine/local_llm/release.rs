//! B4 #8：本地模型「翻完自動關閉」——以輪次編號避免關閉競態（後端這一側的保險）。
//!
//! 前端每一輪開始時呼叫 [`begin_round`] 拿編號，結束時帶著編號呼叫 [`release_after_run`]。
//! 只有「編號還是最新一輪、而且沒有翻譯在跑」才真的關閉；否則照實回報為什麼不關。

use std::sync::atomic::{AtomicU64, Ordering};

static ROUND: AtomicU64 = AtomicU64::new(0);

/// 新一輪開始，回傳這一輪的編號（從 1 起算）。
pub fn begin_round() -> u64 {
    ROUND.fetch_add(1, Ordering::SeqCst) + 1
}

pub fn current_round() -> u64 {
    ROUND.load(Ordering::SeqCst)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseDecision {
    /// 關閉
    Stop,
    /// 已經有更新的一輪開始了：不關
    NewerRound { current: u64 },
    /// 翻譯還在跑：不關
    Busy,
}

/// 純判斷（方便測試）。
pub fn decide(round: u64, current: u64, translation_active: bool) -> ReleaseDecision {
    if round != current {
        return ReleaseDecision::NewerRound { current };
    }
    if translation_active {
        return ReleaseDecision::Busy;
    }
    ReleaseDecision::Stop
}

/// 給使用者看的一句話（白話＋輪次）。
pub fn message(round: u64, decision: &ReleaseDecision) -> String {
    match decision {
        ReleaseDecision::Stop => format!("第 {round} 輪翻譯結束，已關閉本地模型，釋放記憶體與顯示卡資源。"),
        ReleaseDecision::NewerRound { current } => format!(
            "第 {round} 輪結束時不關閉本地模型，因為第 {current} 輪已經開始使用它（會在第 {current} 輪結束後關閉）。"
        ),
        ReleaseDecision::Busy => format!("第 {round} 輪結束，但還有翻譯在進行，暫不關閉本地模型。"),
    }
}

/// 依判斷結果真的關閉（`stop` 由呼叫端傳入，測試不會關到真的程式）。
pub fn release_after_run(round: u64, translation_active: bool, stop: impl FnOnce()) -> (bool, String) {
    let decision = decide(round, current_round(), translation_active);
    let text = message(round, &decision);
    crate::dev_log!("local", "{text}");
    let stopped = decision == ReleaseDecision::Stop;
    if stopped {
        stop();
    }
    (stopped, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_latest_idle_round_may_stop_the_model() {
        assert_eq!(decide(3, 3, false), ReleaseDecision::Stop);
        assert_eq!(decide(2, 3, false), ReleaseDecision::NewerRound { current: 3 }, "翻完立刻接續補完：舊的關閉不得關掉新一輪的模型");
        assert_eq!(decide(3, 3, true), ReleaseDecision::Busy);
    }

    #[test]
    fn stale_release_never_calls_stop() {
        let first = begin_round();
        let _second = begin_round();
        let mut called = false;
        let (stopped, text) = release_after_run(first, false, || called = true);
        assert!(!stopped && !called, "{text}");
        assert!(text.contains(&format!("第 {first} 輪")), "{text}");
    }

    #[test]
    fn messages_are_plain_language_with_round_numbers() {
        assert!(message(4, &ReleaseDecision::Stop).contains("已關閉本地模型"));
        let m = message(4, &ReleaseDecision::NewerRound { current: 5 });
        assert!(m.contains("第 4 輪") && m.contains("第 5 輪"), "{m}");
    }
}
