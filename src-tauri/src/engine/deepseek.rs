//! AI 補譯層。**只在本機整理完、確定缺中文時才會用到。**
//!
//! 送出前會依序擋掉不必送的東西，AI 只翻真正剩下的：
//! 1. 相同英文去重
//! 2. 術語表直接命中（官方台灣譯名，免費且一致）
//! 3. 翻譯記憶命中（上次或別的整合包翻過）
//!
//! 收回來後每一條都過佔位符把關，`%s` 被吃掉的譯文一律退回原文——
//! 少一句中文只是可惜，少一個 `%s` 是遊戲當場報錯。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::cancel;
use super::discord_auth::managed_ai_session_cookie;
use super::glossary::{self, Glossary};
use super::jar_scan::LangMap;
use super::placeholder::{self, GuardStats};
use super::secrets::resolve_ai_config;
use super::shared_tm;
use super::tm::Tm;
use super::turnstile::{managed_ai_turnstile_proof, MANAGED_AI_PROTOCOL};

/// 每批條數（去重後的「唯一英文」）
const BATCH: usize = 140;
const RETRY_BATCH: usize = 50;
/// 使用者自備金鑰時的並行批次數（自己的額度自己用，可衝滿）
const PARALLEL: usize = 16;
/// 代管模式的並行批次數。共用金鑰若每人 16 並行，多人同時就會撞 DeepSeek 限流（429），
/// 反而大家都失敗。降到 4 當好公民，換取共用額度下的穩定。
const PARALLEL_MANAGED: usize = 4;
/// 連續幾輪「整組都沒譯出」判定可能沒額度
const EMPTY_ROUNDS_ABORT: usize = 3;

/// 一次補譯的成果，供覆蓋範圍說明與 UI 顯示。
#[derive(Debug, Clone, Default)]
pub struct AiFillReport {
    /// 實際寫進語言表的條數
    pub filled: usize,
    pub glossary_hits: usize,
    pub tm_hits: usize,
    /// 社群共享翻譯記憶命中（免送 AI）
    pub shared_hits: usize,
    pub ai_translated: usize,
    /// 佔位符壞掉、已退回原文
    pub rejected: usize,
    pub notes: Vec<String>,
}

impl AiFillReport {
    pub fn note(&self) -> String {
        let mut parts = vec![format!(
            "補譯 {} 條（術語表 {}、共享庫 {}、翻譯記憶 {}、AI {}）",
            self.filled, self.glossary_hits, self.shared_hits, self.tm_hits, self.ai_translated
        )];
        if self.rejected > 0 {
            parts.push(format!("{} 條因佔位符不符退回原文", self.rejected));
        }
        parts.extend(self.notes.iter().cloned());
        parts.join("；")
    }
}

// ═══ 玩家向訊息 ═══════════════════════════════════════════════

/// 給玩家看的額度／無回應說明（不提服務商名稱）
fn ai_quota_support_message(detail: &str) -> String {
    let d = sanitize_provider_name(detail);
    format!(
        "【AI 額度可能已用完或金鑰無法使用】\n\
{d}\n\n\
連續多次沒有收到可用的翻譯回應。\n\
開發者目前沒有餘力再為 AI 加值了，需要你的支持。\n\
• 若你有自己的 API 金鑰：請在「填寫／修改 AI 金鑰」重新儲存後再試\n\
• 也歡迎主畫面「請我喝珍奶」支持開發（自願）\n\
https://zeitfrei.bobaboba.me"
    )
}

fn sanitize_provider_name(s: &str) -> String {
    let mut out = s.to_string();
    for needle in [
        "deepseek",
        "DeepSeek",
        "DEEPSEEK",
        "api.deepseek.com",
        "deepseek-chat",
    ] {
        out = out.replace(needle, "AI 服務");
    }
    out
}

fn looks_like_quota_or_auth_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("insufficient")
        || m.contains("balance")
        || m.contains("quota")
        || m.contains("billing")
        || m.contains("payment")
        || m.contains("exceed")
        || m.contains("credit")
        || m.contains("402")
        || m.contains("401")
        || m.contains("403")
        || m.contains("invalid api key")
        || m.contains("authentication")
        || m.contains("unauthorized")
        || m.contains("沒有額度")
        || m.contains("額度")
        || m.contains("餘額")
        || m.contains("金鑰無效")
        || m.contains("無回應")
        || m.contains("免費翻譯")
}

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

