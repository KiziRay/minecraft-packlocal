//! B4 第二輪 F1／F2：本地模型逾時的累計、等待延長與「放棄」判斷。
//!
//! 每次翻譯呼叫（一個 AI 引擎）各有一份，不是全域：新一輪一開始倍數一定是 ×1，
//! 平行跑的測試也不會互相影響。
//!
//! 規則（待使用者實測）：
//! - 每逾時一次，之後的等待上限 ×1.5，最多 ×8（總上限另有 900 秒）；
//! - 有回應就算一次成功，每 3 次成功把倍數 ÷1.5（最低 ×1）；
//! - 放棄（本地模型在這台電腦上不可用）：倍數已到 ×8 後又連續逾時 3 次，
//!   或這一輪累計逾時等了 30 分鐘——取較先發生者。

use std::time::Duration;

const MAX_STRETCH_X100: u64 = 800;
/// 倍數到頂後再連續逾時幾次就放棄。
pub const GIVE_UP_AFTER_AT_MAX: u32 = 3;
/// 這一輪累計逾時等待的上限。
pub const LOCAL_TIMEOUT_BUDGET: Duration = Duration::from_secs(30 * 60);
/// 每幾次成功回降一次。
const SUCCESSES_PER_STEP_DOWN: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum TimeoutVerdict {
    /// 繼續（拆小重送），之後的等待上限倍數
    Continue { stretch: f64 },
    /// 放棄：白話原因（走「AI 不可用」：保留已翻、停下這一輪的 AI）
    GiveUp(String),
}

#[derive(Debug, Clone)]
pub struct TimeoutTracker {
    stretch_x100: u64,
    total_waited: Duration,
    consecutive_at_max: u32,
    successes: u32,
    budget: Duration,
}

impl Default for TimeoutTracker {
    fn default() -> Self {
        Self::with_budget(LOCAL_TIMEOUT_BUDGET)
    }
}

impl TimeoutTracker {
    pub fn with_budget(budget: Duration) -> Self {
        Self {
            stretch_x100: 100,
            total_waited: Duration::ZERO,
            consecutive_at_max: 0,
            successes: 0,
            budget,
        }
    }

    pub fn stretch(&self) -> f64 {
        self.stretch_x100.max(100) as f64 / 100.0
    }

    /// 記下一次逾時（等了 `waited`）。
    pub fn on_timeout(&mut self, waited: Duration) -> TimeoutVerdict {
        self.total_waited += waited;
        self.successes = 0;
        let was_at_max = self.stretch_x100 >= MAX_STRETCH_X100;
        self.stretch_x100 = (self.stretch_x100.max(100) * 3 / 2).min(MAX_STRETCH_X100);
        if was_at_max {
            self.consecutive_at_max += 1;
        }
        if self.consecutive_at_max >= GIVE_UP_AFTER_AT_MAX || self.total_waited >= self.budget {
            return TimeoutVerdict::GiveUp(give_up_message(self.total_waited));
        }
        TimeoutVerdict::Continue { stretch: self.stretch() }
    }

    /// 有回應（不論內容好壞）：連續逾時歸零，每 3 次成功把等待倍數回降一級。
    pub fn on_success(&mut self) {
        self.consecutive_at_max = 0;
        self.successes += 1;
        if self.successes >= SUCCESSES_PER_STEP_DOWN {
            self.successes = 0;
            self.stretch_x100 = (self.stretch_x100 * 2 / 3).max(100);
        }
    }
}

fn give_up_message(waited: Duration) -> String {
    let minutes = (waited.as_secs() + 59) / 60;
    format!(
        "本地模型在這台電腦上一直等不到回應（這一輪累計等了約 {minutes} 分鐘），先停下本地翻譯。\
電腦可能記憶體不足或太慢；已翻好的部分都保留。建議改用線上 AI（自訂 API 或 GPT），\
或關閉其他程式後按「接續補完」從停下的地方繼續。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_grows_to_the_cap_then_gives_up_after_three_more() {
        let mut t = TimeoutTracker::default();
        let mut verdicts = Vec::new();
        for _ in 0..20 {
            let v = t.on_timeout(Duration::from_secs(1));
            let stop = matches!(v, TimeoutVerdict::GiveUp(_));
            verdicts.push(v);
            if stop {
                break;
            }
        }
        // ×1.5 六次到 ×8，再 3 次放棄
        assert_eq!(verdicts.len(), 9, "{verdicts:?}");
        match verdicts.last().unwrap() {
            TimeoutVerdict::GiveUp(m) => assert!(m.contains("記憶體不足") && m.contains("接續補完") && m.contains("線上 AI"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn total_wait_budget_gives_up_first_when_it_is_reached_first() {
        let mut t = TimeoutTracker::with_budget(Duration::from_secs(600));
        assert!(matches!(t.on_timeout(Duration::from_secs(400)), TimeoutVerdict::Continue { .. }));
        assert!(matches!(t.on_timeout(Duration::from_secs(300)), TimeoutVerdict::GiveUp(_)));
    }

    #[test]
    fn successes_step_the_wait_back_down_and_reset_the_count() {
        let mut t = TimeoutTracker::default();
        for _ in 0..6 {
            t.on_timeout(Duration::from_secs(1));
        }
        assert_eq!(t.stretch(), 8.0);
        t.on_timeout(Duration::from_secs(1)); // 到頂後第 1 次
        t.on_success(); // 有回應：連續逾時歸零
        t.on_timeout(Duration::from_secs(1));
        t.on_timeout(Duration::from_secs(1));
        assert!(matches!(t.on_success(), ()));
        for _ in 0..3 {
            t.on_success();
        }
        assert!(t.stretch() < 8.0, "連續成功要回降：{}", t.stretch());
        for _ in 0..30 {
            t.on_success();
        }
        assert_eq!(t.stretch(), 1.0, "最低回到 ×1");
    }
}
