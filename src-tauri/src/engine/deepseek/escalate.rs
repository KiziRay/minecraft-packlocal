//! 批次停止、寫入翻譯記憶、雲端補完與長句拆分。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

/// B4：沒回應的句子在同一輪最多再縮小批次重送幾次（每次每批再切一半）。
pub(super) const NO_ANSWER_RESEND_PASSES: usize = 2;

/// B4：run_batches 提早結束的原因 → resolve_unique 的「AI 停下」狀態（只記第一個）。
pub(super) fn take_batch_stop(batch_state: &mut BatchRuntimeState, ai_stop: &mut Option<(String, bool)>) {
    let Some(stop) = batch_state.stop.take() else { return };
    if ai_stop.is_some() {
        return;
    }
    *ai_stop = match stop {
        BatchStop::Cancelled => Some((CANCEL_MESSAGE.to_string(), true)),
        BatchStop::AiUnavailable(reason) => Some((reason, false)),
        // 連續沒有新譯文：不算 AI 壞掉，但這一輪不再硬送（避免空轉），沒翻到的留缺口
        BatchStop::NoProgress(reason) => Some((reason, false)),
    };
}

/// 過關的譯文寫進翻譯記憶（Force 模式覆寫舊記憶）。
pub(super) fn remember_in_tm(tm: &mut TmSaveGuard, reuse_tm: bool, source: &str, safe: &str, context: Option<&'static str>) {
    match (reuse_tm, context) {
        (true, Some(value)) => tm.insert_with_context(source, safe, Some(value)),
        (true, None) => tm.insert(source, safe),
        (false, Some(value)) => tm.upsert_with_context(source, safe, Some(value)),
        (false, None) => tm.upsert(source, safe),
    }
}