fn kind_of_segment(seg: &str) -> Option<&'static str> {
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
pub fn fill_missing_with_ai<F>(
    zh: &mut LangMap,
    en_only: &LangMap,
    use_ai: bool,
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
            if t.is_empty() || looks_untranslatable(t) {
                continue;
            }
            jobs.push(shared_tm::SharedTmJob {
                namespace: ns.clone(),
                key: k.clone(),
                source: en.clone(),
                context: context_hint(k).map(str::to_owned),
            });
        }
    }

    if jobs.is_empty() {
        on_progress(88, "沒有需要 AI 補的文字，略過網路翻譯");
        return Ok(AiFillReport::default());
    }

    let mut report = AiFillReport::default();
    let mut guard = GuardStats::default();

    // ── 社群共享翻譯記憶（keyed：模組·key·原文）：先撈，命中就免送 AI ──
    // 隱藏、預設開；查不到／服務未就緒都靜默略過。以 (ns,key,srcHash) 為單位，跨整合包安全。
    on_progress(43, "查詢社群共享翻譯（不需你設定）…");
    let shared = shared_tm::lookup(&jobs);
    let mut shared_done: std::collections::HashSet<usize> = std::collections::HashSet::new();
    if !shared.is_empty() {
        for (i, job) in jobs.iter().enumerate() {
            if let Some(cand) = shared.get(&i) {
                // 共享來的一樣要過佔位符守衛才敢用
                if let Some(safe) = placeholder::guard(&job.source, cand, &mut guard) {
                    zh.entry(job.namespace.clone()).or_default().insert(job.key.clone(), safe);
                    report.filled += 1;
                    report.shared_hits += 1;
                    shared_done.insert(i);
                }
            }
        }
        if report.shared_hits > 0 {
            on_progress(
                44,
                &format!("社群共享庫命中 {} 條（免送 AI）", report.shared_hits),
            );
        }
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
        let resolved = resolve_unique(&unique, &ctx, use_ai, 44, 44, &mut on_progress)?;
        // 併入子報告的計數
        let sub = &resolved.report;
        report.glossary_hits += sub.glossary_hits;
        report.tm_hits += sub.tm_hits;
        report.ai_translated += sub.ai_translated;
        report.rejected += sub.rejected;
        report.notes.extend(sub.notes.clone());

        // 寫回語言表 + 蒐集「這次新由 AI 產出的」以貢獻給社群
        let mut to_share: Vec<(String, String, String, String, Option<String>)> = Vec::new();
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
                if resolved.ai_uids.contains(&uid) {
                    to_share.push((
                        job.namespace.clone(),
                        job.key.clone(),
                        job.source.clone(),
                        text.clone(),
                        job.context.clone(),
                    ));
                }
            }
        }
        // 匿名回饋給社群（fire-and-forget；失敗不影響）
        if !to_share.is_empty() {
            shared_tm::contribute(&to_share);
        }
    }

    on_progress(90, &report.note());
    Ok(report)
}

/// 翻譯任意字串列表（任務書／書本／覆寫文字用），回傳與輸入等長（缺則空字串）。
pub fn translate_plain_strings<F>(
    texts: &[String],
    mut on_progress: F,
) -> Result<Vec<String>, String>
where
    F: FnMut(u8, &str),
{
    if texts.is_empty() {
        return Ok(vec![]);
    }

    let mut unique: Vec<String> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut idx_uid: Vec<usize> = Vec::with_capacity(texts.len());
    for t in texts {
        if let Some(&id) = seen.get(t) {
            idx_uid.push(id);
        } else {
            let id = unique.len();
            seen.insert(t.clone(), id);
            unique.push(t.clone());
            idx_uid.push(id);
        }
    }
    let ctx = vec![None; unique.len()];

    let resolved = resolve_unique(&unique, &ctx, true, 0, 99, &mut on_progress)?;

    Ok(idx_uid
        .into_iter()
        .map(|uid| resolved.translations.get(&uid).cloned().unwrap_or_default())
        .collect())
}

// ═══ 共用流程 ═════════════════════════════════════════════════

struct Resolved {
    translations: HashMap<usize, String>,
    report: AiFillReport,
    /// 這些 uid 的譯文是「本次新由 AI 產出」（用來只回饋新內容給社群，不重傳術語表/記憶）。
    ai_uids: std::collections::HashSet<usize>,
}

