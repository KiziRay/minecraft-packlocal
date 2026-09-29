use super::*;
use crate::engine::secrets::{provider_capabilities, AiProvider, MaxTokensField};
use std::collections::HashMap;

fn masked(uid: usize, source: &str, context: Option<&'static str>) -> MaskedItem {
    let (masked, tokens) = placeholder::mask(source);
    MaskedItem {
        uid,
        source: source.to_string(),
        masked,
        tokens,
        context,
    }
}

/// AI 掛掉時，資料層命中的譯文不可以被誤標為「使用 AI 完成」。
///
/// 實測代價：站長那一包 33233 條待譯裡，共享翻譯庫命中 32151、共享術語 671，
/// 真正送 AI 的只有 271 條。舊版在 `Engine::connect()?` 一拋，
/// 那 32822 條查得到的譯文雖然留在記憶體，卻被當成整輪完成而寫出——
/// FTB Quests 與 FancyMenu 的缺口因此被掩蓋。
///
/// 這條測試守的是：額度問題不是使用者取消；必須留下可診斷原因，
/// 由呼叫端停止這次有選 AI 的翻譯，不能寫出假完成結果。
#[test]
fn ai_failure_must_not_discard_what_the_data_layers_already_found() {
    // 「AI 不可用」與「使用者取消」是兩件不同的事，只有後者該中斷
    assert!(
        !is_cancel_message("ChatGPT 暫時不接受翻譯請求：The usage limit has been reached"),
        "端點拒絕不是使用者取消，不可以當成中斷"
    );
    assert!(
        !is_cancel_message("本地模型尚未就緒。請先完成安裝並等到健康檢查通過。"),
        "本地模型沒裝好也不是取消"
    );
    assert!(is_cancel_message(CANCEL_MESSAGE), "只有這個才是真的取消");

    // 報告要能表達「資料層有成果，但 AI 沒跑成」這個中間狀態，
    // 呼叫端才有辦法把它算成「部分完成」而不是「完成」。
    let mut report = AiFillReport {
        shared_hits: 32151,
        shared_glossary_hits: 671,
        ..Default::default()
    };
    assert!(report.ai_unavailable.is_none(), "預設是 AI 正常");
    report.ai_unavailable = Some("ChatGPT 暫時不接受翻譯請求".into());
    assert!(
        report.ai_unavailable.is_some(),
        "AI 不可用必須留下痕跡，否則結尾會誤報完成"
    );
    // 資料層的成果一條都不能少
    assert_eq!(report.shared_hits + report.shared_glossary_hits, 32822);
}

#[test]
fn gpt_endpoint_rejection_never_claims_the_chatgpt_account_is_exhausted() {
    let message = ai_quota_support_message_for(
        "ChatGPT 暫時不接受翻譯請求：The usage limit has been reached",
        AiProvider::Codex,
        false,
    );
    assert!(message.contains("不一定代表你平常聊天的額度用完了"));
    assert!(message.contains("翻譯會消耗你的 ChatGPT 帳號額度"), "要誠實說明會用到額度");
    // B5c（規格 §5.3）：「沒寫入」只限試翻時；翻到一半停下要說已翻好的保留
    assert!(message.contains("試翻時停下，還沒寫入任何翻譯") && message.contains("已翻好的部分都保留"));
    assert!(!message.contains("GPT 帳號額度或速率限制"));
    let own_text = message.replace("The usage limit has been reached", "");
    for word in ["Codex", "端點", "權杖", "探測", "預檢", "429", "Token", "token"] {
        assert!(!own_text.contains(word), "玩家看得到的文字不可出現「{word}」");
    }
}

#[test]
fn translation_probe_is_a_real_zh_translation_but_has_no_persistent_target() {
    // 這是唯一用於預檢的文字；它不是玩家整合包中的鍵值，也不會交給寫檔／TM／共享庫流程。
    assert_eq!(AI_TRANSLATION_PROBE_SOURCE, "Iron Sword");
    assert!(quality_fail_reason(AI_TRANSLATION_PROBE_SOURCE, "鐵劍").is_ok());
    assert!(quality_fail_reason(AI_TRANSLATION_PROBE_SOURCE, "Iron Sword").is_err());
}

