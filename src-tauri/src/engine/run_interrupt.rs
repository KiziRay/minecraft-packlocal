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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Halt {
    pub kind: HaltKind,
    /// 給使用者看的白話原因
    pub reason: String,
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
    store::set(Some(Halt {
        kind: HaltKind::AiUnavailable,
        reason: reason.trim().to_string(),
    }));
}

/// 使用者按了停止（覆蓋「AI 不可用」：停止要優先處理）。
pub fn stop_by_user(reason: &str) {
    store::set(Some(Halt {
        kind: HaltKind::UserStop,
        reason: reason.trim().to_string(),
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
        HaltKind::UserStop => "已依你的要求停止翻譯。已翻好的部分都有寫出並裝進遊戲；\
之後按「接續補完」，會從停下的地方繼續，不會從頭翻。"
            .to_string(),
        HaltKind::AiUnavailable => format!(
            "AI 在這一輪中途停下（{first}）。已翻好的部分都保留，並照樣寫出、裝進遊戲；\
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

    #[test]
    fn notes_say_what_happened_and_what_to_do_next() {
        let note = player_note(&Halt {
            kind: HaltKind::AiUnavailable,
            reason: "帳號餘額不足\n細節".into(),
        });
        assert!(note.contains("帳號餘額不足") && note.contains("接續補完") && note.contains("裝進遊戲"), "{note}");
        let stop = player_note(&Halt {
            kind: HaltKind::UserStop,
            reason: String::new(),
        });
        assert!(stop.contains("不會從頭翻"), "{stop}");
    }
}
