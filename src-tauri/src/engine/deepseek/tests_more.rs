//! deepseek 單元測試後半段（T1 自 tests.rs 拆出）。

use super::*;

#[test]
fn track_batching_is_deterministic() {
    let long_story = "A".repeat(260);
    let solo_story = "B".repeat(2201);
    let items_a = vec![
        masked(4, &solo_story, Some("任務文字")),
        masked(1, "Diamond Sword", Some("物品名")),
        masked(3, &long_story, Some("任務文字")),
        masked(2, "Open Quest Book", Some("介面文字")),
    ];
    let items_b = vec![
        items_a[2].clone(),
        items_a[0].clone(),
        items_a[3].clone(),
        items_a[1].clone(),
    ];
    let summary = |plans: Vec<BatchPlan>| {
        plans
            .into_iter()
            .map(|plan| {
                (
                    plan.track,
                    plan.items.iter().map(|item| item.uid).collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(summary(plan_track_batches_for(&items_a, false, false)), summary(plan_track_batches_for(&items_b, false, false)));
}

#[test]
fn max_tokens_are_clamped() {
    assert_eq!(clamp_completion_tokens_for(10, 1, false), 512);
    assert_eq!(clamp_completion_tokens_for(1000, 1, false), 800);
    assert_eq!(clamp_completion_tokens_for(20_000, 1, false), 8192);
    // 多條時依條數提高下限（48 * 40 = 1920）
    assert_eq!(clamp_completion_tokens_for(10, 40, false), 1920);
}

#[test]
fn name_ui_batch_size_is_48() {
    assert_eq!(BatchTrackKind::Name.batch_size_for(false, false), 48);
    assert_eq!(BatchTrackKind::Ui.batch_size_for(false, false), 48);
    assert_eq!(BatchTrackKind::Story.batch_size_for(false, false), 20);
}

#[test]
fn length_truncated_detected_from_finish_reason() {
    let v = json!({
        "choices": [{
            "finish_reason": "length",
            "message": { "content": "" }
        }]
    });
    assert!(is_length_truncated(&v));
    assert_eq!(LENGTH_TRUNCATION_RETRY_LIMIT, 0);
    let stop = json!({
        "choices": [{
            "finish_reason": "stop",
            "message": { "content": "" }
        }]
    });
    assert!(!is_length_truncated(&stop));
}

#[test]
fn user_payload_requires_json_output() {
    let items = vec![masked(0, "Sword", None)];
    let payload = build_user_payload(&items);
    assert!(payload.contains("JSON"));
    assert!(payload.contains("\"r\""));
    assert!(payload.contains("Sword"));
}

#[test]
fn usage_parser_supports_both_shapes() {
    let deepseek = json!({
        "usage": {
            "prompt_cache_hit_tokens": 12,
            "prompt_cache_miss_tokens": 34,
            "completion_tokens": 56
        }
    });
    assert_eq!(
        parse_usage_totals(&deepseek),
        AiUsageTotals {
            prompt_cache_hit_tokens: 12,
            prompt_cache_miss_tokens: 34,
            completion_tokens: 56,
        }
    );

    let prompt_details = json!({
        "usage": {
            "prompt_tokens": 120,
            "completion_tokens": 30,
            "prompt_tokens_details": { "cached_tokens": 40 }
        }
    });
    assert_eq!(
        parse_usage_totals(&prompt_details),
        AiUsageTotals {
            prompt_cache_hit_tokens: 40,
            prompt_cache_miss_tokens: 80,
            completion_tokens: 30,
        }
    );

    let responses_details = json!({
        "usage": {
            "input_tokens": 200,
            "output_tokens": 50,
            "input_tokens_details": { "cached_tokens": 75 }
        }
    });
    assert_eq!(
        parse_usage_totals(&responses_details),
        AiUsageTotals {
            prompt_cache_hit_tokens: 75,
            prompt_cache_miss_tokens: 125,
            completion_tokens: 50,
        }
    );
}

#[test]
fn degrade_once_per_step_and_reuse_flags() {
    let engine = test_engine();
    assert!(engine.maybe_degrade_for_unsupported(
        400,
        "unsupported parameter: response_format"
    ));
    assert_eq!(engine.drain_notices().len(), 1);
    assert!(!engine.current_features().send_response_format);

    assert!(!engine.maybe_degrade_for_unsupported(
        400,
        "unsupported parameter: response_format"
    ));
    assert!(engine.drain_notices().is_empty());

    assert!(engine.maybe_degrade_for_unsupported(
        400,
        "unsupported parameter: max_tokens"
    ));
    assert_eq!(
        engine.current_features().max_tokens_field,
        MaxTokensField::MaxCompletionTokens
    );
}

#[test]
fn retry_only_resends_unresolved_indices() {
    let items = vec![
        masked(10, "First", None),
        masked(20, "Second", None),
        masked(30, "Third", None),
    ];
    let resolved = HashMap::from([
        (10usize, "甲".to_string()),
        (30usize, "丙".to_string()),
    ]);
    let unresolved = collect_unresolved_items(&items, &resolved);
    assert_eq!(
        unresolved.into_iter().map(|item| item.uid).collect::<Vec<_>>(),
        vec![20]
    );
}

#[test]
fn progress_never_exceeds_its_span() {
    // fill_missing 用 44..88 這一段，不可以蓋掉後面的打包階段
    for done in 0..=10 {
        let p = by_batch_or_strings(done, 10, done * 5, 50, 44, 44);
        assert!((44..=88).contains(&p), "超出範圍：{p}");
    }
}

#[test]
fn provider_name_is_never_leaked_to_players() {
    let msg = ai_quota_support_message("deepseek-chat 402 Insufficient Balance");
    assert!(!msg.to_lowercase().contains("deepseek-chat"));
    assert!(msg.contains("AI 服務"));
    assert!(msg.contains("自訂 API") || msg.contains("服務商"));
}

#[test]
fn auth_failure_is_not_classified_as_quota_failure() {
    assert!(is_auth_relogin_error("第 1 批失敗：使用開發者代管 AI 前，請先登入 Discord。"));
    assert!(is_auth_relogin_error("安全驗證已過期，請回到工具重新驗證。"));
    assert!(!is_auth_relogin_error(
        "Discord 登入／會員驗證暫時無法連線，請稍後再試；或改用自訂 API。"
    ));
    assert!(is_auth_unavailable_error(
        "Discord 登入／會員驗證暫時無法連線，請稍後再試；或改用自訂 API。"
    ));
    assert!(!is_auth_relogin_error("帳號餘額不足"));
    assert!(!is_auth_relogin_error(
        "代管 AI 暫時無法使用（503）：<!DOCTYPE html> cloudflare"
    ));
    assert!(!is_auth_relogin_error("AI 上游拒絕請求（401）：invalid api key"));
    assert!(auth_relogin_message().contains("Discord"));
    let peek = vec![
        "連線失敗（無回應）：timeout".into(),
        "翻譯前請先登入 Discord。".into(),
    ];
    let classified = classify_batch_abort(&peek).expect("classified");
    assert!(!classified.contains("額度可能已用完"));
    assert!(classified.contains("Discord"));
    // auth_unavailable 不秒殺
    assert!(classify_batch_abort(&[auth_unavailable_message()]).is_none());
}

#[test]
fn quota_message_recommends_deepseek_platform() {
    let msg = ai_quota_support_message("402 Insufficient Balance");
    assert!(msg.contains("自訂 API"));
    assert!(msg.contains("platform.deepseek.com") || msg.contains("服務商"));
    assert!(!msg.to_lowercase().contains("deepseek-chat"));
    assert!(!msg.contains("免費代管"));
}

#[test]
fn managed_503_legacy_turnstile_is_not_labeled_maintenance() {
    let body = r#"{"error":{"message":"Cloudflare Turnstile is not configured","type":"turnstile_unavailable"}}"#;
    let mapped = map_chat_http_error(503, body, true).expect("mapped");
    assert!(mapped.message.contains("Discord") || mapped.message.contains("自訂 API"));
    assert!(!mapped.message.contains("維護中"));
    assert_eq!(mapped.kind, ChatErrorKind::Fatal);
    assert_eq!(
        extract_proxy_error_type(body).as_deref(),
        Some("turnstile_unavailable")
    );
}

#[test]
fn managed_503_missing_key_is_maintenance() {
    let body =
        r#"{"error":{"message":"managed translation not configured","type":"server_not_ready"}}"#;
    let mapped = map_chat_http_error(503, body, true).expect("mapped");
    assert!(mapped.message.contains("維護中"));
    assert_eq!(mapped.kind, ChatErrorKind::Fatal);
}

#[test]
fn managed_503_auth_unavailable_is_transient() {
    let body =
        r#"{"error":{"message":"login verification unavailable","type":"auth_unavailable"}}"#;
    let mapped = map_chat_http_error(503, body, true).expect("mapped");
    assert!(mapped.message.contains("Discord"));
    assert!(!mapped.message.contains("維護中"));
    assert_eq!(mapped.kind, ChatErrorKind::Transient);
    assert!(!is_auth_relogin_error(&mapped.message));
}

#[test]
fn managed_guild_required_is_transient_for_retry() {
    let body =
        r#"{"error":{"message":"official discord membership required","type":"guild_required"}}"#;
    let mapped = map_chat_http_error(403, body, true).expect("mapped");
    assert!(mapped.message.contains("Discord"));
    assert_eq!(mapped.kind, ChatErrorKind::Transient);
    assert_eq!(
        extract_proxy_error_type(body).as_deref(),
        Some("guild_required")
    );
}

#[test]
fn managed_429_without_quota_type_is_rate_limit() {
    let mapped = map_chat_http_error(429, "{}", true).expect("mapped");
    assert!(mapped.message.contains("請求太頻繁"));
    assert_eq!(mapped.kind, ChatErrorKind::Transient);
}

#[test]
fn managed_429_insufficient_quota_is_quota() {
    let body = r#"{"error":{"message":"daily free translation budget reached","type":"insufficient_quota"}}"#;
    let mapped = map_chat_http_error(429, body, true).expect("mapped");
    assert!(mapped.message.contains("當日額度"));
    assert_eq!(mapped.kind, ChatErrorKind::Quota);
}

#[test]
fn managed_401_without_login_type_is_not_discord_relogin() {
    let body = r#"{"error":{"message":"Invalid Authentication","type":"authentication_error"}}"#;
    let mapped = map_chat_http_error(401, body, true).expect("mapped");
    assert!(mapped.message.contains("上游"));
    assert!(!is_auth_relogin_error(&mapped.message));
    assert_eq!(mapped.kind, ChatErrorKind::Fatal);
}

#[test]
fn managed_401_login_required_is_relogin() {
    let body = r#"{"error":{"message":"discord login required","type":"login_required"}}"#;
    let mapped = map_chat_http_error(401, body, true).expect("mapped");
    assert!(is_auth_relogin_error(&mapped.message));
    assert_eq!(mapped.kind, ChatErrorKind::Relogin);
}

#[test]
fn cloudflare_html_503_is_not_relogin() {
    let body = "<!DOCTYPE html><html>cloudflare</html>";
    let mapped = map_chat_http_error(503, body, true).expect("mapped");
    assert_eq!(mapped.kind, ChatErrorKind::Transient);
    assert!(!is_auth_relogin_error(&mapped.message));
}

#[test]
fn empty_content_transient_does_not_mark_round_congestion() {
    // 空 content 是 Transient 但 congestion=false；AIMD 不得因此減半並行
    assert!(!should_mark_round_congestion(false));
    assert!(should_mark_round_congestion(true));
}

#[test]
fn verify_custom_api_requires_custom_mode() {
    if get_ai_mode() == "custom" {
        return;
    }
    let err = verify_custom_api().unwrap_err();
    assert!(err.contains("自訂"));
}

#[test]
fn diagnose_empty_choice_includes_finish_reason() {
    let v = json!({
        "choices": [{
            "finish_reason": "length",
            "message": { "content": null, "refusal": null }
        }]
    });
    let msg = diagnose_empty_choice(&v);
    assert!(msg.contains("沒有回傳翻譯內容"));
    assert!(msg.contains("finish_reason=length"));
    assert!(msg.contains("choice_keys="));
    assert!(msg.contains("message_keys="));
}

#[test]
fn diagnose_empty_choice_includes_refusal() {
    let v = json!({
        "choices": [{
            "finish_reason": "content_filter",
            "message": { "content": "", "refusal": "policy block" }
        }]
    });
    let msg = diagnose_empty_choice(&v);
    assert!(msg.contains("refusal=policy block"));
    assert!(msg.contains("finish_reason=content_filter"));
}

#[test]
fn extract_content_uses_reasoning_when_parseable_json() {
    let v = json!({
        "choices": [{
            "message": {
                "content": null,
                "reasoning_content": "{\"r\":[{\"i\":0,\"t\":\"鑽石劍\"}]}"
            }
        }]
    });
    assert_eq!(
        extract_message_content(&v),
        "{\"r\":[{\"i\":0,\"t\":\"鑽石劍\"}]}"
    );
}

#[test]
fn extract_content_ignores_non_translation_reasoning() {
    let v = json!({
        "choices": [{
            "message": {
                "content": "",
                "reasoning_content": "thinking about swords..."
            }
        }]
    });
    assert!(extract_message_content(&v).trim().is_empty());
}

#[test]
fn empty_content_backoff_grows_with_attempt() {
    assert_eq!(empty_content_backoff_ms(1), 900);
    assert_eq!(empty_content_backoff_ms(2), 1400);
    assert!(empty_content_backoff_ms(4) > empty_content_backoff_ms(1));
}

#[test]
fn split_queued_splits_name_ui_batches() {
    let items: Vec<MaskedItem> = (0..4).map(|i| masked(i, "Sword", Some("物品名"))).collect();
    let queued = QueuedPlan {
        batch_no: 7,
        plan: BatchPlan {
            track: BatchTrackKind::Name,
            items,
        },
        requeues: 0,
    };
    let parts = split_queued_for_retry(queued);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].plan.items.len(), 2);
    assert_eq!(parts[1].plan.items.len(), 2);
    assert_eq!(parts[0].requeues, 1);
    assert_eq!(parts[1].batch_no, 7);
}

#[test]
fn split_queued_keeps_story_intact() {
    let items = vec![
        masked(0, "A long story line here", Some("任務")),
        masked(1, "Another story line here", Some("任務")),
    ];
    let queued = QueuedPlan {
        batch_no: 3,
        plan: BatchPlan {
            track: BatchTrackKind::Story,
            items,
        },
        requeues: 0,
    };
    let parts = split_queued_for_retry(queued);
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].plan.items.len(), 2);
    assert_eq!(parts[0].requeues, 1);
}

