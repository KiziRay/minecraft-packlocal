//! 語境判斷與主要入口：補語言表缺漏、共享術語。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

// ═══ 語境判斷 ═════════════════════════════════════════════════

/// 由 lang key 推測語境，讓 AI 知道這是物品名還是整句提示。
///
/// `item.create.wrench` → 物品名 → AI 會給「扳手」而不是「一把用來旋轉的工具」。
pub fn context_hint(lang_key: &str) -> Option<&'static str> {
    let lower = lang_key.to_ascii_lowercase();
    let mut segments = lower.split('.');
    if let Some(hit) = segments.next().and_then(kind_of_segment) {
        return Some(hit);
    }
    // `create.tooltip.xxx`：模組把自己的 id 放最前面，往後找
    lower.split('.').find_map(kind_of_segment)
}

pub(super) fn kind_of_segment(seg: &str) -> Option<&'static str> {
    Some(match seg {
        "item" | "itemgroup" | "item_group" => "物品名",
        "block" => "方塊名",
        "fluid" => "液體名",
        "entity" => "生物名",
        "biome" => "生態域名",
        "effect" | "mob_effect" | "potion" => "狀態效果名",
        "enchantment" => "附魔名",
        "advancement" | "advancements" => "進度名稱或說明",
        "death" => "死亡訊息",
        "subtitles" | "subtitle" => "音效字幕",
        "key" | "keybind" | "keybinds" => "按鍵綁定名稱",
        "gui" | "menu" | "screen" | "container" | "options" | "button" => "介面文字",
        "tooltip" | "desc" | "description" | "info" | "hint" => "提示說明",
        "command" | "commands" | "argument" => "指令訊息",
        "config" | "configuration" => "設定項",
        "chat" | "message" | "msg" => "訊息",
        "quest" | "quests" => "任務文字",
        "curios" | "trinket" | "trinkets" => "飾品名",
        _ => return None,
    })
}

// ═══ 主要入口：補語言表缺漏 ═══════════════════════════════════

/// `en_only`: ns -> (key -> 原文)。翻好的寫回 `zh`。
///
/// `use_ai == false` 時仍會跑術語表與翻譯記憶（兩者都不需要網路），
/// 只是不呼叫 AI。沒有金鑰的玩家因此還是拿得到官方譯名與先前翻過的內容。
#[allow(dead_code)]
pub fn fill_missing_with_ai<F>(
    zh: &mut LangMap,
    en_only: &LangMap,
    use_ai: bool,
    on_progress: F,
) -> Result<AiFillReport, String>
where
    F: FnMut(u8, &str),
{
    fill_missing_with_mode(
        zh,
        en_only,
        use_ai,
        false,
        TranslationQuality::Balanced,
        None,
        on_progress,
    )
}

pub fn fill_missing_with_ai_with_scope<F>(
    zh: &mut LangMap,
    en_only: &LangMap,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    on_progress: F,
) -> Result<AiFillReport, String>
where
    F: FnMut(u8, &str),
{
    // B4：這個入口的呼叫者（KubeJS 顯示字串）沒有「部分完成」的處理——
    // AI 沒跑完就照舊回錯，讓它不 commit 產出清單（B3 規則），而不是把半成品當成功。
    let report = fill_missing_with_mode(
        zh,
        en_only,
        use_ai,
        false,
        TranslationQuality::Balanced,
        scope,
        on_progress,
    )?;
    match &report.ai_unavailable {
        Some(reason) => Err(reason.clone()),
        None => Ok(report),
    }
}