/// 術語表 → 翻譯記憶 → AI，三層依序解決 `unique` 裡的每一條。
///
/// 前兩層完全離線，所以 `use_ai == false` 時仍值得跑。
fn resolve_unique(
    unique: &[String],
    ctx: &[Option<&'static str>],
    use_ai: bool,
    base_pct: u8,
    span_pct: u8,
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
    let mut tm = Tm::load();
    let mut translations: HashMap<usize, String> = HashMap::new();
    let mut report = AiFillReport::default();
    let mut guard = GuardStats::default();
    let mut ai_uids: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // ── 第 1、2 層：查表，完全不用網路 ──
    let mut need_ai: Vec<(usize, String)> = Vec::new();
    for (uid, src) in unique.iter().enumerate() {
        if let Some(zh) = gloss.exact(src) {
            translations.insert(uid, zh.to_string());
            report.glossary_hits += 1;
            continue;
        }
        if let Some(zh) = tm.get(src) {
            translations.insert(uid, zh);
            report.tm_hits += 1;
            continue;
        }
        need_ai.push((uid, src.clone()));
    }

    let pre = report.glossary_hits + report.tm_hits;
    if pre > 0 {
        on_progress(
            base_pct,
            &format!(
                "免費命中 {} 句（術語表 {}、翻譯記憶 {}），只剩 {} 句要送 AI",
                pre,
                report.glossary_hits,
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
            ai_uids,
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
            ai_uids,
        });
    }

    // ── 第 3 層：AI ──
    let engine = Engine::connect()?;
    on_progress(
        base_pct,
        &format!("AI 翻譯 {} 句（已扣掉重複與已知譯名）…", need_ai.len()),
    );

    // 遮罩：送出前把 %s／{k}／§c／$(…) 換成 {0}{1}，收回再還原。
    // 模型看不到脆弱 token，弄壞的機率大降；還原後仍過 guard 當最後防線。
    let mut tokens_by_uid: HashMap<usize, Vec<String>> = HashMap::new();
    let masked_need_ai: Vec<(usize, String)> = need_ai
        .iter()
        .map(|(uid, src)| {
            let (masked, tokens) = placeholder::mask(src);
            tokens_by_uid.insert(*uid, tokens);
            (*uid, masked)
        })
        .collect();

    let raw = run_batches(
        &engine,
        &masked_need_ai,
        &gloss,
        ctx,
        base_pct,
        span_pct,
        on_progress,
    )?;

    // ── 還原遮罩 + 佔位符把關 + 寫入翻譯記憶 ──
    for (uid, src) in &need_ai {
        let Some(masked_out) = raw.get(uid) else {
            continue;
        };
        let empty = Vec::new();
        let tokens = tokens_by_uid.get(uid).unwrap_or(&empty);
        let candidate = placeholder::unmask(masked_out, tokens);
        match placeholder::guard(src, &candidate, &mut guard) {
            Some(safe) => {
                tm.insert(src, &safe);
                translations.insert(*uid, safe);
                report.ai_translated += 1;
                ai_uids.insert(*uid);
            }
            None => {
                // 退回原文：語言表維持英文，遊戲不會壞
                report.rejected += 1;
            }
        }
    }

    if let Err(e) = tm.save() {
        report.notes.push(format!("翻譯記憶未能存檔：{e}"));
    }
    report.notes.push(tm.note());
    if guard.repaired > 0 || guard.rejected > 0 {
        report.notes.push(guard.note());
    }

    Ok(Resolved {
        translations,
        report,
        ai_uids,
    })
}

// ═══ 連線 ═════════════════════════════════════════════════════

struct Engine {
    client: Arc<reqwest::blocking::Client>,
    url: Arc<String>,
    /// 代管模式為空——不送 Authorization，金鑰由 Worker 注入。
    api_key: Arc<String>,
    model: Arc<String>,
    /// 只在代管模式使用；送往自家 Worker，由 Worker 驗證登入與伺服器會員身分。
    managed_session: Arc<String>,
    /// Cloudflare Siteverify 通過後由 Worker 簽發；只存在記憶體，且綁定 Discord 使用者。
    managed_turnstile: Arc<String>,
    managed: bool,
}

impl Engine {
    fn connect() -> Result<Self, String> {
        // AI 來源由使用者明確選擇；自訂模式缺金鑰時直接回報，代管模式再驗 Discord。
        let cfg = resolve_ai_config()?;
        let (managed_session, managed_turnstile) = if cfg.managed {
            (
                managed_ai_session_cookie()?,
                // Turnstile 為選用：拿不到通行憑證也照樣走（Worker 端未設定金鑰就不強制），
                // 才不會在還沒設定 Turnstile 時把代管翻譯整個擋掉。
                managed_ai_turnstile_proof().unwrap_or_default(),
            )
        } else {
            (String::new(), String::new())
        };

        if let Err(probe) = probe_ai_ready(
            &cfg.base_url,
            &cfg.api_key,
            &cfg.model,
            cfg.managed,
            &managed_session,
            &managed_turnstile,
        ) {
            return Err(if cfg.managed {
                probe
            } else {
                ai_quota_support_message(&probe)
            });
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(180))
            .pool_max_idle_per_host(PARALLEL + 2)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Engine {
            client: Arc::new(client),
            url: Arc::new(format!(
                "{}/v1/chat/completions",
                cfg.base_url.trim_end_matches('/')
            )),
            api_key: Arc::new(cfg.api_key),
            model: Arc::new(cfg.model),
            managed_session: Arc::new(managed_session),
            managed_turnstile: Arc::new(managed_turnstile),
            managed: cfg.managed,
        })
    }
}