#[test]
fn is_empty_response_error_detects_messages() {
    assert!(is_empty_response_error("第 1 批失敗：服務有連線但沒有回傳翻譯內容"));
    assert!(is_empty_response_error(
        "服務有連線但沒有回傳翻譯內容（finish_reason=length）"
    ));
    assert!(is_empty_response_error("回傳為空 JSON 物件（無譯文）"));
    assert!(!is_empty_response_error("請求太頻繁，稍後再試"));
}

#[test]
fn local_llm_output_cap_fits_inside_the_context_window() {
    // 這條釘死本輪修掉的矛盾：上下文 4096、輸出上限 8192，長文本必爆。
    // 輸出上限必須 ≤ 最小上下文的四分之一，剩下的留給 prompt。
    let min_ctx = crate::engine::local_llm::context_size_for(0) as usize;
    assert!(min_ctx >= 8192, "最小上下文 {min_ctx}");
    assert!(
        LOCAL_LLM_MAX_COMPLETION_TOKENS * 4 <= min_ctx,
        "輸出 {LOCAL_LLM_MAX_COMPLETION_TOKENS} 對上下文 {min_ctx} 太大"
    );
    // 不論丟多長的輸入，本地那條路都不會超過上限
    for chars in [10usize, 5_000, 100_000] {
        for items in [1usize, 12, 48] {
            let got = clamp_completion_tokens_for(chars, items, true);
            assert!(got <= LOCAL_LLM_MAX_COMPLETION_TOKENS, "chars={chars} items={items} got={got}");
        }
    }
    // 雲端那條路維持原本的大上限，不受影響
    assert!(clamp_completion_tokens_for(100_000, 48, false) > LOCAL_LLM_MAX_COMPLETION_TOKENS);
}