#[test]
fn no_answer_items_must_not_be_written_back_as_english_or_negatively_cached() {
    // 站長那一輪的實況：一批用完重試額度後整批消失，那些句子被寫回英文
    // 並進負向快取，於是「補充漏翻」連試都不試——永遠救不回來。
    let mut translations: HashMap<usize, String> = HashMap::new();
    let mut report = AiFillReport::default();
    let mut deferred: HashSet<usize> = HashSet::new();
    let pending = vec![
        PendingItem {
            uid: 1,
            source: "Deals %s extra damage".into(),
            reason: PendingReason::NoAnswer,
        },
        PendingItem {
            uid: 2,
            source: "Costs %s mana".into(),
            reason: PendingReason::PlaceholderBroken,
        },
    ];

    let (broken, unanswered) =
        drain_pending_ai(&pending, &mut translations, &mut report, &mut deferred);
    assert_eq!((broken, unanswered), (1, 1));

    // 沒拿到回應的：維持缺口，並排進補完佇列
    assert!(
        !translations.contains_key(&1),
        "沒拿到回應的句子不可以被寫回英文假裝翻完了"
    );
    assert!(deferred.contains(&1), "要排進補完佇列才救得回來");
    assert!(
        !is_placeholder_negatively_cached("Deals %s extra damage"),
        "沒拿到回應不代表翻不動，不可以進負向快取"
    );

    // 真的把格式符號弄壞的：維持舊行為（英文保底 ＋ 不重燒）
    assert_eq!(translations.get(&2).map(String::as_str), Some("Costs %s mana"));
    assert!(!deferred.contains(&2));
    assert!(is_placeholder_negatively_cached("Costs %s mana"));

    // 兩種原因要分開講，不可以混成一句
    let notes = report.notes.join("
");
    assert!(notes.contains("佔位符失敗"), "{notes}");
    assert!(notes.contains("沒拿到 AI 回應"), "{notes}");
}

#[test]
fn unresolved_batch_items_are_tagged_as_no_answer() {
    // run_batches 放棄的批次會讓這些 uid 不出現在結果裡。
    // 它們必須被標成「沒拿到回應」，而不是「翻不動」。
    let items = vec![
        MaskedItem {
            uid: 7,
            source: "Iron Sword".into(),
            masked: "Iron Sword".into(),
            tokens: vec![],
            context: None,
        },
        MaskedItem {
            uid: 8,
            source: "Gold Sword".into(),
            masked: "Gold Sword".into(),
            tokens: vec![],
            context: None,
        },
    ];
    let mut resolved = HashMap::new();
    resolved.insert(7usize, "鐵劍".to_string());

    let unresolved = collect_unresolved_items(&items, &resolved);
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].uid, 8);
    assert_eq!(unresolved[0].reason, PendingReason::NoAnswer);
}

#[test]
fn keep_english_skip_quality_does_not_write_translation() {
    let mut translations = HashMap::new();
    let mut report = AiFillReport::default();
    keep_english_skip(
        &mut translations,
        &mut report,
        7,
        "Diamond Sword",
        false,
        Some(QualityFailReason::SameAsSource),
    );
    assert!(translations.get(&7).is_none(), "quality fail must not write English");
    assert_eq!(report.quality_skipped, 1);
    assert_eq!(report.quality_same_as_source, 1);
    assert_eq!(report.rejected, 0);
    keep_english_skip(&mut translations, &mut report, 8, "Hello %s", true, None);
    assert_eq!(translations.get(&8).map(String::as_str), Some("Hello %s"));
    assert_eq!(report.rejected, 1);
}

#[test]
fn classify_still_english_is_quality_fail_not_placeholder() {
    let (class, safe, _, reason) = classify_candidate("Diamond Sword", "Diamond Sword");
    assert_eq!(class, CandidateClass::QualityFail);
    assert!(safe.is_none());
    assert_eq!(reason, Some(QualityFailReason::SameAsSource));
}

#[test]
fn classify_broken_placeholder_is_placeholder_fail() {
    let (class, safe, _, reason) = classify_candidate("Hello %s", "你好");
    assert_eq!(class, CandidateClass::PlaceholderFail);
    assert!(safe.is_none());
    assert!(reason.is_none());
}