/// 代管 AI（免金鑰）是否可用——一定可用（URL 內建）。留給 UI 判斷是否預設開 AI。
pub fn managed_ai_available() -> bool {
    true
}

// ═══ 批次執行 ═════════════════════════════════════════════════

/// 分組並行送出，回傳 uid → 譯文（未經佔位符把關的原始結果）。
fn run_batches(
    engine: &Engine,
    items: &[(usize, String)],
    gloss: &Glossary,
    ctx: &[Option<&'static str>],
    base_pct: u8,
    span_pct: u8,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<HashMap<usize, String>, String> {
    let chunks: Vec<Vec<(usize, String)>> = items.chunks(BATCH).map(|c| c.to_vec()).collect();
    let total_batches = chunks.len().max(1);
    let total_unique = items.len();

    let translations: Arc<Mutex<HashMap<usize, String>>> = Arc::new(Mutex::new(HashMap::new()));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut empty_rounds = 0usize;
    let mut finished_batches = 0usize;
    let phase_start = Instant::now();

    let pct = |done: usize, got: usize| -> u8 {
        by_batch_or_strings(done, total_batches, got, total_unique, base_pct, span_pct)
    };

    // 代管（共用金鑰）降並行，避免多人同時把 DeepSeek 打到限流。
    let parallel = if engine.managed {
        PARALLEL_MANAGED
    } else {
        PARALLEL
    };

    let mut group_start = 0usize;
    while group_start < chunks.len() {
        cancel::check()?;
        let group_end = (group_start + parallel).min(chunks.len());
        let group = &chunks[group_start..group_end];
        let group_n = group.len();
        let got_before = translations.lock().map(|t| t.len()).unwrap_or(0);

        let (tx, rx) = mpsc::channel::<Result<(usize, usize), String>>();
        let mut handles = Vec::new();
        for (gi, chunk) in group.iter().enumerate() {
            let batch_no = group_start + gi + 1;
            let client = Arc::clone(&engine.client);
            let url = Arc::clone(&engine.url);
            let api_key = Arc::clone(&engine.api_key);
            let model = Arc::clone(&engine.model);
            let managed_session = Arc::clone(&engine.managed_session);
            let managed_turnstile = Arc::clone(&engine.managed_turnstile);
            let managed = engine.managed;
            let translations = Arc::clone(&translations);
            let items: Vec<(usize, String)> = chunk.clone();
            let hints = gloss.hints_for(&chunk.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>());
            let contexts: Vec<Option<&'static str>> = chunk
                .iter()
                .map(|(uid, _)| ctx.get(*uid).copied().flatten())
                .collect();
            let tx = tx.clone();

            handles.push(thread::spawn(move || {
                let run_one = |slice: &[(usize, String)],
                               slice_ctx: &[Option<&'static str>]|
                 -> Result<usize, String> {
                    if cancel::is_cancelled() {
                        return Ok(0);
                    }
                    let map = translate_chunk(
                        &client,
                        &url,
                        &api_key,
                        &model,
                        managed,
                        &managed_session,
                        &managed_turnstile,
                        slice,
                        slice_ctx,
                        &hints,
                    )?;
                    let mut n = 0usize;
                    if let Ok(mut tr) = translations.lock() {
                        for (local_i, t) in map {
                            if let Some((uid, _)) = slice.get(local_i) {
                                if !t.trim().is_empty() {
                                    tr.insert(*uid, t);
                                    n += 1;
                                }
                            }
                        }
                    }
                    Ok(n)
                };

                let result = match run_one(&items, &contexts) {
                    Ok(n) if n > 0 || items.is_empty() => Ok((batch_no, n)),
                    Ok(_) if cancel::is_cancelled() => Ok((batch_no, 0)),
                    Ok(_) => Err(format!("第 {batch_no} 批無回應（空結果）")),
                    Err(e) if items.len() > RETRY_BATCH => {
                        // 整批失敗常是單一條目太長／太怪，切小再試
                        let mut total_n = 0usize;
                        let mut last = e;
                        let mut off = 0usize;
                        while off < items.len() {
                            let end = (off + RETRY_BATCH).min(items.len());
                            match run_one(&items[off..end], &contexts[off..end]) {
                                Ok(n) => total_n += n,
                                Err(se) => last = se,
                            }
                            off = end;
                        }
                        if total_n > 0 {
                            Ok((batch_no, total_n))
                        } else {
                            Err(format!(
                                "第 {batch_no} 批失敗：{}",
                                sanitize_provider_name(&last)
                            ))
                        }
                    }
                    Err(e) => Err(format!(
                        "第 {batch_no} 批失敗：{}",
                        sanitize_provider_name(&e)
                    )),
                };
                let _ = tx.send(result);
            }));
        }
        drop(tx);

        let mut done_in_group = 0usize;
        let group_t0 = Instant::now();
        while done_in_group < group_n {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(outcome) => {
                    done_in_group += 1;
                    finished_batches += 1;
                    let failed = outcome.is_err();
                    if let Err(e) = outcome {
                        if let Ok(mut er) = errors.lock() {
                            er.push(e);
                        }
                    }
                    let got = translations.lock().map(|t| t.len()).unwrap_or(0);
                    on_progress(
                        pct(finished_batches, got),
                        &format!(
                            "AI 翻譯中… {}／{} 批{} · 已得 {} 句 · 已進行 {} 秒",
                            finished_batches.min(total_batches),
                            total_batches,
                            if failed { "（部分失敗）" } else { "" },
                            got,
                            phase_start.elapsed().as_secs()
                        ),
                    );
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let got = translations.lock().map(|t| t.len()).unwrap_or(0);
                    on_progress(
                        pct(finished_batches, got),
                        &format!(
                            "AI 翻譯中…等待本輪回應（本輪 {} 秒／合計 {} 秒）· 已完成 {}／{} 批 · {} 句",
                            group_t0.elapsed().as_secs(),
                            phase_start.elapsed().as_secs(),
                            finished_batches.min(total_batches),
                            total_batches,
                            got
                        ),
                    );
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        for h in handles {
            let _ = h.join();
        }
        cancel::check()?;

        let got = translations.lock().map(|t| t.len()).unwrap_or(0);
        if got.saturating_sub(got_before) == 0 {
            empty_rounds += 1;
            on_progress(
                pct(finished_batches, got),
                &format!("AI 這一輪沒有新譯文（連續 {} 次）…", empty_rounds),
            );
            let err_peek = errors.lock().map(|e| e.clone()).unwrap_or_default();
            let quota_hit = err_peek.iter().any(|e| looks_like_quota_or_auth_error(e));
            if empty_rounds >= EMPTY_ROUNDS_ABORT || quota_hit {
                let detail = err_peek
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "多次請求沒有有效回應".into());
                return Err(ai_quota_support_message(&detail));
            }
        } else {
            empty_rounds = 0;
        }

        group_start = group_end;
    }

    let out = translations.lock().map_err(|e| e.to_string())?.clone();
    let err_list = errors.lock().map_err(|e| e.to_string())?.clone();

    if out.is_empty() {
        let detail = err_list
            .first()
            .cloned()
            .unwrap_or_else(|| "全部請求都沒有回應".into());
        return Err(ai_quota_support_message(&detail));
    }
    if !err_list.is_empty() {
        on_progress(
            pct(total_batches, out.len()),
            &format!(
                "AI 有 {} 批失敗（其餘已完成）；持續失敗通常代表額度用完",
                err_list.len()
            ),
        );
    }
    Ok(out)
}

/// 進度取「批次進度」與「句數進度」較高者，避免久卡同一個數字像當機。
fn by_batch_or_strings(
    done_batches: usize,
    total_batches: usize,
    got: usize,
    total_unique: usize,
    base: u8,
    span: u8,
) -> u8 {
    let ratio = |a: usize, b: usize| -> u32 {
        if b == 0 {
            0
        } else {
            (a as u32 * span as u32 / b as u32).min(span as u32)
        }
    };
    let p = base as u32 + ratio(done_batches, total_batches).max(ratio(got, total_unique));
    p.min((base as u32 + span as u32).min(100)) as u8
}

// ═══ 單批請求 ═════════════════════════════════════════════════

fn build_user_prompt(
    chunk: &[(usize, String)],
    contexts: &[Option<&'static str>],
    hints: &[(String, String)],
) -> String {
    let items: Vec<Value> = chunk
        .iter()
        .enumerate()
        .map(|(i, (_, text))| match contexts.get(i).copied().flatten() {
            Some(c) => json!({ "i": i, "t": text, "c": c }),
            None => json!({ "i": i, "t": text }),
        })
        .collect();

    let mut p = String::with_capacity(512);
    p.push_str("把下列 Minecraft 模組文字翻成台灣繁體中文（台灣用語）。\n");
    p.push_str(
        "只輸出一個 JSON 陣列 [{\"i\":編號,\"t\":\"譯文\"}]，i 與輸入相同，不要任何說明文字。\n",
    );
    p.push_str("規則：\n");
    p.push_str("1. 文中的 {0} {1} {2}… 是佔位符，必須原封不動保留（數量一致，可依語序移動位置），不要翻譯、不要改成全形、不要新增或刪除。\n");
    p.push_str("2. 保留原文開頭與結尾的空白。\n");
    p.push_str("3. c 是語境。物品名／方塊名／生物名要像「名稱」，簡短不成句。\n");
    p.push_str("4. 已經是中文的照原樣輸出；純代號、路徑、id 照原樣輸出。\n");
    if !hints.is_empty() {
        p.push_str("5. 這些詞用固定譯名：");
        let joined: Vec<String> = hints.iter().map(|(en, zh)| format!("{en}={zh}")).collect();
        p.push_str(&joined.join("、"));
        p.push('\n');
    }
    p.push_str("\n輸入：\n");
    p.push_str(&serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()));
    p
}

#[allow(clippy::too_many_arguments)]
fn translate_chunk(
    client: &reqwest::blocking::Client,
    url: &str,
    api_key: &str,
    model: &str,
    managed: bool,
    managed_session: &str,
    managed_turnstile: &str,
    chunk: &[(usize, String)],
    contexts: &[Option<&'static str>],
    hints: &[(String, String)],
) -> Result<HashMap<usize, String>, String> {
    let body = json!({
        "model": model,
        "temperature": 0.1,
        "messages": [
            {"role": "system", "content": "你是 Minecraft 模組的繁體中文（台灣）在地化譯者。只輸出合法 JSON 陣列，無其他文字。"},
            {"role": "user", "content": build_user_prompt(chunk, contexts, hints)}
        ]
    });

    let mut last_err = String::new();
    for attempt in 0..3 {
        if cancel::is_cancelled() {
            return Ok(HashMap::new());
        }
        // 代管模式不送上游 Authorization；只送 ZeitFrei session 給自家 Worker 驗證。
        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body);
        if managed {
            req = req
                .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
                .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
                .header("X-Zeitfrei-Session", managed_session)
                .header("X-Zeitfrei-Turnstile", managed_turnstile);
        } else if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }
        let resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("連線失敗（無回應）：{e}");
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(400 + attempt as u64 * 400));
                    continue;
                }
                return Err(last_err);
            }
        };

        let status = resp.status();
        let code = status.as_u16();
        if code == 503 {
            // 代管 Worker 尚未設定 DeepSeek secret（開發者端）。對玩家而言是「免費翻譯暫停」。
            let t = resp.text().unwrap_or_default();
            if managed || t.contains("server_not_ready") {
                return Err(
                    "免費翻譯暫時無法使用（服務端維護中）。你可以自行填入 AI 金鑰，或稍後再試"
                        .into(),
                );
            }
            last_err = "服務暫時無法使用（503），稍後再試".into();
            thread::sleep(Duration::from_millis(600 + attempt as u64 * 600));
            continue;
        }
        if code == 429 {
            // 代管模式的 429＝當日免費額度用完，直接導向贊助提示。
            if managed {
                return Err("免費翻譯的當日額度已用完".into());
            }
            last_err = "請求太頻繁，稍後再試".into();
            thread::sleep(Duration::from_millis(800 + attempt as u64 * 1200));
            continue;
        }
        if code == 426 && managed {
            return Err("這個版本已不能使用開發者代管 AI，請更新工具後再試。".into());
        }
        if code == 428 && managed {
            return Err("Cloudflare 安全驗證已過期，請回到工具重新驗證。".into());
        }
        if code == 401 && managed {
            return Err("使用開發者代管 AI 前，請先登入 Discord。".into());
        }
        if code == 403 && managed {
            return Err("使用開發者代管 AI 前，請先加入 ZeitFrei 官方 Discord 伺服器。".into());
        }
        if code == 401 || code == 403 {
            let t = resp.text().unwrap_or_default();
            return Err(format!(
                "金鑰無效或無權限：{}",
                sanitize_provider_name(&t.chars().take(120).collect::<String>())
            ));
        }
        if code == 402 {
            let t = resp.text().unwrap_or_default();
            return Err(format!(
                "帳號餘額不足：{}",
                sanitize_provider_name(&t.chars().take(120).collect::<String>())
            ));
        }
        if !status.is_success() {
            let t = resp.text().unwrap_or_default();
            let snippet = sanitize_provider_name(&t.chars().take(200).collect::<String>());
            if looks_like_quota_or_auth_error(&snippet) || looks_like_quota_or_auth_error(&t) {
                return Err(format!("可能額度不足或金鑰問題（{code}）：{snippet}"));
            }
            return Err(format!("服務錯誤 {code}：{snippet}"));
        }

        let v: Value = match resp.json() {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("回應無法解析（無有效內容）：{e}");
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(200));
                    continue;
                }
                return Err(last_err);
            }
        };
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim();
        if content.is_empty() {
            last_err = "服務有連線但沒有回傳翻譯內容".into();
            if attempt < 2 {
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            return Err(last_err);
        }

        match parse_translation_array(content) {
            Ok(map) if !map.is_empty() => return Ok(map),
            Ok(_) => {
                last_err = "回傳為空陣列（無譯文）".into();
                if attempt < 2 {
                    continue;
                }
                return Err(last_err);
            }
            Err(e) => {
                last_err = e;
                if attempt < 2 {
                    thread::sleep(Duration::from_millis(150));
                    continue;
                }
                return Err(last_err);
            }
        }
    }
    Err(if last_err.is_empty() {
        "多次請求沒有有效回應".into()
    } else {
        last_err
    })
}

