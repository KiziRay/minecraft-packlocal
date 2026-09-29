//! 單批請求的等待上限、本地模型狀態與速度。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

// ═══ 單批請求 ═════════════════════════════════════════════════

/// 測試用：依伺服器位址縮短單批等待上限（模擬「本地模型等不到」）。
/// 用位址當鍵：批次在工作執行緒上送出，平行測試各用各的假伺服器、互不干擾。
#[cfg(test)]
pub(super) fn test_request_timeouts() -> &'static Mutex<HashMap<String, Duration>> {
    static MAP: OnceLock<Mutex<HashMap<String, Duration>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
pub(super) fn set_test_request_timeout_for(url: &str, timeout: Option<Duration>) {
    let mut map = retry_policy::lock_or_recover(test_request_timeouts());
    match timeout {
        Some(t) => map.insert(url.to_string(), t),
        None => map.remove(url),
    };
}

/// B4：這一批最多等多久。雲端照軌道；本地依批次大小、同時處理數與實測速度估（local_llm/sizing.rs）。
pub(super) fn request_timeout_for(engine: &Engine, plan: &BatchPlan, max_tokens: usize) -> Duration {
    #[cfg(test)]
    if let Some(t) = retry_policy::lock_or_recover(test_request_timeouts()).get(engine.url.as_str()) {
        return *t;
    }
    if !matches!(engine.provider, AiProvider::LocalLlm) {
        return Duration::from_secs(plan.track.timeout_secs());
    }
    let state = crate::engine::local_llm::load_state();
    Duration::from_secs(crate::engine::local_llm::sizing::request_timeout_secs(
        plan.items.len(),
        max_tokens,
        local_parallel_slots(engine, &state),
        crate::engine::local_llm::sizing::assumed_tps(state.ngl, state.layers),
        crate::engine::local_llm::sizing::observed_speed(),
        retry_policy::lock_or_recover(&engine.local_timeouts).stretch(),
    ))
}

/// 本地模型實際的同時處理數：啟動時記在狀態裡（記憶體不夠時可能降成 1），送出端不超過它。
pub(super) fn local_parallel_slots(engine: &Engine, state: &crate::engine::local_llm::ServerStateView) -> u32 {
    let configured = engine.capabilities.start_parallel.max(1) as u32;
    if state.slots > 0 {
        configured.min(state.slots)
    } else {
        configured
    }
}

/// B4：自動重試時給使用者看的一句話（進度區會顯示「重試中」）。
pub(super) fn notify_auto_retry(engine: &Engine, class: retry_policy::FailureClass, attempt: usize, wait: Duration) {
    engine.push_notice(format!(
        "AI {}，自動重試中（第 {} 次，等 {:.1} 秒）…",
        class.label_zh(),
        attempt,
        wait.as_secs_f64()
    ));
}

/// B4：本地模型程式不在了嗎？在就回 `None`；不在回白話說明。
///
/// 只認「這個工具自己啟動、而且確定已經結束」的程式；不是這次啟動的（無從判斷）
/// 就回 `None`，交給一般重試流程，重試完仍連不上才算不在（見 [`local_not_running_message`]）。
pub(super) fn local_process_gone_message(context: Option<&str>) -> Option<String> {
    match crate::engine::local_llm::own_server_liveness() {
        crate::engine::local_llm::OwnServerLiveness::Exited(code) => Some(format!(
            "{}{mark}（{code}），通常是這台電腦的記憶體不足。已翻好的部分都會保留；\
關閉其他程式後按「接續補完」，工具會重新啟動模型並從停下的地方繼續。",
            context.map(|c| format!("{c}：")).unwrap_or_default(),
            mark = super::super::run_interrupt::LOCAL_GONE_MARK
        )),
        _ => None,
    }
}

pub(super) fn local_not_running_message() -> String {
    // 開頭字樣＝run_interrupt::LOCAL_UNREACHABLE_MARK（cause_for 認這個）
    format!(
        "{}：它沒有在執行（可能已經當掉或被關閉）。已翻好的部分都會保留；\
按「接續補完」會重新啟動模型並從停下的地方繼續。",
        super::super::run_interrupt::LOCAL_UNREACHABLE_MARK
    )
}

/// B4：從 llama-server 的回應讀出實際速度（`timings.predicted_per_second`），用來調整等待上限；
/// 速度第一次量到、或變化很大時寫一行給使用者看。
pub(super) fn note_local_speed(engine: &Engine, response: &Value) {
    let Some(tps) = response
        .get("timings")
        .and_then(|t| t.get("predicted_per_second"))
        .and_then(|v| v.as_f64())
    else {
        return;
    };
    let before = crate::engine::local_llm::sizing::observed_speed();
    crate::engine::local_llm::sizing::record_speed(tps);
    let after = crate::engine::local_llm::sizing::observed_speed().unwrap_or(tps);
    let changed = match before {
        None => true,
        Some(prev) => (after - prev).abs() > prev * 0.3,
    };
    if changed {
        engine.push_notice(format!(
            "本地模型實測速度：每秒約 {after:.1} 個字詞（等待上限會依這個速度調整）"
        ));
    }
}