#[test]
fn classify_good_zh_is_accept() {
    let (class, safe, _, reason) = classify_candidate("Diamond Sword", "鑽石劍");
    assert_eq!(class, CandidateClass::Accept);
    assert_eq!(safe.as_deref(), Some("鑽石劍"));
    assert!(reason.is_none());
}

#[test]
fn classify_mixed_fragment_reason() {
    let (class, safe, _, reason) =
        classify_candidate("x", "黑色Argillite Brick 階梯");
    assert_eq!(class, CandidateClass::QualityFail);
    assert!(safe.is_none());
    assert_eq!(reason, Some(QualityFailReason::MixedFragment));
}

#[test]
fn classify_proper_noun_with_enough_cjk_accepts() {
    let (class, safe, _, _) = classify_candidate("Flan Assault Rifle", "Flan 突擊步槍");
    assert_eq!(class, CandidateClass::Accept);
    assert_eq!(safe.as_deref(), Some("Flan 突擊步槍"));
}

#[test]
fn strict_retry_cap_splits_overflow() {
    let items: Vec<MaskedItem> = (0..100)
        .map(|i| masked(i, &format!("Hello {i}"), None))
        .collect();
    let (to_strict, overflow) = split_strict_retry_cap(items, STRICT_PLACEHOLDER_RETRY_CAP);
    assert_eq!(to_strict.len(), STRICT_PLACEHOLDER_RETRY_CAP);
    assert_eq!(overflow.len(), 100 - STRICT_PLACEHOLDER_RETRY_CAP);
    assert_eq!(STRICT_PLACEHOLDER_RETRY_CAP, 48);
}

#[test]
fn fill_report_note_separates_quality_and_placeholder() {
    let report = AiFillReport {
        filled: 1,
        ai_translated: 1,
        quality_skipped: 3,
        rejected: 2,
        ..Default::default()
    };
    let note = report.note();
    assert!(note.contains("品質未過"), "{note}");
    assert!(note.contains("佔位符不符"), "{note}");
}

fn test_engine() -> Engine {
    Engine {
        client: Arc::new(reqwest::blocking::Client::new()),
        base_url: Arc::new("https://example.com".into()),
        url: Arc::new("https://example.com/v1/chat/completions".into()),
        api_key: Arc::new(String::new()),
        model: Arc::new("demo".into()),
        managed_session: Arc::new(Mutex::new(String::new())),
        managed: false,
        provider: AiProvider::Deepseek,
        capabilities: provider_capabilities(AiProvider::Deepseek),
        degraded: Arc::new(Mutex::new(RequestDegradeState::default())),
        usage: Arc::new(Mutex::new(AiUsageTotals::default())),
        notices: Arc::new(Mutex::new(Vec::new())),
        local_timeouts: Arc::new(Mutex::new(Default::default())),
    }
}

#[test]
fn only_local_llm_gets_the_thinking_disabled_kwarg() {
    // 這條釘死本輪修掉的空回應 bug：DeepSeek 用 `thinking`，本地模型用
    // `chat_template_kwargs.enable_thinking`，兩者是不同 API，不能共用判斷式，
    // 也不能誤送給不支援這個欄位的服務商。
    let mut engine = test_engine();
    engine.provider = AiProvider::LocalLlm;
    assert!(should_disable_local_thinking(&engine));
    assert!(!should_send_thinking_disabled(&engine));

    engine.provider = AiProvider::Deepseek;
    assert!(!should_disable_local_thinking(&engine));
    assert!(should_send_thinking_disabled(&engine));

    for other in [AiProvider::Openai, AiProvider::Glm, AiProvider::Qwen, AiProvider::Codex] {
        engine.provider = other;
        assert!(!should_disable_local_thinking(&engine), "{other:?} 不該被關思考");
    }
}

#[test]
fn context_hint_reads_leading_segment() {
    assert_eq!(context_hint("item.minecraft.diamond_sword"), Some("物品名"));
    assert_eq!(context_hint("block.create.cogwheel"), Some("方塊名"));
    assert_eq!(context_hint("entity.minecraft.creeper"), Some("生物名"));
}

