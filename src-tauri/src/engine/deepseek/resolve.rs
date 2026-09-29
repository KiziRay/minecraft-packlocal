//! 共用流程：術語表 → 翻譯記憶 → AI 三層解決（resolve_unique）。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

// ═══ 共用流程 ═════════════════════════════════════════════════

pub(super) struct Resolved {
    pub(super) translations: HashMap<usize, String>,
    pub(super) report: AiFillReport,
    pub(super) quality_deferred: HashSet<usize>,
    /// B4：AI 沒回應（不是品質沒過）的 uid：留在缺口，補充漏翻／接續補完會再送
    pub(super) no_answer: HashSet<usize>,
}

/// 術語表 → 翻譯記憶 → AI，三層依序解決 `unique` 裡的每一條。
///
/// 前兩層完全離線，所以 `use_ai == false` 時仍值得跑。
pub(super) fn resolve_unique(
    unique: &[String],
    ctx: &[Option<&'static str>],
    use_ai: bool,
    reuse_tm: bool,
    allow_quality_retry: bool,
    _quality: TranslationQuality,
    base_pct: u8,
    span_pct: u8,
    scope: Option<&TranslationScope>,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<Resolved, String> {
    cancel::check()?;

    let gloss = glossary::load(None);
    if gloss.user_entries > 0 {
        on_progress(
            base_pct,
            &format!("已套用你自訂的 {} 條譯名", gloss.user_entries),
        );
    }
    // user → builtin（glossary::load 已合併；後寫入的 user 覆寫 builtin）
    let mut tm = TmSaveGuard::new(Tm::load());
    let mut translations: HashMap<usize, String> = HashMap::new();
    let mut report = AiFillReport::default();
    let mut quality_deferred: HashSet<usize> = HashSet::new();
    let mut guard = GuardStats::default();

    let reuse_shared = !super::super::shared_tm::skip_shared_lookup();
    if reuse_shared {
        on_progress(base_pct, "查詢共享庫與術語表（免送 AI）…");
    }

    let glossary_jobs: Vec<shared_glossary::SharedGlossaryJob> = unique
        .iter()
        .enumerate()
        .map(|(uid, source)| shared_glossary::SharedGlossaryJob {
            source: source.clone(),
            context: ctx.get(uid).copied().flatten().map(str::to_owned),
            scope: scope.cloned(),
        })
        .collect();
    let shared_glossary_lookup = if reuse_shared {
        shared_glossary::lookup_detailed(&glossary_jobs)
    } else {
        shared_glossary::LookupResult::default()
    };
    report.notes.push(shared_glossary_lookup.player_note());
    let shared_glossary_hits = &shared_glossary_lookup.hits;

    // 優先序：user glossary → builtin → shared → tm → AI
    let mut need_ai: Vec<(usize, String)> = Vec::new();
    let mut mechanism_kept = 0usize;
    for (uid, src) in unique.iter().enumerate() {
        if let Some(zh) = gloss.exact(src) {
            if let Some(safe) = placeholder::guard(src, zh, &mut guard)
                .filter(|safe| is_usable_zh(src, safe) && !is_poisoned_mech_translation(src, safe))
            {
                translations.insert(uid, safe);
                report.glossary_hits += 1;
                continue;
            }
        }
        if let Some(candidate) = shared_glossary_hits.get(&uid) {
            if let Some(safe) = placeholder::guard(src, candidate, &mut guard) {
                if is_usable_zh(src, &safe) && !is_poisoned_mech_translation(src, &safe) {
                    translations.insert(uid, safe);
                    report.shared_glossary_hits += 1;
                    continue;
                }
            }
        }
        let context = ctx.get(uid).copied().flatten();
        if reuse_tm {
            let tm_hit = match context {
                Some(value) => tm.get_with_context(src, Some(value)),
                None => tm.get(src),
            };
            if let Some(zh) = tm_hit {
                if is_usable_zh(src, &zh) && !is_poisoned_mech_translation(src, &zh) {
                    translations.insert(uid, zh);
                    report.tm_hits += 1;
                    continue;
                }
            }
        }
        // 機制 token／FancyMenu meta：不送 AI（避免再產生毒譯文）
        if skip_before_ai(src) {
            mechanism_kept += 1;
            continue;
        }
        need_ai.push((uid, src.clone()));
    }
    if mechanism_kept > 0 {
        report
            .notes
            .push(format!("{mechanism_kept} 項是機制代號或檔案路徑，保留原文（翻了會失效）"));
    }

    let pre = report.glossary_hits
        + report.shared_glossary_hits
        + report.shared_hits
        + report.tm_hits;
    if pre > 0 {
        on_progress(
            base_pct,
            &format!(
                "免費命中 {} 句（本機術語 {}、共享術語 {}、共享庫 {}、翻譯記憶 {}），只剩 {} 句要送 AI",
                pre,
                report.glossary_hits,
                report.shared_glossary_hits,
                report.shared_hits,
                report.tm_hits,
                need_ai.len()
            ),
        );
    }

    if need_ai.is_empty() {
        report.notes.push(tm.note());
        return Ok(Resolved {
            translations,
            report,
            quality_deferred,
            no_answer: HashSet::new(),
        });
    }

    if !use_ai {
        // 沒勾 AI：前兩層照樣有貢獻，剩下的保留原文
        on_progress(
            base_pct + span_pct.min(100 - base_pct),
            &format!(
                "未使用 AI：離線補了 {} 句，其餘 {} 句保留原文",
                pre,
                need_ai.len()
            ),
        );
        report
            .notes
            .push(format!("未使用 AI，{} 句維持原文", need_ai.len()));
        report.notes.push(tm.note());
        return Ok(Resolved {
            translations,
            report,
            quality_deferred,
            no_answer: HashSet::new(),
        });
    }

    // ── 第 3 層：AI（限次重試佔位符拒譯；負向快取避免同輪重燒）──
    //
    // ⚠️ 這一段的失敗**絕對不可以用 `?` 往上拋**。
    //
    // `translations` 此刻已經裝著資料層（術語表／共享術語／共享翻譯庫／翻譯記憶）
    // 的全部命中。實測站長一包：33233 條待譯裡，共享庫命中 32151、共享術語 671，
    // 真正要送 AI 的只有 271 條。舊版在這裡 `?` 一拋，那 32822 條查得到的譯文
    // **全部被丟掉**，整個階段 0 產出——FTB Quests 與文字覆寫在 GPT 額度用盡時
    // 一個字都沒翻出來，就是這樣來的。
    //
    // 使用者明確選了 AI，就不能在 AI 無法翻譯時靜默降級成「不使用 AI」。
    // 那會把「有 AI 品質把關」的期待變成只有共享庫／快取的部分結果，還讓人誤以為
    // 整包已照選擇翻完。開工前已有嚴格探測；若執行中端點仍失效，也必須中止並保留
    // 可診斷原因，而不是改走另一個未被選擇的流程。
    //
    // B4：AI 不可用（或這一輪已經停過）時，改成「停下 AI、保留資料層成果、其餘留在缺口」，
    // 並在 report.ai_unavailable 寫下原因——呼叫端據此把這一輪算成**部分完成**、
    // 不宣稱完成，也不會再去空轉重試。這仍然不是「靜默降級」：原因會寫進日誌與結果。
    let halted_already = super::super::run_interrupt::current();
    let ai_ready = match &halted_already {
        Some(halt) => Err(halt.reason.clone()),
        None => {
            on_progress(base_pct, "連線 AI 並探測服務…");
            ai_preflight()
        }
    };
    let engine = match ai_ready {
        Ok(engine) => engine,
        Err(reason) => {
            let by_user = is_cancel_message(&reason)
                || halted_already
                    .as_ref()
                    .is_some_and(|h| h.kind == super::super::run_interrupt::HaltKind::UserStop);
            let short = reason.lines().next().unwrap_or("AI 不可用").to_string();
            if halted_already.is_none() {
                if by_user {
                    super::super::run_interrupt::stop_by_user(&reason);
                } else {
                    super::super::run_interrupt::halt_ai(&reason);
                }
            }
            on_progress(
                base_pct,
                &format!(
                    "AI 這一輪不能用（{short}）：已先保留免費命中的 {pre} 句，其餘 {} 句留在缺口，之後接續補完再翻。",
                    need_ai.len()
                ),
            );
            crate::dev_log!("ai", "AI 不可用，保留資料層 {} 句、缺口 {} 句：{}", pre, need_ai.len(), reason);
            report.ai_unavailable = Some(reason);
            report.stopped_by_user = by_user;
            report.notes.push(tm.note());
            let no_answer = need_ai.iter().map(|(uid, _)| *uid).collect();
            return Ok(Resolved {
                translations,
                report,
                quality_deferred,
                no_answer,
            });
        }
    };
    let is_local_model = matches!(engine.provider, AiProvider::LocalLlm);
    if is_local_model {
        // B4：本地模型剛啟動時的決策（上下文、同時處理數、顯示卡層數）寫進使用者看得到的日誌
        if let Some(note) = crate::engine::local_llm::take_start_note() {
            on_progress(base_pct, &note);
            report.notes.push(note);
        }
    }
    // 本地模型重試只花時間、不花錢也不吃額度，所以品質沒過就該再試一次，
    // 不必像雲端那樣等使用者選「強制模式」。這是本地與雲端成本結構的根本差異。
    let allow_quality_retry = allow_quality_retry || is_local_model;
    let system_prompt = build_system_prompt(&gloss, unique, is_local_model);
    // 審查 1c：本地模型的並行不超過伺服器實際開的同時處理數（記憶體不夠時會降成 1）
    let parallel_cap = if is_local_model {
        local_parallel_slots(&engine, &crate::engine::local_llm::load_state()) as usize
    } else {
        engine.capabilities.start_parallel
    };
    let mut batch_state = BatchRuntimeState::new(system_prompt, parallel_cap);
    let mut term_stats = TermConsistencyStats::default();
    let mut pending_ai: Vec<PendingItem> = need_ai
        .into_iter()
        .filter_map(|(uid, src)| {
            if is_placeholder_negatively_cached(&src) {
                report.rejected += 1;
                None
            } else {
                Some(PendingItem {
                    uid,
                    source: src,
                    reason: PendingReason::NoAnswer,
                })
            }
        })
        .collect();
    on_progress(
        base_pct,
        &format!("AI 翻譯 {} 句（已扣掉重複與已知譯名）…", pending_ai.len()),
    );

    // 長句先拆句再送。本地小模型碰到長從屬句常「只翻到一半」甚至語意反轉
    // （實測：`Heal … Or Harm Undead Creatures` 被譯成「造成 4 點傷害」，
    // 治療變傷害而且整個 Or 子句消失）。拆開之後每一段都短、指涉清楚。
    // 這一步失敗時項目原封不動退回下面的主迴圈整句翻，不會留下半成品。
    // B4：AI 中途停下的原因（`(白話原因, 是不是使用者按停止)`）。停下之後不再送任何 AI 請求。
    let mut ai_stop: Option<(String, bool)> = None;
    if !pending_ai.is_empty() {
        let before_split = pending_ai.clone();
        pending_ai = match translate_long_sentences_split(
            &engine,
            &mut batch_state,
            &gloss,
            &mut term_stats,
            &mut guard,
            &mut tm,
            &mut translations,
            &mut report,
            pending_ai,
            ctx,
            reuse_tm,
            base_pct,
            span_pct,
            on_progress,
        ) {
            Ok(left) => left,
            // 取消：已翻好的都在 translations 裡，沒翻的原樣留在缺口（舊版 `?` 會整段丟掉）
            Err(e) => {
                ai_stop = Some((e, true));
                before_split
                    .into_iter()
                    .filter(|item| !translations.contains_key(&item.uid))
                    .collect()
            }
        };
        take_batch_stop(&mut batch_state, &mut ai_stop);
    }

    let mut attempt = 0usize;
    // B4：沒回應的句子同一輪縮小批次重送（每次再切一半）；格式符號壞掉的放一邊，最後照舊處理
    let mut no_answer_pass = 0usize;
    let mut broken_final: Vec<PendingItem> = Vec::new();
    while !pending_ai.is_empty() && ai_stop.is_none() {
        if attempt > 0 {
            on_progress(
                base_pct.saturating_add(span_pct / 2),
                &format!(
                    "AI 沒回應的 {} 句：同一輪縮小批次重送（第 {} 次，每批約原本的 1/{}）…",
                    pending_ai.len(),
                    no_answer_pass,
                    batch_state.batch_divisor
                ),
            );
        }
        let masked_need_ai = build_masked_items(&pending_ai, ctx);
        // B4 #6：每一批回來就先把過關的譯文寫進翻譯記憶並落盤，程式中途被關掉也不會整段白翻
        let raw = {
            let by_uid: HashMap<usize, &MaskedItem> =
                masked_need_ai.iter().map(|item| (item.uid, item)).collect();
            let mut scratch_stats = TermConsistencyStats::default();
            let mut sink = |batch: &HashMap<usize, String>| {
                for (uid, masked_out) in batch {
                    let Some(item) = by_uid.get(uid) else { continue };
                    let candidate = placeholder::unmask(masked_out, &item.tokens);
                    let candidate = gloss.enforce_terms(&item.source, &candidate, &mut scratch_stats);
                    if let ((CandidateClass::Accept, Some(safe), _, _), _) =
                        classify_candidate_for(&item.source, &candidate, is_local_model)
                    {
                        remember_in_tm(&mut tm, reuse_tm, &item.source, &safe, item.context);
                    }
                }
                if let Err(e) = tm.flush_if_due() {
                    crate::dev_log!("ai", "翻譯記憶逐批落盤失敗（不影響翻譯）：{e}");
                }
            };
            match run_batches_with(
                &engine,
                &mut batch_state,
                &masked_need_ai,
                base_pct,
                span_pct,
                on_progress,
                false,
                Some(&mut sink),
            ) {
                Ok(raw) => raw,
                Err(e) => {
                    if ai_stop.is_none() && batch_state.stop.is_none() {
                        ai_stop = Some((e.clone(), is_cancel_message(&e)));
                    }
                    HashMap::new()
                }
            }
        };
        take_batch_stop(&mut batch_state, &mut ai_stop);

        let mut next_pending = collect_unresolved_items(&masked_need_ai, &raw);
        let unresolved_ids: HashSet<usize> = next_pending.iter().map(|item| item.uid).collect();
        let mut placeholder_retry: Vec<MaskedItem> = Vec::new();
        let mut quality_retry: Vec<MaskedItem> = Vec::new();
        for item in &masked_need_ai {
            if unresolved_ids.contains(&item.uid) {
                continue;
            }
            let Some(masked_out) = raw.get(&item.uid) else {
                continue;
            };
            let candidate = placeholder::unmask(masked_out, &item.tokens);
            let candidate = gloss.enforce_terms(&item.source, &candidate, &mut term_stats);
            let ((class, safe, attempt_guard, qreason), local_issue) =
                classify_candidate_for(&item.source, &candidate, is_local_model);
            record_local_issue(&mut report, local_issue);
            guard.checked += attempt_guard.checked;
            guard.repaired += attempt_guard.repaired;
            guard.rejected += attempt_guard.rejected;
            match class {
                CandidateClass::Accept => {
                    let safe = safe.expect("Accept always has safe text");
                    let context = item.context;
                    if reuse_tm {
                        if let Some(value) = context {
                            tm.insert_with_context(&item.source, &safe, Some(value));
                        } else {
                            tm.insert(&item.source, &safe);
                        }
                    } else if let Some(value) = context {
                        tm.upsert_with_context(&item.source, &safe, Some(value));
                    } else {
                        tm.upsert(&item.source, &safe);
                    }
                    translations.insert(item.uid, safe);
                    report.ai_translated += 1;
                }
                CandidateClass::QualityFail => {
                    if allow_quality_retry {
                        // 強制模式才允許同輪品質複查一次。
                        quality_retry.push(item.clone());
                    } else {
                        // 一般模式不重送相同項目；保留具體 uid 供工作階段暫緩。
                        keep_english_skip(
                            &mut translations,
                            &mut report,
                            item.uid,
                            &item.source,
                            false,
                            qreason,
                        );
                        quality_deferred.insert(item.uid);
                    }
                }
                CandidateClass::PlaceholderFail => {
                    placeholder_retry.push(item.clone());
                }
            }
        }
        if !quality_retry.is_empty() && ai_stop.is_some() {
            // AI 已停：不再複查，照一般品質暫緩處理
            for item in &quality_retry {
                keep_english_skip(&mut translations, &mut report, item.uid, &item.source, false, None);
                quality_deferred.insert(item.uid);
            }
            quality_retry.clear();
        }
        if !quality_retry.is_empty() {
            on_progress(
                base_pct.saturating_add(span_pct / 4),
                &format!(
                    "品質複查：強制模式重試 {} 句…",
                    quality_retry.len()
                ),
            );
            let quality_raw = match run_batches(
                &engine,
                &mut batch_state,
                &quality_retry,
                base_pct,
                span_pct,
                on_progress,
                false,
            ) {
                Ok(raw) => raw,
                Err(e) if is_cancel_message(&e) => {
                    // B4：取消不再整段丟掉：主批成果保留，這批照品質暫緩處理
                    ai_stop.get_or_insert((e, true));
                    for item in &quality_retry {
                        keep_english_skip(&mut translations, &mut report, item.uid, &item.source, false, None);
                    }
                    HashMap::new()
                }
                Err(e) => {
                    report.notes.push(format!(
                        "品質再試中斷（已保留主批成果）：{}",
                        e.lines().next().unwrap_or("連線問題")
                    ));
                    for item in &quality_retry {
                        keep_english_skip(
                            &mut translations,
                            &mut report,
                            item.uid,
                            &item.source,
                            false,
                            None,
                        );
                    }
                    HashMap::new()
                }
            };
            take_batch_stop(&mut batch_state, &mut ai_stop);
            let mut quality_skip_round = 0usize;
            for item in &quality_retry {
                let Some(masked_out) = quality_raw.get(&item.uid) else {
                    if quality_raw.is_empty() {
                        // 已在 Err 分支計入 quality_skipped
                        quality_skip_round += 1;
                        continue;
                    }
                    keep_english_skip(
                        &mut translations,
                        &mut report,
                        item.uid,
                        &item.source,
                        false,
                        None,
                    );
                    quality_skip_round += 1;
                    continue;
                };
                let candidate = placeholder::unmask(masked_out, &item.tokens);
                let candidate = gloss.enforce_terms(&item.source, &candidate, &mut term_stats);
                let ((class, safe, attempt_guard, qreason), local_issue) =
                    classify_candidate_for(&item.source, &candidate, is_local_model);
                record_local_issue(&mut report, local_issue);
                guard.checked += attempt_guard.checked;
                guard.repaired += attempt_guard.repaired;
                guard.rejected += attempt_guard.rejected;
                match class {
                    CandidateClass::Accept => {
                        let safe = safe.expect("Accept always has safe text");
                        if reuse_tm {
                            if let Some(value) = item.context {
                                tm.insert_with_context(&item.source, &safe, Some(value));
                            } else {
                                tm.insert(&item.source, &safe);
                            }
                        } else if let Some(value) = item.context {
                            tm.upsert_with_context(&item.source, &safe, Some(value));
                        } else {
                            tm.upsert(&item.source, &safe);
                        }
                        translations.insert(item.uid, safe);
                        report.ai_translated += 1;
                    }
                    CandidateClass::QualityFail | CandidateClass::PlaceholderFail => {
                        keep_english_skip(
                            &mut translations,
                            &mut report,
                            item.uid,
                            &item.source,
                            false,
                            qreason,
                        );
                        // 品質未過一律排進補完佇列——包含**沒有 qreason 的那種**。
                        // 本地模型被擋下的退化輸出（`{0` 亂碼、只剩半個詞）走的就是
                        // qreason=None 這條路；舊版因此把它們排除在補完之外，
                        // 等於「本地翻不好的句子永遠翻不好」。
                        if matches!(class, CandidateClass::QualityFail) {
                            quality_deferred.insert(item.uid);
                        }
                        quality_skip_round += 1;
                    }
                }
            }
            if quality_skip_round > 0 {
                on_progress(
                    base_pct.saturating_add(span_pct / 4),
                    &format!(
                        "品質未過略過 {} 句（未寫入）",
                        quality_skip_round
                    ),
                );
            }
        }

        if !placeholder_retry.is_empty() && ai_stop.is_some() {
            // AI 已停：格式符號壞掉的先當成「沒翻到」，下次接續補完再送（不寫英文、不進負向快取）
            for item in placeholder_retry.drain(..) {
                next_pending.push(PendingItem {
                    uid: item.uid,
                    source: item.source,
                    reason: PendingReason::NoAnswer,
                });
            }
        }
        if !placeholder_retry.is_empty() {
            let total_ph = placeholder_retry.len();
            let (to_strict, overflow) =
                split_strict_retry_cap(placeholder_retry, STRICT_PLACEHOLDER_RETRY_CAP);
            if !overflow.is_empty() {
                let n = overflow.len();
                for item in &overflow {
                    keep_english_skip(
                        &mut translations,
                        &mut report,
                        item.uid,
                        &item.source,
                        true,
                        None,
                    );
                }
                on_progress(
                    base_pct.saturating_add(span_pct / 2),
                    &format!(
                        "佔位符失敗 {} 句：嚴格重試至多 {} 句，已略過 {} 句（超過嚴格上限）",
                        total_ph,
                        STRICT_PLACEHOLDER_RETRY_CAP,
                        n
                    ),
                );
                report.notes.push(format!(
                    "佔位符嚴格重試上限 {}：略過 {} 句（保留英文，不重燒）",
                    STRICT_PLACEHOLDER_RETRY_CAP, n
                ));
            }
            if !to_strict.is_empty() {
                on_progress(
                    base_pct.saturating_add(span_pct / 2),
                    &format!(
                        "佔位符嚴格重試 {} 句（上限 {}）…",
                        to_strict.len(),
                        STRICT_PLACEHOLDER_RETRY_CAP
                    ),
                );
                let strict_raw = match run_batches(
                    &engine,
                    &mut batch_state,
                    &to_strict,
                    base_pct,
                    span_pct,
                    on_progress,
                    true,
                ) {
                    Ok(raw) => raw,
                    Err(e) => {
                        if is_cancel_message(&e) {
                            ai_stop.get_or_insert((e.clone(), true));
                        }
                        report.notes.push(format!(
                            "佔位符嚴格重試中斷（已保留既有成果）：{}",
                            e.lines().next().unwrap_or("連線問題")
                        ));
                        for item in &to_strict {
                            next_pending.push(PendingItem {
                                uid: item.uid,
                                source: item.source.clone(),
                                reason: PendingReason::NoAnswer,
                            });
                        }
                        HashMap::new()
                    }
                };
                take_batch_stop(&mut batch_state, &mut ai_stop);
                for item in &to_strict {
                    let Some(masked_out) = strict_raw.get(&item.uid) else {
                        if strict_raw.is_empty() {
                            continue;
                        }
                        next_pending.push(PendingItem {
                            uid: item.uid,
                            source: item.source.clone(),
                            reason: PendingReason::NoAnswer,
                        });
                        continue;
                    };
                    let candidate = placeholder::unmask(masked_out, &item.tokens);
                    let candidate = gloss.enforce_terms(&item.source, &candidate, &mut term_stats);
                    let ((class, safe, attempt_guard, qreason), local_issue) =
                        classify_candidate_for(&item.source, &candidate, is_local_model);
                    record_local_issue(&mut report, local_issue);
                    guard.checked += attempt_guard.checked;
                    guard.repaired += attempt_guard.repaired;
                    guard.rejected += attempt_guard.rejected;
                    match class {
                        CandidateClass::Accept => {
                            let safe = safe.expect("Accept always has safe text");
                            if reuse_tm {
                                if let Some(value) = item.context {
                                    tm.insert_with_context(&item.source, &safe, Some(value));
                                } else {
                                    tm.insert(&item.source, &safe);
                                }
                            } else if let Some(value) = item.context {
                                tm.upsert_with_context(&item.source, &safe, Some(value));
                            } else {
                                tm.upsert(&item.source, &safe);
                            }
                            translations.insert(item.uid, safe);
                            report.ai_translated += 1;
                        }
                        CandidateClass::QualityFail => {
                            keep_english_skip(
                                &mut translations,
                                &mut report,
                                item.uid,
                                &item.source,
                                false,
                                qreason,
                            );
                            // 沒有 qreason 的品質未過＝本地模型退化被擋下，
                            // 同樣要排進補完佇列（見上面同類註解）
                            quality_deferred.insert(item.uid);
                        }
                        CandidateClass::PlaceholderFail => {
                            next_pending.push(PendingItem {
                                uid: item.uid,
                                source: item.source.clone(),
                                reason: PendingReason::PlaceholderBroken,
                            });
                        }
                    }
                }
            }
        }

        // 沒回應的繼續下一次（縮小批次）；格式符號壞掉的放一邊
        let (no_answer_items, broken): (Vec<PendingItem>, Vec<PendingItem>) = next_pending
            .into_iter()
            .partition(|item| item.reason == PendingReason::NoAnswer);
        broken_final.extend(broken);
        pending_ai = no_answer_items;
        attempt += 1;
        if pending_ai.is_empty() || ai_stop.is_some() || no_answer_pass >= NO_ANSWER_RESEND_PASSES {
            break;
        }
        no_answer_pass += 1;
        batch_state.batch_divisor = 1usize << no_answer_pass;
        crate::dev_log!(
            "ai",
            "沒回應 {} 句：同輪縮小批次重送（第 {} 次，每批切成 {} 份）",
            pending_ai.len(),
            no_answer_pass,
            batch_state.batch_divisor
        );
    }
    batch_state.batch_divisor = 1;
    pending_ai.extend(broken_final);
    if let Some((reason, by_user)) = &ai_stop {
        // B4 #7：AI 不可用要正確設值——資料層與已翻部分都保留，但這一輪只算部分完成
        report.ai_unavailable = Some(reason.clone());
        report.stopped_by_user = *by_user;
        if *by_user {
            super::super::run_interrupt::stop_by_user(reason);
        } else {
            super::super::run_interrupt::halt_ai(reason);
        }
        report.notes.push(format!(
            "AI 在這一輪中途停下：{}。已翻好的都保留；沒翻到的 {} 句留在缺口，之後接續補完再翻。",
            reason.lines().next().unwrap_or("連線問題"),
            pending_ai.len()
        ));
    }

    if report.quality_skipped > 0 {
        report.notes.push(format!(
            "品質未過略過 {} 句（未寫入；{}={}／{}={}／{}={}）",
            report.quality_skipped,
            QualityFailReason::StillEnglish.as_str(),
            report.quality_still_english,
            QualityFailReason::MixedFragment.as_str(),
            report.quality_mixed_fragment,
            QualityFailReason::SameAsSource.as_str(),
            report.quality_same_as_source
        ));
    }
    if !report.local_degenerate.is_empty() {
        // 這段話要讓「不懂技術的玩家」看得懂發生什麼事、要不要理它。
        // 關鍵是分清楚兩種情況：**有翻出來但不夠好**、和**沒翻出來留英文**——
        // 前者不必做什麼，後者才需要考慮改用線上 AI。
        let detail = report
            .local_degenerate
            .iter()
            .map(|(label, count)| format!("{label} {count} 句"))
            .collect::<Vec<_>>()
            .join("、");
        let total: usize = report.local_degenerate.iter().map(|(_, c)| *c).sum();
        report.notes.push(format!(
            "本地模型有 {total} 句翻得不太好（{detail}）。這是離線小模型的能力限制，\
不是工具漏掉沒翻。其中只有會誤導你的（數字對不上、整段重複）才會留原文，\
其餘照樣採用——讀起來不完美，但看得懂。想要更好的品質，可以在工作台的「AI 輔助翻譯」改選「自訂 API」或「GPT」。"
        ));
    }
    let mut no_answer: HashSet<usize> = HashSet::new();
    drain_pending_ai(
        &pending_ai,
        &mut translations,
        &mut report,
        &mut no_answer,
    );

    // 本地模式的最後一步：小模型過不了品質關、或這一輪沒拿到回應的句子，
    // 改用雲端翻一次。
    //
    // 「本地翻譯品質要跟雲端一樣」這個要求，靠調本地模型本身是做不到的——
    // 小模型的能力就是不如大模型。但**交付出去的結果**可以一樣：
    // 讓本地負責它做得好的（絕大多數），做不好的那些交給雲端補完。
    //
    // 這段必須排在 pending_ai 收尾**之後**：沒拿到回應的句子是在上面那段
    // 才進 quality_deferred 的，排在前面就會漏掉它們（舊版就是排在前面）。
    //
    // B4 #7：只呼叫一次（舊版上下各一次，雲端也翻不好的句子會被送兩次、重複花錢）。
    // 使用者按了停止就不補。
    let stopped_by_user = ai_stop.as_ref().is_some_and(|(_, by_user)| *by_user);
    if is_local_model && !stopped_by_user && (!quality_deferred.is_empty() || !no_answer.is_empty()) {
        let mut to_top_up: HashSet<usize> = quality_deferred.union(&no_answer).copied().collect();
        escalate_to_cloud(
            &gloss,
            &mut term_stats,
            &mut guard,
            &mut tm,
            &mut translations,
            &mut report,
            &mut to_top_up,
            unique,
            ctx,
            reuse_tm,
            base_pct,
            span_pct,
            on_progress,
        );
        // 補好的從兩份清單移除；補好的品質暫緩句不該再算在「品質未過」
        let fixed_quality = quality_deferred.iter().filter(|uid| !to_top_up.contains(uid)).count();
        report.quality_skipped = report.quality_skipped.saturating_sub(fixed_quality);
        quality_deferred.retain(|uid| to_top_up.contains(uid));
        no_answer.retain(|uid| to_top_up.contains(uid));
    }

    report.usage = engine.usage_snapshot();
    for note in engine.drain_notices() {
        report.notes.push(note);
    }
    if let Some(cost_note) = engine.capabilities.cost_note(
        engine.provider,
        report.usage.prompt_cache_hit_tokens,
        report.usage.prompt_cache_miss_tokens,
        report.usage.completion_tokens,
    ) {
        report.notes.push(cost_note);
    }

    on_progress(
        base_pct.saturating_add(span_pct.saturating_mul(95) / 100),
        "正在儲存本機翻譯記憶…",
    );
    if let Err(e) = tm.save() {
        report.notes.push(format!("翻譯記憶未能存檔：{e}"));
    }
    report.notes.push(tm.note());
    if guard.repaired > 0 || guard.rejected > 0 {
        report.notes.push(guard.note());
    }
    if let Some(note) = term_stats.note() {
        report.notes.push(note);
    }

    // 社群貢獻改由 fill_missing 寫回後的 keyed to_share 一次送出（避免雙重上傳卡住 UI）
    Ok(Resolved {
        translations,
        report,
        quality_deferred,
        no_answer,
    })
}
