//! 批次執行（工作池、重試與停止判斷）。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

// ═══ 批次執行 ═════════════════════════════════════════════════

pub(super) fn run_batches(
    engine: &Engine,
    batch_state: &mut BatchRuntimeState,
    items: &[MaskedItem],
    base_pct: u8,
    span_pct: u8,
    on_progress: &mut dyn FnMut(u8, &str),
    strict_single: bool,
) -> Result<HashMap<usize, String>, String> {
    run_batches_with(engine, batch_state, items, base_pct, span_pct, on_progress, strict_single, None)
}

/// 送出前等太久沒有進展時，「連線中斷」最多等多久才讓 AI 停下（待使用者實測）。
pub(super) const OUTAGE_BUDGET: Duration = if cfg!(test) {
    Duration::from_secs(8)
} else {
    Duration::from_secs(300)
};

/// 審查 4c：同一批（含拆出來的子批）最多耗多久，超過就先列為沒回應、換下一批（待使用者實測）。
pub(super) const BATCH_WAIT_CAP: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(300)
};

/// 這個錯誤是「連線／伺服器暫時不行」嗎？是的話等它恢復，不消耗重排額度。
pub(super) fn is_outage_error(err: &ChunkError) -> bool {
    matches!(err.kind, ChatErrorKind::Transient)
        && matches!(
            retry_policy::classify_message(&err.message),
            retry_policy::FailureClass::Network
                | retry_policy::FailureClass::Timeout
                | retry_policy::FailureClass::ServerBusy
                | retry_policy::FailureClass::RateLimited
        )
}

/// 這個錯誤要不要把整批拆半再送：內容太長、輸出被截斷、空回應、JSON 壞掉、回應對不上。
pub(super) fn wants_split(err: &ChunkError) -> bool {
    matches!(err.kind, ChatErrorKind::TooLarge | ChatErrorKind::LocalTimeout)
        || is_empty_response_error(&err.message)
        || err.message.contains("回傳格式不對")
        || err.message.contains("批次回應與送出內容對不上")
}

/// 去掉 run_batches 加的「第 N 批失敗：」前綴。
pub(super) fn strip_batch_prefix(message: &str) -> &str {
    match (message.strip_prefix("第 "), message.find("批失敗：")) {
        (Some(_), Some(at)) => &message[at + "批失敗：".len()..],
        _ => message,
    }
}

/// 這個錯誤代表 AI 這一輪不能再用了嗎？回白話原因（額度、金鑰、登入、本地程式消失）。
pub(super) fn stop_reason_for(err: &ChunkError) -> Option<String> {
    if is_cancel_message(&err.message) {
        return None;
    }
    match err.kind {
        // 給使用者看的原因：拿掉「第 N 批失敗：」包裝（那是批次紀錄用的）
        ChatErrorKind::ProcessGone | ChatErrorKind::LocalUnusable => {
            return Some(strip_batch_prefix(&err.message).to_string())
        }
        ChatErrorKind::Relogin => return Some(auth_relogin_message()),
        ChatErrorKind::Quota => return Some(ai_quota_support_message(&err.message)),
        _ => {}
    }
    if matches!(err.kind, ChatErrorKind::Fatal) && looks_like_quota_or_auth_error(&err.message) {
        return Some(ai_quota_support_message(&err.message));
    }
    if matches!(err.kind, ChatErrorKind::Fatal) && is_auth_relogin_error(&err.message) {
        return Some(auth_relogin_message());
    }
    None
}