#[test]
fn context_hint_finds_kind_after_mod_id() {
    // 很多模組把自己的 id 放最前面
    assert_eq!(context_hint("create.tooltip.hold_shift"), Some("提示說明"));
    assert_eq!(context_hint("mekanism.gui.energy"), Some("介面文字"));
}

#[test]
fn context_hint_returns_none_for_unknown_shapes() {
    assert!(context_hint("somemod.random_thing").is_none());
}

#[test]
fn parses_plain_json_object() {
    let m = parse_translation_object(r#"{"r":[{"i":0,"t":"鑽石劍"}]}"#).unwrap();
    assert_eq!(m.get(&0).map(|s| s.as_str()), Some("鑽石劍"));
}

#[test]
fn parses_object_wrapped_in_code_fence_and_prose() {
    let raw =
        "當然可以：\n```json\n{\"r\":[{\"i\":0,\"t\":\"鑽石劍\"},{\"i\":1,\"t\":\"金錠\"}]}\n```";
    let m = parse_translation_object(raw).unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m.get(&1).map(|s| s.as_str()), Some("金錠"));
}

#[test]
fn malformed_response_is_an_error_not_a_panic() {
    assert!(parse_translation_object("完全不是 JSON").is_err());
}

#[test]
fn a_clean_batch_reports_no_problems() {
    let (map, audit) = parse_translation_object_audited(
        r#"{"r":[{"i":1,"t":"鑽石劍"},{"i":2,"t":"鐵鎬"}]}"#,
        &[1, 2],
    )
    .unwrap();
    assert!(audit.is_clean(), "{}", audit.summary());
    assert!(!audit.needs_diagnostic_split());
    assert_eq!(audit.resolved, 2);
    assert_eq!(map.len(), 2);
}

#[test]
fn hallucinated_ids_are_dropped_and_force_a_split() {
    // 模型回了一個我們沒送過的 id。舊版會把它放進 map（雖然後續查不到），
    // 但更重要的是：這代表模型沒照格式走，整批都不該當成可靠輸出。
    let (map, audit) = parse_translation_object_audited(
        r#"{"r":[{"i":1,"t":"鑽石劍"},{"i":99,"t":"我自己編的"}]}"#,
        &[1, 2],
    )
    .unwrap();
    assert_eq!(audit.unknown, vec![99]);
    assert!(!map.contains_key(&99), "沒送過的 id 絕不可寫進結果");
    assert!(audit.needs_diagnostic_split());
    assert!(!audit.is_clean());
}

#[test]
fn same_id_with_two_different_answers_is_not_guessed() {
    // 同一個 id 給了兩種譯文＝不知道哪個對。舊版是「後面的蓋掉前面的」，
    // 靜默選一個。現在兩個都不採用，整批進拆半重試。
    let (map, audit) = parse_translation_object_audited(
        r#"{"r":[{"i":1,"t":"鑽石劍"},{"i":1,"t":"鑽石之劍"}]}"#,
        &[1],
    )
    .unwrap();
    assert_eq!(audit.conflicting, vec![1]);
    assert!(!map.contains_key(&1), "衝突的 id 不該猜一個寫進去");
    assert!(audit.needs_diagnostic_split());
}

#[test]
fn duplicate_but_identical_answers_are_harmless() {
    let (map, audit) = parse_translation_object_audited(
        r#"{"r":[{"i":1,"t":"鑽石劍"},{"i":1,"t":"鑽石劍"}]}"#,
        &[1],
    )
    .unwrap();
    assert!(audit.conflicting.is_empty(), "重複但內容相同不算問題");
    assert!(audit.is_clean());
    assert_eq!(map.get(&1).map(String::as_str), Some("鑽石劍"));
}

#[test]
fn a_few_missing_entries_are_normal_but_mass_truncation_is_not() {
    // 少數沒回來＝下一輪補，不必整批重試
    let few = audit_batch_response(
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        &[(1, "a".into()), (2, "b".into()), (3, "c".into()), (4, "d".into()),
          (5, "e".into()), (6, "f".into()), (7, "g".into()), (8, "h".into()),
          (9, "i".into())],
    );
    assert_eq!(few.missing, vec![10]);
    assert!(!few.needs_diagnostic_split(), "掉一筆不該讓整批重跑");

    // 掉太多＝這批本身有問題（多半被截斷），要拆半
    let many = audit_batch_response(
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        &[(1, "a".into()), (2, "b".into())],
    );
    assert_eq!(many.missing.len(), 8);
    assert!(many.needs_diagnostic_split());
}

