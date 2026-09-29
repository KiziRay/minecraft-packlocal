//! 任意字串翻譯入口（任務書、書本、覆寫文字）。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

/// 翻譯任意字串列表（任務書／書本／覆寫文字用），回傳與輸入等長（缺則空字串）。
#[allow(dead_code)]
pub fn translate_plain_strings<F>(
    texts: &[String],
    on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    translate_plain_strings_with_scope(texts, None, on_progress)
}

pub fn translate_plain_strings_with_scope<F>(
    texts: &[String],
    scope: Option<&TranslationScope>,
    on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    translate_plain_strings_ex(texts, scope, &[], on_progress)
}

/// 與 `translate_plain_strings_with_scope` 相同，但依原文對應模組／pack namespace。
pub fn translate_plain_strings_mapped<F>(
    texts: &[String],
    scope: Option<&TranslationScope>,
    ns_by_src: &HashMap<String, String>,
    on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    let namespaces = super::super::shared_identity::aligned_namespaces(texts, ns_by_src, scope);
    translate_plain_strings_ex(texts, scope, &namespaces, on_progress)
}

/// 與 `translate_plain_strings_with_scope` 相同，但可帶每條模組／pack namespace。
///
/// B4：AI 沒跑完（額度、斷線、停止）時照舊回 `Err`——這支的呼叫者沒有「部分完成」的處理，
/// 回錯才不會 commit 產出清單（B3 規則）。要保留已翻部分的呼叫者改用
/// [`translate_plain_strings_partial`]。
pub fn translate_plain_strings_ex<F>(
    texts: &[String],
    scope: Option<&TranslationScope>,
    namespaces: &[String],
    on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    let outcome = translate_plain_strings_partial(texts, scope, namespaces, on_progress)?;
    match outcome.incomplete {
        Some(reason) => Err(reason),
        None => Ok(outcome.out),
    }
}

/// B4：任意字串翻譯的結果，連同「AI 有沒有跑完」。
#[derive(Debug, Clone, Default)]
pub struct PlainOutcome {
    /// 與輸入等長；沒翻到的是空字串
    pub out: Vec<String>,
    /// AI 沒跑完的原因（`None`＝完整跑完）。有值時 `out` 是部分結果：產出者照樣寫出，但不可 commit。
    pub incomplete: Option<String>,
    /// 使用者按了停止
    pub stopped_by_user: bool,
}

/// B4：同 `translate_plain_strings_mapped`，但 AI 失敗、停止時**不丟已翻部分**。
pub fn translate_plain_strings_mapped_partial<F>(
    texts: &[String],
    scope: Option<&TranslationScope>,
    ns_by_src: &HashMap<String, String>,
    on_progress: F,
) -> Result<PlainOutcome, String>
where
    F: FnMut(u8, &str),
{
    let namespaces = super::super::shared_identity::aligned_namespaces(texts, ns_by_src, scope);
    translate_plain_strings_partial(texts, scope, &namespaces, on_progress)
}

/// B4：任意字串翻譯；AI 失敗、停止時回部分結果＋原因（不回 `Err`，除非一開始就被停止）。
pub fn translate_plain_strings_partial<F>(
    texts: &[String],
    scope: Option<&TranslationScope>,
    namespaces: &[String],
    mut on_progress: F,
) -> Result<PlainOutcome, String>
where
    F: FnMut(u8, &str),
{
    if texts.is_empty() {
        return Ok(PlainOutcome::default());
    }

    let reuse_shared = !super::super::shared_tm::skip_shared_lookup();
    let jobs: Vec<shared_tm::SharedTmJob> = texts
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let raw_ns = namespaces
                .get(i)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| super::super::shared_identity::pack_namespace(scope));
            shared_tm::SharedTmJob {
                namespace: super::super::shared_identity::sanitize_share_ns(&raw_ns, scope),
                key: "overlay".into(),
                source: source.clone(),
                context: None,
                scope: scope.cloned(),
            }
        })
        .collect();

    let mut out = vec![String::new(); texts.len()];
    let mut guard = GuardStats::default();
    let mut shared_done: HashSet<usize> = HashSet::new();
    if reuse_shared {
        on_progress(0, "查詢共享庫（覆寫／任務）…");
        let hits = shared_tm::lookup(&jobs);
        for (i, job) in jobs.iter().enumerate() {
            if let Some(cand) = hits.get(&i) {
                if let Some(safe) = placeholder::guard(&job.source, cand, &mut guard) {
                    if is_usable_zh(&job.source, &safe)
                        && !is_poisoned_mech_translation(&job.source, &safe)
                    {
                        out[i] = safe;
                        shared_done.insert(i);
                    }
                }
            }
        }
    }

    let remaining_idx: Vec<usize> = (0..texts.len())
        .filter(|i| !shared_done.contains(i))
        .collect();
    if remaining_idx.is_empty() {
        contribute_plain_job_outputs(&jobs, &out);
        return Ok(PlainOutcome {
            out,
            incomplete: None,
            stopped_by_user: false,
        });
    }

    let mut unique: Vec<String> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut idx_uid: Vec<(usize, usize)> = Vec::new();
    for &i in &remaining_idx {
        let t = &texts[i];
        if let Some(&id) = seen.get(t) {
            idx_uid.push((i, id));
        } else {
            let id = unique.len();
            seen.insert(t.clone(), id);
            unique.push(t.clone());
            idx_uid.push((i, id));
        }
    }
    let ctx = vec![None; unique.len()];
    let resolved = resolve_unique(
        &unique,
        &ctx,
        true,
        true,
        true,
        TranslationQuality::Balanced,
        0,
        99,
        scope,
        &mut on_progress,
    )?;
    for (i, uid) in idx_uid {
        if let Some(text) = resolved.translations.get(&uid) {
            out[i] = text.clone();
        }
    }
    contribute_plain_job_outputs(&jobs, &out);
    Ok(PlainOutcome {
        out,
        incomplete: resolved.report.ai_unavailable.clone(),
        stopped_by_user: resolved.report.stopped_by_user,
    })
}

pub(super) fn contribute_plain_job_outputs(jobs: &[shared_tm::SharedTmJob], out: &[String]) {
    let mut entries: Vec<shared_tm::SharedTmEntry> = Vec::new();
    for (job, zh) in jobs.iter().zip(out.iter()) {
        let t = zh.trim();
        if t.is_empty() || t == job.source.trim() {
            continue;
        }
        if !is_usable_zh(&job.source, t)
            || is_poisoned_mech_translation(&job.source, t)
            || !super::super::output_guard::passes(&job.source, t)
        {
            continue;
        }
        entries.push(shared_tm::SharedTmEntry {
            namespace: job.namespace.clone(),
            key: job.key.clone(),
            source: job.source.clone(),
            translated: t.to_string(),
            context: job.context.clone(),
            scope: job.scope.clone(),
        });
    }
    if !entries.is_empty() {
        let _ = shared_tm::contribute(&entries);
    }
}
