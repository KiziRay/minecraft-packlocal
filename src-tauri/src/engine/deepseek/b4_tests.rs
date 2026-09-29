//! B4：漏翻與中斷的回歸測試（全部走假伺服器，不打真實 AI）。

use super::*;
use crate::engine::ai_fake_server::{dead_url, FakeReply, FakeServer};
use crate::engine::secrets::provider_capabilities;
use std::time::Instant;

fn engine_at(url: &str, provider: AiProvider) -> Engine {
    Engine {
        client: Arc::new(reqwest::blocking::Client::new()),
        base_url: Arc::new(url.trim_end_matches("/v1/chat/completions").to_string()),
        url: Arc::new(url.to_string()),
        api_key: Arc::new(String::new()),
        model: Arc::new("fake".into()),
        managed_session: Arc::new(Mutex::new(String::new())),
        managed: false,
        provider,
        capabilities: provider_capabilities(provider),
        degraded: Arc::new(Mutex::new(RequestDegradeState::default())),
        usage: Arc::new(Mutex::new(AiUsageTotals::default())),
        notices: Arc::new(Mutex::new(Vec::new())),
        local_timeouts: Arc::new(Mutex::new(Default::default())),
    }
}

fn item(uid: usize, source: &str) -> MaskedItem {
    let (masked, tokens) = placeholder::mask(source);
    MaskedItem {
        uid,
        source: source.into(),
        masked,
        tokens,
        context: None,
    }
}

fn one_item_plan() -> BatchPlan {
    BatchPlan {
        track: BatchTrackKind::Ui,
        items: vec![item(0, "Iron Sword")],
    }
}

const OK_IRON_SWORD: &str = r#"{"r":[{"i":0,"t":"鐵劍"}]}"#;

// ═══ #4 錯誤分類、退避、字串 id ═════════════════════════════════

#[test]
fn rate_limit_exceeded_is_not_treated_as_quota() {
    assert!(!looks_like_quota_or_auth_error("第 1 批失敗：Rate limit exceeded, slow down"));
    assert!(
        classify_batch_abort(&["第 2 批失敗：maximum context length exceeded".to_string()]).is_none(),
        "內容太長不是額度用完，不可以停掉整輪 AI"
    );
    assert!(looks_like_quota_or_auth_error("帳號餘額不足：Insufficient Balance"));
}

