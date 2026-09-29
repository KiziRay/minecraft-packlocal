//! 單批請求（translate_chunk）。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

pub(super) fn translate_chunk(
    engine: &Engine,
    system_prompt: &Arc<String>,
    plan: &BatchPlan,
) -> Result<ChunkSuccess, ChunkError> {
    let payload = build_user_payload(&plan.items);
    let max_tokens = clamp_completion_tokens_for(
        payload.chars().count(),
        plan.items.len(),
        matches!(engine.provider, AiProvider::LocalLlm),
    );
    let wanted: HashSet<usize> = plan.items.iter().map(|item| item.uid).collect();
    let is_local = matches!(engine.provider, AiProvider::LocalLlm);
    // B4：本地模型的等待上限依批次大小與實測速度估（不再寫死 60／90 秒）
    let timeout = request_timeout_for(engine, plan, max_tokens);
    let mut attempts = 0usize;
    let mut length_retries = 0usize;
    loop {
        if cancel::is_cancelled() {
            return Err(ChunkError {
                message: CANCEL_MESSAGE.to_string(),
                congestion: false,
                retries: attempts,
                kind: ChatErrorKind::Fatal,
            });
        }
        let features = engine.current_features();
        crate::dev_log!(
            "ai",
            "送出批次 track={:?} 條數={} 首句={:?} 上限tokens={} 嘗試={}",
            plan.track,
            plan.items.len(),
            plan.items.first().map(|i| i.source.chars().take(48).collect::<String>()),
            max_tokens,
            attempts
        );
        let mut body = json!({
            "model": engine.model.as_str(),
            "messages": [
                {"role": "system", "content": system_prompt.as_str()},
                {"role": "user", "content": payload.as_str()}
            ]
        });
        if features.send_temperature {
            body["temperature"] = json!(if plan.items.len() == 1 { 0.0 } else { 0.1 });
        }
        if matches!(engine.provider, AiProvider::LocalLlm) {
            // 重複懲罰是本地小模型最有效的一道防線：低溫取樣容易卡在同一個 token 上，
            // 吐出「造成傷害傷害傷害傷害」這種輸出。`local_quality.rs` 會在事後抓到它，
            // 但在源頭少發生一次，就少一次重翻。
            // 這兩個是 llama.cpp 的取樣參數（我們自己安裝的 llama-server 一定支援），
            // 只送給本地端，不影響任何雲端服務。
            body["repeat_penalty"] = json!(1.15);
            body["repeat_last_n"] = json!(128);
        }
        if features.send_response_format {
            body["response_format"] = json!({ "type": "json_object" });
        }
        if should_send_thinking_disabled(engine) {
            // https://api-docs.deepseek.com/zh-cn/guides/thinking_mode — 預設開啟；批次翻譯關閉
            body["thinking"] = json!({ "type": "disabled" });
        }
        if should_disable_local_thinking(engine) {
            // 現場實測過：不關會把 max_tokens 燒在思考過程上，content 留空、
            // finish_reason=length；送 "Iron Sword" 給 32 tokens 全被 reasoning_content
            // 吃光。關掉後同一句話 3 個 token 就答完，且是正確答案「鐵劍」；真實批次
            // （4 條混雜句子）也驗證過完整正確、finish_reason=stop。翻譯不需要推理過程。
            body["chat_template_kwargs"] = json!({ "enable_thinking": false });
        }
        match features.max_tokens_field {
            MaxTokensField::MaxTokens => body["max_tokens"] = json!(max_tokens),
            MaxTokensField::MaxCompletionTokens => {
                body["max_completion_tokens"] = json!(max_tokens)
            }
        }

        let response_json: Value = if matches!(engine.provider, AiProvider::Codex) {
            match codex_chat::complete_chat(
                engine.client.as_ref(),
                engine.model.as_str(),
                &body,
                timeout,
            ) {
                Ok(value) => value,
                Err(error) => {
                    let code = error.status_code.unwrap_or(0);
                    let clean = sanitize_provider_name(&error.message);
                    // B4：只看當下這個錯誤分類；額度用完不重送（重送只會空轉）
                    let class = if code == 0 || code == 200 {
                        retry_policy::classify_message(&clean)
                    } else {
                        retry_policy::classify_http(code, &clean)
                    };
                    crate::dev_log!(
                        "gpt",
                        "GPT 翻譯端點回應失敗 | HTTP={} | 分類={} | 模型={} | 嘗試={}",
                        code,
                        class.label_zh(),
                        engine.model,
                        attempts
                    );
                    let congestion = matches!(
                        class,
                        retry_policy::FailureClass::RateLimited
                            | retry_policy::FailureClass::ServerBusy
                            | retry_policy::FailureClass::Timeout
                    );
                    if class.should_split() {
                        return Err(ChunkError {
                            message: format!("內容太長，改拆小重送：{clean}"),
                            congestion: false,
                            retries: attempts,
                            kind: ChatErrorKind::TooLarge,
                        });
                    }
                    let retryable = !class.stops_ai()
                        && (class.retry_same_batch() || code == 0 || code == 200);
                    if retryable && attempts < retry_policy::MAX_TRANSIENT_RETRIES {
                        let wait = retry_policy::backoff_delay(
                            attempts,
                            error.retry_after,
                            retry_policy::jitter_seed(),
                        );
                        attempts += 1;
                        notify_auto_retry(engine, class, attempts, wait);
                        thread::sleep(wait);
                        continue;
                    }
                    let kind = if class == retry_policy::FailureClass::QuotaExhausted || code == 402 {
                        ChatErrorKind::Quota
                    } else if class == retry_policy::FailureClass::AuthInvalid || code == 401 || code == 403 {
                        ChatErrorKind::Fatal
                    } else if retryable {
                        ChatErrorKind::Transient
                    } else {
                        ChatErrorKind::Fatal
                    };
                    return Err(ChunkError {
                        message: clean,
                        congestion,
                        retries: attempts,
                        kind,
                    });
                }
            }
        } else {
            let mut req = engine
                .client
                .post(engine.url.as_str())
                .header("Content-Type", "application/json")
                .timeout(timeout)
                .json(&body);
            if engine.managed {
                req = req
                    .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
                    .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
                    .header("X-Zeitfrei-Session", engine.session_cookie());
            } else if !engine.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", engine.api_key));
            }

            let resp = match req.send() {
                Ok(resp) => resp,
                Err(err) => {
                    let is_timeout = err.is_timeout();
                    // B4：本地模型連不上時先看程式還在不在——當掉（多半是記憶體不足）
                    // 當下就知道，不必等逾時、也不必重送。
                    if is_local && !is_timeout {
                        if let Some(gone) = local_process_gone_message(None) {
                            crate::dev_log!("local", "本地模型程式已經不在：{gone}");
                            return Err(ChunkError {
                                message: gone,
                                congestion: false,
                                retries: attempts,
                                kind: ChatErrorKind::ProcessGone,
                            });
                        }
                    }
                    if is_local && is_timeout {
                        // 本地模型等不到＝這批對這台電腦太大：拆小比原樣重送有用。
                        // 審查 1a：同時把之後的等待上限拉長（×1.5，最多 900 秒），並寫給使用者看。
                        // 審查 1b：伺服器端那個請求不一定跟著取消（可能還在算），寫明等了多久，方便對照 CPU 使用率。
                        let verdict = retry_policy::lock_or_recover(&engine.local_timeouts).on_timeout(timeout);
                        let stretch = match verdict {
                            crate::engine::local_llm::timeouts::TimeoutVerdict::Continue { stretch } => stretch,
                            // 審查 F1：卡住的本地模型不要一路拆批等好幾個小時——停下並說明
                            crate::engine::local_llm::timeouts::TimeoutVerdict::GiveUp(message) => {
                                crate::dev_log!("local", "本地模型逾時太多次，停下：{message}");
                                return Err(ChunkError {
                                    message,
                                    congestion: false,
                                    retries: attempts,
                                    kind: ChatErrorKind::LocalUnusable,
                                });
                            }
                        };
                        crate::dev_log!(
                            "local",
                            "本地模型逾時 | 批次 {} 條 | 等待上限 {} 秒 | 嘗試 {} | 之後倍數 {:.2}",
                            plan.items.len(),
                            timeout.as_secs(),
                            attempts,
                            stretch
                        );
                        engine.push_notice(format!(
                            "本地模型這批 {} 條等了 {:.0} 秒還沒回應，改拆小重送；之後每批的等待上限拉長為原本的 {:.1} 倍（最多 900 秒）",
                            plan.items.len(),
                            timeout.as_secs_f64(),
                            stretch
                        ));
                        return Err(ChunkError {
                            message: format!(
                                "本地模型回應太慢（這批 {} 條等了 {:.0} 秒），改拆小重送",
                                plan.items.len(),
                                timeout.as_secs_f64()
                            ),
                            congestion: false,
                            retries: attempts,
                            kind: ChatErrorKind::LocalTimeout,
                        });
                    }
                    let err_msg = if is_timeout {
                        "等待 AI 回應逾時".into()
                    } else {
                        format!("連線失敗（無回應）：{err}")
                    };
                    let class = if is_timeout {
                        retry_policy::FailureClass::Timeout
                    } else {
                        retry_policy::FailureClass::Network
                    };
                    if attempts < retry_policy::MAX_TRANSIENT_RETRIES {
                        let wait = retry_policy::backoff_delay(attempts, None, retry_policy::jitter_seed());
                        attempts += 1;
                        notify_auto_retry(engine, class, attempts, wait);
                        thread::sleep(wait);
                        continue;
                    }
                    if is_local && err.is_connect() {
                        let gone = local_process_gone_message(Some(super::super::run_interrupt::LOCAL_UNREACHABLE_MARK))
                            .unwrap_or_else(local_not_running_message);
                        return Err(ChunkError {
                            message: gone,
                            congestion: false,
                            retries: attempts,
                            kind: ChatErrorKind::ProcessGone,
                        });
                    }
                    return Err(ChunkError {
                        message: err_msg,
                        congestion: is_timeout,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
            };

            let status = resp.status();
            let code = status.as_u16();
            // B4：伺服器說要等多久就等多久（Retry-After／retry-after-ms）
            let retry_hint = retry_policy::parse_retry_after(
                resp.headers().get("retry-after").and_then(|v| v.to_str().ok()),
                std::time::SystemTime::now(),
            )
            .or_else(|| {
                retry_policy::parse_retry_after_ms(
                    resp.headers().get("retry-after-ms").and_then(|v| v.to_str().ok()),
                )
            });
            let body_text = resp.text().unwrap_or_default();
            // 本地模型的失敗診斷：實測有一批 8 條被送九次仍失敗，而當時的紀錄
            // 只寫「批失敗」，看不出模型到底回了什麼。依 rules/50 的升級梯，
            // 這一輪先收證據（HTTP 狀態、finish_reason、回應前 200 字），
            // 不憑猜測調 prompt 或參數。
            if is_local && !status.is_success() {
                crate::dev_log!(
                    "local",
                    "本地模型 HTTP {} | 批次 {} 條 | 嘗試 {} 次 | 上限tokens={} | 回應前 200 字={:?}",
                    code,
                    plan.items.len(),
                    attempts,
                    max_tokens,
                    body_text.chars().take(200).collect::<String>()
                );
            }
            if !status.is_success() {
                if engine.maybe_degrade_for_unsupported(code, &body_text) {
                    continue;
                }
                if let Some(mapped) = map_chat_http_error(code, &body_text, engine.managed) {
                    let congestion = matches!(mapped.kind, ChatErrorKind::Transient)
                        || code == 429
                        || code >= 500;
                    if matches!(mapped.kind, ChatErrorKind::Transient)
                        && attempts < retry_policy::MAX_TRANSIENT_RETRIES
                    {
                        let wait = retry_policy::backoff_delay(attempts, retry_hint, retry_policy::jitter_seed());
                        attempts += 1;
                        notify_auto_retry(engine, retry_policy::FailureClass::ServerBusy, attempts, wait);
                        thread::sleep(wait);
                        continue;
                    }
                    return Err(ChunkError {
                        message: mapped.message,
                        congestion,
                        retries: attempts,
                        kind: mapped.kind,
                    });
                }
                let snippet =
                    sanitize_provider_name(&body_text.chars().take(200).collect::<String>());
                let class = retry_policy::classify_http(code, &body_text);
                crate::dev_log!(
                    "ai",
                    "AI HTTP {} 分類為「{}」| 批次 {} 條 | 嘗試 {}",
                    code,
                    class.label_zh(),
                    plan.items.len(),
                    attempts
                );
                match class {
                    retry_policy::FailureClass::TooLarge => {
                        return Err(ChunkError {
                            message: format!("內容太長（{code}），改拆小重送：{snippet}"),
                            congestion: false,
                            retries: attempts,
                            kind: ChatErrorKind::TooLarge,
                        });
                    }
                    retry_policy::FailureClass::QuotaExhausted => {
                        return Err(ChunkError {
                            message: if code == 402 {
                                format!(
                                    "帳號餘額不足：{}",
                                    sanitize_provider_name(&body_text.chars().take(120).collect::<String>())
                                )
                            } else {
                                format!("額度可能已用完（{code}）：{snippet}")
                            },
                            congestion: false,
                            retries: attempts,
                            kind: ChatErrorKind::Quota,
                        });
                    }
                    retry_policy::FailureClass::AuthInvalid => {
                        return Err(ChunkError {
                            message: format!(
                                "金鑰無效或無權限：{}",
                                sanitize_provider_name(&body_text.chars().take(120).collect::<String>())
                            ),
                            congestion: false,
                            retries: attempts,
                            kind: ChatErrorKind::Fatal,
                        });
                    }
                    class if class.retry_same_batch() => {
                        let err_msg = if code == 429 {
                            "請求太頻繁，稍後再試".into()
                        } else if code == 503 {
                            "服務暫時無法使用（503），稍後再試".into()
                        } else {
                            format!("服務錯誤 {code}：{snippet}")
                        };
                        if attempts < retry_policy::MAX_TRANSIENT_RETRIES {
                            let wait = retry_policy::backoff_delay(attempts, retry_hint, retry_policy::jitter_seed());
                            attempts += 1;
                            notify_auto_retry(engine, class, attempts, wait);
                            thread::sleep(wait);
                            continue;
                        }
                        return Err(ChunkError {
                            message: err_msg,
                            congestion: true,
                            retries: attempts,
                            kind: ChatErrorKind::Transient,
                        });
                    }
                    _ => {
                        return Err(ChunkError {
                            message: format!("服務錯誤 {code}：{snippet}"),
                            congestion: false,
                            retries: attempts,
                            kind: ChatErrorKind::Fatal,
                        });
                    }
                }
            }

            match serde_json::from_str(&body_text) {
                Ok(value) => value,
                Err(err) => {
                    let err_msg = format!("回應無法解析（無有效內容）：{err}");
                    if attempts < 2 {
                        attempts += 1;
                        thread::sleep(Duration::from_millis(200));
                        continue;
                    }
                    return Err(ChunkError {
                        message: err_msg,
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
            }
        };
        if is_local {
            note_local_speed(engine, &response_json);
            // 有回應＝沒卡住：連續逾時歸零、等待倍數逐步回降
            retry_policy::lock_or_recover(&engine.local_timeouts).on_success();
        }
        engine.record_usage(&parse_usage_totals(&response_json));
        let content = extract_message_content(&response_json);
        if content.trim().is_empty() {
            let msg = diagnose_empty_choice(&response_json);
            if matches!(engine.provider, AiProvider::LocalLlm) {
                // 本地模型回空 content 的證據：光看「批失敗」看不出是被截斷、
                // 被拒答、還是 JSON 解析不出來。留下 finish_reason 與原始回應
                // 才有辦法對症——這一輪只收證據，不調參數。
                crate::dev_log!(
                    "local",
                    "本地模型回空 content | {} | 批次 {} 條 | 嘗試 {} 次 | 上限tokens={} | 首句={:?}",
                    msg,
                    plan.items.len(),
                    attempts,
                    max_tokens,
                    plan.items
                        .first()
                        .map(|i| i.source.chars().take(48).collect::<String>())
                );
            }
            if is_length_truncated(&response_json) {
                if length_retries < LENGTH_TRUNCATION_RETRY_LIMIT {
                    length_retries += 1;
                    attempts += 1;
                    engine.push_notice(format!(
                        "本批疑似輸出截斷（finish_reason=length），第 {length_retries} 次同尺寸重試…"
                    ));
                    thread::sleep(Duration::from_millis(empty_content_backoff_ms(attempts)));
                    continue;
                }
                engine.push_notice(
                    "本批疑似輸出截斷（finish_reason=length），改拆半或重排…".into(),
                );
                return Err(ChunkError {
                    message: msg,
                    congestion: false,
                    retries: attempts,
                    kind: ChatErrorKind::Transient,
                });
            }
            if attempts < EMPTY_CONTENT_RETRY_LIMIT {
                attempts += 1;
                engine.push_notice(format!(
                    "本批空回應，第 {attempts} 次重試…"
                ));
                thread::sleep(Duration::from_millis(empty_content_backoff_ms(attempts)));
                continue;
            }
            return Err(ChunkError {
                message: msg,
                congestion: false,
                retries: attempts,
                kind: ChatErrorKind::Transient,
            });
        }

        let requested_ids: Vec<usize> = wanted.iter().copied().collect();
        match parse_translation_object_audited(&content, &requested_ids) {
            Ok((map, audit)) => {
                // 對帳結果決定這批可不可信（Phase 3）。
                // 模型自己編出來的 id、或同一個 id 給了兩種不同譯文，代表它沒有照
                // 格式走；此時整批的其他答案也不值得採信。少數幾筆沒回來則屬常態，
                // 交給下一輪補——兩者的處理方式必須不同，混在一起就會把
                // 「模型亂回」靜默降級成「這次少翻了幾句」。
                if audit.needs_diagnostic_split() {
                    engine.push_notice(format!("{}；改拆半重試…", audit.summary()));
                    return Err(ChunkError {
                        message: format!("批次回應與送出內容對不上（{}）", audit.summary()),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
                // 沒到要拆半的程度，但也不是乾淨的一批：把差額講出來。
                // 這些沒回來的句子會留在待補清單裡，使用者看得到原因，
                // 而不是事後發現「明明說翻完了卻少了幾句」。
                if !audit.is_clean() {
                    engine.push_notice(audit.summary());
                }
                let filtered = map
                    .into_iter()
                    .filter(|(uid, translated)| wanted.contains(uid) && !translated.trim().is_empty())
                    .collect::<HashMap<_, _>>();
                if !filtered.is_empty() {
                    return Ok(ChunkSuccess {
                        map: filtered,
                        retries: attempts,
                    });
                }
                if is_length_truncated(&response_json) {
                    if length_retries < LENGTH_TRUNCATION_RETRY_LIMIT {
                        length_retries += 1;
                        attempts += 1;
                        engine.push_notice(format!(
                            "本批截斷後無譯文（finish_reason=length），第 {length_retries} 次同尺寸重試…"
                        ));
                        thread::sleep(Duration::from_millis(empty_content_backoff_ms(attempts)));
                        continue;
                    }
                    engine.push_notice(
                        "本批截斷後無譯文（finish_reason=length），改拆半或重排…".into(),
                    );
                    return Err(ChunkError {
                        message: diagnose_empty_choice(&response_json),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
                if attempts < EMPTY_CONTENT_RETRY_LIMIT {
                    attempts += 1;
                    engine.push_notice(format!(
                        "本批回傳空 JSON，第 {attempts} 次重試…"
                    ));
                    thread::sleep(Duration::from_millis(empty_content_backoff_ms(attempts)));
                    continue;
                }
                return Err(ChunkError {
                    message: "回傳為空 JSON 物件（無譯文）".into(),
                    congestion: false,
                    retries: attempts,
                    kind: ChatErrorKind::Transient,
                });
            }
            Err(err) => {
                if is_length_truncated(&response_json) {
                    engine.push_notice(
                        "本批疑似截斷導致 JSON 不完整，改拆半或重排…".into(),
                    );
                    return Err(ChunkError {
                        message: format!("{err}（finish_reason=length）"),
                        congestion: false,
                        retries: attempts,
                        kind: ChatErrorKind::Transient,
                    });
                }
                if attempts < 2 {
                    attempts += 1;
                    thread::sleep(Duration::from_millis(150));
                    continue;
                }
                return Err(ChunkError {
                    message: err,
                    congestion: false,
                    retries: attempts,
                    kind: ChatErrorKind::Transient,
                });
            }
        }
    }
}