#[test]
fn empty_translations_are_flagged_not_written() {
    let (map, audit) = parse_translation_object_audited(
        r#"{"r":[{"i":1,"t":""},{"i":2,"t":"鐵鎬"}]}"#,
        &[1, 2],
    )
    .unwrap();
    assert_eq!(audit.empty, vec![1]);
    assert_eq!(audit.resolved, 1);
    assert!(!audit.is_clean());
    // map 裡雖然有 key，但呼叫端會用 trim().is_empty() 濾掉；
    // 這裡確認稽核本身有把它標出來，不是靜默當成翻好了
    assert_eq!(map.get(&1).map(String::as_str), Some(""));
}

#[test]
fn system_prompt_is_batch_independent() {
    let gloss = glossary::load(None);
    let texts = vec![
        "Creeper Head".to_string(),
        "Quest Start".to_string(),
        "Diamond Sword".to_string(),
    ];
    let prompt = build_system_prompt(&gloss, &texts, false);
    let again = build_system_prompt(&gloss, &texts, false);
    assert_eq!(prompt, again);
    let payload_a = build_user_payload(&[masked(0, "Creeper {0}", Some("生物名"))]);
    let payload_b = build_user_payload(&[masked(7, "Quest line", Some("任務文字"))]);
    assert_ne!(payload_a, payload_b);
    assert!(prompt.contains("固定譯名"));
    assert!(prompt.starts_with("你是 Minecraft"));
    assert!(
        prompt.len() >= 400,
        "system 前綴應夠長以利 Context Caching，實際 {}",
        prompt.len()
    );
    assert!(!prompt.contains("\"i\":0"));
    assert!(!prompt.contains("Quest line"));
    // 不同句集合若術語命中不同，譯名區塊可不同；但規則前綴必須相同
    let other = build_system_prompt(&gloss, &["Totally Unique Widget Name XYZ".into()], false);
    assert!(other.starts_with("你是 Minecraft"));
    assert_eq!(
        prompt.find("固定譯名"),
        other.find("固定譯名")
    );
}

#[test]
fn system_prompt_stable_prefix_is_long_enough_for_cache() {
    let gloss = glossary::load(None);
    let prompt = build_system_prompt(&gloss, &[], false);
    assert!(prompt.contains("CachePrefixPad") || prompt.len() >= 500);
    assert!(prompt.contains("輸出格式"));
}

#[test]
fn local_prompt_is_much_shorter_than_the_cloud_one() {
    // 本地模型的 8192 視窗是 prompt 與輸出共用的：system 佔掉的每個 token
    // 都是從可翻譯的內容裡扣的。而且規則越多，小模型的指令遵循越差。
    let gloss = glossary::load(None);
    let texts = vec!["Diamond Sword".to_string(), "Creeper Head".to_string()];
    let cloud = build_system_prompt(&gloss, &texts, false);
    let local = build_system_prompt(&gloss, &texts, true);
    assert!(
        local.chars().count() * 2 < cloud.chars().count(),
        "本地 prompt 應不到雲端的一半：本地 {} 字、雲端 {} 字",
        local.chars().count(),
        cloud.chars().count()
    );
    // 精簡不等於拿掉會壞掉的規則
    assert!(local.contains("%s"), "佔位符規則不可省");
    assert!(local.contains("數字"), "數字照抄的規則不可省");
    assert!(local.contains("輸出格式"), "JSON 輸出格式不可省");
    assert!(local.contains("固定譯名"), "術語表仍要帶上");
    // 雲端那份的台灣用語長清單對小模型是反效果，不該出現在本地版
    assert!(!local.contains("苦力怕、殭屍"), "百餘字的用語清單不該進本地 prompt");
}

// T1：測試過長，後半段拆到 tests_more.rs（共用本檔的 helper）。
#[path = "tests_more.rs"]
mod more;