#[test]
fn string_ids_from_the_model_are_accepted() {
    let (map, audit) =
        parse_translation_object_audited(r#"{"r":[{"i":"0","t":"鐵劍"},{"i":1,"t":"金劍"}]}"#, &[0, 1]).unwrap();
    assert_eq!(map.get(&0).map(String::as_str), Some("鐵劍"));
    assert_eq!(map.get(&1).map(String::as_str), Some("金劍"));
    assert!(audit.is_clean(), "{}", audit.summary());
}

#[test]
fn retry_after_is_honoured_before_resending() {
    let server = FakeServer::start(|i, _| {
        if i == 0 {
            FakeReply::json(429, r#"{"error":{"message":"Too many requests"}}"#).header("Retry-After", "1")
        } else {
            FakeReply::chat(OK_IRON_SWORD)
        }
    });
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let started = Instant::now();
    let ok = translate_chunk(&engine, &Arc::new("sys".into()), &one_item_plan()).expect("second try succeeds");
    assert_eq!(ok.map.get(&0).map(String::as_str), Some("鐵劍"));
    assert_eq!(server.hits(), 2);
    assert!(started.elapsed() >= Duration::from_millis(950), "要照伺服器說的等 1 秒：{:?}", started.elapsed());
}

#[test]
fn quota_errors_are_not_retried() {
    let server = FakeServer::start(|_, _| FakeReply::json(402, r#"{"error":{"message":"Insufficient Balance"}}"#));
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let err = translate_chunk(&engine, &Arc::new("sys".into()), &one_item_plan()).unwrap_err();
    assert_eq!(err.kind, ChatErrorKind::Quota);
    assert_eq!(server.hits(), 1, "額度用完重送只會空轉");

    let server = FakeServer::start(|_, _| {
        FakeReply::json(429, r#"{"error":{"type":"insufficient_quota","message":"You exceeded your current quota"}}"#)
    });
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let err = translate_chunk(&engine, &Arc::new("sys".into()), &one_item_plan()).unwrap_err();
    assert_eq!(err.kind, ChatErrorKind::Quota);
    assert_eq!(server.hits(), 1, "429 帶額度字樣＝額度用完，不重送");
}

#[test]
fn context_too_long_is_classified_for_split_not_quota() {
    let server = FakeServer::start(|_, _| {
        FakeReply::json(
            400,
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens, you requested 9000 (exceeded)","code":"context_length_exceeded"}}"#,
        )
    });
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let err = translate_chunk(&engine, &Arc::new("sys".into()), &one_item_plan()).unwrap_err();
    assert_ne!(err.kind, ChatErrorKind::Quota, "{}", err.message);
    assert!(err.message.contains("內容太長"), "{}", err.message);
    assert_eq!(server.hits(), 1, "同一批再送一次結果一樣，要拆小");
}

#[test]
fn transient_server_errors_get_exponential_retries() {
    let server = FakeServer::start(|i, _| {
        if i < 3 {
            FakeReply::json(503, "busy")
        } else {
            FakeReply::chat(OK_IRON_SWORD)
        }
    });
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let ok = translate_chunk(&engine, &Arc::new("sys".into()), &one_item_plan()).expect("4th attempt succeeds");
    assert_eq!(ok.retries, 3);
    assert_eq!(server.hits(), 4);
}

#[test]
fn dead_local_server_is_reported_as_gone_not_as_timeout() {
    // 沒有人在聽的位址＝連線被拒：這不是「慢」，是程式不在了
    let engine = engine_at(&dead_url(), AiProvider::LocalLlm);
    let err = translate_chunk(&engine, &Arc::new("sys".into()), &one_item_plan()).unwrap_err();
    assert!(!err.message.contains("逾時"), "{}", err.message);
    assert_eq!(err.kind, ChatErrorKind::ProcessGone, "{}", err.message);
    assert!(err.message.contains("接續補完"), "要講下一步：{}", err.message);
}

#[test]
fn slow_local_model_is_split_instead_of_resent_at_the_same_size() {
    // 本地模型等不到回應：同樣大小重送只會再等一次逾時
    let server = FakeServer::start(|_, _| FakeReply::chat(OK_IRON_SWORD).delayed(Duration::from_secs(3)));
    let engine = engine_at(&server.chat_url(), AiProvider::LocalLlm);
    set_test_request_timeout_for(&server.chat_url(), Some(Duration::from_millis(500)));
    let result = translate_chunk(&engine, &Arc::new("sys".into()), &one_item_plan());
    set_test_request_timeout_for(&server.chat_url(), None);
    let err = result.unwrap_err();
    assert_eq!(err.kind, ChatErrorKind::LocalTimeout, "{}", err.message);
    assert!(wants_split(&err), "要拆小重送");
    assert_eq!(server.hits(), 1);
}

// ═══ #7 持續補位的工作池、拆批、斷線等待 ═════════════════════════

/// 從請求本文取出送出的 (id, 原文)。
fn sent_rows(body: &str) -> Vec<(usize, String)> {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let user = v["messages"][1]["content"].as_str().unwrap_or("");
    let json_part = user.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
    let data: Value = serde_json::from_str(json_part).unwrap_or(Value::Null);
    data["r"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| Some((row["i"].as_u64()? as usize, row["t"].as_str()?.to_string())))
        .collect()
}

/// 數字轉成中文字（譯文裡出現原文沒有的阿拉伯數字，會被當成「數字漂移」）。
fn cn(i: usize) -> String {
    const D: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    i.to_string().chars().map(|c| D[c.to_digit(10).unwrap_or(0) as usize]).collect()
}

/// 把送來的每一條都「翻」成固定中文，照 id 回。
fn echo_reply(body: &str) -> FakeReply {
    let rows: Vec<Value> = sent_rows(body)
        .into_iter()
        .map(|(i, _)| json!({ "i": i, "t": format!("測試譯文{}", cn(i)) }))
        .collect();
    FakeReply::chat(&json!({ "r": rows }).to_string())
}

fn items(sources: &[&str]) -> Vec<MaskedItem> {
    sources.iter().enumerate().map(|(i, s)| item(i, s)).collect()
}

fn state(parallel: usize) -> BatchRuntimeState {
    let mut st = BatchRuntimeState::new("sys".into(), parallel);
    st.current_parallel = parallel;
    st.warmed_up = true;
    st
}

#[test]
fn pool_refills_a_free_slot_without_waiting_for_the_slowest_batch() {
    let arrivals: Arc<Mutex<Vec<(String, Instant)>>> = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&arrivals);
    let server = FakeServer::start(move |_, body| {
        let rows = sent_rows(body);
        let first = rows.first().map(|(_, t)| t.clone()).unwrap_or_default();
        log.lock().unwrap().push((first.clone(), Instant::now()));
        let reply = echo_reply(body);
        if first.starts_with("Slow") {
            reply.delayed(Duration::from_millis(1500))
        } else {
            reply.delayed(Duration::from_millis(50))
        }
    });
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let mut st = state(2);
    let started = Instant::now();
    let out = run_batches(
        &engine,
        &mut st,
        &items(&["Slow Item A", "Fast Item B", "Fast Item C"]),
        0,
        100,
        &mut |_, _| {},
        true,
    )
    .unwrap();
    assert_eq!(out.len(), 3);
    let c_arrived = arrivals
        .lock()
        .unwrap()
        .iter()
        .find(|(t, _)| t == "Fast Item C")
        .map(|(_, at)| at.duration_since(started))
        .expect("C was sent");
    assert!(
        c_arrived < Duration::from_millis(900),
        "空出來的位置要馬上補下一批，不要等最慢的那批：C 在 {c_arrived:?} 才送出"
    );
}

#[test]
fn too_large_batches_are_halved_until_they_fit() {
    let server = FakeServer::start(|_, body| {
        if sent_rows(body).len() > 2 {
            FakeReply::json(
                400,
                r#"{"error":{"message":"maximum context length exceeded","code":"context_length_exceeded"}}"#,
            )
        } else {
            echo_reply(body)
        }
    });
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let mut st = state(4);
    let sources = ["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel"];
    let out = run_batches(&engine, &mut st, &items(&sources), 0, 100, &mut |_, _| {}, false).unwrap();
    assert_eq!(out.len(), 8, "拆到放得下為止，一句都不能少");
    assert_eq!(server.hits(), 7, "8→4+4→2+2+2+2");
    let sizes: Vec<usize> = server.bodies().iter().map(|b| sent_rows(b).len()).collect();
    assert_eq!(sizes.iter().filter(|n| **n == 2).count(), 4, "最後都拆成兩條一批：{sizes:?}");
}

#[test]
fn a_short_network_outage_is_waited_out_instead_of_dropping_batches() {
    let server = FakeServer::start(|i, body| if i < 14 { FakeReply::hang_up() } else { echo_reply(body) });
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let mut st = state(1);
    let out = run_batches(&engine, &mut st, &items(&["Iron Sword", "Gold Sword"]), 0, 100, &mut |_, _| {}, false)
        .expect("連線恢復後要繼續，不是整批放棄");
    assert_eq!(out.len(), 2);
}


// ═══ #1 #2 #7 resolve_unique：沒回應分開、同輪縮小重送、失敗不丟、AI 不可用正確設值 ═══

fn resolve_with(
    sources: &[&str],
    ctx: Vec<Option<&'static str>>,
    reuse_tm: bool,
) -> Result<Resolved, String> {
    let _skip = shared_tm::SkipSharedLookupGuard::enter(true);
    let unique: Vec<String> = sources.iter().map(|s| s.to_string()).collect();
    resolve_unique(
        &unique,
        &ctx,
        true,
        reuse_tm,
        false,
        TranslationQuality::Balanced,
        0,
        100,
        None,
        &mut |_, _| {},
    )
}

#[test]
fn quota_mid_run_keeps_translated_part_and_marks_ai_unavailable() {
    let server = FakeServer::start(|i, body| {
        if i == 0 {
            echo_reply(body)
        } else {
            FakeReply::json(402, r#"{"error":{"message":"Insufficient Balance"}}"#)
        }
    });
    test_hooks::set_preflight(Some(Ok(engine_at(&server.chat_url(), AiProvider::Openai))));
    let name = Some("物品名");
    let result = resolve_with(
        &["Zq Quota Blade", "Zq Quota Axe", "Zq Quota Bow", "Zq quota line one", "Zq quota line two", "Zq quota line three"],
        vec![name, name, name, None, None, None],
        false,
    );
    test_hooks::set_preflight(None);
    let resolved = result.expect("額度用完不可以把已翻好的丟掉");
    for uid in 0..3 {
        assert!(resolved.translations.contains_key(&uid), "第一批已翻好：uid {uid}");
    }
    assert!(resolved.report.ai_unavailable.is_some(), "AI 沒跑完要留下原因，結尾才不會謊報完成");
    for uid in 3..6 {
        assert!(!resolved.translations.contains_key(&uid));
        assert!(
            !resolved.quality_deferred.contains(&uid),
            "沒拿到回應不是品質沒過，不可以混進品質暫緩（補充漏翻會跳過它們）"
        );
    }
    assert_eq!(server.hits(), 2, "額度用完之後不可以再送");
}

#[test]
fn preflight_failure_keeps_data_layer_hits_instead_of_erroring() {
    {
        let mut tm = Tm::load();
        tm.insert("Zq Seeded Widget", "測試小工具");
        let _ = tm.save();
    }
    test_hooks::set_preflight(Some(Err("帳號餘額不足：Insufficient Balance".into())));
    let result = resolve_with(&["Zq Seeded Widget", "Zq Missing Thing B4"], vec![None, None], true);
    test_hooks::set_preflight(None);
    let resolved = result.expect("AI 連不上時，資料層（翻譯記憶）已查到的譯文要保留");
    assert_eq!(resolved.translations.get(&0).map(String::as_str), Some("測試小工具"));
    assert!(!resolved.translations.contains_key(&1));
    assert!(resolved.report.ai_unavailable.is_some());
}

#[test]
fn unanswered_items_are_resent_in_smaller_batches_in_the_same_round() {
    let server = FakeServer::start(|_, body| {
        let rows = sent_rows(body);
        let keep: Vec<Value> = rows
            .iter()
            .filter(|(_, t)| rows.len() == 1 || t != "Zq Stubborn Line")
            .map(|(i, _)| json!({ "i": i, "t": format!("測試譯文{}", cn(*i)) }))
            .collect();
        FakeReply::chat(&json!({ "r": keep }).to_string())
    });
    test_hooks::set_preflight(Some(Ok(engine_at(&server.chat_url(), AiProvider::Openai))));
    let result = resolve_with(
        &["Zq Resend One", "Zq Resend Two", "Zq Stubborn Line", "Zq Resend Four"],
        vec![None; 4],
        false,
    );
    test_hooks::set_preflight(None);
    let resolved = result.unwrap();
    assert!(
        resolved.translations.contains_key(&2),
        "這一批漏回的句子要在同一輪縮小批次重送，不是留到下次"
    );
    assert_eq!(resolved.translations.len(), 4);
}

#[test]
fn cloud_top_up_runs_once_not_twice() {
    // 本地模型一直回英文（品質沒過）；雲端也一直回英文——雲端只能被找一次
    let local = FakeServer::start(|_, body| {
        let rows: Vec<Value> = sent_rows(body).into_iter().map(|(i, t)| json!({ "i": i, "t": t })).collect();
        FakeReply::chat(&json!({ "r": rows }).to_string())
    });
    let cloud = FakeServer::start(|_, body| {
        let rows: Vec<Value> = sent_rows(body).into_iter().map(|(i, t)| json!({ "i": i, "t": t })).collect();
        FakeReply::chat(&json!({ "r": rows }).to_string())
    });
    test_hooks::set_preflight(Some(Ok(engine_at(&local.chat_url(), AiProvider::LocalLlm))));
    test_hooks::set_cloud(Some(engine_at(&cloud.chat_url(), AiProvider::Openai)));
    let result = resolve_with(&["Zq hard sentence for escalation"], vec![None], false);
    test_hooks::set_preflight(None);
    test_hooks::set_cloud(None);
    let resolved = result.unwrap();
    assert!(!resolved.translations.contains_key(&0));
    assert_eq!(cloud.hits(), 1, "雲端補完只能呼叫一次（舊版呼叫兩次、重複花錢）");
}

#[test]
fn cloud_translation_that_stops_mid_sentence_is_not_accepted() {
    let source = "This sword deals extra damage to undead creatures and slowly heals the wielder";
    let ((class, _, _, _), _) = classify_candidate_for(source, "這把", false);
    assert_eq!(class, CandidateClass::QualityFail, "雲端譯文被截斷到只剩兩個字，不能當成翻好了");
    let ((ok, _, _, _), _) = classify_candidate_for(source, "這把劍對不死生物造成額外傷害，並緩慢治療持有者", false);
    assert_eq!(ok, CandidateClass::Accept);
}

// ═══ #6 鎖中毒時復原 ═══════════════════════════════════════════

#[test]
fn a_crashed_worker_does_not_wipe_usage_or_notices() {
    let engine = engine_at(&dead_url(), AiProvider::Openai);
    engine.record_usage(&AiUsageTotals {
        completion_tokens: 7,
        ..Default::default()
    });
    engine.push_notice("保留這一行".into());
    let usage = Arc::clone(&engine.usage);
    let notices = Arc::clone(&engine.notices);
    let _ = std::thread::spawn(move || {
        let _a = usage.lock().unwrap();
        let _b = notices.lock().unwrap();
        panic!("worker crashed while holding the locks");
    })
    .join();
    assert_eq!(engine.usage_snapshot().completion_tokens, 7, "鎖中毒時要復原，不能回空值");
    assert_eq!(engine.drain_notices(), vec!["保留這一行".to_string()]);
}


// ═══ 審查修正 ═══════════════════════════════════════════════════

#[test]
fn fix1a_consecutive_local_timeouts_do_not_stop_the_whole_run() {
    // 慢機器：多條一批一律等不到，拆到一條才來得及——不可以因為連續逾時就停掉整輪
    let server = FakeServer::start(|_, body| {
        let reply = echo_reply(body);
        if sent_rows(body).len() > 1 {
            reply.delayed(Duration::from_millis(900))
        } else {
            reply
        }
    });
    let engine = engine_at(&server.chat_url(), AiProvider::LocalLlm);
    set_test_request_timeout_for(&server.chat_url(), Some(Duration::from_millis(300)));
    let mut st = state(1);
    let sources = ["Zq Slow A", "Zq Slow B", "Zq Slow C", "Zq Slow D", "Zq Slow E", "Zq Slow F", "Zq Slow G", "Zq Slow H"];
    let result = run_batches(&engine, &mut st, &items(&sources), 0, 100, &mut |_, _| {}, false);
    set_test_request_timeout_for(&server.chat_url(), None);
    assert!(st.stop.is_none(), "逾時不算「沒有進展」，不可以停整輪：{:?}", st.stop);
    assert_eq!(result.unwrap().len(), 8);
}

#[test]
fn fix4c_a_batch_that_keeps_failing_is_given_up_so_the_next_batch_runs() {
    let server = FakeServer::start(|_, body| {
        if sent_rows(body).iter().any(|(_, t)| t == "Zq Always Busy") {
            FakeReply::json(503, "busy")
        } else {
            echo_reply(body)
        }
    });
    let engine = engine_at(&server.chat_url(), AiProvider::Openai);
    let mut st = state(1);
    let started = Instant::now();
    let out = run_batches(
        &engine,
        &mut st,
        &items(&["Zq Always Busy", "Zq Next Batch"]),
        0,
        100,
        &mut |_, _| {},
        true,
    )
    .unwrap_or_default();
    assert!(out.contains_key(&1), "一直失敗的那批等到上限就先列為沒回應，下一批要照送");
    assert!(!out.contains_key(&0));
    assert!(started.elapsed() < Duration::from_secs(7), "{:?}", started.elapsed());
}

#[test]
fn fix2_f1_a_stuck_local_model_is_given_up_instead_of_waiting_for_hours() {
    // 伺服器活著但卡住（記憶體交換）：每批都逾時。倍數到頂後再連續逾時幾次就要停下，
    // 走「AI 不可用」保留已翻、說明原因，而不是一路拆批等好幾個小時
    let server = FakeServer::start(|_, body| echo_reply(body).delayed(Duration::from_millis(900)));
    let engine = engine_at(&server.chat_url(), AiProvider::LocalLlm);
    set_test_request_timeout_for(&server.chat_url(), Some(Duration::from_millis(250)));
    let mut st = state(1);
    let sources: Vec<String> = (0..16).map(|i| format!("Zq Stuck {i}")).collect();
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let _ = run_batches(&engine, &mut st, &items(&refs), 0, 100, &mut |_, _| {}, false);
    set_test_request_timeout_for(&server.chat_url(), None);
    match &st.stop {
        Some(BatchStop::AiUnavailable(reason)) => {
            assert!(reason.contains("本地模型") && reason.contains("接續補完"), "{reason}");
            assert!(!reason.starts_with("第 "), "給使用者看的原因不要帶「第 N 批失敗：」前綴：{reason}");
        }
        other => panic!("卡住的本地模型要停下並說明：{other:?}（送了 {} 次）", server.hits()),
    }
    assert!(server.hits() <= 12, "不可以一直拆批空等：送了 {} 次", server.hits());
}