/// 與一般補翻相同，但 Force 模式會略過本機翻譯記憶，避免重跑時一直沿用舊機翻。
pub fn fill_missing_with_mode<F>(
    zh: &mut LangMap,
    en_only: &LangMap,
    use_ai: bool,
    force_refresh: bool,
    quality: TranslationQuality,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<AiFillReport, String>
where
    F: FnMut(u8, &str),
{
    on_progress(42, "AI：組裝待譯清單…");

    let mut jobs: Vec<shared_tm::SharedTmJob> = Vec::new();
    for (ns, map) in en_only {
        for (k, en) in map {
            if zh.get(ns).and_then(|m| m.get(k)).is_some() {
                continue;
            }
            let t = en.trim();
            // 集中判定（P0-03）。這裡是 AI／共享庫／TM 的共同入口，
            // 而且手上同時有 key 與 value——舊版只把 value 丟給 looks_untranslatable，
            // 於是 `botania.entry.bcIntegration` 這種語言 key 一路送進 AI 並寫進共享庫。
            let verdict = eligibility::classify(eligibility::Candidate {
                source_kind: "lang",
                logical_key: k,
                text: t,
            });
            if !verdict.may_send_to_ai() {
                continue;
            }
            jobs.push(shared_tm::SharedTmJob {
                namespace: ns.clone(),
                key: k.clone(),
                source: en.clone(),
                context: context_hint(k).map(str::to_owned),
                scope: scope.cloned(),
            });
        }
    }

    jobs.sort_by(|a, b| (&a.namespace, &a.key).cmp(&(&b.namespace, &b.key)));

    if jobs.is_empty() {
        on_progress(88, "沒有需要 AI 補的文字，略過網路翻譯");
        return Ok(AiFillReport::default());
    }

    let mut report = AiFillReport::default();
    let mut guard = GuardStats::default();

    // ── 社群共享翻譯記憶（keyed：模組·key·原文）：先撈，命中就免送 AI ──
    // 隱藏、預設開；查不到／服務未就緒都略過，但寫可觀測 note。
    on_progress(43, "查詢社群共享翻譯（不需你設定）…");
    let skip_shared_lookup = super::super::shared_tm::skip_shared_lookup();
    let reuse_tm = !force_refresh;
    let shared_lookup = if skip_shared_lookup {
        shared_tm::LookupResult::default()
    } else {
        shared_tm::lookup_detailed(&jobs)
    };
    let shared = &shared_lookup.hits;
    report.notes.push(shared_lookup.player_note());
    let mut shared_done: std::collections::HashSet<usize> = std::collections::HashSet::new();
    if !shared.is_empty() {
        for (i, job) in jobs.iter().enumerate() {
            if let Some(cand) = shared.get(&i) {
                // 共享來的一樣要過佔位符守衛＋品質閘門才敢用
                if let Some(safe) = placeholder::guard(&job.source, cand, &mut guard) {
                    if is_usable_zh(&job.source, &safe) {
                        zh.entry(job.namespace.clone())
                            .or_default()
                            .insert(job.key.clone(), safe);
                        report.filled += 1;
                        report.shared_hits += 1;
                        shared_done.insert(i);
                    }
                }
            }
        }
        if report.shared_hits > 0 {
            on_progress(
                44,
                &format!("社群共享庫命中 {} 條（免送 AI）", report.shared_hits),
            );
        }
    } else if shared_lookup.status == shared_tm::LookupStatus::Empty {
        on_progress(44, "社群共享庫已連線，本次 0 命中");
    } else if shared_lookup.status == shared_tm::LookupStatus::Failed {
        on_progress(44, "社群共享庫查詢失敗，已略過（不影響本機翻譯）");
    }

    // 剩下未命中的才進去重＋術語表＋本機記憶＋AI
    let remaining: Vec<usize> = (0..jobs.len())
        .filter(|i| !shared_done.contains(i))
        .collect();
    let mut unique: Vec<String> = Vec::new();
    let mut ctx: Vec<Option<&'static str>> = Vec::new();
    let mut seen: HashMap<(String, Option<&'static str>), usize> = HashMap::new();
    let mut job_uid: HashMap<usize, usize> = HashMap::new(); // job index → uid
    for &i in &remaining {
        let job = &jobs[i];
        let hint = context_hint(&job.key);
        let dedupe_key = (job.source.clone(), hint);
        if let Some(&id) = seen.get(&dedupe_key) {
            job_uid.insert(i, id);
        } else {
            let id = unique.len();
            seen.insert(dedupe_key, id);
            unique.push(job.source.clone());
            ctx.push(hint);
            job_uid.insert(i, id);
        }
    }

    if !unique.is_empty() {
        let resolved = resolve_unique(
            &unique,
            &ctx,
            use_ai,
            reuse_tm,
            force_refresh,
            quality,
            44,
            44,
            scope,
            &mut on_progress,
        )?;
        let quality_failed_uids = resolved.quality_deferred.clone();
        let no_answer_uids = resolved.no_answer.clone();
        // 併入子報告的計數
        let sub = &resolved.report;
        report.glossary_hits += sub.glossary_hits;
        report.tm_hits += sub.tm_hits;
        report.shared_hits += sub.shared_hits;
        report.shared_glossary_hits += sub.shared_glossary_hits;
        report.ai_translated += sub.ai_translated;
        report.rejected += sub.rejected;
        report.quality_skipped += sub.quality_skipped;
        report.local_degenerate.extend(sub.local_degenerate.clone());
        // AI 不可用要一路帶到最外層，呼叫端才知道這一輪只有資料層的成果，
        // 不可以講成「完成」。
        if report.ai_unavailable.is_none() {
            report.ai_unavailable = sub.ai_unavailable.clone();
        }
        report.stopped_by_user |= sub.stopped_by_user;
        report.notes.extend(sub.notes.clone());

        // 寫回語言表 + 蒐集「這次新由 AI 產出的」以貢獻給社群
        let mut to_share: Vec<shared_tm::SharedTmEntry> = Vec::new();
        let mut glossary_share: Vec<shared_glossary::SharedGlossaryEntry> = Vec::new();
        let write_total = remaining.len();
        on_progress(
            86,
            &format!("正在寫回譯文到語言表（0/{write_total}）…"),
        );
        let mut write_done = 0usize;
        for &i in &remaining {
            let Some(&uid) = job_uid.get(&i) else {
                continue;
            };
            if let Some(text) = resolved.translations.get(&uid) {
                let job = &jobs[i];
                zh.entry(job.namespace.clone())
                    .or_default()
                    .insert(job.key.clone(), text.clone());
                report.filled += 1;
                if job.scope.is_some()
                    && is_usable_zh(&job.source, text)
                    && !is_poisoned_mech_translation(&job.source, text)
                    && super::super::output_guard::passes(&job.source, text)
                {
                    to_share.push(shared_tm::SharedTmEntry {
                        namespace: job.namespace.clone(),
                        key: job.key.clone(),
                        source: job.source.clone(),
                        translated: text.clone(),
                        context: job.context.clone(),
                        scope: job.scope.clone(),
                    });
                    if let Some(scope) = job.scope.clone() {
                        if is_shared_term_candidate(&job.source, job.context.as_deref()) {
                            glossary_share.push(shared_glossary::SharedGlossaryEntry {
                                source: job.source.clone(),
                                translated: text.clone(),
                                context: job.context.clone(),
                                scope,
                            });
                        }
                    }
                }
            }
            write_done += 1;
            if write_done == 1
                || write_done == write_total
                || write_done % 2000 == 0
            {
                on_progress(
                    86,
                    &format!("正在寫回譯文到語言表（{write_done}/{write_total}）…"),
                );
            }
            if quality_failed_uids.contains(&uid) {
                let job = &jobs[i];
                report
                    .quality_deferred
                    .entry(job.namespace.clone())
                    .or_default()
                    .insert(job.key.clone(), job.source.clone());
            } else if no_answer_uids.contains(&uid) && !resolved.translations.contains_key(&uid) {
                let job = &jobs[i];
                report
                    .no_answer
                    .entry(job.namespace.clone())
                    .or_default()
                    .insert(job.key.clone(), job.source.clone());
            }
        }
        // 匿名回饋給社群（失敗／逾時不影響本機；有牆鐘預算）
        if !to_share.is_empty() {
            on_progress(
                88,
                &format!(
                    "正在回饋社群共享庫（{} 條，逾時會暫存稍後再送）…",
                    to_share.len()
                ),
            );
            let result = shared_tm::contribute(&to_share);
            if let Some(note) = result.player_note() {
                report.notes.push(note);
            }
            on_progress(89, "社群共享庫回饋結束（本機翻譯不受影響）");
        }
        if !glossary_share.is_empty() {
            on_progress(
                89,
                &format!("正在回饋共享術語（{} 條）…", glossary_share.len()),
            );
            let g = shared_glossary::contribute(&glossary_share);
            if let Some(note) = g.player_note() {
                report.notes.push(note);
            }
        }
    }

    on_progress(90, &report.note());
    Ok(report)
}

pub(super) fn is_shared_term_candidate(source: &str, _context: Option<&str>) -> bool {
    let trimmed = source.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 120
        && !trimmed.contains(['\n', '\r'])
}

/// 收尾：把合格短詞貢獻到社群共享術語；回傳玩家可見說明（若有）。
pub fn contribute_shared_glossary_from_langmaps(
    en: &LangMap,
    zh: &LangMap,
    scope: &TranslationScope,
) -> Option<String> {
    let entries = collect_glossary_share_from_langmaps(en, zh, scope);
    shared_glossary::contribute(&entries).player_note()
}

/// 從最終 en∩zh 蒐集可貢獻的共享術語（短詞；ctx 可空，與 lookup hash 規則一致）。
pub fn collect_glossary_share_from_langmaps(
    en: &LangMap,
    zh: &LangMap,
    scope: &TranslationScope,
) -> Vec<shared_glossary::SharedGlossaryEntry> {
    if !scope.is_known() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (ns, en_map) in en {
        let Some(zh_map) = zh.get(ns) else {
            continue;
        };
        for (key, source) in en_map {
            let Some(translated) = zh_map.get(key) else {
                continue;
            };
            if !is_usable_zh(source, translated)
                || is_poisoned_mech_translation(source, translated)
                || !placeholder::is_compatible(source, translated)
                || !super::super::output_guard::passes(source, translated)
            {
                continue;
            }
            let ctx = context_hint(key);
            if !is_shared_term_candidate(source, ctx) {
                continue;
            }
            let gh = shared_glossary::glossary_hash(source, ctx);
            if !seen.insert(gh) {
                continue;
            }
            out.push(shared_glossary::SharedGlossaryEntry {
                source: source.clone(),
                translated: translated.clone(),
                context: ctx.map(str::to_owned),
                scope: scope.clone(),
            });
        }
    }
    out
}