/// 解析模型回覆：容忍 ```json 圍欄與前後贅字。
fn parse_translation_array(content: &str) -> Result<HashMap<usize, String>, String> {
    let body = extract_json_array(strip_code_fence(content));
    let arr: Vec<Value> = serde_json::from_str(body).map_err(|e| {
        format!(
            "回傳格式不對：{e} / {}",
            body.chars().take(80).collect::<String>()
        )
    })?;
    let mut by_i: HashMap<usize, String> = HashMap::new();
    for item in arr {
        let Some(i) = item["i"].as_u64() else {
            continue;
        };
        if let Some(t) = item["t"].as_str() {
            by_i.insert(i as usize, t.to_string());
        }
    }
    Ok(by_i)
}

fn extract_json_array(s: &str) -> &str {
    let s = s.trim();
    if let (Some(a), Some(b)) = (s.find('['), s.rfind(']')) {
        if a < b {
            return &s[a..=b];
        }
    }
    s
}

fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim();
    }
    if let Some(rest) = s.strip_prefix("```") {
        return rest.trim_end_matches("```").trim();
    }
    s
}

/// 輕量探測：餘額 API 或迷你 chat；網路抖動不阻擋，明確額度／金鑰問題直接回錯。
fn probe_ai_ready(
    base: &str,
    api_key: &str,
    model: &str,
    managed: bool,
    managed_session: &str,
    managed_turnstile: &str,
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|e| format!("無法建立連線：{e}"))?;
    let base = base.trim_end_matches('/');

    // 代管 Worker 沒有 /user/balance 端點，跳過餘額查詢，直接用迷你 chat 探測。
    if !managed {
        let bal_url = format!("{base}/user/balance");
        if let Ok(resp) = client
            .get(&bal_url)
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
        {
            let code = resp.status().as_u16();
            if code == 401 || code == 403 {
                return Err("金鑰無效或無權限".into());
            }
            if code == 402 {
                return Err("帳號餘額不足".into());
            }
            if resp.status().is_success() {
                if let Ok(v) = resp.json::<Value>() {
                    if balance_looks_empty(&v) {
                        return Err("帳號餘額為零或不足".into());
                    }
                }
            }
        }
    }

    let url = format!("{base}/v1/chat/completions");
    let probe = vec![(0usize, "OK".to_string())];
    match translate_chunk(
        &client,
        &url,
        api_key,
        model,
        managed,
        managed_session,
        managed_turnstile,
        &probe,
        &[None],
        &[],
    ) {
        Ok(m) if !m.is_empty() => Ok(()),
        Ok(_) if cancel::is_cancelled() => Ok(()),
        Ok(_) => Err("探測成功連線但沒有內容回應".into()),
        Err(e) => {
            if looks_like_quota_or_auth_error(&e) {
                Err(e)
            } else {
                // 網路暫時失敗：不在此硬擋，讓主流程再試
                Ok(())
            }
        }
    }
}