#[test]
fn local_llm_uses_smaller_batches() {
    for track in [BatchTrackKind::Name, BatchTrackKind::Ui, BatchTrackKind::Story] {
        let cloud = track.batch_size_for(false, false);
        let local = track.batch_size_for(false, true);
        assert!(local < cloud, "{track:?}: local={local} cloud={cloud}");
        assert!(local >= 1);
    }
    assert_eq!(BatchTrackKind::Solo.batch_size_for(false, true), 1);
    // strict_single 仍然壓過一切
    assert_eq!(BatchTrackKind::Name.batch_size_for(true, true), 1);
}

#[test]
fn local_llm_parallel_matches_server_slots_and_uses_placeholder_guard() {
    // 送出端的併發必須跟 llama-server 實際開的 slot 數一致，否則多開的 slot
    // 沒人用（還是序列），或送太多請求排隊反而更慢。
    let slots = crate::engine::local_llm::recommended_parallel_slots() as usize;
    assert_eq!(provider_capabilities(AiProvider::LocalLlm).start_parallel, slots);
    assert!((1..=3).contains(&slots), "併發數要保守：實得 {slots}");
    let mut st = crate::engine::placeholder::GuardStats::default();
    assert!(crate::engine::placeholder::guard("Deals %s damage", "造成傷害", &mut st).is_none());
    assert!(crate::engine::placeholder::guard("Deals %s damage", "造成 %s 傷害", &mut st).is_some());
}