/// 本地模型過不了品質關的句子，改用雲端補完，讓最終品質與全程走雲端一致。
///
/// 為什麼需要這一步：小模型的**能力**不可能等於大模型，這是模型本身的限制，
/// 調參數、換 prompt 都繞不過去。但站長要的是「**結果**一樣」——那就讓本地做它
/// 做得好的（絕大多數短句、名稱、術語），把它做不好的那一小撮交給雲端。
///
/// 沒有可用雲端時（沒設自訂 API、也沒登入 GPT）不是失敗，只是補不了；
/// 這時要明講「設定哪一個就能自動補完」，而不是留下一句沒人看得懂的錯誤。
#[allow(clippy::too_many_arguments)]
pub(super) fn escalate_to_cloud(
    gloss: &Glossary,
    term_stats: &mut TermConsistencyStats,
    guard: &mut GuardStats,
    tm: &mut TmSaveGuard,
    translations: &mut HashMap<usize, String>,
    report: &mut AiFillReport,
    quality_deferred: &mut HashSet<usize>,
    unique: &[String],
    ctx: &[Option<&'static str>],
    reuse_tm: bool,
    base_pct: u8,
    span_pct: u8,
    on_progress: &mut dyn FnMut(u8, &str),
) {
    let pending: Vec<PendingItem> = {
        let mut ids: Vec<usize> = quality_deferred.iter().copied().collect();
        ids.sort_unstable(); // HashSet 疊代順序不穩定，排序才能重現
        ids.into_iter()
            .filter_map(|uid| {
                unique.get(uid).map(|source| PendingItem {
                    uid,
                    source: source.clone(),
                    reason: PendingReason::NoAnswer,
                })
            })
            .collect()
    };
    if pending.is_empty() {
        return;
    }

    let Some(engine) = Engine::connect_cloud_fallback() else {
        // 兩種原因：玩家自己關掉了補完，或是還沒有可用的線上 AI。
        // 分開講，因為要採取的行動完全不同。
        if !super::super::secrets::cloud_topup_enabled() {
            report.notes.push(format!(
                "有 {} 句本地模型翻不好，依你的設定保留原文（沒有送到線上 AI）。",
                pending.len()
            ));
        } else {
            report.notes.push(format!(
                "有 {} 句本地模型翻不好，先保留原文。到工作台的「AI 輔助翻譯」改選「自訂 API」或「GPT」之後，工具會自動用線上 AI 把這些句子補完，品質就跟全程用線上 AI 一樣。",
                pending.len()
            ));
        }
        return;
    };
    on_progress(
        base_pct,
        &format!(
            "本地模型翻不好的 {} 句，改用線上 AI 補完（讓品質一致）…",
            pending.len()
        ),
    );

    let items = build_masked_items(&pending, ctx);
    let system_prompt = build_system_prompt(gloss, unique, false);
    let mut state = BatchRuntimeState::new(system_prompt, engine.capabilities.start_parallel);
    let raw = match run_batches(&engine, &mut state, &items, base_pct, span_pct, on_progress, false)
    {
        Ok(raw) => raw,
        Err(e) => {
            // 補完失敗不影響本地已經翻好的部分，照原樣留在待補清單
            report.notes.push(format!(
                "線上 AI 補完沒有完成，這些句子先保留原文：{}",
                e.lines().next().unwrap_or("連線問題")
            ));
            return;
        }
    };

    let mut fixed = 0usize;
    for item in &items {
        let Some(masked_out) = raw.get(&item.uid) else {
            continue;
        };
        let candidate = placeholder::unmask(masked_out, &item.tokens);
        let candidate = gloss.enforce_terms(&item.source, &candidate, term_stats);
        // 雲端的產出一樣要過把關；這裡不開本地專屬檢查（大模型不犯那些病）
        let ((class, safe, attempt_guard, _), _) =
            classify_candidate_for(&item.source, &candidate, false);
        guard.checked += attempt_guard.checked;
        guard.repaired += attempt_guard.repaired;
        guard.rejected += attempt_guard.rejected;
        if !matches!(class, CandidateClass::Accept) {
            continue;
        }
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
        quality_deferred.remove(&item.uid);
        report.ai_translated += 1;
        // 「品質未過」的扣減由呼叫端依原清單處理（沒回應的句子本來就不在那個計數裡）
        fixed += 1;
    }

    report.usage.add(&engine.usage_snapshot());
    if fixed > 0 {
        report.notes.push(format!(
            "已用線上 AI 補完 {fixed} 句本地模型翻不好的內容，品質與全程使用線上 AI 一樣。"
        ));
    }
    let left = pending.len().saturating_sub(fixed);
    if left > 0 {
        report
            .notes
            .push(format!("還有 {left} 句連線上 AI 也沒過品質檢查，已列進沒翻到的清單。"));
    }
}

/// 本地小模型專用：把長句拆成子句分別送翻，再依原順序組回去。
///
/// 回傳「這一輪沒解決」的項目，交給主迴圈照原本的整句流程處理。
/// 只要拆句這條路有任何一步不順（子句沒回來、組回來的譯文沒過把關、
/// 連線失敗），該句就原封不動退回去整句翻——**寧可品質差一點，
/// 也不要交出半中半英或順序錯亂的東西**。
///
/// 防錯位的關鍵：每個子句配一個**獨立 uid**（排在真實 uid 之後），
/// 送進既有批次引擎後用 uid 一一取回，完全不依賴模型維持順序或數量。
#[allow(clippy::too_many_arguments)]
pub(super) fn translate_long_sentences_split(
    engine: &Engine,
    batch_state: &mut BatchRuntimeState,
    gloss: &Glossary,
    term_stats: &mut TermConsistencyStats,
    guard: &mut GuardStats,
    tm: &mut TmSaveGuard,
    translations: &mut HashMap<usize, String>,
    report: &mut AiFillReport,
    pending: Vec<PendingItem>,
    ctx: &[Option<&'static str>],
    reuse_tm: bool,
    base_pct: u8,
    span_pct: u8,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<Vec<PendingItem>, String> {
    let is_local = matches!(engine.provider, AiProvider::LocalLlm);
    let mut leftover: Vec<PendingItem> = Vec::with_capacity(pending.len());
    let mut jobs: Vec<(PendingItem, Vec<String>)> = Vec::new();
    for item in pending {
        // 用 _safe 版本：拆句只是最佳化，不該有能力中斷整輪翻譯。
        // v21 就是這裡一個位元組索引 bug 讓整包翻譯在 63% 全毀。
        let segments = if sentence_split::should_split_safe(&item.source, is_local) {
            sentence_split::split_sentence_safe(&item.source)
        } else {
            Vec::new()
        };
        if segments.len() >= 2 {
            jobs.push((item, segments));
        } else {
            leftover.push(item);
        }
    }
    if jobs.is_empty() {
        return Ok(leftover);
    }

    // 子句 uid 一律排在所有真實 uid 之後，確保不會和任何待譯項目相撞
    let mut next_uid = ctx
        .len()
        .max(jobs.iter().map(|(item, _)| item.uid + 1).max().unwrap_or(0));
    let mut segment_items: Vec<MaskedItem> = Vec::new();
    // (原句 uid, 該句的子句 uid 依原順序)
    let mut segment_ids: Vec<Vec<usize>> = Vec::with_capacity(jobs.len());
    for (item, segments) in &jobs {
        let context = ctx.get(item.uid).copied().flatten();
        let mut ids = Vec::with_capacity(segments.len());
        for segment in segments {
            let (masked, tokens) = placeholder::mask(segment);
            segment_items.push(MaskedItem {
                uid: next_uid,
                source: segment.clone(),
                masked,
                tokens,
                context,
            });
            ids.push(next_uid);
            next_uid += 1;
        }
        segment_ids.push(ids);
    }

    on_progress(
        base_pct,
        &format!(
            "長句拆解：{} 句較長的說明先拆成 {} 個子句分別翻譯（降低漏譯與語意錯誤）…",
            jobs.len(),
            segment_items.len()
        ),
    );

    let raw = match run_batches(
        engine,
        batch_state,
        &segment_items,
        base_pct,
        span_pct,
        on_progress,
        false,
    ) {
        Ok(raw) => raw,
        Err(e) if is_cancel_message(&e) => return Err(e),
        Err(e) => {
            // 拆句這條路壞掉不該影響正事：整批退回整句翻
            report.notes.push(format!(
                "長句拆解未完成，改用整句翻譯：{}",
                e.lines().next().unwrap_or("連線問題")
            ));
            leftover.extend(jobs.into_iter().map(|(item, _)| item));
            return Ok(leftover);
        }
    };

    let by_uid: HashMap<usize, &MaskedItem> =
        segment_items.iter().map(|item| (item.uid, item)).collect();
    let mut merged = 0usize;
    for ((item, segments), ids) in jobs.into_iter().zip(segment_ids.into_iter()) {
        let pieces: Vec<Option<String>> = ids
            .iter()
            .map(|id| {
                let segment = by_uid.get(id)?;
                let masked_out = raw.get(id)?;
                let text = placeholder::unmask(masked_out, &segment.tokens);
                Some(gloss.enforce_terms(&segment.source, &text, term_stats))
            })
            .collect();
        let Some(joined) = sentence_split::rejoin(&segments, &pieces) else {
            leftover.push(item);
            continue;
        };
        // 組回來的整句要重新過一次佔位符與品質檢查——拆句不是免檢通道
        let ((class, safe, attempt_guard, _), local_issue) =
            classify_candidate_for(&item.source, &joined, is_local);
        record_local_issue(report, local_issue);
        guard.checked += attempt_guard.checked;
        guard.repaired += attempt_guard.repaired;
        guard.rejected += attempt_guard.rejected;
        match class {
            CandidateClass::Accept => {
                let safe = safe.expect("Accept always has safe text");
                let context = ctx.get(item.uid).copied().flatten();
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
                merged += 1;
            }
            // 拆句版本沒過把關就當作沒發生過，退回整句翻
            CandidateClass::QualityFail | CandidateClass::PlaceholderFail => leftover.push(item),
        }
    }
    if merged > 0 {
        on_progress(
            base_pct,
            &format!("長句拆解完成 {merged} 句；其餘退回整句翻譯"),
        );
    }
    Ok(leftover)
}