/// 分批送出，回傳 uid → 譯文（未經佔位符把關的原始結果）。
///
/// B4：改成**持續補位的工作池**——同時最多 `current_parallel` 批在路上，
/// 哪一批先回來就馬上補下一批，不再「一組全部回來才送下一組」（舊版一條慢的就讓其他位置閒著）。
///
/// `on_batch`：每一批成功回來時在主執行緒呼叫一次（resolve_unique 用它把譯文逐批寫進翻譯記憶，
/// 程式中途被關掉也不會整段白翻）。
///
/// 提早結束的原因寫在 `batch_state.stop`：使用者停止、AI 不能再用（額度／金鑰／本地程式消失）、
/// 連線中斷太久、或連續很多批都沒有新譯文。已收到的譯文一律回傳，不丟。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_batches_with(
    engine: &Engine,
    batch_state: &mut BatchRuntimeState,
    items: &[MaskedItem],
    base_pct: u8,
    span_pct: u8,
    on_progress: &mut dyn FnMut(u8, &str),
    strict_single: bool,
    mut on_batch: Option<&mut dyn FnMut(&HashMap<usize, String>)>,
) -> Result<HashMap<usize, String>, String> {
    let plans = plan_track_batches_for(
        items,
        strict_single,
        matches!(engine.provider, AiProvider::LocalLlm),
    );
    let plans = shrink_plans(plans, batch_state.batch_divisor);
    let total_batches = plans.len().max(1);
    let total_unique = items.len();

    let mut translations: HashMap<usize, String> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();
    let mut finished_batches = 0usize;
    let mut retried_batches = 0usize;
    let mut retry_attempts = 0usize;
    let mut failed_batches = 0usize;
    let mut next_batch_no = 1usize;
    let phase_start = Instant::now();
    // 沒有新譯文的連續失敗數、連線中斷從何時開始
    let mut no_progress_streak = 0usize;
    let mut outage_since: Option<Instant> = None;
    let mut successes_since_adjust = 0usize;
    let mut completions_since_throttle = usize::MAX;
    let mut stop: Option<BatchStop> = None;

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

    let (tx, rx) = mpsc::channel::<(QueuedPlan, Result<ChunkSuccess, ChunkError>)>();
    let mut handles: Vec<thread::JoinHandle<()>> = Vec::new();
    let mut in_flight = 0usize;
    // 審查 4c：每一批（依批號，拆出來的子批共用）第一次送出的時間
    let mut batch_started: HashMap<usize, Instant> = HashMap::new();
    let mut wait_t0 = Instant::now();
    // 斷線等待：下一批最早什麼時候可以送
    let mut resume_at: Option<Instant> = None;

    loop {
        if stop.is_none() && cancel::is_cancelled() {
            stop = Some(BatchStop::Cancelled);
        }
        // 補位：有空位、還有批、沒有要停、不在斷線等待中，就立刻送下一批
        let parallel = batch_state.current_parallel.max(1).min(batch_state.parallel_cap);
        let waiting_for_link = resume_at.is_some_and(|at| Instant::now() < at);
        while stop.is_none() && !waiting_for_link && in_flight < parallel {
            let Some(queued) = pending.pop_front() else { break };
            let engine = engine.clone();
            let prompt = Arc::clone(&batch_state.system_prompt);
            let tx = tx.clone();
            in_flight += 1;
            batch_started.entry(queued.batch_no).or_insert_with(Instant::now);
            handles.push(thread::spawn(move || {
                let batch_no = queued.batch_no;
                let result = translate_chunk(&engine, &prompt, &queued.plan).map_err(|err| ChunkError {
                    message: if is_cancel_message(&err.message) {
                        err.message
                    } else {
                        format!("第 {batch_no} 批失敗：{}", sanitize_provider_name(&err.message))
                    },
                    congestion: err.congestion,
                    retries: err.retries,
                    kind: err.kind,
                });
                let _ = tx.send((queued, result));
            }));
            wait_t0 = Instant::now();
        }
        if in_flight == 0 {
            if stop.is_some() || pending.is_empty() {
                break;
            }
            // 只剩斷線等待：睡到可以送為止（可停止）
            if let Some(at) = resume_at {
                let now = Instant::now();
                if now < at {
                    thread::sleep((at - now).min(Duration::from_millis(250)));
                }
                if Instant::now() >= at {
                    resume_at = None;
                }
                continue;
            }
        }

        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok((queued, outcome)) => {
                in_flight = in_flight.saturating_sub(1);
                finished_batches += 1;
                completions_since_throttle = completions_since_throttle.saturating_add(1);
                match outcome {
                    Ok(success) => {
                        if success.retries > 0 {
                            retried_batches += 1;
                            retry_attempts += success.retries;
                        }
                        let fresh: HashMap<usize, String> = success
                            .map
                            .into_iter()
                            .filter(|(_, t)| !t.trim().is_empty())
                            .collect();
                        if fresh.is_empty() {
                            // 成功但空 map：可恢復則拆半重排
                            no_progress_streak += 1;
                            if queued.requeues < MAX_PLAN_REQUEUE {
                                push_requeue_plans_front(&mut pending, queued, Some(false));
                            }
                        } else {
                            no_progress_streak = 0;
                            outage_since = None;
                            if let Some(sink) = on_batch.as_mut() {
                                sink(&fresh);
                            }
                            translations.extend(fresh);
                            // 加速：累積「一個並行數」的成功批才加一次，避免一下子衝太快
                            successes_since_adjust += 1;
                            if successes_since_adjust >= batch_state.current_parallel.max(1) {
                                successes_since_adjust = 0;
                                batch_state.finish_round(false, true);
                            }
                        }
                    }
                    Err(err) => {
                        failed_batches += 1;
                        retry_attempts += err.retries;
                        if err.retries > 0 {
                            retried_batches += 1;
                        }
                        // 審查 1a：本地模型逾時不算「沒有進展」——慢機器連續逾時是正常的，
                        // 拆小之後就會回來；算進去會讓整輪被誤判停掉
                        if !matches!(err.kind, ChatErrorKind::LocalTimeout) {
                            no_progress_streak += 1;
                        }
                        // 審查 4c：同一批（含拆出來的）總共等超過上限，先列為沒回應，換下一批
                        // （本地模型逾時不適用：那由拆批層數控制，等待上限本身可能就超過這個時間）
                        let over_wait_cap = !matches!(err.kind, ChatErrorKind::LocalTimeout)
                            && batch_started
                                .get(&queued.batch_no)
                                .is_some_and(|t| t.elapsed() >= BATCH_WAIT_CAP);
                        let detail = truncate_err_msg(&err.message, 220);
                        errors.push(err.message.clone());
                        on_progress(pct(finished_batches, translations.len()), &detail);

                        if is_cancel_message(&err.message) {
                            stop = Some(BatchStop::Cancelled);
                        } else if let Some(reason) = stop_reason_for(&err) {
                            crate::dev_log!("ai", "AI 停下：{}", reason.lines().next().unwrap_or(""));
                            stop = Some(BatchStop::AiUnavailable(reason));
                            // 還沒送出的這一批放回佇列，讓呼叫端知道它沒翻到
                            pending.push_front(queued);
                        } else if over_wait_cap {
                            let note = format!(
                                "第 {} 批已經等了超過 {} 秒仍沒成功，先列為「沒回應」（之後接續補完會再翻），繼續下一批",
                                queued.batch_no,
                                BATCH_WAIT_CAP.as_secs()
                            );
                            crate::dev_log!("ai", "{note}");
                            on_progress(pct(finished_batches, translations.len()), &note);
                            resume_at = None;
                        } else if is_outage_error(&err) {
                            // 連線中斷／伺服器忙：等它恢復，不消耗重排額度
                            let since = *outage_since.get_or_insert_with(Instant::now);
                            if since.elapsed() >= OUTAGE_BUDGET {
                                stop = Some(BatchStop::AiUnavailable(format!(
                                    "AI 連線中斷超過 {} 秒仍未恢復（最後一次：{}）。已翻好的部分都會保留；\
網路恢復後按「接續補完」，會從停下的地方繼續。",
                                    OUTAGE_BUDGET.as_secs(),
                                    truncate_err_msg(&err.message, 80)
                                )));
                                pending.push_front(queued);
                            } else {
                                pending.push_front(queued);
                                // 降到 1 條慢慢試；並依連續失敗次數退避
                                if completions_since_throttle >= batch_state.current_parallel.max(1) {
                                    let old = batch_state.current_parallel;
                                    batch_state.finish_round(true, false);
                                    completions_since_throttle = 0;
                                    if batch_state.current_parallel < old {
                                        on_progress(
                                            pct(finished_batches, translations.len()),
                                            &format!(
                                                "AI 限流：並行 {}→{}，已降速並重送失敗批",
                                                old, batch_state.current_parallel
                                            ),
                                        );
                                    }
                                }
                                let wait = retry_policy::backoff_delay(
                                    no_progress_streak.min(8),
                                    None,
                                    retry_policy::jitter_seed(),
                                );
                                resume_at = Some(Instant::now() + wait);
                                on_progress(
                                    pct(finished_batches, translations.len()),
                                    &format!(
                                        "AI 連線中斷或忙碌，等待恢復並自動重試（已等 {} 秒，最多等 {} 秒；已翻好的不會遺失）…",
                                        since.elapsed().as_secs(),
                                        OUTAGE_BUDGET.as_secs()
                                    ),
                                );
                            }
                        } else if wants_split(&err) {
                            // 太長／截斷／格式壞：拆小再送（單條拆不動就再試一次）
                            let limit = if queued.plan.items.len() > 1 {
                                MAX_SPLIT_DEPTH
                            } else {
                                MAX_PLAN_REQUEUE.min(1)
                            };
                            if queued.requeues < limit {
                                let force = matches!(err.kind, ChatErrorKind::TooLarge | ChatErrorKind::LocalTimeout)
                                    || err.message.contains("finish_reason=length");
                                push_requeue_plans_front(&mut pending, queued, Some(force));
                            }
                        } else if (matches!(err.kind, ChatErrorKind::Transient)
                            || (err.congestion && is_recoverable_batch_error(&err.message)))
                            && queued.requeues < MAX_PLAN_REQUEUE
                        {
                            if should_mark_round_congestion(err.congestion)
                                && completions_since_throttle >= batch_state.current_parallel.max(1)
                            {
                                batch_state.finish_round(true, false);
                                completions_since_throttle = 0;
                            }
                            push_requeue_plans_front(&mut pending, queued, None);
                        }
                        // 其他（Fatal 且不是額度／金鑰）：這批放棄，句子留在缺口讓之後補
                    }
                }
                let got = translations.len();
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
                        if failed_batches > 0 { " · 本輪含批失敗" } else { "" }
                    ),
                );
                // 連續很多批都沒有新譯文（而且不是在等連線恢復）：提前結束，保留已得
                let streak_limit = EMPTY_ROUNDS_ABORT * batch_state.parallel_cap.max(1);
                if stop.is_none() && outage_since.is_none() && no_progress_streak >= streak_limit.max(EMPTY_ROUNDS_ABORT) {
                    stop = Some(BatchStop::NoProgress(format!(
                        "AI 連續 {no_progress_streak} 批{}，提前結束；已保留已成功譯文（最後一次：{}）",
                        super::super::run_interrupt::NO_PROGRESS_MARK,
                        truncate_err_msg(errors.last().map(String::as_str).unwrap_or("沒有回應"), 80)
                    )));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let got = translations.len();
                let wait_secs = wait_t0.elapsed().as_secs();
                flush_notices(engine, pct(finished_batches, got), on_progress);
                if wait_secs == 0 || wait_secs % 10 == 0 {
                    let usage = engine.usage_snapshot();
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
    drop(tx);
    for handle in handles {
        let _ = handle.join();
    }

    let out = translations;
    if let Some(stop) = &stop {
        let note = match stop {
            BatchStop::Cancelled => "已依你的要求停止；已保留本階段已成功譯文。".to_string(),
            BatchStop::AiUnavailable(reason) => format!(
                "AI 提前結束（{}）；已保留已成功譯文。",
                reason.lines().next().unwrap_or("連線問題")
            ),
            BatchStop::NoProgress(reason) => reason.clone(),
        };
        engine.push_notice(note);
    }

    if !errors.is_empty() {
        let mut seen = HashSet::new();
        let mut summary = Vec::new();
        for e in &errors {
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
            errors.len(),
            summary.len()
        ));
    }

    flush_notices(engine, pct(total_batches, out.len()), on_progress);
    // 呼叫端靠這個知道「AI 這一輪沒跑完」，不能再把它當成完成
    if stop.is_some() {
        batch_state.stop = stop.clone();
    }

    if out.is_empty() {
        if let Some(stop) = stop {
            return Err(match stop {
                BatchStop::Cancelled => CANCEL_MESSAGE.to_string(),
                BatchStop::AiUnavailable(reason) | BatchStop::NoProgress(reason) => reason,
            });
        }
        if !errors.is_empty() {
            let detail = errors.last().cloned().unwrap_or_else(|| "全部請求都沒有回應".into());
            if let Some(classified) = classify_batch_abort(&errors) {
                return Err(classified);
            }
            return Err(detail);
        }
        return Ok(out);
    }
    if !errors.is_empty() {
        on_progress(
            pct(total_batches, out.len()),
            &format!(
                "AI 有 {} 批失敗（其餘已完成）；失敗批已重試或略過，可稍後補翻未完成句",
                errors.len()
            ),
        );
    }
    Ok(out)
}
