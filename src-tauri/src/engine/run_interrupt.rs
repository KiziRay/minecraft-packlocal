//! B4：這一輪翻譯的「AI 已停」狀態。
//!
//! AI 在某一步停下（額度用完、金鑰無效、本地模型當掉、使用者按停止）之後，
//! 同一輪後面的步驟（補充來源、補強重試、長句拆解）**不可以再去打 AI**——那只會空轉、
//! 拖時間，還可能繼續扣錢。它們改成只用免費的資料層（術語表、翻譯記憶、共享庫），
//! 沒翻到的句子留在缺口裡，並把這一輪標成「部分完成」，之後按「接續補完」從停下的地方繼續。
//!
//! 每一輪開始時呼叫 [`reset`]。
//!
//! 正式執行時是全域狀態（主流程與額外來源的並行執行緒都要看得到）；
//! 測試時每條執行緒各自一份，平行測試互不干擾。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltKind {
    /// 使用者按了停止：寫出已翻好的部分、裝進遊戲，不再做後面的翻譯步驟
    UserStop,
    /// AI 不能再用：後面的步驟照樣跑，但只用免費的資料層
    AiUnavailable,
}

/// B5c：AI 為什麼停（完成卡的原因句依這個分類寫，不再從中文句子猜；規格 §3.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltCause {
    /// 使用者按了停止
    UserStop,
    /// 額度用完、帳戶沒餘額、用量上限
    Quota,
    /// 金鑰被服務商拒絕、沒有權限
    AuthRejected,
    /// Discord 登入失效、要重新確認會員
    Relogin,
    /// 本地模型一直等不到回應（跑不動）
    LocalStuck,
    /// 本地模型程式結束或連不上（多半記憶體不足）
    LocalGone,
    /// AI 連續好幾批都給不出可用的譯文
    NoOutput,
    /// 連線中斷、逾時太久
    Network,
    /// 其他
    Other,
}