fn balance_looks_empty(v: &Value) -> bool {
    if let Some(false) = v.get("is_available").and_then(|x| x.as_bool()) {
        return true;
    }
    if let Some(arr) = v.get("balance_infos").and_then(|x| x.as_array()) {
        let mut any_positive = false;
        for item in arr {
            let total = item
                .get("total_balance")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| item.get("total_balance").and_then(|x| x.as_f64()))
                .unwrap_or(0.0);
            if total > 0.0001 {
                any_positive = true;
            }
        }
        return !any_positive && !arr.is_empty();
    }
    false
}

fn looks_untranslatable(t: &str) -> bool {
    if t.len() <= 1 {
        return true;
    }
    if t.chars().filter(|c| c.is_alphabetic()).count() == 0 {
        return true;
    }
    if t.starts_with("http://") || t.starts_with("https://") {
        return true;
    }
    if t.contains("://") && !t.contains(' ') {
        return true;
    }
    if is_resource_location(t) {
        return true;
    }
    false
}

/// `minecraft:stone_sword`、`create:andesite_alloy` 這種資源 id。
///
/// 翻成「我的世界:石劍」會直接讓配方／JEI 對不上，所以連送都不要送給 AI。
/// 判準刻意嚴格：全小寫、無空白——`Warning: fire` 或 `HP:100` 不會被誤殺。
fn is_resource_location(t: &str) -> bool {
    if !t.contains(':') || t.contains(char::is_whitespace) {
        return false;
    }
    t.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, ':' | '_' | '/' | '.' | '-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn untranslatable_filter_skips_ids_and_urls() {
        assert!(looks_untranslatable("https://example.com"));
        assert!(looks_untranslatable("minecraft:stone_sword"));
        assert!(looks_untranslatable("create:andesite_alloy"));
        assert!(looks_untranslatable("123"));
        assert!(!looks_untranslatable("Diamond Sword"));
    }

    #[test]
    fn colon_text_meant_for_players_is_still_translated() {
        // 這些看起來有冒號，但是真的要翻的句子，不能被 id 判斷誤殺
        assert!(!looks_untranslatable("Warning: fire"));
        assert!(!looks_untranslatable("HP:100"));
        assert!(!looks_untranslatable("Tier: Advanced"));
    }

    #[test]
    fn parses_plain_json_array() {
        let m = parse_translation_array(r#"[{"i":0,"t":"鑽石劍"}]"#).unwrap();
        assert_eq!(m.get(&0).map(|s| s.as_str()), Some("鑽石劍"));
    }

    #[test]
    fn parses_array_wrapped_in_code_fence_and_prose() {
        let raw = "當然可以：\n```json\n[{\"i\":0,\"t\":\"鑽石劍\"},{\"i\":1,\"t\":\"金錠\"}]\n```";
        let m = parse_translation_array(raw).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m.get(&1).map(|s| s.as_str()), Some("金錠"));
    }

    #[test]
    fn malformed_response_is_an_error_not_a_panic() {
        assert!(parse_translation_array("完全不是 JSON").is_err());
    }

    #[test]
    fn prompt_carries_context_and_glossary_hints() {
        let chunk = vec![(0usize, "Creeper".to_string())];
        let hints = vec![("Creeper".to_string(), "苦力怕".to_string())];
        let p = build_user_prompt(&chunk, &[Some("生物名")], &hints);
        assert!(p.contains("Creeper=苦力怕"));
        assert!(p.contains("生物名"));
        assert!(p.contains("{0}"), "必須明確要求保留遮罩後的佔位符");
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
        assert!(!msg.to_lowercase().contains("deepseek"));
        assert!(msg.contains("AI 服務"));
    }
}