impl HaltCause {
    /// 前端用的分類碼（`interruption.cause`）。
    pub fn code(self) -> &'static str {
        match self {
            Self::UserStop => "user_stop",
            Self::Quota => "quota",
            Self::AuthRejected => "auth",
            Self::Relogin => "relogin",
            Self::LocalStuck => "local_stuck",
            Self::LocalGone => "local_gone",
            Self::NoOutput => "no_output",
            Self::Network => "network",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Halt {
    pub kind: HaltKind,
    /// 給使用者看的白話原因
    pub reason: String,
    /// B5c：原因分類
    pub cause: HaltCause,
    /// B5c：服務商錯誤的分類（retry_policy::FailureClass）；本地模型、沒有進展、使用者停止時是 `None`
    pub class: Option<FailureClass>,
}

use super::retry_policy::{classify_message, FailureClass};

/// B5c 審查 4：cause_for 依賴的固定字樣。產生這些句子的地方（local_llm/timeouts.rs、deepseek.rs）
/// 一律用這些常數組句，改字只改這裡，分類不會悄悄失效。
pub const LOCAL_STUCK_MARK: &str = "本地模型在這台電腦上一直等不到回應";
pub const LOCAL_GONE_MARK: &str = "本地模型程式已經結束";
pub const LOCAL_UNREACHABLE_MARK: &str = "連不上本地模型";
pub const NO_PROGRESS_MARK: &str = "都沒有新譯文";
pub const RELOGIN_MARK: &str = "Discord 登入已失效";
pub const CHATGPT_BUSY_TITLE: &str = "【ChatGPT 暫時不能翻譯】";

/// 前端用的 FailureClass 代碼（`interruption.failureClass`）。
fn failure_class_code(class: FailureClass) -> &'static str {
    match class {
        FailureClass::RateLimited => "rate_limited",
        FailureClass::ServerBusy => "server_busy",
        FailureClass::Network => "network",
        FailureClass::Timeout => "timeout",
        FailureClass::QuotaExhausted => "quota_exhausted",
        FailureClass::AuthInvalid => "auth_invalid",
        FailureClass::TooLarge => "too_large",
        FailureClass::Other => "other",
    }
}

/// 從停下的原因分出類別。本地模型、沒有進展、Discord 的句子是工具自己寫的（字樣固定）；
/// 服務商的錯誤交給 retry_policy 分類——但要先拿掉工具包裝的標題（「【自訂 API 額度或金鑰無法使用】」
/// 這種標題同時提到額度與金鑰，不能拿來分類）與後面的建議段落。
pub fn cause_for(reason: &str) -> (HaltCause, Option<FailureClass>) {
    let text = reason.trim();
    if text.contains(LOCAL_STUCK_MARK) {
        return (HaltCause::LocalStuck, None);
    }
    if text.contains(LOCAL_GONE_MARK) || text.contains(LOCAL_UNREACHABLE_MARK) {
        return (HaltCause::LocalGone, None);
    }
    if text.contains(NO_PROGRESS_MARK) {
        return (HaltCause::NoOutput, None);
    }
    if text.contains(RELOGIN_MARK) {
        return (HaltCause::Relogin, None);
    }
    let detail = if text.starts_with('【') {
        // 標題行之後、第一個空行之前才是服務商原本的錯誤
        text.lines().skip(1).take_while(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(" ")
    } else {
        text.to_string()
    };
    let class = classify_message(&detail);
    let cause = match class {
        FailureClass::QuotaExhausted => HaltCause::Quota,
        FailureClass::AuthInvalid => HaltCause::AuthRejected,
        FailureClass::Network | FailureClass::Timeout => HaltCause::Network,
        _ if text.starts_with(CHATGPT_BUSY_TITLE) => HaltCause::Quota,
        _ => HaltCause::Other,
    };
    (cause, Some(class))
}

#[cfg(not(test))]
mod store {
    use super::Halt;
    use std::sync::Mutex;

    static STATE: Mutex<Option<Halt>> = Mutex::new(None);

    pub fn get() -> Option<Halt> {
        STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set(value: Option<Halt>) {
        *STATE.lock().unwrap_or_else(|e| e.into_inner()) = value;
    }
}

#[cfg(test)]
mod store {
    use super::Halt;
    use std::cell::RefCell;

    thread_local! {
        static STATE: RefCell<Option<Halt>> = const { RefCell::new(None) };
    }

    pub fn get() -> Option<Halt> {
        STATE.with(|s| s.borrow().clone())
    }

    pub fn set(value: Option<Halt>) {
        STATE.with(|s| *s.borrow_mut() = value);
    }
}

/// 新一輪開始：清掉上一輪的狀態。
pub fn reset() {
    store::set(None);
}

/// AI 不能再用了。已經是「使用者停止」就不覆蓋（停止優先）；已經停過就保留第一個原因。
pub fn halt_ai(reason: &str) {
    if store::get().is_some() {
        return;
    }
    let (cause, class) = cause_for(reason);
    store::set(Some(Halt {
        kind: HaltKind::AiUnavailable,
        reason: reason.trim().to_string(),
        cause,
        class,
    }));
}

/// 使用者按了停止（覆蓋「AI 不可用」：停止要優先處理）。
pub fn stop_by_user(reason: &str) {
    store::set(Some(Halt {
        kind: HaltKind::UserStop,
        reason: reason.trim().to_string(),
        cause: HaltCause::UserStop,
        class: None,
    }));
}

/// 目前的狀態（`None`＝AI 正常）。
pub fn current() -> Option<Halt> {
    store::get()
}

pub fn is_user_stop() -> bool {
    matches!(store::get(), Some(Halt { kind: HaltKind::UserStop, .. }))
}

/// 給使用者看的一段話：AI 為什麼停、已翻好的去哪了、下一步做什麼。
pub fn player_note(halt: &Halt) -> String {
    let first = halt.reason.lines().next().unwrap_or("").trim();
    match halt.kind {
        HaltKind::UserStop => "已依你的要求停止翻譯。已翻好的部分都有寫出並套用到遊戲；\
之後按「接續補完」，會從停下的地方繼續，不會從頭翻。"
            .to_string(),
        HaltKind::AiUnavailable => format!(
            "AI 在這一輪中途停下（{first}）。已翻好的部分都保留，並照樣寫出、套用到遊戲；\
後面的步驟只用免費的資料（術語表、翻譯記憶、共享庫），不再空轉重試。\
排除原因後按「接續補完」，會從停下的地方繼續。"
        ),
    }
}

/// 翻譯結果裡「這一輪有沒有中途停下」的資料（B8 完成卡顯示；ui-contract「AI 額度用完」橫幅）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptionView {
    /// AI 中途停下的白話說明（`None`＝AI 正常跑完）
    pub ai_stopped: Option<String>,
    /// 停下的原因分類：`user_stop`／`ai_unavailable`
    pub kind: Option<&'static str>,
    /// 使用者按了停止
    pub stopped_by_user: bool,
    /// AI 沒回應、留在缺口的條數（不是翻不好；接續補完會再翻）
    pub no_answer: usize,
    /// 品質沒過、暫緩的條數
    pub quality_deferred: usize,
    /// B5c：停下的原因分類碼（user_stop／quota／auth／relogin／local_stuck／local_gone／no_output／network／other）
    pub cause: Option<&'static str>,
    /// B5c：服務商錯誤的分類（retry_policy::FailureClass 的代碼）
    pub failure_class: Option<&'static str>,
}

/// 依目前狀態組出結果欄位。
pub fn view(no_answer: usize, quality_deferred: usize) -> InterruptionView {
    let halt = current();
    InterruptionView {
        ai_stopped: halt.as_ref().map(player_note),
        kind: halt.as_ref().map(|h| match h.kind {
            HaltKind::UserStop => "user_stop",
            HaltKind::AiUnavailable => "ai_unavailable",
        }),
        stopped_by_user: is_user_stop(),
        no_answer,
        quality_deferred,
        cause: halt.as_ref().map(|h| h.cause.code()),
        failure_class: halt.as_ref().and_then(|h| h.class).map(failure_class_code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_reports_counts_separately_and_the_reason() {
        reset();
        let v = view(3, 5);
        assert_eq!((v.no_answer, v.quality_deferred), (3, 5));
        assert!(v.ai_stopped.is_none() && !v.stopped_by_user);
        halt_ai("帳號餘額不足");
        let v = view(0, 0);
        assert_eq!(v.kind, Some("ai_unavailable"));
        assert!(v.ai_stopped.unwrap().contains("帳號餘額不足"));
        reset();
    }

    #[test]
    fn first_reason_wins_and_user_stop_takes_priority() {
        reset();
        assert!(current().is_none());
        halt_ai("額度用完");
        halt_ai("另一個原因");
        assert_eq!(current().unwrap().reason, "額度用完");
        stop_by_user("停止");
        assert!(is_user_stop());
        halt_ai("之後的 AI 錯誤");
        assert!(is_user_stop(), "停止不可被後來的 AI 錯誤蓋掉");
        reset();
        assert!(current().is_none());
    }

    /// B5c：halt_ai 記下原因分類，完成卡才分得出額度用完／金鑰被拒／本地模型卡住（規格 §3.3 原因句）。
    #[test]
    fn b5c_halt_records_the_cause_so_the_card_can_say_why() {
        let cases: &[(&str, &str)] = &[
            ("【自訂 API 額度或金鑰無法使用】
HTTP 402 insufficient balance

自訂 API 使用你填入服務商的金鑰與額度。", "quota"),
            ("【自訂 API 額度或金鑰無法使用】
HTTP 401 invalid api key

自訂 API 使用你填入服務商的金鑰與額度。", "auth"),
            ("【ChatGPT 暫時不能翻譯】
usage limit reached

這次 ChatGPT 不接受翻譯請求", "quota"),
            ("本地模型在這台電腦上一直等不到回應（這一輪累計等了約 31 分鐘），先停下本地翻譯。", "local_stuck"),
            ("本地模型程式已經結束（exit code: 3），通常是這台電腦的記憶體不足。", "local_gone"),
            ("連不上本地模型：它沒有在執行（可能已經當掉或被關閉）。", "local_gone"),
            ("AI 連續 12 批都沒有新譯文，提前結束；已保留已成功譯文（最後一次：沒有回應）", "no_output"),
            ("Discord 登入已失效或需重新確認會員資格，請回到工具重新登入後再試。", "relogin"),
            ("連線失敗：error sending request", "network"),
            ("看不懂的錯誤", "other"),
        ];
        for (reason, code) in cases {
            reset();
            halt_ai(reason);
            let v = view(0, 0);
            assert_eq!(v.cause, Some(*code), "{reason}");
            assert_eq!(v.kind, Some("ai_unavailable"));
        }
        reset();
        stop_by_user("已依你的要求停止");
        assert_eq!(view(0, 0).cause, Some("user_stop"));
        reset();
        assert_eq!(view(0, 0).cause, None, "AI 正常跑完沒有原因");
    }

    /// 審查 4：產生端真的用這些字樣組句（改一邊另一邊跟著變）
    #[test]
    fn b5c_fix4_producers_use_the_shared_marks() {
        let src = |rel: &str| std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join(rel)).unwrap();
        let timeouts = src("engine/local_llm/timeouts.rs");
        assert!(timeouts.contains("run_interrupt::LOCAL_STUCK_MARK"));
        let ds = src("engine/deepseek.rs");
        for mark in ["LOCAL_GONE_MARK", "LOCAL_UNREACHABLE_MARK", "NO_PROGRESS_MARK", "RELOGIN_MARK", "CHATGPT_BUSY_TITLE"] {
            assert!(ds.contains(&format!("run_interrupt::{mark}")), "{mark}");
        }
        for (reason, code) in [
            (format!("{LOCAL_STUCK_MARK}（約 3 分鐘）"), "local_stuck"),
            (format!("{LOCAL_GONE_MARK}（exit 1）"), "local_gone"),
            (format!("AI 連續 3 批{NO_PROGRESS_MARK}"), "no_output"),
            (format!("{RELOGIN_MARK}或需重新確認會員資格"), "relogin"),
        ] {
            assert_eq!(cause_for(&reason).0.code(), code, "{reason}");
        }
    }

    #[test]
    fn b5c_failure_class_is_kept_with_the_halt() {
        reset();
        halt_ai("HTTP 402 insufficient balance");
        assert_eq!(current().unwrap().class, Some(crate::engine::retry_policy::FailureClass::QuotaExhausted));
        let v = view(0, 0);
        assert_eq!(v.failure_class, Some("quota_exhausted"));
        reset();
    }

    #[test]
    fn notes_say_what_happened_and_what_to_do_next() {
        let note = player_note(&Halt {
            kind: HaltKind::AiUnavailable,
            reason: "帳號餘額不足\n細節".into(),
            cause: HaltCause::Quota,
            class: None,
        });
        assert!(note.contains("帳號餘額不足") && note.contains("接續補完") && note.contains("套用到遊戲"), "{note}");
        let stop = player_note(&Halt {
            kind: HaltKind::UserStop,
            reason: String::new(),
            cause: HaltCause::UserStop,
            class: None,
        });
        assert!(stop.contains("不會從頭翻"), "{stop}");
    }
}
