//! 模組包一鍵繁中 — 後端
//! 進度以事件即時推送，重工作在背景執行緒，避免視窗假死。

mod engine;

use engine::{
    apply_font_pack_to_instance, apply_to_instance, build_font_pack_str_with_options,
    read_font_preview_base64,
    build_resource_pack, cancel_discord_login, build_pack_name, resolve_output_pack_name,
    detect_pack_version,
    cancel_gpt_login,
    cancel_turnstile_verification, check_cancelled, is_cancelled, check_discord_auth_status,
    classify_diagnosis, clear_turnstile_proof,
    convert_langmap_s2tw_selective, convert_langmap_s2tw_with_progress,
    converter_name, count_map,
    apply_phrase_dict, strip_of_suffix_zhi,
    classify_diagnosis_input, detect_minecraft_version, detect_pack_format, diagnose_launch,
    diagnose_pack_dir, discover_default_reference,
    try_download_cfpa_pack,
    cleanup_transient_work, ensure_result_layout, ensure_ready_to_write, ensure_space, ensure_user_glossary_template,
    prune_empty_result_dirs,
    ensure_minecraft_version_for_translate,
    extract_jar_documentation,
    rewrite_translated_jars, translate_jar_display_texts, translate_jar_patchouli,
    fill_missing_with_mode, seed_tm_from_langmaps, verify_ai_assistance, verify_custom_api, AiFillReport,
    contribute_shared_glossary_from_langmaps,
    find_pack_near, find_session_file, get_ai_mode,
    is_tool_resource_pack,
    get_api_settings_public, get_gpt_model, get_minimize_on_close, has_session_file, load_pack_zh,
    discover_prior_zh_sources,
    load_phrase_dict, load_reference_zh_tw, load_session, login_discord_blocking, gpt_login_blocking,
    gpt_auth_status, gpt_logout, logout_discord,
    is_probably_network_path, merge_fill_missing, normalize_user_path,
    package_translation, has_shareable_content,
    upload_share_package,
    pack_format_for_version,
    probe_apply_targets,
    remaining_pending, request_cancel, reset_cancel, resolve_canonical_tool_zip,
    resolve_minecraft_dir, restore_last_apply_in,
    filter_quality_deferred, merge_pending, prune_quality_deferred, rework_unusable_zh,
    delete_apply_backups_in, has_apply_backups_in,
    save_api_settings, save_api_settings_with_provider, save_session,
    scan_instance, set_ai_mode, set_gpt_model,
    run_search_pipeline, write_search_artifacts,
    set_minimize_on_close, subtract_covered, translate_ftbquests,
    translate_archive_overlays, translate_jar_origins, translate_kubejs_literals, translate_minemenu, translate_origins,
    translate_quests_books, translate_text_overlays,
    mode_note, skip_complete_namespaces_with_provenance, TranslationMode, TranslationQuality,
    user_glossary_path, validate_instance_path, validate_open_url, verify_turnstile_blocking, consistency_suggestions_path, consistency_suggestions_status, merge_consistency_suggestions, write_consistency_hints, write_coverage_report,
    write_gap_summary_file,
    map_stage_progress, CoverageSourceFlags,
    ApiSettingsPublic, ApplyResult, ApplyStatus, BuildOptions, PackVersionInfo,
    CoverageStats, DiscordAuthStatus, FontPackApplyResult, FontPackOptions, FontPackResult,
    InstanceValidation, JarDocumentationReport, JarTranslationReport,
    LangMap, LaunchDiagnosis, ShareUploadResult, LangSource, ProvenanceMap,
    GptAuthStatus,
    DeleteBackupResult, RestoreResult, ScanReport, TranslateSession, UpdateCheck, CANCEL_MESSAGE, DISCORD_INVITE_URL,
    MIN_FREE_BYTES, RESULT_DIR_NAME, SESSION_FILE,
    TranslationScope,
    TranslationHelperStatus,
    cleanup_translation_helper, inspect_translation_helper, prepare_translation_helper,
    contribute_lang_maps, contribute_lang_maps_limited, ContributeLangMapsOpts,
    filter_local_untranslatable, flush_shared_contribute_queue,
    reset_contribute_tracker, SkipSharedLookupGuard,
    submit_diagnose_report, DiagnoseReportRequest, DiagnoseReportResult,
    submit_issue_report, SubmitIssueReportResult,
    submit_usage_feedback_cmd as submit_usage_feedback_impl,
    SubmitUsageFeedbackCmdResult,
    dev_progress,
};
use engine::{check_update_engine, cleanup_update_residuals, download_and_launch};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
// append_error_file uses fs
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// 關閉視窗時是否縮小（與 secrets 同步）
static MINIMIZE_ON_CLOSE: AtomicBool = AtomicBool::new(true);
/// 正在套用更新並即將 exit：關閉視窗時不可改成「縮到背景」。
static UPDATE_EXITING: AtomicBool = AtomicBool::new(false);
/// 翻譯（含補翻／修復）是否正在跑。
///
/// 使用者實測：翻到一半把工具關掉，重開續翻時前一小時的紀錄被蓋掉、
/// 「不備份直接覆蓋」的選擇也不見了。上一輪已經把紀錄與偏好做成可持久化，
/// 但**關閉的那一刻沒有任何保護**——沒有警告、也沒有把當下狀態寫出去。
/// 有了這個旗標，關閉流程才能先問過使用者、並讓前端把進度落檔再退出。
static TRANSLATION_ACTIVE: AtomicBool = AtomicBool::new(false);
/// 「已縮到背景」的解釋只講一次；每次都彈 blocking 對話框等於擋路。
static MINIMIZE_HINT_SHOWN: AtomicBool = AtomicBool::new(false);

const STATE_RUNNING: &str = "running";
const STATE_WAITING: &str = "waiting";
const STATE_RETRYING: &str = "retrying";
const STATE_THROTTLED: &str = "throttled";
const STATE_DEGRADED: &str = "degraded";
const STATE_CANCELLING: &str = "cancelling";
const STATE_COMPLETED: &str = "completed";
const STATE_COMPLETED_WITH_PENDING: &str = "completed_with_pending";
const PROGRESS_THROTTLE_MS: u64 = 110;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressMetricsPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    glossary: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tm: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shared: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ai: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped: Option<u64>,
    /// 品質閘略過（保留英文），與完整度略過分開。
    #[serde(skip_serializing_if = "Option::is_none")]
    quality_skipped: Option<u64>,
    /// 同包接續併入條數。
    #[serde(skip_serializing_if = "Option::is_none")]
    prior: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending: Option<u64>,
    /// 全包仍缺（覆蓋報告軌），與本輪 AI 佇列 pending 分開。
    #[serde(skip_serializing_if = "Option::is_none")]
    pack_pending: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_retry: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_retry_batches: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_fail: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_hit_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_miss_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_hit_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_percent: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    percent: u8,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step_total: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metrics: Option<ProgressMetricsPayload>,
    /// 子階段：讓「補充」這種長步驟看得出裡面跑到哪一段
    #[serde(skip_serializing_if = "Option::is_none")]
    substage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    substage_index: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    substage_total: Option<u8>,
}

/// 「補充」步驟底下的子階段數。使用者反映補充階段長時間卡在 95%，
/// 卻看不出裡面到底在做什麼——拆成具名的子階段後，畫面能顯示「第 N / M 段」。
const SUPPLEMENT_SUBSTAGES: u8 = 5;

#[derive(Debug, Clone, Default)]
struct ProgressHint {
    stage: Option<&'static str>,
    step: Option<u8>,
    step_total: Option<u8>,
    done: Option<u64>,
    total: Option<u64>,
    unit: Option<&'static str>,
    detail: Option<String>,
    state: Option<&'static str>,
    metrics: Option<ProgressMetricsPayload>,
    /// 子階段名稱。像「補充」這種長步驟其實包含好幾段（補語言檔→重建 JAR→
    /// 覆寫文字→ZIP 文字→品質重試），使用者只看到一個不動的 95% 會以為卡住。
    substage: Option<&'static str>,
    substage_index: Option<u8>,
    substage_total: Option<u8>,
}

#[derive(Debug, Default)]
struct ProgressEmitState {
    last_percent: u8,
    last_stage: Option<String>,
    last_state: Option<String>,
    last_detail: Option<String>,
    last_message: String,
    last_emit_at: Option<Instant>,
}

fn progress_emit_state() -> &'static Mutex<ProgressEmitState> {
    static STATE: OnceLock<Mutex<ProgressEmitState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(ProgressEmitState::default()))
}

fn reset_progress_emit_state() {
    let mut guard = progress_emit_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *guard = ProgressEmitState::default();
}

fn regex_pair() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d+)\s*[／/]\s*(\d+)").unwrap())
}

fn regex_ai_prehits() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"免費命中\s+(\d+)\s+句（本機術語\s+(\d+)、共享術語\s+(\d+)、共享庫\s+(\d+)、翻譯記憶\s+(\d+)），只剩\s+(\d+)\s+句")
            .unwrap()
    })
}

fn regex_ai_final_hits() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"補譯\s+(\d+)\s+條（術語\s+(\d+)、共享術語\s+(\d+)、共享庫\s+(\d+)、翻譯記憶\s+(\d+)、AI\s+(\d+)）",
        )
        .unwrap()
    })
}

fn regex_quality_skipped() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"品質未過略過\s*(\d+)\s*句|(\d+)\s*條因品質未過保留英文").unwrap()
    })
}

fn regex_prior_merged() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:接續上次|累計併入|併入)\s*(\d+)\s*條").unwrap())
}

fn regex_pack_pending() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"【仍待譯】約\s*(\d+)\s*條").unwrap())
}

fn regex_ai_batches() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d+)\s*[／/]\s*(\d+)\s*批").unwrap())
}

fn regex_ai_done() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"已得\s+(\d+)\s*[／/]\s*(\d+)\s*句").unwrap())
}

fn regex_ai_retry_fail() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"重試\s+(\d+)（(\d+)\s*批）\s*·\s*批失敗\s+(\d+)").unwrap())
}

fn regex_ai_tokens_runtime() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"token\s+命中\s+(\d+)／未命中\s+(\d+)／輸出\s+(\d+)").unwrap())
}

fn regex_ai_tokens_note() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"AI token：快取命中\s+(\d+)、未命中\s+(\d+)、輸出\s+(\d+)").unwrap())
}

fn regex_pending() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:只剩|仍缺|待補|尚可 AI 補|尚待本機資料或手動翻譯|剩餘約)\s*(\d+)\s*(?:句|條)")
            .unwrap()
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogPayload {
    /// info | warn | error
    level: String,
    message: String,
}

fn backup_status(applied: &ApplyResult) -> String {
    if !applied.is_applied() {
        "尚未套用到遊戲（見上方說明）".into()
    } else if applied.backup_reused {
        format!("沿用既有備份：{}", applied.backup_dir)
    } else if applied.backup_created {
        format!("新建備份：{}", applied.backup_dir)
    } else {
        "未建立備份（依你的選擇）".into()
    }
}

fn metrics_has_value(metrics: &ProgressMetricsPayload) -> bool {
    metrics.glossary.is_some()
        || metrics.tm.is_some()
        || metrics.shared.is_some()
        || metrics.ai.is_some()
        || metrics.skipped.is_some()
        || metrics.quality_skipped.is_some()
        || metrics.prior.is_some()
        || metrics.pending.is_some()
        || metrics.pack_pending.is_some()
        || metrics.batch_done.is_some()
        || metrics.batch_total.is_some()
        || metrics.batch_retry.is_some()
        || metrics.batch_retry_batches.is_some()
        || metrics.batch_fail.is_some()
        || metrics.cache_hit_tokens.is_some()
        || metrics.cache_miss_tokens.is_some()
        || metrics.completion_tokens.is_some()
        || metrics.cache_hit_percent.is_some()
}

fn merge_metrics(base: &mut Option<ProgressMetricsPayload>, extra: Option<ProgressMetricsPayload>) {
    let Some(extra) = extra else {
        return;
    };
    let metrics = base.get_or_insert_with(ProgressMetricsPayload::default);
    if metrics.glossary.is_none() {
        metrics.glossary = extra.glossary;
    }
    if metrics.tm.is_none() {
        metrics.tm = extra.tm;
    }
    if metrics.shared.is_none() {
        metrics.shared = extra.shared;
    }
    if metrics.ai.is_none() {
        metrics.ai = extra.ai;
    }
    if metrics.skipped.is_none() {
        metrics.skipped = extra.skipped;
    }
    if metrics.quality_skipped.is_none() {
        metrics.quality_skipped = extra.quality_skipped;
    }
    if metrics.prior.is_none() {
        metrics.prior = extra.prior;
    }
    if metrics.pending.is_none() {
        metrics.pending = extra.pending;
    }
    if metrics.pack_pending.is_none() {
        metrics.pack_pending = extra.pack_pending;
    }
    if metrics.batch_done.is_none() {
        metrics.batch_done = extra.batch_done;
    }
    if metrics.batch_total.is_none() {
        metrics.batch_total = extra.batch_total;
    }
    if metrics.batch_retry.is_none() {
        metrics.batch_retry = extra.batch_retry;
    }
    if metrics.batch_retry_batches.is_none() {
        metrics.batch_retry_batches = extra.batch_retry_batches;
    }
    if metrics.batch_fail.is_none() {
        metrics.batch_fail = extra.batch_fail;
    }
    if metrics.cache_hit_tokens.is_none() {
        metrics.cache_hit_tokens = extra.cache_hit_tokens;
    }
    if metrics.cache_miss_tokens.is_none() {
        metrics.cache_miss_tokens = extra.cache_miss_tokens;
    }
    if metrics.completion_tokens.is_none() {
        metrics.completion_tokens = extra.completion_tokens;
    }
    if metrics.cache_hit_percent.is_none() {
        metrics.cache_hit_percent = extra.cache_hit_percent;
    }
}

fn metrics_from_ai_fill(report: &AiFillReport) -> ProgressMetricsPayload {
    let mut metrics = ProgressMetricsPayload {
        glossary: Some(report.glossary_hits as u64),
        tm: Some(report.tm_hits as u64),
        shared: Some((report.shared_hits + report.shared_glossary_hits) as u64),
        ai: Some(report.ai_translated as u64),
        ..Default::default()
    };
    if report.quality_skipped > 0 {
        metrics.quality_skipped = Some(report.quality_skipped as u64);
    }
    let hit = report.usage.prompt_cache_hit_tokens as u64;
    let miss = report.usage.prompt_cache_miss_tokens as u64;
    let completion = report.usage.completion_tokens as u64;
    if hit > 0 || miss > 0 || completion > 0 {
        metrics.cache_hit_tokens = Some(hit);
        metrics.cache_miss_tokens = Some(miss);
        metrics.completion_tokens = Some(completion);
        let denom = hit.saturating_add(miss);
        if denom > 0 {
            metrics.cache_hit_percent = Some(((hit * 100) / denom) as u8);
        }
    }
    metrics
}

fn infer_progress_unit(message: &str) -> Option<&'static str> {
    if message.contains('句') {
        Some("句")
    } else if message.contains('條') {
        Some("條")
    } else if message.contains("JAR") {
        Some("個 JAR")
    } else if message.contains("資源") {
        Some("個")
    } else {
        None
    }
}

fn infer_progress_hint(message: &str) -> ProgressHint {
    let mut hint = ProgressHint::default();
    let mut metrics = ProgressMetricsPayload::default();
    if message.contains("等待本輪回應") {
        hint.state = Some(STATE_WAITING);
    } else if message.contains("已降速（限流）") || message.contains("請求太頻繁") {
        hint.state = Some(STATE_THROTTLED);
    } else if message.contains("相容降級") || message.contains("已降級") {
        hint.state = Some(STATE_DEGRADED);
    } else if message.contains("只重送仍未解決")
        || message.contains("嚴格重試")
        || message.contains("自動重試")
        || message.contains("縮小批次重送")
    {
        hint.state = Some(STATE_RETRYING);
    } else if message.contains("已停止")
        || message.contains("已依你的要求停止")
        || message.contains("停止中")
    {
        hint.state = Some(STATE_CANCELLING);
    } else if !message.trim().is_empty() {
        hint.state = Some(STATE_RUNNING);
    }

    if let Some(caps) = regex_ai_prehits().captures(message) {
        metrics.glossary = caps.get(2).and_then(|m| m.as_str().parse().ok());
        metrics.shared = Some(
            caps.get(3)
                .and_then(|m| m.as_str().parse::<u64>().ok())
                .unwrap_or(0)
                + caps
                    .get(4)
                    .and_then(|m| m.as_str().parse::<u64>().ok())
                    .unwrap_or(0),
        );
        metrics.tm = caps.get(5).and_then(|m| m.as_str().parse().ok());
        metrics.pending = caps.get(6).and_then(|m| m.as_str().parse().ok());
    }
    if let Some(caps) = regex_ai_final_hits().captures(message) {
        metrics.glossary = caps.get(2).and_then(|m| m.as_str().parse().ok());
        metrics.shared = Some(
            caps.get(3)
                .and_then(|m| m.as_str().parse::<u64>().ok())
                .unwrap_or(0)
                + caps
                    .get(4)
                    .and_then(|m| m.as_str().parse::<u64>().ok())
                    .unwrap_or(0),
        );
        metrics.tm = caps.get(5).and_then(|m| m.as_str().parse().ok());
        metrics.ai = caps.get(6).and_then(|m| m.as_str().parse().ok());
    }
    if let Some(caps) = regex_quality_skipped().captures(message) {
        metrics.quality_skipped = caps
            .get(1)
            .or_else(|| caps.get(2))
            .and_then(|m| m.as_str().parse().ok());
    }
    if let Some(caps) = regex_prior_merged().captures(message) {
        metrics.prior = caps.get(1).and_then(|m| m.as_str().parse().ok());
    }
    if let Some(caps) = regex_pack_pending().captures(message) {
        metrics.pack_pending = caps.get(1).and_then(|m| m.as_str().parse().ok());
    }
    if let Some(caps) = regex_ai_batches().captures(message) {
        metrics.batch_done = caps.get(1).and_then(|m| m.as_str().parse().ok());
        metrics.batch_total = caps.get(2).and_then(|m| m.as_str().parse().ok());
    }
    if let Some(caps) = regex_ai_done().captures(message) {
        hint.done = caps.get(1).and_then(|m| m.as_str().parse().ok());
        hint.total = caps.get(2).and_then(|m| m.as_str().parse().ok());
        hint.unit = Some("句");
        metrics.ai = hint.done;
    }
    if let Some(caps) = regex_ai_retry_fail().captures(message) {
        metrics.batch_retry = caps.get(1).and_then(|m| m.as_str().parse().ok());
        metrics.batch_retry_batches = caps.get(2).and_then(|m| m.as_str().parse().ok());
        metrics.batch_fail = caps.get(3).and_then(|m| m.as_str().parse().ok());
    }
    if let Some(caps) = regex_ai_tokens_runtime().captures(message) {
        metrics.cache_hit_tokens = caps.get(1).and_then(|m| m.as_str().parse().ok());
        metrics.cache_miss_tokens = caps.get(2).and_then(|m| m.as_str().parse().ok());
        metrics.completion_tokens = caps.get(3).and_then(|m| m.as_str().parse().ok());
    } else if let Some(caps) = regex_ai_tokens_note().captures(message) {
        metrics.cache_hit_tokens = caps.get(1).and_then(|m| m.as_str().parse().ok());
        metrics.cache_miss_tokens = caps.get(2).and_then(|m| m.as_str().parse().ok());
        metrics.completion_tokens = caps.get(3).and_then(|m| m.as_str().parse().ok());
    }
    if metrics.pending.is_none() {
        metrics.pending = regex_pending()
            .captures(message)
            .and_then(|caps| caps.get(1))
            .and_then(|m| m.as_str().parse().ok());
    }
    if hint.done.is_none() {
        if let Some(caps) = regex_pair().captures_iter(message).last() {
            hint.done = caps.get(1).and_then(|m| m.as_str().parse().ok());
            hint.total = caps.get(2).and_then(|m| m.as_str().parse().ok());
            hint.unit = infer_progress_unit(message);
        }
    }
    if let (Some(hit), Some(miss)) = (metrics.cache_hit_tokens, metrics.cache_miss_tokens) {
        let total = hit.saturating_add(miss);
        if total > 0 {
            metrics.cache_hit_percent = Some(((hit * 100) / total).min(100) as u8);
        }
    }
    if message.starts_with("AI 翻譯中…") {
        // 詳細數字已在 metrics；detail 只留短標籤，避免步驟／狀態／summary 三重複
        if message.contains("等待本輪回應") {
            hint.detail = Some("等待本輪回應".into());
        } else if message.contains("本輪含批失敗") {
            hint.detail = Some("本輪含批失敗".into());
        } else {
            hint.detail = None;
        }
    } else if message.starts_with("AI 限流：") || message.starts_with("AI 相容降級：") {
        hint.detail = Some(message.to_string());
    } else if message.contains("等待 Discord") || message.contains("重新登入 Discord") {
        hint.detail = Some("等待 Discord 登入".into());
        hint.state = Some(STATE_WAITING);
    }
    if metrics_has_value(&metrics) {
        hint.metrics = Some(metrics);
    }
    hint
}

fn merge_progress_hint(base: &mut ProgressHint, extra: ProgressHint) {
    if base.step.is_none() {
        base.step = extra.step;
    }
    if base.step_total.is_none() {
        base.step_total = extra.step_total;
    }
    if base.done.is_none() {
        base.done = extra.done;
    }
    if base.total.is_none() {
        base.total = extra.total;
    }
    if base.unit.is_none() {
        base.unit = extra.unit;
    }
    if base.detail.is_none() {
        base.detail = extra.detail;
    }
    if base.state.is_none() {
        base.state = extra.state;
    }
    merge_metrics(&mut base.metrics, extra.metrics);
}

fn emit_progress_ex(app: &AppHandle, percent: Option<u8>, message: &str, mut hint: ProgressHint) {
    let inferred = infer_progress_hint(message);
    merge_progress_hint(&mut hint, inferred);
    if hint.step.is_none() || hint.step_total.is_none() {
        if let Some(stage) = hint.stage {
            if let Some(spec) = dev_progress::stage_progress_spec(stage) {
                hint.step.get_or_insert(spec.step);
                hint.step_total.get_or_insert(dev_progress::UI_STEP_TOTAL);
            }
        }
    }
    let computed_percent = match (hint.stage, hint.done, hint.total) {
        (Some(stage), Some(done), Some(total)) => dev_progress::weighted_percent(stage, done, total),
        _ => None,
    };
    let mut payload = ProgressPayload {
        percent: computed_percent.or(percent).unwrap_or(0).min(100),
        message: message.to_string(),
        stage: hint.stage.map(str::to_string),
        step: hint.step,
        step_total: hint.step_total,
        done: hint.done,
        total: hint.total,
        unit: hint.unit.map(str::to_string),
        detail: hint.detail.filter(|s| !s.trim().is_empty()),
        state: hint.state.map(str::to_string),
        metrics: hint.metrics.filter(metrics_has_value),
        substage: hint.substage.map(str::to_string),
        substage_index: hint.substage_index,
        substage_total: hint.substage_total,
    };

    let mut guard = progress_emit_state()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    payload.percent = payload.percent.max(guard.last_percent);
    let stage_changed = payload.stage.as_deref() != guard.last_stage.as_deref();
    let state_changed = payload.state.as_deref() != guard.last_state.as_deref();
    let detail_changed = payload.detail.as_deref() != guard.last_detail.as_deref();
    let percent_increased = payload.percent > guard.last_percent;
    let message_changed = payload.message != guard.last_message;
    let force_emit = guard.last_emit_at.is_none()
        || stage_changed
        || state_changed
        || percent_increased
        || payload.percent == 100
        || payload.message == "已停止";
    let elapsed_ok = guard
        .last_emit_at
        .map(|t| t.elapsed() >= Duration::from_millis(PROGRESS_THROTTLE_MS))
        .unwrap_or(true);
    if force_emit || (elapsed_ok && (message_changed || detail_changed)) {
        let _ = app.emit("translate-progress", payload.clone());
        guard.last_percent = payload.percent;
        guard.last_stage = payload.stage;
        guard.last_state = payload.state;
        guard.last_detail = payload.detail;
        guard.last_message = payload.message;
        guard.last_emit_at = Some(Instant::now());
    }
}

fn emit_progress_stage(app: &AppHandle, stage: &'static str, percent: Option<u8>, message: &str) {
    emit_progress_ex(
        app,
        percent,
        message,
        ProgressHint {
            stage: Some(stage),
            ..Default::default()
        },
    );
}

fn emit_progress(app: &AppHandle, percent: u8, message: &str) {
    emit_progress_ex(app, Some(percent), message, ProgressHint::default());
}

fn emit_log(app: &AppHandle, level: &str, message: &str) {
    let _ = app.emit(
        "translate-log",
        LogPayload {
            level: level.to_string(),
            message: message.to_string(),
        },
    );
}

fn emit_pruned_tool_pack_log(app: &AppHandle, pruned: &[String]) {
    if pruned.is_empty() {
        return;
    }
    emit_log(
        app,
        "info",
        &format!(
            "已移除 {} 個舊版工具資源包：{}",
            pruned.len(),
            pruned.join("、")
        ),
    );
}

fn emit_error(app: &AppHandle, message: &str) {
    emit_log(app, "error", &format!("【錯誤】{message}"));
}

fn emit_warn(app: &AppHandle, message: &str) {
    emit_log(app, "warn", &format!("【警告】{message}"));
}

/// 寫入結果目錄的錯誤／警告檔，方便離開工具後排查
fn append_error_file(work: &Path, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let p = work.join("翻譯錯誤日誌.txt");
    let header = format!("\n======== {} ========\n", chrono_like_now());
    let body = lines.join("\n") + "\n";
    let mut content = String::new();
    if p.is_file() {
        if let Ok(old) = fs::read_to_string(&p) {
            content = old;
        }
    } else {
        content = "【模組包繁中翻譯 — 錯誤／警告日誌】\n有問題時請把本檔內容一併提供。\n".into();
    }
    content.push_str(&header);
    content.push_str(&body);
    let _ = fs::write(p, content);
}

/// 錯誤日誌的時間戳。舊版寫的是 `unix=1754…`，玩家回報問題時根本對不上時間，
/// 這裡自己換算成看得懂的 UTC 日期時間（不為了這件事多拉一個 crate）。
fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_utc(secs)
}

fn format_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

/// Howard Hinnant 的 civil_from_days：把 1970-01-01 起的天數換回年月日。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OneClickResult {
    report: ScanReport,
    pack_path: String,
    /// 實際「翻譯結果」工作根（工具自動建立）
    work_root: String,
    namespaces: usize,
    files_written: usize,
    keys_total: usize,
    ai_filled: usize,
    pending_count: usize,
    coverage_percent: u8,
    completed_with_pending: bool,
    jar_translation: JarTranslationReport,
    minemenu_msg: Option<String>,
    player_summary: String,
    /// 套用完成後，若在同一個上層資料夾（例如 PrismLauncher 的 `instances\`）
    /// 找到模組內容跟這次套用對象一模一樣的其他資料夾，提醒使用者確認
    /// 有沒有選錯／實例被改名或複製過，避免以為套用失敗。
    sibling_instance_warning: Option<String>,
    /// 本來就不該翻、已原樣保留的項目數（附魔等級的羅馬數字、單位符號、
    /// 字型圖示、品牌名、註解鍵等）。跟「待補」分開報，避免使用者把
    /// 「維持原文才正確」的東西誤會成工具漏翻。
    stays_unchanged: usize,
    /// 這次**實際採用**的設定，以及每個與使用者選擇不同的欄位的原因（P0-01）。
    /// 前端照這份顯示，不自己推測後端做了什麼。
    run_plan: engine::run_plan::RunPlan,
    /// 是否有任何欄位與使用者的選擇不同。UI 用它決定要不要顯示說明區塊。
    run_plan_has_overrides: bool,
    /// 套用到遊戲的結果。不是 `applied` 時＝翻譯已完成、還沒裝進遊戲（遊戲開著、
    /// 還沒啟動過遊戲、還沒選備份、不備份要確認覆蓋），前端顯示原因並提供「套用到遊戲」。
    apply_status: ApplyStatus,
    /// 還沒套用時給玩家看的說明；已套用時為空
    apply_message: String,
    /// 不備份模式下，等玩家確認才會覆蓋的原檔
    pending_overwrites: Vec<String>,
    /// B2：顯示安全——退回英文清單與「字體可能不支援中文」（B8 顯示）
    display_safety: engine::DisplaySafety,
    /// B4：這一輪有沒有中途停下（使用者停止／AI 不可用）、沒回應與品質沒過各幾條（B8 顯示）
    interruption: engine::run_interrupt::InterruptionView,
    /// B5c：套用到遊戲的完整結果（備份位置、語言、隔離、模組已更新、舊產物、退休、讀不到…），
    /// 完成卡「已幫你做的事」「還是英文的部分」照這份寫，不再從中文句子猜。由 with_apply_notice 填入。
    apply_result: Option<ApplyResult>,
}

/// 把套用狀態接到翻譯結果上；還沒裝進遊戲時，結論的第一句就要講這件事。
fn with_apply_notice(mut result: OneClickResult, applied: &ApplyResult) -> OneClickResult {
    result.display_safety = engine::take_run_report(Path::new(&result.report.minecraft_dir));
    result.apply_status = applied.status;
    result.pending_overwrites = applied.pending_overwrites.clone();
    result.apply_result = Some(applied.clone());
    if !applied.is_applied() {
        result.apply_message = applied.player_summary.clone();
        result.player_summary = format!(
            "【翻譯已完成，還沒套用到遊戲】\n{}\n\n{}",
            applied.player_summary, result.player_summary
        );
    }
    // B4：AI 中途停下時，結論第一句先講這件事（已翻好的保留、下一步按接續補完）
    if let Some(note) = &result.interruption.ai_stopped {
        result.player_summary = format!("【這一輪中途停下】\n{note}\n\n{}", result.player_summary);
    }
    result
}

/// 翻譯流程最後的套用：備份做法照設定（第一次會先問），遊戲開著等情況回「已翻完、未套用」，
/// 不算翻譯失敗。
fn apply_after_run(
    app: &AppHandle,
    instance: &Path,
    work: &Path,
    pack_name: &str,
) -> Result<ApplyResult, String> {
    // 開始翻譯時「保留／不保留翻譯結果」只管結果資料夾；備份一律照設定
    let policy = engine::apply_record::policy_for_run(false);
    // B5c：複製資料夾被擋＝已翻完、還沒套用（不是翻譯失敗；零寫入，G1.4／G1.24）
    let applied = engine::pending_when_copied(apply_to_instance(instance, work, Some(pack_name), policy))?;
    if !applied.is_applied() {
        emit_warn(app, &applied.player_summary);
    }
    Ok(applied)
}

/// 套用成功後順手檢查一次：同一個上層資料夾底下有沒有其他資料夾的模組內容
/// 跟這次套用對象一模一樣。找到就回一句可以直接顯示給玩家看的提醒；沒找到、
/// 或掃描失敗都回 None——這是提醒用，不影響套用流程本身是否成功。
fn detect_sibling_instance_warning(instance: &Path) -> Option<String> {
    let siblings = engine::find_sibling_instances_with_same_mods(instance);
    if siblings.is_empty() {
        return None;
    }
    let names: Vec<String> = siblings
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    if names.is_empty() {
        return None;
    }
    Some(format!(
        "偵測到 {} 個資料夾的模組內容跟這次套用的一模一樣：{}。如果你實際遊玩的是其中一個，翻譯不會出現在裡面，請改選該資料夾重新套用。",
        names.len(),
        names.join("、")
    ))
}

#[derive(Debug, Clone, Copy)]
enum ExtraSourceKind {
    Ftbquests,
    TextOverlay,
    ArchiveOverlay,
    Origins,
    QuestsBooks,
    ScriptLiterals,
    MineMenu,
}

#[derive(Debug, Clone, Copy)]
struct ExtraSourceTask {
    kind: ExtraSourceKind,
    base: u8,
    span: u8,
}

#[derive(Debug, Default)]
struct ExtraSourceOutcome {
    note: String,
    errors: Vec<String>,
    /// 這個階段掃到幾個可處理單位、實際做成幾個。
    ///
    /// 用來判斷「找到東西卻一個都沒做成」——那種情況結尾不准說「完成」。
    /// `None` 代表這個階段還沒提供計數（過渡期），帳本會當成未知而不是 0。
    counts: Option<(usize, usize)>,
}

#[derive(Debug, Default)]
struct ExtraSourceSummary {
    notes: Vec<String>,
    skipped: Vec<String>,
    errors: Vec<String>,
    cancelled: bool,
    /// 各階段的完整性帳本（見 engine/coverage_ledger.rs）。
    ledger: engine::CoverageLedger,
}

impl ExtraSourceKind {
    fn label(self) -> &'static str {
        match self {
            Self::Ftbquests => "FTB Quests",
            Self::TextOverlay => "文字覆寫",
            Self::ArchiveOverlay => "ZIP 文字",
            Self::Origins => "Origins",
            Self::QuestsBooks => "任務／書本",
            Self::ScriptLiterals => "KubeJS 顯示字串",
            Self::MineMenu => "快捷選單",
        }
    }

    fn skipped_note(self) -> String {
        format!("完整度略過：{}", self.label())
    }
}

impl ExtraSourceSummary {
    fn combined_note(&self) -> String {
        self.notes
            .iter()
            .filter(|note| !note.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("；")
    }
}

/// 路徑可含空白；去掉首尾空白與貼上時多帶的引號；阻擋可疑路徑
fn normalize_path(s: &str) -> PathBuf {
    normalize_user_path(s).unwrap_or_else(|_| {
        let t = s.trim().trim_matches('"').trim_matches('\'');
        PathBuf::from(t)
    })
}

fn normalize_path_strict(s: &str) -> Result<PathBuf, String> {
    normalize_user_path(s)
}

fn extra_source_tasks(sources: CoverageSourceFlags, base: u8, span: u8) -> (Vec<ExtraSourceTask>, Vec<String>) {
    let candidates = [
        (ExtraSourceKind::Ftbquests, sources.ftbquests),
        (ExtraSourceKind::TextOverlay, sources.text_overlay),
        (ExtraSourceKind::ArchiveOverlay, sources.archive_overlay),
        (ExtraSourceKind::Origins, sources.origins),
        (ExtraSourceKind::QuestsBooks, sources.quests_books),
        (ExtraSourceKind::ScriptLiterals, sources.script_literals),
        // 與文字覆寫同階：標題硬編碼在 minemenu/menu.json
        (ExtraSourceKind::MineMenu, sources.text_overlay),
    ];
    let enabled_total = candidates.iter().filter(|(_, enabled)| *enabled).count().max(1);
    let mut enabled_seen = 0usize;
    let mut tasks = Vec::new();
    let mut skipped = Vec::new();
    for (kind, enabled) in candidates {
        if !enabled {
            skipped.push(kind.skipped_note());
            continue;
        }
        let start = base as u16 + (enabled_seen as u16 * span as u16 / enabled_total as u16);
        let end = base as u16 + ((enabled_seen + 1) as u16 * span as u16 / enabled_total as u16);
        let source_span = end.saturating_sub(start).max(1).min(100) as u8;
        tasks.push(ExtraSourceTask {
            kind,
            base: start.min(100) as u8,
            span: source_span,
        });
        enabled_seen += 1;
    }
    (tasks, skipped)
}

fn run_one_extra_source(
    app: AppHandle,
    mc: &Path,
    work: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    task: ExtraSourceTask,
    index: usize,
    total: usize,
) -> Result<ExtraSourceOutcome, String> {
    check_cancelled()?;
    let mut outcome = ExtraSourceOutcome::default();
    let label = task.kind.label();
    let stage = format!("extra:{label}");
    let prefix = format!("補充 {}/{} · {}", index + 1, total, label);
    dev_progress::enter(&stage);
    emit_progress_stage(
        &app,
        dev_progress::STAGE_EXTRAS,
        Some(task.base),
        &format!("{prefix}：開始{}", if use_ai { "" } else { "離線" }),
    );
    match task.kind {
        ExtraSourceKind::Ftbquests => {
            let app_q = app.clone();
            let q = translate_ftbquests(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress_stage(
                    &app_q,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, pct)),
                    msg,
                );
            })?;
            if q.files_written > 0 {
                emit_progress_stage(
                    &app,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, 100)),
                    &format!("任務已寫出 {} 個檔到「翻譯結果」", q.files_written),
                );
            }
            outcome.note = q.note;
        }
        ExtraSourceKind::TextOverlay => {
            let app_o = app.clone();
            let o = translate_text_overlays(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress_stage(
                    &app_o,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, pct)),
                    msg,
                );
            })?;
            if o.files_written > 0 {
                emit_progress_stage(
                    &app,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, 100)),
                    &format!("覆寫文字已寫出 {} 個檔", o.files_written),
                );
            }
            outcome.note = o.note;
        }
        ExtraSourceKind::ArchiveOverlay => {
            let app_a = app.clone();
            let a = translate_archive_overlays(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress_stage(
                    &app_a,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, pct)),
                    msg,
                );
            })?;
            // resourcepacks 內的 ZIP 是設計上的正常略過，只在摘要裡帶一句，
            // 不進錯誤日誌（實測會多出 162 行雜訊，把真正的解析失敗淹沒）。
            let normal_skip_note = if a.skipped_resourcepack_zips > 0 {
                format!(
                    "；另有 {} 個 resourcepacks 內的 ZIP 依設計不重建（語言檔已併入主資源包，屬正常）",
                    a.skipped_resourcepack_zips
                )
            } else {
                String::new()
            };
            outcome.note = if a.skipped.is_empty() {
                format!(
                    "ZIP 文字：掃描 {} 個、重建 {} 個、寫入 {} 個項目{normal_skip_note}",
                    a.archives_scanned, a.archives_rewritten, a.entries_rewritten
                )
            } else {
                let problem_count = a.skipped.len();
                for skipped in a.skipped {
                    outcome.errors.push(format!("ZIP 文字：{skipped}"));
                }
                format!(
                    "ZIP 文字：掃描 {} 個、重建 {} 個；{} 個因問題略過（詳見錯誤日誌）{normal_skip_note}",
                    a.archives_scanned, a.archives_rewritten, problem_count
                )
            };
        }
        ExtraSourceKind::Origins => {
            let app_or = app.clone();
            let o = translate_origins(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress_stage(
                    &app_or,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, pct)),
                    msg,
                );
            })?;
            if o.files_written > 0 {
                emit_progress_stage(
                    &app,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, 100)),
                    &format!("Origins 能力已寫出 {} 個檔", o.files_written),
                );
            }
            // 鬆散資料包掃完，再掃 JAR 內的能力檔。
            //
            // Origins 系整合包通常把 powers 打包在模組自己的 JAR 裡，
            // 只掃資料夾會整片漏掉（能力名稱與說明全留英文）。
            let app_jo = app.clone();
            match translate_jar_origins(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress_stage(
                    &app_jo,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, pct)),
                    msg,
                );
            }) {
                Ok(j) => {
                    if j.strings_translated > 0 {
                        outcome.note = format!("{}；{}", o.note, j.note);
                    } else {
                        outcome.note = o.note;
                    }
                    for skipped in j.skipped {
                        outcome.errors.push(format!("JAR 能力檔：{skipped}"));
                    }
                }
                Err(error) => {
                    // JAR 內能力檔翻不了不該讓整輪失敗——鬆散資料包那部分已經完成
                    outcome.errors.push(format!("JAR 內 Origins 略過：{error}"));
                    outcome.note = o.note;
                }
            }
        }
        ExtraSourceKind::QuestsBooks => {
            let app_qb = app.clone();
            let o = translate_quests_books(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress_stage(
                    &app_qb,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, pct)),
                    msg,
                );
            })?;
            if o.files_written > 0 {
                emit_progress_stage(
                    &app,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, 100)),
                    &format!("任務／書本已寫出 {} 個檔", o.files_written),
                );
            }
            outcome.note = o.note;
        }
        ExtraSourceKind::ScriptLiterals => {
            let app_s = app.clone();
            let s = translate_kubejs_literals(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress_stage(
                    &app_s,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, pct)),
                    msg,
                );
            })?;
            outcome.note = s.note;
        }
        ExtraSourceKind::MineMenu => {
            let app_m = app.clone();
            let note = translate_minemenu(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress_stage(
                    &app_m,
                    dev_progress::STAGE_EXTRAS,
                    Some(map_stage_progress(task.base, task.span, pct)),
                    msg,
                );
            })?;
            outcome.note = note;
        }
    }
    dev_progress::leave(&stage);
    Ok(outcome)
}

fn run_extra_sources(
    app: &AppHandle,
    mc: &Path,
    work: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    sources: CoverageSourceFlags,
    base: u8,
    span: u8,
) -> ExtraSourceSummary {
    let (tasks, skipped) = extra_source_tasks(sources, base, span);
    let mut summary = ExtraSourceSummary {
        skipped,
        ..Default::default()
    };
    if tasks.is_empty() {
        return summary;
    }
    let total = tasks.len();

    let mut collect = |task: ExtraSourceTask, result: std::thread::Result<Result<ExtraSourceOutcome, String>>| {
        match result {
            Ok(Ok(outcome)) => {
                if let Some((found, done)) = outcome.counts {
                    summary
                        .ledger
                        .record(engine::StageEntry::ok(task.kind.label(), found, done));
                }
                if !outcome.note.trim().is_empty() {
                    emit_log(app, "info", &outcome.note);
                    summary.notes.push(outcome.note);
                }
                summary.errors.extend(outcome.errors);
            }
            Ok(Err(error)) => {
                // 這個階段整段失敗：記進帳本，結尾就不會謊報「完成」。
                // found 取「至少 1」——我們知道它有東西要做（不然不會走到這裡），
                // 精確數量由各階段自己回報。
                summary.ledger.record(engine::StageEntry::failed_unknown_count(
                    task.kind.label(),
                    error.lines().next().unwrap_or("失敗").to_string(),
                ));
                let line = format!("{} 略過／失敗：{error}", task.kind.label());
                if looks_like_cancel_message(&error) {
                    emit_warn(app, &line);
                } else {
                    emit_error(app, &line);
                }
                summary.notes.push(line.clone());
                summary.errors.push(line);
                if looks_like_cancel_message(&error) {
                    summary.cancelled = true;
                }
            }
            Err(_) => {
                summary.ledger.record(engine::StageEntry::failed_unknown_count(
                    task.kind.label(),
                    "背景工作發生錯誤".to_string(),
                ));
                let line = format!("{} 略過／失敗：背景工作發生 panic", task.kind.label());
                emit_error(app, &line);
                summary.notes.push(line.clone());
                summary.errors.push(line);
            }
        }
    };

    emit_log(
        app,
        "info",
        if use_ai {
            "AI 補充來源最多 3 路安全並行；寫入與翻譯記憶會自動合併。"
        } else {
            "未勾選 AI：額外來源最多 3 路並行整理。"
        },
    );
    let indexed: Vec<(usize, ExtraSourceTask)> = tasks.into_iter().enumerate().collect();
    for chunk in indexed.chunks(3) {
        if is_cancelled() {
            summary.cancelled = true;
            break;
        }
        std::thread::scope(|scope_thread| {
            let mut handles = Vec::new();
            for &(i, task) in chunk {
                let app_task = app.clone();
                handles.push((
                    task,
                    scope_thread.spawn(move || {
                        run_one_extra_source(app_task, mc, work, use_ai, scope, task, i, total)
                    }),
                ));
            }
            for (task, handle) in handles {
                collect(task, handle.join());
            }
        });
    }

    if !summary.cancelled {
        emit_progress_stage(
            app,
            dev_progress::STAGE_EXTRAS,
            Some(base.saturating_add(span).min(100)),
            &format!("補充來源全部完成（共 {total} 項）"),
        );
    }

    summary
}

/// 非同步 command：UI 不會卡住；進度用事件推送
/// 背景工作整個掛掉（panic 或被取消）時，要對使用者說什麼。
///
/// 站長那份執行紀錄最後一行是：
/// `工作中斷：task 203 panicked with message "byte index 2 is not a char boundary…"`
/// ——把 Rust 的內部錯誤原封不動丟給玩家，他既看不懂、也不知道該做什麼，
/// 只知道等了三小時的翻譯沒了。
///
/// 技術細節仍然要留（寫進日誌檔給我們除錯），但畫面上要講人話：
/// 這不是他的錯、他可以做什麼。
fn describe_worker_failure<E: std::fmt::Display>(e: &E) -> String {
    let detail = e.to_string();
    if detail.contains("panicked") {
        format!(
            "翻譯中途發生程式錯誤，已經停在這裡。這不是你的操作問題，已完成的部分都有保留，可以直接再按一次「開始翻譯」接續。方便的話請用頁尾的「回報」告訴我們，我們會修。
（技術細節：{detail}）"
        )
    } else {
        format!("工作中斷：{detail}")
    }
}

#[tauri::command]
async fn one_click_translate(
    app: AppHandle,
    instance_path: String,
    output_dir: String,
    pack_name: String,
    use_ai: bool,
    // 開始翻譯時選「保留翻譯結果」＝true；只管結果資料夾，不影響備份
    keep_results: bool,
    reference_pack: Option<String>,
    target_version: Option<String>,
    translation_mode: Option<String>,
    translation_quality: Option<String>,
    coverage_tier: Option<String>,
    advanced_unpack: Option<bool>,
) -> Result<OneClickResult, String> {
    reset_progress_emit_state();
    let instance = match normalize_path_strict(&instance_path) {
        Ok(p) => p,
        Err(e) => {
            emit_error(&app, &e);
            return Err(e);
        }
    };
    let out = match normalize_path_strict(&output_dir) {
        Ok(p) => p,
        Err(e) => {
            emit_error(&app, &e);
            return Err(e);
        }
    };
    let (pack_name, pack_version, pack_name_custom) =
        resolve_output_pack_name(&pack_name, &instance);
    emit_log(
        &app,
        "info",
        &format!(
            "這次資源包版本：{}（{}）\n輸出名稱：{}（{}）",
            pack_version.version,
            pack_version.source,
            pack_name,
            if pack_name_custom {
                "自訂"
            } else {
                "系統"
            }
        ),
    );
    let use_ai = use_ai;
    let reference_pack = reference_pack
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let target_version = target_version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let _ = advanced_unpack;
    let advanced_unpack = true;

    reset_cancel();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_one_click(
            &app2,
            instance,
            out,
            pack_name,
            use_ai,
            keep_results,
            reference_pack,
            target_version,
            translation_mode,
            translation_quality,
            coverage_tier,
            advanced_unpack,
        )
    })
    .await
    .map_err(|e| describe_worker_failure(&e))?;
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            dev_progress::finish(&format!("error:{e}"));
            report_failure(&app, &e);
            Err(e)
        }
    }
}

/// 取消與失敗要分開講：使用者按了停止不該看到滿畫面紅字。
/// 取消時把當下有效 en∩zh 掃尾進共享庫（成功路徑請 `disarm()`）。
struct OnCancelShare {
    active: bool,
    app: *const AppHandle,
    en: *const LangMap,
    zh: *const LangMap,
    scope: *const TranslationScope,
    /// 用來認出「哪些譯文是從本機參考包合併進來的」——那些不上傳
    provenance: *const ProvenanceMap,
}

// Safety: 指標在 one_click 內指向同函數棧上的 zh／en／scope／app，
// Drop 時發生於提前 return／panic 路徑，此時已無其他 &mut 借用。
unsafe impl Send for OnCancelShare {}

impl OnCancelShare {
    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for OnCancelShare {
    fn drop(&mut self) {
        if !self.active || !is_cancelled() {
            return;
        }
        // Safety: 見 struct 註解
        let (app, en, zh, scope, provenance) = unsafe {
            (
                &*self.app,
                &*self.en,
                &*self.zh,
                &*self.scope,
                &*self.provenance,
            )
        };
        emit_progress_stage(
            app,
            dev_progress::STAGE_PACKAGE,
            Some(96),
            "共享庫（停止掃尾）…",
        );
        // 停止：3s／1 chunk／最多 3000／不 flush 舊佇列——寧可少傳，不可卡住 Drop
        let contrib = contribute_lang_maps_limited(
            en,
            zh,
            scope,
            Some(provenance),
            ContributeLangMapsOpts::cancel_sweep(),
        );
        let mut detail = format!(
            "共享庫（停止掃尾）：accepted={}／衝突 {}／送出 {}",
            contrib.accepted, contrib.conflicts, contrib.attempted
        );
        if contrib.deferred > 0 {
            detail.push_str(&format!("；略過／暫緩 {} 條（避免卡住）", contrib.deferred));
        }
        if contrib.failed {
            detail.push_str("（失敗已排程重試）");
        }
        if contrib.attempted == 0 && contrib.deferred == 0 && !contrib.failed {
            detail.push_str("（本輪已送過或無可送）");
        }
        emit_log(app, "info", &detail);
        emit_progress_stage(
            app,
            dev_progress::STAGE_PACKAGE,
            Some(96),
            "共享庫停止掃尾結束",
        );
    }
}

fn looks_like_cancel_message(message: &str) -> bool {
    message.contains("已依你的要求停止")
}

fn report_failure(app: &AppHandle, message: &str) {
    if looks_like_cancel_message(message) {
        emit_warn(app, CANCEL_MESSAGE);
        emit_progress_ex(
            app,
            Some(0),
            "已停止",
            ProgressHint {
                state: Some(STATE_CANCELLING),
                ..Default::default()
            },
        );
    } else {
        emit_error(app, message);
    }
}

fn rewrite_jars_and_log(
    app: &AppHandle,
    instance: &Path,
    work: &Path,
    translated: &LangMap,
    fallback_english: &LangMap,
) -> Result<JarTranslationReport, String> {
    let report = rewrite_translated_jars(instance, translated, fallback_english, work, |done, total, msg| {
        emit_progress_ex(
            app,
            None,
            msg,
            ProgressHint {
                stage: Some(dev_progress::STAGE_EXTRAS),
                done: Some(done),
                total: Some(total),
                unit: Some("個 JAR"),
                detail: Some("JAR 翻譯副本".into()),
                state: Some(STATE_RUNNING),
                ..Default::default()
            },
        );
    })?;
    emit_log(
        app,
        "info",
        &format!(
            "JAR 翻譯副本：掃描 {} 個、重建 {} 個、寫入 {} 個語言檔、{} 個字串。",
            report.jars_scanned, report.jars_rewritten, report.lang_files_written, report.keys_written
        ),
    );
    if report.fallback_keys_kept > 0 {
        emit_warn(
            app,
            &format!(
                "JAR 仍有 {} 個字串保留原文；可再次複查、使用翻譯服務或手動補上。",
                report.fallback_keys_kept
            ),
        );
    }
    for error in &report.errors {
        emit_warn(app, &format!("JAR 翻譯副本略過：{error}"));
    }
    if !report.errors.is_empty() {
        append_error_file(work, &report.errors);
        emit_warn(
            app,
            &format!(
                "有 {} 個 JAR 無法建立翻譯副本；詳細原因已寫入：{}",
                report.errors.len(),
                work.join("翻譯錯誤日誌.txt").display()
            ),
        );
    }
    Ok(report)
}

/// 中止進行中的長任務（掃描／補譯／覆寫）。已完成的部分留在結果資料夾。
#[tauri::command]
fn cancel_task() -> String {
    request_cancel();
    "已送出停止要求，正在收尾…".into()
}

/// 偵測整合包的 Minecraft 版本（給 UI 預填版本選單）。偵測不到回 null。
#[tauri::command(async)]
fn detect_mc_version(instance_path: String) -> Option<String> {
    let inst = normalize_path(&instance_path);
    let mc = resolve_minecraft_dir(&inst).unwrap_or(inst);
    detect_minecraft_version(&mc)
}

/// Returns the resource-pack version used in the generated pack name.  This
/// intentionally does not expose or reuse the application version.
#[tauri::command(async)]
fn detect_pack_translation_name(instance_path: String) -> Result<PackVersionInfo, String> {
    let instance = normalize_path_strict(&instance_path)?;
    let (_, info) = build_pack_name(&instance);
    Ok(info)
}

#[tauri::command]
async fn inspect_jar_documentation(
    instance_path: String,
    output_dir: String,
) -> Result<JarDocumentationReport, String> {
    let instance = normalize_path_strict(&instance_path)?;
    let output = normalize_path_strict(&output_dir)?;
    tauri::async_runtime::spawn_blocking(move || {
        let layout = ensure_result_layout(&output)?;
        extract_jar_documentation(&instance, &layout.work_root)
    })
    .await
    .map_err(|e| format!("JAR 文件複查工作失敗：{e}"))?
}

/// 診斷「遊戲／世界開不起來」：讀當機報告與 log，判斷是缺模組還是我們的檔。
#[tauri::command]
async fn diagnose_launch_failure(instance_path: String) -> Result<LaunchDiagnosis, String> {
    let inst = normalize_path_strict(&instance_path)?;
    tauri::async_runtime::spawn_blocking(move || diagnose_launch(&inst))
        .await
        .map_err(|e| describe_worker_failure(&e))
}

/// 明確以整合包／實例目錄做記錄＋ mods 交叉驗證。
#[tauri::command]
async fn diagnose_pack_dir_cmd(path: String) -> Result<LaunchDiagnosis, String> {
    let pack = normalize_path_strict(&path)?;
    tauri::async_runtime::spawn_blocking(move || diagnose_pack_dir(&pack))
        .await
        .map_err(|e| describe_worker_failure(&e))
}

#[tauri::command]
fn diagnose_error_text(text: String) -> LaunchDiagnosis {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return classify_diagnosis("", "貼上的錯誤文字");
    }
    // 單行且為既有目錄 → pack_dir；否則 pasted_log。
    classify_diagnosis_input(trimmed)
}

/// 一鍵還原上次套用（新增的刪掉、覆蓋的還原），用來排除「是不是翻譯造成開不起來」。
#[tauri::command]
async fn restore_last_apply_cmd(
    app: AppHandle,
    instance_path: String,
    output_dir: Option<String>,
) -> Result<RestoreResult, String> {
    reset_progress_emit_state();
    let inst = normalize_path_strict(&instance_path)?;
    let result_root = output_dir
        .as_deref()
        .map(normalize_path_strict)
        .transpose()?;
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        emit_progress(&app2, 20, "還原：讀取上次套用的備份…");
        let r = restore_last_apply_in(&inst, result_root.as_deref());
        if r.is_ok() {
            emit_progress(&app2, 100, "還原完成");
        }
        r
    })
    .await
    .map_err(|e| describe_worker_failure(&e))?;
    if let Err(e) = &r {
        emit_error(&app, e);
    }
    r
}

#[tauri::command]
async fn submit_diagnose_report_cmd(
    app: AppHandle,
    request: DiagnoseReportRequest,
) -> Result<DiagnoseReportResult, String> {
    reset_progress_emit_state();
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        emit_progress(&app2, 15, "診斷回報：打包並上傳…");
        let r = submit_diagnose_report(&request);
        if r.is_ok() {
            emit_progress(&app2, 100, "診斷回報已送出");
        }
        r
    })
    .await
    .map_err(|e| describe_worker_failure(&e))?;
    if let Err(e) = &r {
        emit_error(&app, e);
    }
    r
}

#[tauri::command]
async fn submit_issue_report_cmd(
    summary: String,
    cause: String,
    detail: Option<String>,
    idempotency_key: Option<String>,
) -> SubmitIssueReportResult {
    let r = tauri::async_runtime::spawn_blocking(move || submit_issue_report(summary, cause, detail, idempotency_key))
        .await;
    match r {
        Ok(v) => v,
        Err(_) => SubmitIssueReportResult {
            ok: false,
            case_id: None,
            delivery: None,
            error_type: Some("request_failed".into()),
            message: Some("站長聯絡通道暫時離線，請稍後再試，或直接到官方 Discord 告訴我們".into()),
        },
    }
}

/// 刪除目前實例旁所有由工具建立的翻譯套用備份。
#[tauri::command]
async fn delete_apply_backups_cmd(
    app: AppHandle,
    instance_path: String,
    output_dir: Option<String>,
) -> Result<DeleteBackupResult, String> {
    // B5a-2 審查 F2：翻譯或套用中不刪（套用正在寫備份）；設定視窗也會先停用按鈕
    if TRANSLATION_ACTIVE.load(Ordering::Relaxed) {
        return Err("正在翻譯或套用，完成後才能刪除備份。".into());
    }
    let inst = normalize_path_strict(&instance_path)?;
    let result_root = output_dir
        .as_deref()
        .map(normalize_path_strict)
        .transpose()?;
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let result = delete_apply_backups_in(&inst, result_root.as_deref())?;
        emit_log(&app2, "warn", &result.player_summary);
        for failure in &result.failed {
            emit_warn(&app2, &format!("備份刪除失敗：{failure}"));
        }
        Ok::<DeleteBackupResult, String>(result)
    })
    .await
    .map_err(|e| format!("刪除備份工作中斷：{e}"))??;
    Ok(result)
}

/// 套用紀錄損壞時的出口：把壞掉的紀錄改名保留（`.broken-時間戳`），重新開始記錄。
#[tauri::command]
fn reset_apply_record_cmd(instance_path: String) -> Result<String, String> {
    let inst = normalize_path_strict(&instance_path)?;
    let mc = resolve_minecraft_dir(&inst)?;
    let kept = engine::apply_record::reset_record(&mc)?;
    Ok(format!(
        "已重設套用紀錄，之後會重新開始記錄。壞掉的那份已改名保留：\n{}",
        kept.display()
    ))
}

/// 把複製出來（或原位置已永久不在）的遊戲資料夾當成新的整合包：建立新的識別碼與紀錄。
#[tauri::command]
fn fork_apply_instance_cmd(instance_path: String) -> Result<String, String> {
    let inst = normalize_path_strict(&instance_path)?;
    let mc = resolve_minecraft_dir(&inst)?;
    engine::apply_identity::fork_instance(&mc)?;
    Ok("已把這份當成新的模組整合包，之後的套用與移除翻譯只會記在這份自己的紀錄裡。現在可以按「套用到遊戲」。".into())
}

/// B5a-2：這個遊戲資料夾的備份實際放在哪（設定視窗 D-07 列出來）。只讀：不建立資料夾、不寫任何東西。
#[tauri::command]
fn apply_backup_location_cmd(instance_path: String) -> Result<String, String> {
    let inst = normalize_path_strict(&instance_path)?;
    let mc = resolve_minecraft_dir(&inst)?;
    Ok(engine::apply_record::instance_backup_dir(&mc).display().to_string())
}

/// 檢查目前實例／結果位置是否有可還原的工具備份；只讀取，不會修改檔案。
#[tauri::command(async)]
fn has_apply_backups_cmd(
    instance_path: String,
    output_dir: Option<String>,
) -> Result<bool, String> {
    let inst = normalize_path_strict(&instance_path)?;
    let result_root = output_dir
        .as_deref()
        .map(normalize_path_strict)
        .transpose()?;
    has_apply_backups_in(&inst, result_root.as_deref())
}

fn resolve_translation_mode(override_mode: Option<&str>, session_mode: &str) -> TranslationMode {
    if let Some(raw) = override_mode.map(str::trim).filter(|s| !s.is_empty()) {
        TranslationMode::parse(Some(raw))
    } else {
        TranslationMode::parse(Some(session_mode))
    }
}

/// 遊戲內既有的「繁體中文翻譯」zip／資料夾是不是這個整合包產生的。
///
/// 判準：`{pack}.meta.json`（跟 zip／資料夾同層、由 apply_instance 套用時寫入）裡的
/// `modsFingerprint` 是否等於目前這個實例的 `mods_fingerprint`。任何一邊拿不到指紋
/// （沒有標記檔、標記檔壞掉、或現在讀不到 `mods/`）一律回 false——這裡跟 probe_cache_at
/// 的「0 一律不擋」刻意相反，因為誤合併會把不相干整合包的翻譯內容混進來，
/// 錯誤代價比保守略過大得多。
fn existing_pack_matches_current_mods(mc: &Path, pack_path: &Path, instance: &Path) -> bool {
    let stem = pack_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("繁體中文翻譯");
    let meta_path = mc
        .join("resourcepacks")
        .join(format!("{stem}.meta.json"));
    let Ok(text) = fs::read_to_string(&meta_path) else {
        return false;
    };
    let Ok(meta) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let Some(recorded) = meta.get("modsFingerprint").and_then(|v| v.as_u64()) else {
        return false;
    };
    if recorded == 0 {
        return false;
    }
    let live = engine::mods_fingerprint(instance);
    live != 0 && live == recorded
}

/// B4：使用者按了停止——**不丟已翻部分**：把取消旗標收下（之後的寫檔、套用不會被它打斷），
/// 標成「使用者停止」（後面的翻譯步驟不再做），接著照常寫出資源包、存工作階段、裝進遊戲。
/// 再按一次停止才會真的中斷。
fn begin_user_stop_finalize(app: &AppHandle, user_stopped: &mut bool) {
    if !*user_stopped {
        emit_warn(
            app,
            "已停止翻譯：正在寫出已翻好的部分並套用到遊戲（不會再翻新的內容；之後按「接續補完」會從這裡繼續）…",
        );
    }
    *user_stopped = true;
    reset_cancel();
    engine::run_interrupt::stop_by_user(CANCEL_MESSAGE);
}

fn run_one_click(
    app: &AppHandle,
    instance: PathBuf,
    out: PathBuf,
    pack_name: String,
    use_ai: bool,
    keep_results: bool,
    reference_pack: Option<String>,
    target_version: Option<String>,
    translation_mode: Option<String>,
    translation_quality: Option<String>,
    coverage_tier: Option<String>,
    _advanced_unpack: bool,
) -> Result<OneClickResult, String> {
    // B2：每輪開始先清空上一輪的退回紀錄
    engine::begin_guard_run();
    // B4：清掉上一輪的「AI 已停」狀態
    engine::run_interrupt::reset();
    preflight_selected_ai(app, use_ai, "開始掃描整合包")?;
    // 上次沒送成功的社群共享庫貢獻，開工前先補送一次。
    // 這件事本來就會做，但過去完全不出聲，使用者看到「859 條暫存稍後再送」之後
    // 就再也沒有下文，無從判斷到底送出去了沒有。共享庫愈大，所有人免 AI 的
    // 命中率就愈高（這次 99% 免 AI 就是靠它），值得讓使用者看見它在運作。
    let queued = flush_shared_contribute_queue();
    if queued.accepted > 0 {
        emit_log(
            app,
            "info",
            &format!("已把上次暫存的 {} 條譯文送進社群共享庫（讓下次翻譯更少需要 AI）", queued.accepted),
        );
    } else if queued.deferred > 0 {
        emit_log(
            app,
            "info",
            // 講清楚那 N 條是「你已經翻好的」，不然玩家會以為這次少翻了 N 條
            &format!(
                "共享庫暫時連不上，{} 條**你已經翻好的**譯文會下次再上傳分享（這次的翻譯結果不受影響）",
                queued.deferred
            ),
        );
    }
    reset_contribute_tracker();
    emit_progress_stage(app, dev_progress::STAGE_PREP, Some(2), "檢查資料夾…");

    // 這次實際會用的設定，由 run_plan 這唯一一處決定（P0-01）。
    // 舊版在這裡直接寫死三個值、把 UI 傳來的選擇丟掉，只留一行日誌——
    // 於是設定畫面看起來可選、實際行為卻固定，而且沒有任何結構化資料能回給前端顯示。
    let plan = engine::run_plan::resolve(
        engine::run_plan::RunIntent::OneClick,
        &engine::run_plan::RunPlanRequest {
            mode: translation_mode.clone(),
            quality: translation_quality.clone(),
            tier: coverage_tier.clone(),
            advanced_unpack: Some(_advanced_unpack),
        },
    );
    let mode = plan.translation_mode();
    let quality = plan.translation_quality();
    let advanced_unpack = plan.advanced_unpack;
    let tier = plan.coverage_tier();
    let sources: CoverageSourceFlags = tier.sources();
    emit_log(app, "info", &tier.note());
    // 被改掉的欄位逐項說明「你選什麼、實際用什麼、為什麼」，不再只講「已忽略」。
    if let Some(summary) = plan.override_summary() {
        emit_log(app, "info", &summary);
    }
    let validation = validate_instance_path(&instance);
    if !validation.ok {
        let detail = if validation.hints.is_empty() {
            validation.reason.clone()
        } else {
            format!(
                "{}（{}）",
                validation.reason,
                validation.hints.join("；")
            )
        };
        return Err(detail);
    }
    // Minecraft＜1.13 硬擋（年份版 26.x 允許）
    {
        let mc_for_gate = resolve_minecraft_dir(&instance).unwrap_or_else(|_| instance.clone());
        let gated = ensure_minecraft_version_for_translate(
            target_version.as_deref(),
            &mc_for_gate,
        )?;
        emit_log(
            app,
            "info",
            &format!("Minecraft 版本閘門通過：{gated}"),
        );
    }
    emit_log(app, "info", &format!("{}", mode_note(mode, 0)));
    emit_log(app, "info", &format!("翻譯品質：{}", quality.label()));
    emit_log(
        app,
        "info",
        "進階解包：搜尋一併納入進階來源（只寫副本，不改原模組檔）。",
    );
    let mut skipped_by_tier: Vec<String> = Vec::new();
    if !instance.exists() {
        return Err("找不到這個資料夾，請重新選擇或檢查路徑是否正確（可用空白字元）。".into());
    }
    if is_probably_network_path(&instance) {
        emit_warn(
            app,
            "目前路徑看起來是網路磁碟或 UNC 路徑；掃描與套用可能較慢，過程中請不要中斷連線。",
        );
    }
    // 第一次執行時放一份可編輯的術語表範本，讓玩家知道譯名可以自己改
    if let Some(p) = ensure_user_glossary_template() {
        emit_log(
            app,
            "info",
            &format!("想固定某些譯名可編輯：{}", p.display()),
        );
    }
    // 階 3：空間＋寫入權限探針；失敗不開搜尋
    ensure_ready_to_write(&out, MIN_FREE_BYTES)?;
    // 使用者只選根目錄；工具建立 翻譯結果/ 與子目錄
    let layout = ensure_result_layout(&out)?;
    let work = layout.work_root.clone();
    cleanup_transient_work(&work)?;
    dev_progress::start(&work);
    emit_progress_stage(
        app,
        dev_progress::STAGE_PREP,
        Some(3),
        &format!("結果目錄：{}", work.display()),
    );
    dev_progress::mark(&format!("result_dir={}", work.display()));

    let pack_name = if pack_name.is_empty() {
        "繁體中文翻譯".to_string()
    } else {
        pack_name
    };
    let translation_scope = TranslationScope::from_instance(&instance);
    emit_log(
        app,
        "info",
        &format!(
            "共享翻譯分類：{}（pk={}）",
            if translation_scope.is_known() {
                translation_scope.pack_name.as_str()
            } else {
                "未命名整合包"
            },
            translation_scope.pack_key
        ),
    );

    let dict = load_phrase_dict(None);

    // ═══ 階 4：搜尋系統（分析＋理解＋整合→工作圖；進階同意則含進階來源）═══
    {
        dev_progress::enter("search");
        let app_search = app.clone();
        let graph = run_search_pipeline(&instance, advanced_unpack, &mut |pct, msg| {
            emit_progress_stage(&app_search, dev_progress::STAGE_SCAN, Some(pct), msg);
        })?;
        write_search_artifacts(&work, &graph)?;
        emit_log(app, "info", &graph.player_summary);
        if graph.split_polysemy_count > 0 {
            emit_log(
                app,
                "info",
                &format!(
                    "相同用語但意思不同，已分開處理：{} 組。",
                    graph.split_polysemy_count
                ),
            );
        }
        if graph.aligned_count > 0 {
            emit_log(
                app,
                "info",
                &format!("多處出現的同一用語已統一：{} 組。", graph.aligned_count),
            );
        }
        dev_progress::leave("search");
    }

    // ═══ 階段 A：本機掃模組／資源包語言（不 AI）═══
    // JAR 文件複查改延後到 AI 之後（見 sources.jar_documentation），避免擋主翻譯牆鐘。
    dev_progress::enter("jar_scan");
    let app_scan = app.clone();
    let (mut zh, mut en_only, mut provenance, mut report) =
        scan_instance(&instance, &dict, true, true, move |pct, msg| {
            emit_progress_stage(&app_scan, dev_progress::STAGE_SCAN, Some(pct), msg);
        })?;
    // 掃描當下的英文目錄：供收尾貢獻／seed（subtract 後 pending 會少掉已併入鍵）
    let en_catalog = en_only.clone();
    // B2：完整英文原文表存進翻譯結果，補翻／修復／貼回建包時用（不隨待補清單縮減）
    if let Err(e) = engine::source_catalog_save(&work, &engine::snapshot_sources()) {
        emit_warn(app, &format!("英文原文表存檔失敗（之後補翻時會重新讀取遊戲原文）：{e}"));
    }
    // 停止時把當下有效譯文掃尾進共享庫（成功路徑會 disarm）
    let mut stop_share = OnCancelShare {
        active: true,
        app,
        en: &en_catalog as *const _,
        zh: &zh as *const _,
        scope: &translation_scope as *const _,
        provenance: &provenance as *const _,
    };
    if !sources.jar_documentation {
        emit_log(
            app,
            "info",
            "完整度略過：JAR 文件複查（不擋翻譯；盡量完整會在 AI 後補做）。",
        );
    }
    dev_progress::leave("jar_scan");

    emit_progress_stage(app, dev_progress::STAGE_LOCAL, Some(40), "本地整理：詞典…");
    dev_progress::enter("local_merge");
    postprocess_lang_values(&mut zh, &dict);
    // MineMenu 標題翻譯改走額外來源（可 AI）；此處不再只做 unicode 空轉
    let mut minemenu_msg: Option<String> = None;

    // ═══ 階段 A2：本機合併「先前完整繁中參考包」（對齊 CTE2 全翻，不花 AI）═══
    let mut ref_note;
    let user_ref = reference_pack
        .as_ref()
        .map(|s| PathBuf::from(s.trim().trim_matches('"')))
        .filter(|p| p.exists());
    // B2：只有使用者明確指定的參考包算人工譯文（只免長度）；自動搜到的照常完整檢查
    let ref_is_user_choice = user_ref.is_some();
    let ref_path = user_ref.or_else(discover_default_reference);
    if let Some(ref_p) = ref_path {
        emit_progress_stage(
            app,
            dev_progress::STAGE_LOCAL,
            Some(41),
            &format!("本機合併參考翻譯包（不呼叫 AI）：{}", ref_p.display()),
        );
        match load_reference_zh_tw(&ref_p) {
            Ok((ref_zh, files)) => {
                let before = count_map(&zh);
                let filled = engine::source_catalog::merge_reference(&mut zh, &ref_zh, ref_is_user_choice);
                stamp_missing_provenance(&mut provenance, &zh, LangSource::RefPack);
                subtract_covered(&mut en_only, &zh);
                postprocess_lang_values(&mut zh, &dict);
                convert_langmap_s2tw_selective(&mut zh, &|ns, k| {
                    needs_s2tw_key(&provenance, ns, k)
                });
                // B2：參考包人工譯文登記（寫出時只免長度），並把它存進英文原文表
                engine::source_catalog::remember_reference_after_merge(&zh, &ref_zh, &provenance, ref_is_user_choice);
                if let Err(e) = engine::source_catalog_save(&work, &engine::snapshot_sources()) {
                    emit_warn(app, &format!("英文原文表存檔失敗（之後補翻時會重新讀取遊戲原文）：{e}"));
                }
                let after = count_map(&zh);
                ref_note = format!(
                    "參考包 {} 個繁中／簡中語言檔，本機補入 {} 條（{} → {}）",
                    files, filled, before, after
                );
                emit_progress_stage(app, dev_progress::STAGE_LOCAL, Some(42), &ref_note);
                report.keys_zh = after;
            }
            Err(e) => {
                ref_note = format!("參考包未合併：{e}");
                emit_progress_stage(app, dev_progress::STAGE_LOCAL, Some(42), &ref_note);
            }
        }
    } else {
        ref_note = if use_ai {
            "未找到參考包。可在「更多選項→參考翻譯」選本機手翻／社群繁中資源包或 zip（例如 CTE2 全翻 RP），本機合併可大幅減少 AI 用量與假 zh。".into()
        } else {
            "未找到參考包。可在「更多選項→參考翻譯」選本機手翻／社群繁中資源包或 zip，本機合併可補上更多內容。".into()
        };
        emit_progress_stage(app, dev_progress::STAGE_LOCAL, Some(42), &ref_note);
    }

    // 再合併遊戲內既有「繁體中文翻譯」包（若有）— 仍本機
    if let Ok(mc) = resolve_minecraft_dir(&instance) {
        let existing = mc.join("resourcepacks").join("繁體中文翻譯.zip");
        let existing_dir = mc.join("resourcepacks").join("繁體中文翻譯");
        let cand = if existing.is_file() {
            Some(existing)
        } else if existing_dir.is_dir() {
            Some(existing_dir)
        } else {
            None
        };
        // 這個 zip／資料夾只認固定檔名，不代表它一定屬於「這個」整合包：同一個實例
        // 路徑換過完全不同的整合包時，resourcepacks 裡的舊翻譯包不會自動消失。
        // 這裡跟 mods_fingerprint 比對——注意方向刻意跟 probe_cache_at 那個「0 一律
        // 不擋」相反：這裡任何一邊看不到指紋（沒有標記檔／讀不到 mods/）就當作
        // 「無法確認」而跳過合併，因為錯誤方向是「把不相干的舊翻譯塞進新包」，
        // 比「少合併一次本機既有翻譯」嚴重得多，寧可保守。
        let cand = match cand {
            Some(p) if existing_pack_matches_current_mods(&mc, &p, &instance) => Some(p),
            Some(p) => {
                emit_log(
                    app,
                    "info",
                    &format!(
                        "遊戲內既有「{}」與目前整合包內容對不上（或無法確認），略過合併，避免混入其他整合包的翻譯。",
                        p.file_name().and_then(|n| n.to_str()).unwrap_or("繁體中文翻譯")
                    ),
                );
                None
            }
            None => None,
        };
        if let Some(p) = cand {
            if let Ok((ex_zh, _)) = load_reference_zh_tw(&p) {
                let n = merge_fill_missing(&mut zh, &ex_zh);
                if n > 0 {
                    stamp_missing_provenance(&mut provenance, &zh, LangSource::RefPack);
                    subtract_covered(&mut en_only, &zh);
                    ref_note = format!("{ref_note}；遊戲內舊包再補 {n} 條");
                    emit_progress_stage(
                        app,
                        dev_progress::STAGE_LOCAL,
                        Some(43),
                        &format!("本機合併遊戲內舊翻譯包 +{n} 條"),
                    );
                }
            }
        }
    }

    // 接續先前譯文（同機）：精確名／同 version 工具產物／session／遊戲內已套用
    let mut prior_merged = 0usize;
    {
        let pack_version = detect_pack_version(&instance).version;
        let sources = discover_prior_zh_sources(&work, &pack_name, &pack_version, &instance);
        for prior_pack in sources {
            match load_pack_zh(&prior_pack) {
                Ok(prior) => {
                    let n = merge_fill_missing(&mut zh, &prior);
                    if n == 0 {
                        continue;
                    }
                    prior_merged = prior_merged.saturating_add(n);
                    stamp_missing_provenance(&mut provenance, &zh, LangSource::RefPack);
                    subtract_covered(&mut en_only, &zh);
                    postprocess_lang_values(&mut zh, &dict);
                    convert_langmap_s2tw_selective(&mut zh, &|ns, k| {
                        needs_s2tw_key(&provenance, ns, k)
                    });
                    report.keys_zh = count_map(&zh);
                    emit_log(
                        app,
                        "info",
                        &format!(
                            "接續來源：{} 併入 {n} 條（累計 {prior_merged}）",
                            prior_pack.display()
                        ),
                    );
                }
                Err(e) => {
                    emit_warn(
                        app,
                        &format!("接續來源略過 {}：{e}", prior_pack.display()),
                    );
                }
            }
        }
        if prior_merged > 0 {
            emit_progress_ex(
                app,
                Some(43),
                &format!("接續上次：本機累計併入 {prior_merged} 條"),
                ProgressHint {
                    stage: Some(dev_progress::STAGE_LOCAL),
                    metrics: Some(ProgressMetricsPayload {
                        prior: Some(prior_merged as u64),
                        ..Default::default()
                    }),
                    state: Some(STATE_RUNNING),
                    ..Default::default()
                },
            );
        }
    }

    let skipped_complete = if mode == TranslationMode::SkipIfComplete {
        skip_complete_namespaces_with_provenance(&zh, Some(&provenance), &mut en_only, 90)
    } else {
        0
    };
    if skipped_complete > 0 {
        emit_log(app, "info", &mode_note(mode, skipped_complete));
    }

    // AI 只翻字：待補清單
    let mut pending_before = remaining_pending(&en_only, &zh);
    let en_consistency = pending_before.clone();
    // 保留判定前先留一份，才有辦法說明「保留的那些是什麼」。
    let before_filter: Vec<(String, String)> = pending_before
        .iter()
        .flat_map(|(_, m)| m.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
    let skipped_untranslatable = filter_local_untranslatable(&mut pending_before);
    if skipped_untranslatable > 0 {
        // 逐類說明，不再只丟一個數字加一串舉例。
        // 使用者要能分辨「工具在保護我」與「工具漏翻了」。
        let breakdown = engine::eligibility::summarize_keep_reasons(&before_filter, "lang");
        let detail = breakdown
            .iter()
            .map(|(reason, n)| format!("　• {reason}：{n} 項"))
            .collect::<Vec<_>>()
            .join("\n");
        emit_log(
            app,
            "info",
            &format!(
                "原樣保留 {skipped_untranslatable} 項（這些翻了反而會出錯，不算漏翻）：\n{detail}"
            ),
        );
    }
    let pending_before_n = count_map(&pending_before);

    // ── per-candidate outcome ledger ──
    //
    // 這是「已寫入／沿用既有／刻意保留／待人工／可重試」五種終局的逐筆帳本。
    // 舊有的 CoverageLedger 只數階段，回答不了「這一句為什麼沒翻」。
    // 兩條軸刻意分開：shared_sync 不參與任何缺口計算——共享同步失敗
    // 不是翻譯失敗，否則已經翻好的東西會被要求再補翻一次、再花一次 AI 的錢。
    let mut ledger = build_outcome_ledger(&zh, &provenance, &before_filter, &pending_before);
    // 共享同步狀態只寫這一條軸，不影響任何缺口計算。
    //
    // 目前貢獻是「完全隱藏、無勾選、預設開」（見 shared_tm.rs 模組說明），
    // 所以這裡固定為 true。`SkipSharedLookupGuard` 只影響**查找**，不影響貢獻。
    // 總規劃 §6 要把共享治理改成 opt-in——改的時候這裡是唯一要動的地方。
    ledger.mark_sharing(true);
    let reconciliation = ledger.reconcile();
    emit_log(app, "info", &reconciliation.player_summary());
    if !reconciliation.balanced {
        // 對不起來就是帳本本身有問題，不得顯示 completed（總規劃 §5.6）。
        emit_log(
            app,
            "warn",
            &format!(
                "結果對帳不平衡（候選 {} 筆、各終局合計 {} 筆），本次不宣稱完整完成。",
                reconciliation.total_candidates,
                reconciliation.totals.sum()
            ),
        );
    }
    // 兩份：機器讀的逐筆帳本，與人看的明細報告。
    let _ = fs::write(work.join("翻譯結果明細.jsonl"), ledger.to_jsonl());
    let _ = fs::write(work.join("翻譯結果明細.txt"), ledger.to_player_report());

    let _ = save_pending_manifest(&work, &pending_before, pending_before_n, use_ai);
    let _ = save_session(
        &work,
        &TranslateSession {
            version: 1,
            review_pass: 0,
            instance_path: instance.display().to_string(),
            output_dir: work.display().to_string(),
            pack_name: pack_name.clone(),
            pack_path: layout
                .resourcepacks
                .join(format!("{pack_name}.zip"))
                .display()
                .to_string(),
            pending_en: pending_before.clone(),
            pending_count: pending_before_n,
            quality_deferred: HashMap::new(),
            keys_zh: zh.values().map(|m| m.len()).sum(),
            keys_hk_hint: report.keys_from_zh_hk_hint,
            note: if use_ai {
                format!("本地+參考包整理完成，待 AI 僅 {} 條。{} {}", pending_before_n, mode_note(mode, skipped_complete), ref_note)
            } else {
                format!("本地+參考包整理完成，仍待補英文 {} 條。{} {}", pending_before_n, mode_note(mode, skipped_complete), ref_note)
            },
            target_version: target_version.clone(),
            translation_mode: mode.value().into(),
            translation_quality: quality.value().into(),
            coverage_tier: tier.value().into(),
            mods_fingerprint: engine::mods_fingerprint(&instance),
            // 把「這一輪怎麼跑」存進工作階段：中斷續翻時要沿用同樣的選擇。
            // skip_result_folder＝開始時選了「不保留翻譯結果」，只管結果資料夾；
            // 備份不在這裡決定，一律照設定（translate.backupChoice）。
            // backup_before_apply 是舊版工作階段欄位，保留相容、固定為 true（＝照設定）。
            run_preferences: engine::RunPreferences {
                skip_result_folder: !keep_results,
                backup_before_apply: true,
                ai_mode: if use_ai { get_ai_mode() } else { String::new() },
                ..Default::default()
            },
            // 這只是「本地整理完成」的中途快照，AI 與共享庫都還沒跑。
            // 標成 Crashed：真的跑完會在結尾覆寫成 Completed；
            // 中途崩潰或被關掉就停在這裡，計數不可信也不會被拿去講缺漏。
            last_run_outcome: engine::RunOutcome::Crashed,
        },
    );

    let pending_progress = if use_ai {
        format!(
            "本地全部整理完成（模組 {}／資源包 {}／鬆散 {}；中文 {}；待補 {}）",
            report.jars_scanned,
            report.resourcepacks_scanned,
            report.loose_lang_files,
            report.keys_zh,
            pending_before_n
        )
    } else {
        format!(
            "本地全部整理完成（模組 {}／資源包 {}／鬆散 {}；中文 {}；仍缺 {}）",
            report.jars_scanned,
            report.resourcepacks_scanned,
            report.loose_lang_files,
            report.keys_zh,
            pending_before_n
        )
    };
    emit_progress_stage(app, dev_progress::STAGE_LOCAL, Some(41), &pending_progress);

    // 掃描階段的問題：**細節只進錯誤日誌檔，畫面上只講一句話**。
    //
    // 實測站長那一包：4 筆全是「別的語系的語言檔本身格式壞掉」（`lambdynlights`
    // 的 zh_tw、某個 mod 的空 en_us…），對繁中翻譯完全沒有影響，但舊版用
    // 【警告】＋四行【錯誤】列出來，看起來像翻譯出了大事。
    // 玩家在意的只有「這會不會影響我的翻譯」——不會，那就用一句話講完。
    let mut error_lines: Vec<String> = Vec::new();
    if !report.errors.is_empty() {
        emit_log(
            app,
            "info",
            &format!(
                "有 {} 個語言檔本身格式壞掉，已跳過（不影響這次翻譯；細節寫在「翻譯錯誤日誌.txt」）",
                report.errors.len()
            ),
        );
        for (i, e) in report.errors.iter().enumerate() {
            // 只寫檔案，不往畫面上丟
            error_lines.push(format!("掃描問題 [{}/{}]：{}", i + 1, report.errors.len(), e));
        }
    }

    // ═══ 階段 B：補譯（術語表 → 翻譯記憶 → AI）═══
    // 前兩層不需要網路，所以沒勾 AI 也要跑：玩家至少拿得到官方譯名與先前翻過的內容。
    dev_progress::leave("local_merge");
    let mut ai_filled = 0usize;
    let mut glossary_hits = 0usize;
    let mut tm_hits = 0usize;
    let mut shared_hits = 0usize;
    let mut shared_glossary_hits = 0usize;
    let mut quality_deferred: LangMap = HashMap::new();
    // B4：AI 沒回應的（跟品質沒過分開；接續補完一定會再送）
    let mut no_answer: LangMap = HashMap::new();
    // B4：使用者按了停止——寫出已翻好的部分並裝進遊戲，後面的翻譯步驟不做
    let mut user_stopped = false;
    let mut ai_note = String::new();
    let mut ai_usage_note = String::new();
    let mut langmap_stage: Option<engine::StageEntry> = None;
    {
        if pending_before_n > 0 {
            let seeded_early = seed_tm_from_langmaps(&en_catalog, &zh);
            if seeded_early > 0 {
                emit_log(
                    app,
                    "info",
                    &format!("補譯前：已把本包已有譯文寫入翻譯記憶 {seeded_early} 條"),
                );
            }
            dev_progress::enter("ai_fill");
            let app_ai = app.clone();
            match fill_missing_with_mode(&mut zh, &pending_before, use_ai, mode == TranslationMode::Force, quality, Some(&translation_scope), move |pct, msg| {
                emit_progress_stage(
                    &app_ai,
                    dev_progress::STAGE_TRANSLATE,
                    Some(map_stage_progress(55, 20, pct)),
                    msg,
                );
                // 進度字可能含「批失敗」計數，不得當成真正錯誤刷日誌
            }) {
                Ok(r) => {
                    // 語言表也要進帳本：舊版只有「額外來源」進帳，
                    // 所以語言表補譯失敗時結尾照樣說「完成」。
                    let mut entry =
                        engine::StageEntry::ok("語言表", pending_before_n, r.filled);
                    if let Some(reason) = &r.ai_unavailable {
                        entry.reason = Some(
                            reason.lines().next().unwrap_or("AI 不可用").to_string(),
                        );
                    }
                    langmap_stage = Some(entry);
                    merge_pending(&mut quality_deferred, &r.quality_deferred);
                    merge_pending(&mut no_answer, &r.no_answer);
                    if r.stopped_by_user {
                        user_stopped = true;
                    } else if let Some(reason) = &r.ai_unavailable {
                        emit_warn(
                            app,
                            &format!(
                                "AI 中途停下：{}。已翻好的都保留，這一輪後面的步驟只用免費資料（不再空轉重試）。",
                                reason.lines().next().unwrap_or("AI 不可用")
                            ),
                        );
                    }
                    ai_filled = r.filled;
                    glossary_hits = r.glossary_hits;
                    tm_hits = r.tm_hits;
                    shared_hits = r.shared_hits;
                    shared_glossary_hits = r.shared_glossary_hits;
                    ai_note = if use_ai {
                        r.note()
                    } else {
                        format!("本機補譯 {} 條；其餘缺漏保留原文", r.filled)
                    };
                    ai_usage_note = r.usage_note().unwrap_or_default();
                    emit_log(app, "info", &format!("補譯結束：{ai_note}"));
                    let deferred_count = count_map(&r.quality_deferred);
                    if deferred_count > 0 {
                        emit_log(
                            app,
                            "info",
                            &format!("品質暫緩：{deferred_count} 條，本次不重送"),
                        );
                    }
                    let mut fill_metrics = metrics_from_ai_fill(&r);
                    if prior_merged > 0 {
                        fill_metrics.prior = Some(prior_merged as u64);
                    }
                    emit_progress_ex(
                        app,
                        Some(map_stage_progress(55, 20, 100)),
                        &format!("補譯結束：{ai_note}"),
                        ProgressHint {
                            stage: Some(dev_progress::STAGE_TRANSLATE),
                            metrics: Some(fill_metrics),
                            state: Some(STATE_RUNNING),
                            ..Default::default()
                        },
                    );
                    for note in &r.notes {
                        if note.contains("批失敗摘要") || note.contains("批失敗（已去重") {
                            error_lines.push(note.clone());
                        }
                        if note.contains("品質未過") {
                            error_lines.push(note.clone());
                        }
                        if note.contains("提前結束") || note.contains("已保留已成功譯文") {
                            emit_warn(app, note);
                        }
                    }
                    if r.rejected > 0 {
                        let line = format!(
                            "有 {} 條譯文的 %s／§ 等格式符號被 AI 破壞，已退回英文原文（保護遊戲不出錯）",
                            r.rejected
                        );
                        emit_warn(app, &line);
                        error_lines.push(line);
                    }
                }
                // B4：失敗、停止都不再 `return Err` 丟掉整輪——資料層與已翻好的都在 zh 裡，
                // 照樣往下寫出、裝進遊戲，並存工作階段讓「接續補完」從這裡繼續。
                Err(e) => {
                    let line = format!("AI 翻譯中斷：{e}");
                    error_lines.push(line.clone());
                    langmap_stage = Some(engine::StageEntry::failed_unknown_count(
                        "語言表",
                        e.lines().next().unwrap_or("AI 翻譯中斷").to_string(),
                    ));
                    if looks_like_cancel_message(&e) {
                        user_stopped = true;
                    } else {
                        emit_error(app, &line);
                        engine::run_interrupt::halt_ai(&e);
                    }
                }
            }
            postprocess_lang_values(&mut zh, &dict);
            // AI 結果標記來源後只轉需 s2tw 的條目（避免重傷原生 zh_tw）
            stamp_ai_filled(&mut provenance, &zh, &pending_before);
            emit_progress_stage(
                app,
                dev_progress::STAGE_TRANSLATE,
                Some(map_stage_progress(55, 20, 100)),
                "正在把補譯結果轉成台灣正體…",
            );
            convert_langmap_s2tw_selective(&mut zh, &|ns, k| needs_s2tw_key(&provenance, ns, k));
            postprocess_lang_values(&mut zh, &dict);
            report.keys_zh = zh.values().map(|m| m.len()).sum();
            report.keys_need_ai = pending_before_n.saturating_sub(ai_filled);
            dev_progress::leave("ai_fill");
            dev_progress::mark(&format!(
                "ai_filled={ai_filled} glossary={glossary_hits} tm={tm_hits} shared={shared_hits}"
            ));
            if user_stopped || is_cancelled() {
                begin_user_stop_finalize(app, &mut user_stopped);
            }
        } else {
            emit_progress_stage(
                app,
                dev_progress::STAGE_TRANSLATE,
                Some(map_stage_progress(55, 20, 100)),
                "沒有需要補譯的文字",
            );
        }
        if !use_ai {
            emit_log(
                app,
                "info",
                "未勾選 AI：只用內建術語表與翻譯記憶補，其餘缺漏保留原文",
            );
        }
    }

    // JAR 文件複查（耗時）：延後到 AI 主翻譯之後，不擋 8176 句牆鐘
    if sources.jar_documentation && !user_stopped {
        check_cancelled()?;
        emit_progress_stage(
            app,
            dev_progress::STAGE_EXTRAS,
            Some(map_stage_progress(74, 1, 0)),
            "JAR 文件複查（翻譯後補做，不擋主翻譯）…",
        );
        match extract_jar_documentation(&instance, &work) {
            Ok(jar_docs) => emit_log(
                app,
                "info",
                &format!(
                    "JAR 文件複查：{} 個 JAR、{} 個文字文件、{} 個 class 文字線索，寫入 {} 個檔案。",
                    jar_docs.jars_scanned,
                    jar_docs.text_entries,
                    jar_docs.class_files_inspected,
                    jar_docs.files_written
                ),
            ),
            Err(error) => emit_warn(app, &format!("JAR 文件複查略過：{error}")),
        }
    }

    // 最終只對需 s2tw 的來源再檢查（原生台繁不再整包重轉）
    check_cancelled()?;
    emit_progress_stage(
        app,
        dev_progress::STAGE_PACKAGE,
        Some(map_stage_progress(75, 7, 0)),
        "最終台灣正體檢查…",
    );
    convert_langmap_s2tw_selective(&mut zh, &|ns, k| needs_s2tw_key(&provenance, ns, k));

    // ═══ 階段 C：寫出資源包（進度 75–82）═══
    emit_progress_stage(
        app,
        dev_progress::STAGE_PACKAGE,
        Some(map_stage_progress(75, 7, 5)),
        "正在建立翻譯檔與資源包…",
    );
    dev_progress::enter("pack_out");
    let jar_translation = rewrite_jars_and_log(&app, &instance, &work, &zh, &en_only)?;
    let jar_patchouli_note = if user_stopped {
        "已停止：JAR 內 Patchouli 這一輪不處理（接續補完時再做）".to_string()
    } else if sources.jar_patchouli {
        match translate_jar_patchouli(&instance, &work, use_ai, Some(&translation_scope), |pct, msg| {
            emit_progress_stage(
                app,
                dev_progress::STAGE_EXTRAS,
                Some(map_stage_progress(82, 6, pct)),
                msg,
            );
        }) {
            Ok(r) => r.note,
            Err(e) => {
                let note = format!("JAR 內 Patchouli 略過／失敗：{e}");
                error_lines.push(note.clone());
                note
            }
        }
    } else {
        let note = "完整度略過：JAR Patchouli".to_string();
        skipped_by_tier.push(note.clone());
        emit_log(app, "info", &note);
        note
    };
    if user_stopped {
        emit_log(app, "info", "已停止：JAR 顯示文字這一輪不處理（接續補完時再做）");
    } else if sources.jar_display {
        match translate_jar_display_texts(&instance, &work, use_ai, Some(&translation_scope), |pct, msg| {
            emit_progress_stage(
                app,
                dev_progress::STAGE_EXTRAS,
                Some(map_stage_progress(88, 6, pct)),
                msg,
            );
        }) {
            Ok(r) => emit_log(app, "info", &r.note),
            Err(e) => {
                let note = format!("JAR 顯示文字略過／失敗：{e}");
                error_lines.push(note.clone());
                emit_warn(app, &note);
            }
        }
    } else {
        let note = "完整度略過：JAR 顯示文字".to_string();
        skipped_by_tier.push(note.clone());
        emit_log(app, "info", &note);
    }

    let mc_for_fmt = resolve_minecraft_dir(&instance).unwrap_or_else(|_| instance.clone());
    // 模組 JAR 自帶的 zh_tw：資源包只輸出工具補的條目，不整份抄一次模組自帶的翻譯
    let bundled_zh = engine::collect_mod_zh_tw(&mc_for_fmt);
    // 使用者指定版本 → 用它；否則偵測。用來決定 pack.mcmeta 相容宣告。
    let resolved_version = target_version
        .clone()
        .or_else(|| detect_minecraft_version(&mc_for_fmt));
    let pack_format = resolved_version
        .as_deref()
        .and_then(pack_format_for_version)
        .unwrap_or_else(|| detect_pack_format(&mc_for_fmt));
    if let Some(v) = &resolved_version {
        emit_log(
            app,
            "info",
            &format!(
                "目標版本：{v}{}",
                if target_version.is_some() {
                    "（你指定的）"
                } else {
                    "（自動偵測）"
                }
            ),
        );
    }
    emit_progress_stage(
        app,
        dev_progress::STAGE_PACKAGE,
        Some(map_stage_progress(75, 7, 100)),
        "正在寫出資源包 zip…",
    );
    let mut built = engine::build_resource_pack_skipping_bundled(
        &zh,
        &BuildOptions {
            pack_folder_name: pack_name.clone(),
            pack_description: "台灣用語繁體中文翻譯資源包".into(),
            output_dir: work.display().to_string(),
            pack_format,
            target_version: resolved_version.clone(),
        },
        &bundled_zh,
    )?;
    emit_pruned_tool_pack_log(app, &built.pruned_tool_packs);

    // ═══ 階段 D–E4：獨立額外來源（進度 82–97）═══
    // use_ai=true 時維持序列，避免多個來源同時打 AI；use_ai=false 時最多 3 路並行。
    let mc_for_extra = resolve_minecraft_dir(&instance).unwrap_or_else(|_| instance.clone());
    let extra_summary = if user_stopped {
        // 已停止：額外來源這一輪不處理（它們上一輪確認過的產出照樣有效）
        emit_log(app, "info", "已停止：任務書、覆寫文字等額外來源這一輪不處理（接續補完時再做）");
        ExtraSourceSummary::default()
    } else {
        run_extra_sources(
            app,
            &mc_for_extra,
            &work,
            use_ai,
            Some(&translation_scope),
            sources,
            82,
            15,
        )
    };
    if extra_summary.cancelled || is_cancelled() {
        // B4：額外來源途中按停止：已寫出的保留，接著寫出主資源包並裝進遊戲
        begin_user_stop_finalize(app, &mut user_stopped);
    }
    for note in &extra_summary.skipped {
        skipped_by_tier.push(note.clone());
        emit_log(app, "info", note);
    }
    let mut quest_note = extra_summary.combined_note();
    // 階段帳本要留到結尾判斷「能不能說完成」；errors 會被 extend 消耗掉，
    // 所以先把帳本取出來。
    let mut stage_ledger = extra_summary.ledger.clone();
    if let Some(entry) = langmap_stage.take() {
        stage_ledger.record(entry);
    }
    error_lines.extend(extra_summary.errors);
    if let Some(note) = extra_summary
        .notes
        .iter()
        .find(|n| n.contains("快捷選單"))
    {
        minemenu_msg = Some(note.clone());
    }
    if !ai_note.is_empty() {
        quest_note = if quest_note.is_empty() {
            ai_note.clone()
        } else {
            format!("{quest_note}；{ai_note}")
        };
    }
    if !jar_patchouli_note.is_empty() {
        quest_note = if quest_note.is_empty() {
            jar_patchouli_note.clone()
        } else {
            format!("{quest_note}；{jar_patchouli_note}")
        };
    }

    // ═══ 步驟 4：補充仍缺（語言表＋extras，非 Force；已譯不重送）═══
    check_cancelled()?;
    let seeded = seed_tm_from_langmaps(&en_catalog, &zh);
    if seeded > 0 {
        emit_log(
            app,
            "info",
            &format!("補充：已把 {seeded} 條語言表譯文寫入共用字串表（供覆寫／任務重用）"),
        );
    }
    let mut remaining = remaining_pending(&en_only, &zh);
    let deferred_in_same_run = filter_quality_deferred(&mut remaining, &quality_deferred);
    if deferred_in_same_run > 0 {
        emit_log(
            app,
            "info",
            &format!(
                "補充：已排除本輪品質暫緩的 {deferred_in_same_run} 條，避免重複送出"
            ),
        );
    }
    let rem_n = if user_stopped { 0 } else { count_map(&remaining) };
    let mut supplement_filled = 0usize;
    if rem_n > 0 {
        emit_progress_ex(
            app,
            Some(map_stage_progress(88, 6, 0)),
            &format!("補充：語言表仍缺 {rem_n} 條，只補缺…"),
            ProgressHint {
                stage: Some(dev_progress::STAGE_TRANSLATE),
                step: Some(4),
                step_total: Some(dev_progress::UI_STEP_TOTAL),
                state: Some(STATE_RUNNING),
                substage: Some("補語言檔"),
                substage_index: Some(1),
                substage_total: Some(SUPPLEMENT_SUBSTAGES),
                ..Default::default()
            },
        );
        let app_sup = app.clone();
        match fill_missing_with_mode(
            &mut zh,
            &remaining,
            use_ai,
            false,
            quality,
            Some(&translation_scope),
            move |pct, msg| {
                emit_progress_ex(
                    &app_sup,
                    Some(map_stage_progress(88, 6, pct)),
                    &format!("補充：{msg}"),
                    ProgressHint {
                        stage: Some(dev_progress::STAGE_TRANSLATE),
                        step: Some(4),
                        step_total: Some(dev_progress::UI_STEP_TOTAL),
                        state: Some(STATE_RUNNING),
                        substage: Some("補語言檔"),
                        substage_index: Some(1),
                        substage_total: Some(SUPPLEMENT_SUBSTAGES),
                        ..Default::default()
                    },
                );
                // 進度字可能含「批失敗」計數，不得當成真正錯誤刷日誌
            },
        ) {
            Ok(r) => {
                merge_pending(&mut quality_deferred, &r.quality_deferred);
                merge_pending(&mut no_answer, &r.no_answer);
                if r.stopped_by_user {
                    begin_user_stop_finalize(app, &mut user_stopped);
                }
                supplement_filled = r.filled;
                ai_filled = ai_filled.saturating_add(r.filled);
                glossary_hits = glossary_hits.saturating_add(r.glossary_hits);
                tm_hits = tm_hits.saturating_add(r.tm_hits);
                shared_hits = shared_hits.saturating_add(r.shared_hits);
                shared_glossary_hits = shared_glossary_hits.saturating_add(r.shared_glossary_hits);
                emit_log(
                    app,
                    "info",
                    &format!("補充：語言表再補 {} 條（{}）", r.filled, r.note()),
                );
                let deferred_count = count_map(&r.quality_deferred);
                if deferred_count > 0 {
                    emit_log(
                        app,
                        "info",
                        &format!("品質暫緩：{deferred_count} 條，本次不重送"),
                    );
                }
                if r.rejected > 0 {
                    let line = format!(
                        "補充：有 {} 條譯文佔位符不符已退回原文",
                        r.rejected
                    );
                    emit_warn(app, &line);
                    error_lines.push(line);
                }
                postprocess_lang_values(&mut zh, &dict);
                stamp_ai_filled(&mut provenance, &zh, &en_only);
                convert_langmap_s2tw_selective(&mut zh, &|ns, k| needs_s2tw_key(&provenance, ns, k));
                postprocess_lang_values(&mut zh, &dict);
                let _ = seed_tm_from_langmaps(&en_catalog, &zh);
                if supplement_filled > 0 {
                    emit_progress_stage(
                        app,
                        dev_progress::STAGE_PACKAGE,
                        Some(map_stage_progress(88, 6, 70)),
                        "補充：重建資源包與 JAR 副本…",
                    );
                    let _ = rewrite_jars_and_log(&app, &instance, &work, &zh, &en_only)?;
                } else {
                    emit_log(
                        app,
                        "info",
                        "補充：語言表無新增譯文，略過第二次 JAR 翻譯副本重建。",
                    );
                }
            }
            Err(e) => {
                let line = format!("補充：語言表補譯失敗：{e}");
                if looks_like_cancel_message(&e) {
                    // B4：停止不丟已翻部分，照樣寫出並裝進遊戲
                    error_lines.push(line);
                    begin_user_stop_finalize(app, &mut user_stopped);
                } else {
                    emit_error(app, &line);
                    error_lines.push(line);
                }
            }
        }
    } else {
        emit_progress_ex(
            app,
            Some(map_stage_progress(88, 6, 40)),
            "補充：語言表無待補",
            ProgressHint {
                stage: Some(dev_progress::STAGE_TRANSLATE),
                step: Some(4),
                step_total: Some(dev_progress::UI_STEP_TOTAL),
                state: Some(STATE_RUNNING),
                ..Default::default()
            },
        );
    }

    // ═══ 步驟 4.5：自動重試品質暫緩（同一輪內完成，不必使用者再按一次）═══
    //
    // 過去暫緩的句子只有在使用者自己勾「重新翻譯缺漏」才會重試，否則永遠躺著。
    // 但暫緩多半是當下 AI 狀態不好造成的，隔一批通常就過了——要求使用者去勾一個
    // 他不知道意義的選項，只會讓人以為工具沒做完。這裡在同一輪內自動再試一次，
    // 失敗的仍然留在暫緩（不會無限重試），並設上限避免大整合包爆量。
    //
    // B4：上限改成按比例（這一輪送 AI 的量的 25%，至少 50、最多 2000），取代寫死的 400：
    // 小包 400 等於全部重試，大包 400 卻只試到一角。AI 已停（額度、停止）就不重試——那只會空轉。
    let auto_retry_cap =
        engine::retry_policy::proportional_cap(pending_before_n.max(count_map(&quality_deferred)), 25, 50, 2000);
    let deferred_total = count_map(&quality_deferred);
    let ai_halted = engine::run_interrupt::current().is_some();
    if ai_halted && deferred_total > 0 {
        emit_log(app, "info", &format!("補強：AI 這一輪已停下，品質暫緩的 {deferred_total} 條留待接續補完"));
    }
    if use_ai && deferred_total > 0 && !user_stopped && !ai_halted {
        let retry_set = take_capped_langmap(&quality_deferred, auto_retry_cap);
        let retry_n = count_map(&retry_set);
        emit_progress_ex(
            app,
            Some(map_stage_progress(94, 3, 0)),
            &format!("補強：自動重試先前暫緩的 {retry_n} 條…"),
            ProgressHint {
                stage: Some(dev_progress::STAGE_TRANSLATE),
                step: Some(4),
                step_total: Some(dev_progress::UI_STEP_TOTAL),
                state: Some(STATE_RUNNING),
                substage: Some("重試沒通過的句子"),
                substage_index: Some(SUPPLEMENT_SUBSTAGES),
                substage_total: Some(SUPPLEMENT_SUBSTAGES),
                ..Default::default()
            },
        );
        emit_log(
            app,
            "info",
            &if deferred_total > retry_n {
                format!(
                    "補強：自動重試品質暫緩 {retry_n} 條（本輪上限；另有 {} 條留待下次）",
                    deferred_total - retry_n
                )
            } else {
                format!("補強：自動重試品質暫緩 {retry_n} 條")
            },
        );
        let app_retry = app.clone();
        match fill_missing_with_mode(
            &mut zh,
            &retry_set,
            use_ai,
            false,
            quality,
            Some(&translation_scope),
            move |pct, msg| {
                emit_progress_ex(
                    &app_retry,
                    Some(map_stage_progress(94, 3, pct)),
                    &format!("補強：{msg}"),
                    ProgressHint {
                        stage: Some(dev_progress::STAGE_TRANSLATE),
                        step: Some(4),
                        step_total: Some(dev_progress::UI_STEP_TOTAL),
                        state: Some(STATE_RUNNING),
                        ..Default::default()
                    },
                );
            },
        ) {
            Ok(r) => {
                ai_filled = ai_filled.saturating_add(r.filled);
                glossary_hits = glossary_hits.saturating_add(r.glossary_hits);
                tm_hits = tm_hits.saturating_add(r.tm_hits);
                shared_hits = shared_hits.saturating_add(r.shared_hits);
                shared_glossary_hits = shared_glossary_hits.saturating_add(r.shared_glossary_hits);
                if r.filled > 0 {
                    supplement_filled = supplement_filled.saturating_add(r.filled);
                    postprocess_lang_values(&mut zh, &dict);
                    stamp_ai_filled(&mut provenance, &zh, &en_only);
                    convert_langmap_s2tw_selective(&mut zh, &|ns, k| {
                        needs_s2tw_key(&provenance, ns, k)
                    });
                    postprocess_lang_values(&mut zh, &dict);
                    let _ = seed_tm_from_langmaps(&en_catalog, &zh);
                    emit_log(
                        app,
                        "info",
                        &format!("補強：暫緩項目再補回 {} 條", r.filled),
                    );
                    emit_progress_stage(
                        app,
                        dev_progress::STAGE_PACKAGE,
                        Some(map_stage_progress(94, 3, 80)),
                        "補強：重建 JAR 翻譯副本…",
                    );
                    let _ = rewrite_jars_and_log(&app, &instance, &work, &zh, &en_only)?;
                } else {
                    emit_log(app, "info", "補強：暫緩項目這次仍未通過品質檢查，維持原文。");
                }
                // 這次過關的從暫緩清單移除；沒過的留著，下次再說
                prune_quality_deferred(&mut quality_deferred, &zh);
            }
            Err(e) => {
                if looks_like_cancel_message(&e) {
                    // B4：停止不丟已翻部分，照樣寫出並裝進遊戲
                    error_lines.push(format!("補強：自動重試中止：{e}"));
                    begin_user_stop_finalize(app, &mut user_stopped);
                }
                // 自動重試失敗不該讓整輪翻譯失敗——原本的成果都還在
                emit_log(app, "warn", &format!("補強：自動重試未完成（{e}），不影響已完成的翻譯。"));
            }
        }
    }

    emit_progress_stage(
        app,
        dev_progress::STAGE_EXTRAS,
        Some(97),
        "補充來源已完成，即將重建資源包並套用…",
    );

    // 步驟 4 後重建 zip（含補充寫入）
    if rem_n > 0 || supplement_filled > 0 {
        built = engine::build_resource_pack_skipping_bundled(
            &zh,
            &BuildOptions {
                pack_folder_name: pack_name.clone(),
                pack_description: "台灣用語繁體中文翻譯資源包".into(),
                output_dir: work.display().to_string(),
                pack_format,
                target_version: resolved_version.clone(),
            },
            &bundled_zh,
        )?;
        emit_pruned_tool_pack_log(app, &built.pruned_tool_packs);
    }
    report.keys_zh = built.keys_total;

    if !error_lines.is_empty() {
        append_error_file(&work, &error_lines);
        emit_warn(
            app,
            &format!(
                "共記錄 {} 筆錯誤／警告，已寫入：{}",
                error_lines.len(),
                work.join("翻譯錯誤日誌.txt").display()
            ),
        );
    }

    let pending = remaining_pending(&en_only, &zh);
    let pending_count = count_map(&pending);
    // 先算好「補得動的缺口」——下面 pending 會被移進工作階段
    let gaps = engine::count_gaps(&pending);
    prune_quality_deferred(&mut quality_deferred, &zh);
    let quality_deferred_for_view = quality_deferred.clone();
    stop_share.disarm();
    emit_progress_stage(
        app,
        dev_progress::STAGE_PACKAGE,
        Some(96),
        "共享庫掃尾…",
    );
    {
        // 帶上 provenance：本機參考包合併進來的內容不上傳（站長選定）
        let contrib =
            contribute_lang_maps(&en_catalog, &zh, &translation_scope, Some(&provenance));
        if contrib.attempted > 0 || contrib.failed || contrib.deferred > 0 {
            emit_log(
                app,
                "info",
                &format!(
                    "共享庫掃尾：accepted={}／衝突 {}／送出 {}{}{}（可供其他裝置／玩家重用）",
                    contrib.accepted,
                    contrib.conflicts,
                    contrib.attempted,
                    if contrib.deferred > 0 {
                        format!("；暫緩 {} 條", contrib.deferred)
                    } else {
                        String::new()
                    },
                    if contrib.failed {
                        "（失敗已排程重試）"
                    } else {
                        ""
                    }
                ),
            );
        }
        if let Some(note) =
            contribute_shared_glossary_from_langmaps(&en_catalog, &zh, &translation_scope)
        {
            emit_log(app, "info", &format!("{note}（可供其他裝置／玩家重用）"));
        }
    }
    emit_progress_stage(
        app,
        dev_progress::STAGE_PACKAGE,
        Some(96),
        "共享庫掃尾結束",
    );
    if sources.write_gap_summary {
        match write_gap_summary_file(&work, &pending, 120) {
            Ok(p) => emit_log(
                app,
                "info",
                &format!("已寫待補缺口摘要（樣本）：{}", p.display()),
            ),
            Err(e) => emit_warn(app, &format!("待補缺口摘要寫入失敗：{e}")),
        }
        // 完整的一張表（不是樣本），可直接貼給線上 AI 翻完再匯回來。
        // 使用者反映舊做法只能一個一個開檔案複製「有點慘」。
        if count_map(&pending) > 0 {
            match engine::write_failed_items_csv(&work, &pending, "尚未翻譯或品質未通過") {
                Ok(p) => emit_log(
                    app,
                    "info",
                    &format!(
                        "已寫失敗項目表（可用「複製沒翻到的」整批處理）：{}",
                        p.display()
                    ),
                ),
                Err(e) => emit_warn(app, &format!("失敗項目表寫入失敗：{e}")),
            }
        }
    }
    let _ = save_session(
        &work,
        &TranslateSession {
            version: 1,
            review_pass: 0,
            instance_path: instance.display().to_string(),
            output_dir: work.display().to_string(),
            pack_name: pack_name.clone(),
            pack_path: built.pack_path.clone(),
            pending_en: pending,
            pending_count,
            quality_deferred,
            keys_zh: built.keys_total,
            keys_hk_hint: report.keys_from_zh_hk_hint,
            note: format!(
                "完整流程後產生。可按「接續補完」續翻。剩餘約 {} 條。{}",
                pending_count, quest_note
            ),
            target_version: resolved_version.clone(),
            translation_mode: mode.value().into(),
            translation_quality: quality.value().into(),
            coverage_tier: tier.value().into(),
            mods_fingerprint: engine::mods_fingerprint(&instance),
            // 把「這一輪怎麼跑」存進工作階段：中斷續翻時要沿用同樣的選擇。
            // skip_result_folder＝開始時選了「不保留翻譯結果」，只管結果資料夾；
            // 備份不在這裡決定，一律照設定（translate.backupChoice）。
            // backup_before_apply 是舊版工作階段欄位，保留相容、固定為 true（＝照設定）。
            run_preferences: engine::RunPreferences {
                skip_result_folder: !keep_results,
                backup_before_apply: true,
                ai_mode: if use_ai { get_ai_mode() } else { String::new() },
                ..Default::default()
            },
            // 走到這裡代表整條流程真的跑完了，計數是新鮮的——
            // 只有這種狀態的數字可以拿來對使用者講「還缺幾條」。
            // B4：使用者按停止的記成 Aborted（接續補完從 pending_en 繼續）。
            last_run_outcome: if user_stopped {
                engine::RunOutcome::Aborted
            } else {
                engine::RunOutcome::Completed
            },
        },
    );

    let mut coverage_unsupported = report.errors.clone();
    coverage_unsupported.extend(skipped_by_tier.iter().cloned());
    coverage_unsupported.push(
        "Essential／部分客戶端 UI：class 硬編碼或快取文字無法以資源包翻譯（產品紅線：不改 class）。"
            .into(),
    );
    coverage_unsupported.push(
        "MIDI Controllers、Drop Rate 等：若無 lang／可覆寫設定鍵，介面英文屬硬編碼範圍。"
            .into(),
    );

    // 社群誠實原則：寫覆蓋範圍說明
    let _ = write_coverage_report(
        &layout,
        &CoverageStats {
            keys_zh: built.keys_total,
            keys_pending: pending_count,
            keys_tw_playable: report.keys_tw_playable,
            keys_hk_hint: report.keys_from_zh_hk_hint,
            ai_filled,
            ai_enabled: use_ai,
            jars_scanned: report.jars_scanned,
            jars_rewritten: jar_translation.jars_rewritten,
            jar_lang_files: jar_translation.lang_files_written,
            jar_errors: jar_translation.errors.len(),
            quests_note: quest_note.clone(),
            ref_note: ref_note.clone(),
            pack_path: built.pack_path.clone(),
            pack_format,
            source_notes: {
                let mut notes = vec![
                    format!("完整度：{}（{}）", tier.label(), tier.value()),
                    format!("語言表：掃描 {} 個 JAR、{} 個資源包／鬆散來源", report.jars_scanned, report.resourcepacks_scanned + report.loose_lang_files),
                    format!("掃描快取：本次重用 {} 個未變語言檔", report.scan_cache_hits),
                    format!("JAR 翻譯副本：重建 {} 個、寫入 {} 個語言檔", jar_translation.jars_rewritten, jar_translation.lang_files_written),
                    format!(
                        "命中拆分：接續 {}／本機術語 {}／共享術語 {}／共享庫 {}／翻譯記憶 {}",
                        prior_merged,
                        glossary_hits,
                        shared_glossary_hits,
                        shared_hits,
                        tm_hits
                    ),
                    quest_note.clone(),
                ];
                if !ai_usage_note.is_empty() {
                    notes.push(ai_usage_note.clone());
                }
                notes.extend(skipped_by_tier.iter().cloned());
                notes
            },
            unsupported: coverage_unsupported,
            glossary_hits,
            tm_hits,
            shared_hits,
            shared_glossary_hits,
            prior_merged,
            coverage_tier: tier.value().into(),
        },
    );
    emit_progress_ex(
        app,
        Some(97),
        &format!("【仍待譯】約 {pending_count} 條"),
        ProgressHint {
            stage: Some(dev_progress::STAGE_PACKAGE),
            metrics: Some(ProgressMetricsPayload {
                pack_pending: Some(pending_count as u64),
                prior: if prior_merged > 0 {
                    Some(prior_merged as u64)
                } else {
                    None
                },
                glossary: Some(glossary_hits as u64),
                tm: Some(tm_hits as u64),
                shared: Some((shared_hits + shared_glossary_hits) as u64),
                ai: Some(ai_filled as u64),
                ..Default::default()
            }),
            state: Some(STATE_RUNNING),
            ..Default::default()
        },
    );
    if let Some(path) = write_consistency_hints(&layout, &en_consistency, &zh) {
        emit_log(
            app,
            "info",
            &format!(
                "已寫用詞不一致提示（僅供校對）：{}；建議檔：{}",
                path.display(),
                path.with_file_name("用詞不一致建議.json").display()
            ),
        );
    }

    let apply_progress = "正在把翻譯套用到遊戲（備份照你的設定）…";
    emit_progress_stage(
        app,
        dev_progress::STAGE_APPLY,
        Some(map_stage_progress(97, 3, 20)),
        apply_progress,
    );
    emit_log(app, "info", "套用前再確認寫入權限；若遊戲開著請先關閉。");
    dev_progress::leave("pack_out");
    dev_progress::enter("apply");
    probe_apply_targets(&instance)?;
    let applied = apply_after_run(app, &instance, &work, &pack_name)?;
    // 套用完才清空資料夾：套用要從 config／minemenu 讀來源，清早了會少複製東西。
    // 這裡只刪「整個流程跑完仍然一個檔案都沒有」的目錄，避免使用者看到空資料夾
    // 以為「這裡本來該有東西卻沒產出」。
    let pruned_dirs = prune_empty_result_dirs(&work);
    if !pruned_dirs.is_empty() {
        emit_log(
            app,
            "info",
            &format!(
                "已移除沒有內容的資料夾：{}（這個整合包沒有對應的可翻內容，屬正常）",
                pruned_dirs.join("、")
            ),
        );
    }
    if applied.is_applied() {
        emit_log(
            app,
            "info",
            &format!(
                "已套用到遊戲：{}；翻譯過的模組檔 {} 個。備份：{}",
                applied.zip_copied.as_deref().unwrap_or("其他翻譯檔"),
                applied.jars_copied,
                backup_status(&applied)
            ),
        );
    }
    let sibling_instance_warning = detect_sibling_instance_warning(&instance);
    if let Some(ref w) = sibling_instance_warning {
        emit_warn(app, w);
    }
    let coverage_percent = if built.keys_total.saturating_add(pending_count) == 0 {
        100
    } else {
        ((built.keys_total.saturating_mul(100))
            / built.keys_total.saturating_add(pending_count))
            .min(100) as u8
    };
    let completed_with_pending = pending_count > 0 || engine::run_interrupt::current().is_some();
    emit_progress_ex(
        app,
        Some(100),
        if user_stopped {
            "已停止：已翻好的部分已寫出，可按接續補完繼續"
        } else if completed_with_pending {
            "本輪流程完成，仍有內容待補"
        } else {
            "翻譯流程完成"
        },
        ProgressHint {
            stage: Some(dev_progress::STAGE_APPLY),
            state: Some(if completed_with_pending {
                STATE_COMPLETED_WITH_PENDING
            } else {
                STATE_COMPLETED
            }),
            metrics: Some(ProgressMetricsPayload {
                pack_pending: Some(pending_count as u64),
                coverage_percent: Some(coverage_percent),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    dev_progress::leave("apply");
    dev_progress::finish("ok");

    let process_note = if use_ai {
        "（整理＝本機，AI 只翻譯缺漏英文；不宣稱 100%）"
    } else {
        "（整理＝本機與既有參考資料；未使用線上翻譯服務，不宣稱 100%）"
    };
    let translated_count_note = if use_ai {
        format!("• 中文總計約 {} 條（AI 新補 {}）", built.keys_total, ai_filled)
    } else {
        format!("• 中文總計約 {} 條", built.keys_total)
    };
    // 只講「補得動的」。羅馬數字、圖示、單位、品牌名本來就不該翻，
    // 把它們算進待補，使用者永遠看到一個補不完的數字（見 engine/gap_model.rs）。
    // 缺口數字要涵蓋**所有階段**，不能只報語言表。
    // 舊版說「還有 525 條可以再補翻」，但那只算語言表——同一次任務書
    // 46 個檔案與整個選單完全沒翻，數字卻讓人以為只差一點。
    let stage_outstanding = stage_ledger.outstanding();
    let pending_note = if stage_outstanding > 0 {
        format!(
            "• {}（另有其他內容整段沒翻到，見上方）",
            engine::describe_gaps(gaps)
        )
    } else {
        format!("• {}", engine::describe_gaps(gaps))
    };
    // 「完成」這兩個字是有條件的。
    //
    // 實測 2026-09-02：GPT 額度用盡讓任務書（46 個檔案）與選單文字整段沒翻，
    // 工具卻照樣說「完成！可以直接開遊戲了，主要遊戲文字都已是繁體中文」。
    // 使用者拿到半成品還以為好了——這比翻不完更糟。
    //
    // 有任何階段「找到東西卻一個都沒做成」，開頭就要先講那件事。
    // 開發人員模式印出完整帳本並驗算。帳不平＝有單位在中途悄悄消失，
    // 那就是還沒被發現的漏翻來源（站長提醒過「可能還有其他來源」）。
    crate::dev_log!("ledger", "{}", stage_ledger.dev_report());
    for line in stage_ledger.imbalances() {
        emit_log(app, "warn", &format!("完整性檢查：{line}"));
    }
    let stage_failures = stage_ledger.player_summary();
    // B4：AI 中途停下（停止、額度、斷線太久）不可以講成「完成」
    let headline = match (engine::run_interrupt::current().is_some(), applied.is_applied()) {
        (true, true) => "這一輪中途停下，沒有全部完成。已翻好的部分都已套用到遊戲；按「接續補完」會從停下的地方繼續。",
        (true, false) => "這一輪中途停下，沒有全部完成，而且還沒套用到遊戲（原因見最上面）；已翻好的部分都有保留。",
        _ => match (stage_ledger.has_total_failure(), applied.is_applied()) {
        (false, true) => "翻譯流程跑完，已套用到遊戲。實際中文比例與還是英文的部分以完成卡為準（圖片上的字翻不到）。",
        (false, false) => "翻好了，還沒套用到遊戲（原因與下一步見最上面）。",
        (true, true) => "這一輪沒有全部完成。已完成的部分都已套用到遊戲，但有內容完全沒翻到（見下方）。",
        (true, false) => "這一輪沒有全部完成，而且還沒套用到遊戲；有內容完全沒翻到（見下方）。",
        },
    };
    let player_summary = format!(
        "{headline}\n\
{stage_failures}\
{}\n\
{}\n\
{}\n\
• {}\n\
• {}\n\
• 台灣正體轉換：{}\n\
• pack_format：{}\n\
• 結果資料夾（工具自動建立）：\n{}\n\
• 資源包 zip：\n{}\n\
• 詳見「覆蓋範圍說明.txt」\n\n\
【請你】\n\
{}",
        process_note,
        translated_count_note,
        pending_note,
        ref_note,
        if quest_note.is_empty() {
            "任務／覆寫：無".to_string()
        } else {
            quest_note
        },
        converter_name(),
        pack_format,
        work.display(),
        built.pack_path,
        engine::apply_notice::after_run_next_steps(&applied),
    );

    // 沒回應清單只留「現在仍缺」的（同一輪後面的補充可能已補上）
    let no_answer_left = count_map(&remaining_pending(&no_answer, &zh));
    let deferred_left = count_map(&remaining_pending(&quality_deferred_for_view, &zh));
    Ok(with_apply_notice(OneClickResult {
        apply_result: None,
        interruption: engine::run_interrupt::view(no_answer_left, deferred_left),
        display_safety: Default::default(),
        run_plan: plan.clone(),
        run_plan_has_overrides: plan.has_overrides(),
        report,
        pack_path: built.pack_path,
        work_root: work.display().to_string(),
        namespaces: built.namespaces,
        files_written: built.files_written,
        keys_total: built.keys_total,
        ai_filled,
        pending_count,
        coverage_percent,
        completed_with_pending,
        jar_translation,
        minemenu_msg,
        player_summary,
        sibling_instance_warning,
        stays_unchanged: skipped_untranslatable,
            apply_status: applied.status,
            apply_message: String::new(),
            pending_overwrites: Vec::new(),
        }, &applied))
}

/// AI 是本次翻譯的明確選項時，所有入口都必須先做和正式批次相同的真實請求。
///
/// 登入、帳號資料、餘額端點與本地 `/health` 都只是前置狀態；它們不能保證 GPT、
/// 自訂 API 或本地模型真的能輸出譯文。反過來說，使用者明確選「不使用 AI」時，
/// 不能因為 AI 壞掉而阻擋共享庫／本機記憶的離線流程。
fn preflight_selected_ai(app: &AppHandle, use_ai: bool, next_step: &str) -> Result<(), String> {
    if !use_ai {
        return Ok(());
    }
    let mode = get_ai_mode();
    emit_progress_stage(app, dev_progress::STAGE_PREP, Some(1), "確認 AI 能否實際翻譯…");
    emit_log(
        app,
        "info",
        &format!(
            "{mode}：開始前以實際翻譯測試 AI（此測試不會寫入翻譯結果、翻譯記憶或共享庫）…"
        ),
    );
    if let Err(e) = verify_ai_assistance() {
        let message = format!("AI 無法協助翻譯，已停止本次翻譯：{e}");
        emit_error(app, &message);
        return Err(message);
    }
    emit_log(
        app,
        "info",
        &format!("AI 實際翻譯測試通過（未寫入任何翻譯資料），{next_step}。"),
    );
    Ok(())
}

/// 從 LangMap 取出最多 `cap` 條，用於「自動重試」這類需要設上限的批次。
///
/// 命名空間依字典序、鍵依字典序取，讓同一份輸入每次取到同一批——重試的對象
/// 可預測，使用者連跑兩次不會看到忽多忽少的數字。
fn take_capped_langmap(source: &LangMap, cap: usize) -> LangMap {
    let mut out: LangMap = HashMap::new();
    if cap == 0 {
        return out;
    }
    let mut taken = 0usize;
    let mut namespaces: Vec<&String> = source.keys().collect();
    namespaces.sort();
    for ns in namespaces {
        let Some(map) = source.get(ns) else { continue };
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            if taken >= cap {
                return out;
            }
            if let Some(v) = map.get(k) {
                out.entry(ns.clone())
                    .or_default()
                    .insert(k.clone(), v.clone());
                taken += 1;
            }
        }
    }
    out
}

/// 補翻／修復時沿用同一個 pack_format，否則重建出來的 zip 會被遊戲標成「不相容」。
/// 優先用工作階段記錄的目標版本（使用者當初指定的），其次偵測。
fn session_pack_format(session: &TranslateSession) -> u32 {
    if let Some(f) = session
        .target_version
        .as_deref()
        .and_then(pack_format_for_version)
    {
        return f;
    }
    let inst = PathBuf::from(session.instance_path.trim());
    if !inst.exists() {
        return 0; // 交給 build_resource_pack 用保底值
    }
    let mc = resolve_minecraft_dir(&inst).unwrap_or(inst);
    detect_pack_format(&mc)
}

/// 寫出本機整理後的待處理清單，讓使用者知道工具實際掃過哪些內容。
/// 由本次結果組出 per-candidate 帳本。
///
/// 三個來源合起來就是完整候選集合，彼此不重疊：
/// - `zh`：已經有繁中的（本次新翻或沿用既有）
/// - `kept`：判定為刻意保留的（過濾前的快照，扣掉仍在 pending 的）
/// - `pending`：真正還沒翻好的
///
/// 刻意不在這裡碰 `shared_sync`：帳本建立時同步還沒發生，
/// 而且同步狀態永遠不該影響缺口計算。
fn build_outcome_ledger(
    zh: &LangMap,
    provenance: &ProvenanceMap,
    before_filter: &[(String, String)],
    pending: &LangMap,
) -> engine::outcome_ledger::OutcomeLedger {
    use engine::eligibility::{classify, Candidate};
    use engine::outcome_ledger::{
        CandidateOutcome, OutcomeLedger, ResolutionSource, TranslationOutcome,
    };

    let mut ledger = OutcomeLedger::default();

    // 1) 已有繁中：依來源分「本次新翻」與「沿用既有」。
    //    兩者都可套用，但報告要分開講——玩家想知道這次到底做了什麼。
    for (ns, map) in zh {
        for key in map.keys() {
            let source = engine::get_lang_source(provenance, ns, key);
            let (outcome, resolution) = match source {
                Some(LangSource::Ai) => (TranslationOutcome::Written, Some(ResolutionSource::Provider)),
                Some(LangSource::Glossary) => (TranslationOutcome::Written, Some(ResolutionSource::Glossary)),
                Some(LangSource::Tm) => (TranslationOutcome::Written, Some(ResolutionSource::LocalTm)),
                // 原生繁中、簡繁轉換、港繁提示、參考包都屬於「沿用既有」
                Some(_) => (TranslationOutcome::ReusedExistingZh, None),
                None => (TranslationOutcome::Written, None),
            };
            let mut entry = CandidateOutcome::new(ns, key, "lang", outcome);
            if let Some(r) = resolution {
                entry = entry.with_resolution(r);
            }
            ledger.record(entry);
        }
    }

    // 2) 仍在 pending 的 → 依 eligibility 分成待人工或可重試
    let mut pending_keys: std::collections::HashSet<(&str, &str)> = std::collections::HashSet::new();
    for (ns, map) in pending {
        for (key, source) in map {
            pending_keys.insert((ns.as_str(), key.as_str()));
            let verdict = classify(Candidate {
                source_kind: "lang",
                logical_key: key,
                text: source,
            });
            let outcome = engine::outcome_ledger::outcome_for_eligibility(&verdict);
            let mut entry = CandidateOutcome::new(ns, key, "lang", outcome);
            if let Some(reason) = verdict.player_reason() {
                entry = entry.with_reason(reason);
            }
            ledger.record(entry);
        }
    }

    // 3) 過濾前有、過濾後沒有、也還沒有繁中的 → 刻意保留
    //    （`before_filter` 沒有 namespace，用 key 比對即可——同一個 key
    //      在不同 namespace 的保留原因相同，不影響計數正確性）
    let pending_only_keys: std::collections::HashSet<&str> =
        pending_keys.iter().map(|(_, k)| *k).collect();
    let zh_keys: std::collections::HashSet<&str> =
        zh.values().flat_map(|m| m.keys().map(String::as_str)).collect();
    for (key, source) in before_filter {
        if pending_only_keys.contains(key.as_str()) || zh_keys.contains(key.as_str()) {
            continue;
        }
        let verdict = classify(Candidate {
            source_kind: "lang",
            logical_key: key,
            text: source,
        });
        let mut entry = CandidateOutcome::new(
            "",
            key,
            "lang",
            TranslationOutcome::IntentionallyUnchanged,
        );
        if let Some(reason) = verdict.player_reason() {
            entry = entry.with_reason(reason);
        }
        ledger.record(entry);
    }

    ledger
}

fn save_pending_manifest(
    out: &Path,
    pending: &LangMap,
    count: usize,
    ai_enabled: bool,
) -> Result<(), String> {
    let p = out.join("待翻譯清單-本地整理完成.json");
    let note = if ai_enabled {
        "此檔在呼叫 AI 之前寫入。AI 只翻譯這裡的英文，不再掃描 jar／資源包。"
    } else {
        "此檔記錄本次本機整理後仍缺少的英文；本次未使用線上翻譯服務。"
    };
    let obj = serde_json::json!({
        "note": note,
        "pendingCount": count,
        "namespaces": pending.len(),
    });
    std::fs::write(
        p,
        serde_json::to_string_pretty(&obj).unwrap_or_default() + "\n",
    )
    .map_err(|e| e.to_string())
}

/// 接續補完：讀上次工作階段 + 現有資源包，不重掃 mods。AI 是選用功能。
#[tauri::command]
async fn supplement_translate(
    app: AppHandle,
    output_dir: String,
    use_ai: bool,
    translation_mode: Option<String>,
) -> Result<OneClickResult, String> {
    reset_progress_emit_state();
    let out = normalize_path_strict(&output_dir)?;
    reset_cancel();
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        run_supplement(
            &app2,
            out,
            use_ai,
            translation_mode,
        )
    })
        .await
        .map_err(|e| describe_worker_failure(&e))?;
    if let Err(e) = &r {
        report_failure(&app, e);
    }
    r
}

fn run_supplement(
    app: &AppHandle,
    out: PathBuf,
    use_ai: bool,
    translation_mode_override: Option<String>,
) -> Result<OneClickResult, String> {
    // B2：每輪開始先清空上一輪的退回紀錄
    engine::begin_guard_run();
    engine::run_interrupt::reset();
    reset_contribute_tracker();
    preflight_selected_ai(app, use_ai, "開始讀取上次的翻譯工作階段")?;
    emit_progress_stage(app, dev_progress::STAGE_PREP, Some(5), "正在讀取上次的翻譯工作階段…");
    if !out.exists() {
        return Err("輸出資料夾不存在。請選與上次相同的「結果存哪」。".into());
    }
    ensure_space(&out, MIN_FREE_BYTES)?;
    let layout = ensure_result_layout(&out)?;
    let work = layout.work_root.clone();
    cleanup_transient_work(&work)?;
    let (mut session, session_file) = load_session(&out).or_else(|_| load_session(&work))?;
    {
        let instance = PathBuf::from(session.instance_path.trim());
        let mc_for_gate = resolve_minecraft_dir(&instance).unwrap_or_else(|_| instance.clone());
        let gated = ensure_minecraft_version_for_translate(
            session.target_version.as_deref(),
            &mc_for_gate,
        )?;
        emit_log(
            app,
            "info",
            &format!("Minecraft 版本閘門通過：{gated}"),
        );
    }
    let mode = resolve_translation_mode(translation_mode_override.as_deref(), &session.translation_mode);
    let _skip_shared = SkipSharedLookupGuard::enter(mode == TranslationMode::Force);
    // 補翻／修復走 Supplement：使用者已經看過一次結果才按這顆，
    // 不該再替他決定範圍。設定沿用工作階段裡記著的值（也就是他自己選過的）。
    let supplement_plan = engine::run_plan::resolve(
        engine::run_plan::RunIntent::Supplement,
        &engine::run_plan::RunPlanRequest {
            mode: Some(mode.value().to_string()),
            quality: Some(session.translation_quality.clone()),
            tier: Some(session.coverage_tier.clone()),
            advanced_unpack: Some(true),
        },
    );
    let quality = supplement_plan.translation_quality();
    let tier = supplement_plan.coverage_tier();
    let sources: CoverageSourceFlags = tier.sources();
    session.coverage_tier = tier.value().into();
    emit_log(app, "info", &mode_note(mode, 0));
    emit_log(app, "info", &format!("翻譯品質：{}", quality.label()));
    emit_log(app, "info", &tier.note());
    if let Some(summary) = supplement_plan.override_summary() {
        emit_log(app, "info", &summary);
    }
    session.review_pass = session.review_pass.saturating_add(1);
    emit_progress_stage(
        app,
        dev_progress::STAGE_PREP,
        Some(10),
        &format!("已找到工作階段：{}", session_file.display()),
    );

    let session_home = session_file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| work.clone());

    emit_progress_stage(app, dev_progress::STAGE_PREP, Some(15), "正在讀取已有的資源包…");
    let dict = load_phrase_dict(None);
    let mut recovered = false;
    let mut zh = match find_pack_near(&work, &session.pack_name, &session.pack_path)
        .or_else(|| find_pack_near(&out, &session.pack_name, &session.pack_path))
        .or_else(|| find_pack_near(&session_home, &session.pack_name, &session.pack_path))
    {
        Some(pack_path) => {
            session.pack_path = pack_path.display().to_string();
            emit_progress_stage(
                app,
                dev_progress::STAGE_PREP,
                Some(18),
                &format!("讀取資源包：{}", pack_path.display()),
            );
            load_pack_zh(&pack_path)?
        }
        None => {
            // 資源包遺失：用 session 裡的實例路徑「只本地整理」重建中文底稿，不重跑 AI 全量
            recovered = true;
            emit_progress_stage(
                app,
                dev_progress::STAGE_SCAN,
                Some(16),
                "找不到上次資源包檔，改從遊戲本地重新整理中文底稿（不重掃式 AI）…",
            );
            let inst = PathBuf::from(session.instance_path.trim());
            if !inst.exists() {
                return Err(format!(
                    "資源包遺失，且工作階段記錄的遊戲路徑也不存在：\n{}\n\
請把「結果存哪」改回上次目錄，或重新「開始一鍵翻譯」。\n\
工作階段檔：{}",
                    session.instance_path,
                    session_file.display()
                ));
            }
            let app_scan = app.clone();
            let (zh_scan, _en, _prov, _rep) = scan_instance(&inst, &dict, true, true, move |pct, msg| {
                // 映射到 16–24
                let mapped = 16 + (pct as u16 * 8 / 100) as u8;
                emit_progress_stage(&app_scan, dev_progress::STAGE_SCAN, Some(mapped.min(24)), msg);
            })?;
            zh_scan
        }
    };
    postprocess_lang_values(&mut zh, &dict);

    let mut pending = remaining_pending(&session.pending_en, &zh);
    let rework = rework_unusable_zh(&zh);
    merge_pending(&mut pending, &rework);
    // B2：建包前載入不會縮減的英文原文全表（舊結果沒有就先重掃補齊）
    let catalog = engine::source_catalog::prepare_build_sources(&work, Path::new(session.instance_path.trim()));
    if let Some(note) = catalog.player_note() {
        emit_log(app, "info", note);
    }
    // 不合格譯文從 zh 移除，避免寫回資源包繼續鎖死
    for (ns, map) in &rework {
        if let Some(slot) = zh.get_mut(ns) {
            for k in map.keys() {
                slot.remove(k);
            }
        }
    }
    let deferred_skipped = if mode == TranslationMode::Force {
        0
    } else {
        filter_quality_deferred(&mut pending, &session.quality_deferred)
    };
    if deferred_skipped > 0 {
        emit_log(
            app,
            "info",
            &format!(
                "品質暫緩：跳過 {deferred_skipped} 條（這些在翻譯時已自動重試過一次仍未通過）；要再試一次請勾「重新翻譯缺漏」"
            ),
        );
    }
    let need = count_map(&pending);
    if need == 0 {
        // lang 已無 pending：略過 AI fill；額外來源仍可掃「仍為英文的顯示字串」補缺口。
        emit_log(
            app,
            "info",
            "語言表 pending=0（品質閘門後仍無缺），略過 AI fill；額外來源僅補仍為英文的顯示字串",
        );
        let instance = PathBuf::from(session.instance_path.trim());
        let jar_translation = rewrite_jars_and_log(app, &instance, &work, &zh, &session.pending_en)?;
        let supplement_scope = TranslationScope::from_instance(&instance);
        if sources.jar_patchouli {
            let _ = translate_jar_patchouli(&instance, &work, use_ai, Some(&supplement_scope), |pct, msg| {
                emit_progress(app, 20 + (pct as u16 / 10) as u8, msg);
            });
        } else {
            emit_log(app, "info", "完整度略過：JAR Patchouli");
        }
        if sources.jar_display {
            let _ = translate_jar_display_texts(&instance, &work, use_ai, Some(&supplement_scope), |pct, msg| {
                emit_progress(app, 28 + (pct as u16 / 10) as u8, msg);
            });
        } else {
            emit_log(app, "info", "完整度略過：JAR 顯示文字");
        }
        let mut skipped_by_tier = Vec::new();
        let mut source_errors = Vec::new();
        let mut quest_note = String::new();
        if let Ok(mc) = resolve_minecraft_dir(&instance) {
            let extra_summary = run_extra_sources(
                app,
                &mc,
                &work,
                use_ai,
                Some(&supplement_scope),
                sources,
                38,
                50,
            );
            quest_note = extra_summary.combined_note();
            skipped_by_tier.extend(extra_summary.skipped);
            source_errors.extend(extra_summary.errors);
        }
        for note in &skipped_by_tier {
            emit_log(app, "info", note);
        }
        if !skipped_by_tier.is_empty() {
            let skip_join = skipped_by_tier.join("；");
            quest_note = if quest_note.is_empty() {
                skip_join
            } else {
                format!("{quest_note}；{skip_join}")
            };
        }
        if !source_errors.is_empty() {
            append_error_file(&work, &source_errors);
            emit_warn(
                app,
                &format!(
                    "補翻額外來源記錄 {} 筆錯誤／警告，已寫入：{}",
                    source_errors.len(),
                    work.join("翻譯錯誤日誌.txt").display()
                ),
            );
        }
        emit_progress_stage(app, dev_progress::STAGE_APPLY, Some(100), "沒有可再補的缺漏了");
        // 仍重寫 zip，避免玩家手上沒有壓縮檔
        let built = build_resource_pack(
            &zh,
            &BuildOptions {
                pack_folder_name: session.pack_name.clone(),
                pack_description: "台灣用語繁體中文翻譯資源包".into(),
                output_dir: work.display().to_string(),
                pack_format: session_pack_format(&session),
                target_version: session.target_version.clone(),
            },
        )?;
        emit_pruned_tool_pack_log(app, &built.pruned_tool_packs);
        session.pack_path = built.pack_path.clone();
        session.output_dir = work.display().to_string();
        session.keys_zh = built.keys_total;
        let still = remaining_pending(&session.pending_en, &zh);
        let still_n = count_map(&still);
        prune_quality_deferred(&mut session.quality_deferred, &zh);
        session.pending_en = still;
        session.pending_count = still_n;
        let _ = save_session(&work, &session);
        let _ = write_coverage_report(
            &layout,
            &CoverageStats {
                keys_zh: built.keys_total,
                 keys_pending: still_n,
                 keys_tw_playable: built.keys_total.saturating_sub(still_n),
                keys_hk_hint: session.keys_hk_hint,
                ai_filled: 0,
                ai_enabled: use_ai,
                jars_scanned: jar_translation.jars_scanned,
                jars_rewritten: jar_translation.jars_rewritten,
                jar_lang_files: jar_translation.lang_files_written,
                jar_errors: jar_translation.errors.len(),
                quests_note: quest_note.clone(),
                ref_note: "複查流程".into(),
                pack_path: built.pack_path.clone(),
                pack_format: session_pack_format(&session),
                source_notes: {
                    let mut notes = vec![
                         if still_n > 0 {
                             format!("補充：沒有新的可補譯項目，已略過重複 AI 請求；仍待 {} 條", still_n)
                         } else {
                             "複查：語言表 pending=0，略過 AI fill".into()
                         },
                        "額外來源：僅補仍為英文的顯示字串（FTB／書本／覆寫等）".into(),
                    ];
                    if !quest_note.is_empty() {
                        notes.push(quest_note.clone());
                    }
                    notes.extend(skipped_by_tier.iter().cloned());
                    notes
                },
                unsupported: skipped_by_tier,
                glossary_hits: 0,
                tm_hits: 0,
                shared_hits: 0,
                shared_glossary_hits: 0,
                prior_merged: 0,
                coverage_tier: tier.value().into(),
            },
        );
        let instance = PathBuf::from(session.instance_path.trim());
        let applied = apply_after_run(app, &instance, &work, &session.pack_name)?;
        emit_log(
            app,
            "info",
            &format!(
                "{}備份位置：{}",
                engine::apply_notice::reapply_log_line(&applied, "複查"),
                backup_status(&applied)
            ),
        );
        let sibling_instance_warning = detect_sibling_instance_warning(&instance);
        if let Some(ref w) = sibling_instance_warning {
            emit_warn(app, w);
        }
        return Ok(with_apply_notice(OneClickResult {
        apply_result: None,
             interruption: engine::run_interrupt::view(0, count_map(&session.quality_deferred)),
             display_safety: Default::default(),
             run_plan: supplement_plan.clone(),
             run_plan_has_overrides: supplement_plan.has_overrides(),
             report: empty_report(
                &session.instance_path,
                built.keys_total,
                built.namespaces,
                still_n,
            ),
            pack_path: built.pack_path.clone(),
            work_root: work.display().to_string(),
            namespaces: built.namespaces,
            files_written: built.files_written,
            keys_total: built.keys_total,
            ai_filled: 0,
             pending_count: still_n,
             coverage_percent: if built.keys_total.saturating_add(still_n) == 0 {
                 100
             } else {
                 ((built.keys_total.saturating_mul(100))
                     / built.keys_total.saturating_add(still_n))
                     .min(100) as u8
             },
             completed_with_pending: still_n > 0,
            jar_translation,
            minemenu_msg: None,
             player_summary: if still_n > 0 {
                 format!(
                         "本輪沒有新的可補譯項目，已略過重複 AI 請求。\n仍有 {} 條未完成，其中 {} 條已暫緩；勾選「重新翻譯缺漏」才會再次嘗試。\n目前資源包約有 {} 條中文。\n位置：\n{}{}",
                         still_n,
                         count_map(&session.quality_deferred),
                         built.keys_total,
                         built.pack_path,
                         if recovered {
                             "\n（已因遺失資源包而重建 zip）"
                         } else {
                             ""
                         },
                     )
             } else {
                 format!(
                         "沒有還能補的缺漏了。\n目前資源包約有 {} 條中文。\n位置：\n{}{}",
                         built.keys_total,
                         built.pack_path,
                         if recovered {
                             "\n（已因遺失資源包而重建 zip）"
                         } else {
                             ""
                         },
                     )
              },
            sibling_instance_warning,
            stays_unchanged: 0,
            apply_status: applied.status,
            apply_message: String::new(),
            pending_overwrites: Vec::new(),
        }, &applied));
    }

    // 補翻使用目前選擇的 AI 來源（自訂 API／GPT）；翻譯前已另閘 Discord。
    emit_progress_ex(
        app,
        Some(25),
        &format!(
            "補充：工作階段就緒{}。開始只補仍缺約 {} 條…",
            if recovered {
                "（已恢復中文底稿）"
            } else {
                ""
            },
            need,
        ),
        ProgressHint {
            stage: Some(dev_progress::STAGE_TRANSLATE),
            step: Some(4),
            step_total: Some(dev_progress::UI_STEP_TOTAL),
            state: Some(STATE_RUNNING),
            ..Default::default()
        },
    );
    let app_ai = app.clone();
    let supplement_scope = TranslationScope::from_instance(Path::new(session.instance_path.trim()));
    let seeded_sup = seed_tm_from_langmaps(&session.pending_en, &zh);
    if seeded_sup > 0 {
        emit_log(
            app,
            "info",
            &format!("補充前：已把本包已有譯文寫入翻譯記憶 {seeded_sup} 條"),
        );
    }
    let ai_report = fill_missing_with_mode(&mut zh, &pending, use_ai, mode == TranslationMode::Force, quality, Some(&supplement_scope), move |pct, msg| {
        let mapped = 25 + (pct as u16 * 55 / 100) as u8;
        emit_progress_ex(
            &app_ai,
            Some(mapped.min(80)),
            &format!("補充：{msg}"),
            ProgressHint {
                stage: Some(dev_progress::STAGE_TRANSLATE),
                step: Some(4),
                step_total: Some(dev_progress::UI_STEP_TOTAL),
                state: Some(STATE_RUNNING),
                ..Default::default()
            },
        );
    })?;
    merge_pending(&mut session.quality_deferred, &ai_report.quality_deferred);
    // B4：補翻途中按停止（或 AI 停下）：已補好的照樣寫出、裝進遊戲，後面不再翻
    let mut user_stopped = false;
    if ai_report.stopped_by_user || is_cancelled() {
        begin_user_stop_finalize(app, &mut user_stopped);
    } else if let Some(reason) = &ai_report.ai_unavailable {
        emit_warn(
            app,
            &format!(
                "AI 中途停下：{}。已補好的都保留並套用到遊戲；排除原因後再按一次會從停下的地方繼續。",
                reason.lines().next().unwrap_or("AI 不可用")
            ),
        );
    }
    let deferred_after_ai = count_map(&ai_report.quality_deferred);
    if deferred_after_ai > 0 {
        emit_log(
            app,
            "info",
            &format!(
                "品質暫緩：{deferred_after_ai} 條，本次不重送{}",
                if mode == TranslationMode::Force {
                    "；強制模式複查後仍未通過"
                } else {
                    ""
                }
            ),
        );
    } else if mode == TranslationMode::Force && deferred_skipped > 0 {
        emit_log(app, "info", "品質複查完成：已處理先前暫緩項目");
    }
    let ai_filled = ai_report.filled;
    emit_log(app, "info", &ai_report.note());
    postprocess_lang_values(&mut zh, &dict);
    emit_progress_stage(app, dev_progress::STAGE_PACKAGE, Some(88), "補翻結果轉台灣正體…");
    convert_langmap_s2tw_with_progress(&mut zh, Some(&mut |done, total| {
        emit_progress_ex(
            app,
            None,
            &format!("補翻結果台灣正體轉換 {done}/{total}"),
            ProgressHint {
                stage: Some(dev_progress::STAGE_PACKAGE),
                step: Some(4),
                step_total: Some(dev_progress::UI_STEP_TOTAL),
                done: Some(done),
                total: Some(total),
                unit: Some("條"),
                detail: Some("補翻結果轉台灣正體".into()),
                state: Some(STATE_RUNNING),
                ..Default::default()
            },
        );
    }));
    postprocess_lang_values(&mut zh, &dict);

    let instance = PathBuf::from(session.instance_path.trim());
    let jar_translation = rewrite_jars_and_log(app, &instance, &work, &zh, &session.pending_en)?;
    if sources.jar_patchouli {
        let _ = translate_jar_patchouli(&instance, &work, use_ai, Some(&supplement_scope), |pct, msg| {
            emit_progress_stage(
                app,
                dev_progress::STAGE_EXTRAS,
                Some(89 + (pct as u16 / 10) as u8),
                msg,
            );
        });
    } else {
        emit_log(app, "info", "完整度略過：JAR Patchouli");
    }
    if sources.jar_display {
        let _ = translate_jar_display_texts(&instance, &work, use_ai, Some(&supplement_scope), |pct, msg| {
            emit_progress_stage(
                app,
                dev_progress::STAGE_EXTRAS,
                Some(90 + (pct as u16 / 10) as u8),
                msg,
            );
        });
    } else {
        emit_log(app, "info", "完整度略過：JAR 顯示文字");
    }

    emit_progress_stage(app, dev_progress::STAGE_PACKAGE, Some(82), "補充：正在寫回資源包（zip）…");
    let built = build_resource_pack(
        &zh,
        &BuildOptions {
            pack_folder_name: session.pack_name.clone(),
            pack_description: "台灣用語繁體中文翻譯資源包".into(),
            output_dir: work.display().to_string(),
            pack_format: session_pack_format(&session),
            target_version: session.target_version.clone(),
        },
    )?;
    emit_pruned_tool_pack_log(app, &built.pruned_tool_packs);

    // 補翻：額外來源只補仍為英文的顯示字串（已譯不重送）
    let mut skipped_by_tier: Vec<String> = Vec::new();
    let mut source_errors: Vec<String> = Vec::new();
    let inst = PathBuf::from(&session.instance_path);
    let _ = seed_tm_from_langmaps(&session.pending_en, &zh);
    let mut quest_note = if user_stopped {
        "已停止：額外來源這一輪不處理".to_string()
    } else if let Ok(mc) = resolve_minecraft_dir(&inst) {
        emit_progress_stage(app, dev_progress::STAGE_EXTRAS, Some(88), "補充：覆寫／任務仍缺…");
        let extra_summary = run_extra_sources(
            app,
            &mc,
            &work,
            use_ai,
            Some(&supplement_scope),
            sources,
            88,
            8,
        );
        let note = extra_summary.combined_note();
        skipped_by_tier.extend(extra_summary.skipped);
        source_errors.extend(extra_summary.errors);
        note
    } else {
        let note = "補翻略過額外來源：找不到 Minecraft 資料夾".to_string();
        source_errors.push(note.clone());
        note
    };
    for note in &skipped_by_tier {
        emit_log(app, "info", note);
    }
    if !skipped_by_tier.is_empty() {
        let skip_join = skipped_by_tier.join("；");
        quest_note = if quest_note.is_empty() {
            skip_join
        } else {
            format!("{quest_note}；{skip_join}")
        };
    }
    if !source_errors.is_empty() {
        append_error_file(&work, &source_errors);
        emit_warn(
            app,
            &format!(
                "補翻額外來源記錄 {} 筆錯誤／警告，已寫入：{}",
                source_errors.len(),
                work.join("翻譯錯誤日誌.txt").display()
            ),
        );
    }
    {
        // 這裡不需要 provenance：`pending_en` 是主流程「參考包合併之後」還缺中文的
        // 鍵，參考包填掉的鍵根本不在裡面，所以這條路上傳不到參考包的內容。
        let contrib = contribute_lang_maps(&session.pending_en, &zh, &supplement_scope, None);
        if contrib.attempted > 0 || contrib.failed || contrib.deferred > 0 {
            emit_log(
                app,
                "info",
                &format!(
                    "共享庫掃尾：accepted={}／衝突 {}／送出 {}",
                    contrib.accepted, contrib.conflicts, contrib.attempted
                ),
            );
        }
    }

    let still = remaining_pending(&session.pending_en, &zh);
    let still_n = count_map(&still);
    prune_quality_deferred(&mut session.quality_deferred, &zh);
    if sources.write_gap_summary {
        match write_gap_summary_file(&work, &still, 120) {
            Ok(p) => emit_log(
                app,
                "info",
                &format!("已寫待補缺口摘要（樣本）：{}", p.display()),
            ),
            Err(e) => emit_warn(app, &format!("待補缺口摘要寫入失敗：{e}")),
        }
    }
    session.pending_en = still;
    session.pending_count = still_n;
    session.keys_zh = built.keys_total;
    session.pack_path = built.pack_path.clone();
    session.output_dir = work.display().to_string();
    session.note = format!("補翻後剩餘約 {} 條可再補。{}", still_n, quest_note);
    session.coverage_tier = tier.value().into();
    let _ = save_session(&work, &session);

    let _ = write_coverage_report(
        &layout,
        &CoverageStats {
            keys_zh: built.keys_total,
            keys_pending: still_n,
            keys_tw_playable: built.keys_total.saturating_sub(still_n),
            keys_hk_hint: session.keys_hk_hint,
            ai_filled,
            ai_enabled: use_ai,
            jars_scanned: jar_translation.jars_scanned,
            jars_rewritten: jar_translation.jars_rewritten,
            jar_lang_files: jar_translation.lang_files_written,
            jar_errors: jar_translation.errors.len(),
            quests_note: quest_note.clone(),
            ref_note: "補翻流程".into(),
            pack_path: built.pack_path.clone(),
            pack_format: session_pack_format(&session),
            source_notes: {
                let mut notes = vec![
                    format!("完整度：{}（{}）", tier.label(), tier.value()),
                    "補翻：只處理工作階段仍缺少的文字，再重建所有輸出來源".into(),
                    quest_note.clone(),
                ];
                if let Some(note) = ai_report.usage_note() {
                    notes.push(note);
                }
                notes.extend(skipped_by_tier.iter().cloned());
                notes
            },
            unsupported: skipped_by_tier.clone(),
            glossary_hits: ai_report.glossary_hits,
            tm_hits: ai_report.tm_hits,
            shared_hits: ai_report.shared_hits,
            shared_glossary_hits: ai_report.shared_glossary_hits,
            prior_merged: 0,
            coverage_tier: tier.value().into(),
        },
    );

    let instance = PathBuf::from(session.instance_path.trim());
    let applied = apply_after_run(app, &instance, &work, &session.pack_name)?;
    emit_log(
        app,
        "info",
        &format!(
            "{}備份位置：{}",
            engine::apply_notice::reapply_log_line(&applied, "複查"),
            backup_status(&applied)
        ),
    );
    let sibling_instance_warning = detect_sibling_instance_warning(&instance);
    if let Some(ref w) = sibling_instance_warning {
        emit_warn(app, w);
    }
    emit_progress_stage(app, dev_progress::STAGE_APPLY, Some(100), "補翻完成！");

    let ai_result_line = if use_ai {
        format!("• 這次 AI 新補 {} 條{}", ai_filled, if recovered {
            "（先前資源包遺失，已重建）"
        } else {
            ""
        })
    } else {
        "• 這次只使用本機資料與既有翻譯，未使用線上翻譯服務".to_string()
    };

    Ok(with_apply_notice(OneClickResult {
        apply_result: None,
        interruption: engine::run_interrupt::view(
            count_map(&remaining_pending(&ai_report.no_answer, &zh)),
            count_map(&session.quality_deferred),
        ),
        display_safety: Default::default(),
        run_plan: supplement_plan.clone(),
        run_plan_has_overrides: supplement_plan.has_overrides(),
        report: empty_report(
            &session.instance_path,
            built.keys_total,
            built.namespaces,
            still_n,
        ),
        pack_path: built.pack_path.clone(),
        work_root: work.display().to_string(),
        namespaces: built.namespaces,
        files_written: built.files_written,
        keys_total: built.keys_total,
        ai_filled,
        pending_count: still_n,
        coverage_percent: if built.keys_total.saturating_add(still_n) == 0 {
            100
        } else {
            ((built.keys_total.saturating_mul(100))
                / built.keys_total.saturating_add(still_n))
                .min(100) as u8
        },
        completed_with_pending: still_n > 0,
        jar_translation,
        minemenu_msg: None,
        player_summary: format!(
            "補翻完成！\n{}\n\
{}\n\
• 資源包現在約 {} 條中文\n\
• 尚可補約 {} 條\n\
• 結果資料夾：\n{}\n\
• zip：\n{}\n\
• {}\n\
• 見「覆蓋範圍說明.txt」\n\
（任務／覆寫在結果資料夾 config、patchouli_books、kubejs 等）",
            engine::apply_notice::after_run_next_steps(&applied),
            ai_result_line,
            built.keys_total,
            still_n,
            work.display(),
            built.pack_path,
            if quest_note.is_empty() {
                "任務：未更新".into()
            } else {
                quest_note
            },
        ),
        sibling_instance_warning,
        stays_unchanged: 0,
            apply_status: applied.status,
            apply_message: String::new(),
            pending_overwrites: Vec::new(),
        }, &applied))
}

/// 修復翻譯資源包／工作階段（不修遊戲世界閃退）
/// - 找回或重建中文底稿
/// - 重產 .zip
/// - 對齊工作階段路徑
/// - 預設不呼叫 AI；勾 use_ai 時順便補缺漏
#[tauri::command]
async fn repair_translation_pack(
    app: AppHandle,
    output_dir: String,
    use_ai: bool,
    translation_mode: Option<String>,
) -> Result<OneClickResult, String> {
    reset_progress_emit_state();
    let out = normalize_path_strict(&output_dir)?;
    reset_cancel();
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        run_repair(
            &app2,
            out,
            use_ai,
            translation_mode,
        )
    })
        .await
        .map_err(|e| describe_worker_failure(&e))?;
    if let Err(e) = &r {
        report_failure(&app, e);
    }
    r
}

fn run_repair(
    app: &AppHandle,
    out: PathBuf,
    use_ai: bool,
    translation_mode_override: Option<String>,
) -> Result<OneClickResult, String> {
    // B2：每輪開始先清空上一輪的退回紀錄
    engine::begin_guard_run();
    engine::run_interrupt::reset();
    reset_contribute_tracker();
    preflight_selected_ai(app, use_ai, "開始修復翻譯結果")?;
    emit_progress_stage(app, dev_progress::STAGE_PREP, Some(3), "修復：尋找工作階段…");
    if !out.exists() {
        return Err("輸出資料夾不存在。".into());
    }
    ensure_space(&out, MIN_FREE_BYTES)?;
    let layout = ensure_result_layout(&out)?;
    let work = layout.work_root.clone();
    cleanup_transient_work(&work)?;
    let (mut session, session_file) = load_session(&out).or_else(|_| load_session(&work))?;
    {
        let instance = PathBuf::from(session.instance_path.trim());
        let mc_for_gate = resolve_minecraft_dir(&instance).unwrap_or_else(|_| instance.clone());
        let gated = ensure_minecraft_version_for_translate(
            session.target_version.as_deref(),
            &mc_for_gate,
        )?;
        emit_log(
            app,
            "info",
            &format!("修復：Minecraft 版本閘門通過：{gated}"),
        );
    }
    session.review_pass = session.review_pass.saturating_add(1);
    let session_home = session_file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| work.clone());
    emit_progress_stage(
        app,
        dev_progress::STAGE_PREP,
        Some(8),
        &format!("修復：工作階段 → {}", session_file.display()),
    );

    let dict = load_phrase_dict(None);
    let mut actions: Vec<String> = Vec::new();
    actions.push(format!("找到工作階段：{}", session_file.display()));
    actions.push(format!("結果資料夾：{}", work.display()));

    // 1) 載入或重建中文
    emit_progress_stage(app, dev_progress::STAGE_PREP, Some(12), "修復：檢查資源包…");
    let pack_found = find_pack_near(&work, &session.pack_name, &session.pack_path)
        .or_else(|| find_pack_near(&out, &session.pack_name, &session.pack_path))
        .or_else(|| find_pack_near(&session_home, &session.pack_name, &session.pack_path));

    let mut zh = if let Some(ref pack_path) = pack_found {
        actions.push(format!("找到既有資源包：{}", pack_path.display()));
        emit_progress_stage(
            app,
            dev_progress::STAGE_PREP,
            Some(18),
            &format!("讀取：{}", pack_path.display()),
        );
        match load_pack_zh(pack_path) {
            Ok(m) if !m.is_empty() => m,
            Ok(_) | Err(_) => {
                actions.push("資源包讀取失敗或為空，改從遊戲本地重建底稿。".into());
                rebuild_zh_from_instance(app, &session, &dict, &mut actions)?
            }
        }
    } else {
        actions.push("資源包遺失，從遊戲本地重建中文底稿。".into());
        rebuild_zh_from_instance(app, &session, &dict, &mut actions)?
    };
    postprocess_lang_values(&mut zh, &dict);
    actions.push(format!("目前中文底稿約 {} 條", count_map(&zh)));

    // 2) 可選：AI 補缺
    let mut ai_filled = 0usize;
    let mut pending = remaining_pending(&session.pending_en, &zh);
    // B2：建包前載入不會縮減的英文原文全表（舊結果沒有就先重掃補齊）
    let catalog = engine::source_catalog::prepare_build_sources(&work, Path::new(session.instance_path.trim()));
    if let Some(note) = catalog.player_note() {
        actions.push(note.to_string());
    }
    let repair_mode =
        resolve_translation_mode(translation_mode_override.as_deref(), &session.translation_mode);
    // 修復與補翻同屬 Supplement：沿用工作階段裡使用者自己選過的設定，不再覆寫。
    let repair_plan = engine::run_plan::resolve(
        engine::run_plan::RunIntent::Supplement,
        &engine::run_plan::RunPlanRequest {
            mode: Some(repair_mode.value().to_string()),
            quality: Some(session.translation_quality.clone()),
            tier: Some(session.coverage_tier.clone()),
            advanced_unpack: Some(true),
        },
    );
    let _skip_shared = SkipSharedLookupGuard::enter(repair_mode == TranslationMode::Force);
    let repair_deferred_skipped = if repair_mode == TranslationMode::Force {
        0
    } else {
        filter_quality_deferred(&mut pending, &session.quality_deferred)
    };
    let need = count_map(&pending);
    if repair_deferred_skipped > 0 {
        actions.push(format!(
            "品質暫緩：跳過 {repair_deferred_skipped} 條（這些在翻譯時已自動重試過一次仍未通過）；要再試一次請勾「重新翻譯缺漏」"
        ));
    }
    let repair_quality = TranslationQuality::parse(Some(session.translation_quality.as_str()));
    emit_log(app, "info", &mode_note(repair_mode, 0));
    emit_log(
        app,
        "info",
        &format!("翻譯品質：{}", repair_quality.label()),
    );
    let repair_scope = TranslationScope::from_instance(Path::new(session.instance_path.trim()));
    if use_ai && need > 0 {
        emit_progress_ex(
            app,
            Some(40),
            &format!("修復＋補翻：約 {} 條…", need),
            ProgressHint {
                stage: Some(dev_progress::STAGE_TRANSLATE),
                step: Some(4),
                step_total: Some(dev_progress::UI_STEP_TOTAL),
                state: Some(STATE_RUNNING),
                ..Default::default()
            },
        );
        let app_ai = app.clone();
        let r = fill_missing_with_mode(
            &mut zh,
            &pending,
            use_ai,
            repair_mode == TranslationMode::Force,
            repair_quality,
            Some(&repair_scope),
            move |pct, msg| {
                let mapped = 40 + (pct as u16 * 45 / 100) as u8;
                emit_progress_ex(
                    &app_ai,
                    Some(mapped.min(88)),
                    msg,
                    ProgressHint {
                        stage: Some(dev_progress::STAGE_TRANSLATE),
                        step: Some(4),
                        step_total: Some(dev_progress::UI_STEP_TOTAL),
                        state: Some(STATE_RUNNING),
                        ..Default::default()
                    },
                );
            },
        )?;
        ai_filled = r.filled;
        merge_pending(&mut session.quality_deferred, &r.quality_deferred);
        if !r.quality_deferred.is_empty() {
            actions.push(format!(
                "品質暫緩：{} 條，本次不重送",
                count_map(&r.quality_deferred)
            ));
        }
        postprocess_lang_values(&mut zh, &dict);
        actions.push(r.note());
    } else if need > 0 {
        actions.push(format!(
            "尚有約 {} 條還是英文（修復不會連線翻譯；可再按「接續補完」）。",
            need
        ));
    } else {
        actions.push("沒有待補缺漏。".into());
    }

    // 3) 重產 zip + 對齊 session（寫入「翻譯結果」）
    emit_progress_stage(app, dev_progress::STAGE_PACKAGE, Some(90), "修復：重產 zip 資源包…");
    let instance = PathBuf::from(session.instance_path.trim());
    let repair_scope = TranslationScope::from_instance(&instance);
    let jar_translation = rewrite_jars_and_log(app, &instance, &work, &zh, &session.pending_en)?;
    let _ = translate_jar_patchouli(&instance, &work, use_ai, Some(&repair_scope), |pct, msg| {
        emit_progress_stage(
            app,
            dev_progress::STAGE_EXTRAS,
            Some(88 + (pct as u16 / 10) as u8),
            msg,
        );
    });
    let _ = translate_jar_display_texts(&instance, &work, use_ai, Some(&repair_scope), |pct, msg| {
        emit_progress_stage(
            app,
            dev_progress::STAGE_EXTRAS,
            Some(89 + (pct as u16 / 10) as u8),
            msg,
        );
    });
    if let Ok(mc) = resolve_minecraft_dir(&instance) {
        if let Ok(q) = translate_ftbquests(&mc, &work, use_ai, Some(&repair_scope), |_, msg| {
            emit_log(app, "info", msg);
        }) {
            actions.push(q.note);
        }
        if let Ok(o) = translate_text_overlays(&mc, &work, use_ai, Some(&repair_scope), |_, msg| {
            emit_log(app, "info", msg);
        }) {
            actions.push(o.note);
        }
        if let Ok(a) = translate_archive_overlays(&mc, &work, use_ai, Some(&repair_scope), |_, msg| {
            emit_log(app, "info", msg);
        }) {
            actions.push(format!(
                "ZIP 文字：掃描 {} 個、重建 {} 個、寫入 {} 個項目",
                a.archives_scanned, a.archives_rewritten, a.entries_rewritten
            ));
        }
        if let Ok(s) = translate_kubejs_literals(&mc, &work, use_ai, Some(&repair_scope), |_, msg| {
            emit_log(app, "info", msg);
        }) {
            actions.push(s.note);
        }
        if let Ok(o) = translate_origins(&mc, &work, use_ai, Some(&repair_scope), |_, msg| {
            emit_log(app, "info", msg);
        }) {
            if !o.note.is_empty() {
                actions.push(o.note);
            }
        }
        if let Ok(q) = translate_quests_books(&mc, &work, use_ai, Some(&repair_scope), |_, msg| {
            emit_log(app, "info", msg);
        }) {
            if !q.note.is_empty() {
                actions.push(q.note);
            }
        }
    }
    {
        // 同補充漏翻：`pending_en` 不含參考包填掉的鍵（見該處註解）
        let contrib = contribute_lang_maps(&session.pending_en, &zh, &repair_scope, None);
        if contrib.attempted > 0 || contrib.failed || contrib.deferred > 0 {
            actions.push(format!(
                "共享庫掃尾：accepted={}／衝突 {}／送出 {}",
                contrib.accepted, contrib.conflicts, contrib.attempted
            ));
        }
    }

    let pack_name = if session.pack_name.trim().is_empty() {
        "繁體中文翻譯".to_string()
    } else {
        session.pack_name.clone()
    };
    let built = build_resource_pack(
        &zh,
        &BuildOptions {
            pack_folder_name: pack_name.clone(),
            pack_description: "台灣用語繁體中文翻譯資源包（修復重建）".into(),
            output_dir: work.display().to_string(),
            pack_format: session_pack_format(&session),
            target_version: session.target_version.clone(),
        },
    )?;
    emit_pruned_tool_pack_log(app, &built.pruned_tool_packs);
    actions.push(format!("已寫入 zip：{}", built.pack_path));

    let still = remaining_pending(&session.pending_en, &zh);
    let still_n = count_map(&still);
    prune_quality_deferred(&mut session.quality_deferred, &zh);
    session.pack_name = pack_name;
    session.pack_path = built.pack_path.clone();
    session.output_dir = work.display().to_string();
    session.pending_en = still;
    session.pending_count = still_n;
    session.keys_zh = built.keys_total;
    session.note = format!("修復後剩餘可補約 {} 條。", still_n);
    let _ = save_session(&work, &session);
    actions.push("工作階段路徑已對齊並儲存。".into());

    // 4) 快捷選單若可修也做
    let minemenu_msg = if PathBuf::from(&session.instance_path).exists() {
        apply_minemenu_fixed(Path::new(&session.instance_path), &work)
    } else {
        None
    };
    if let Some(ref m) = minemenu_msg {
        actions.push(m.clone());
    }

    let instance = PathBuf::from(session.instance_path.trim());
    let applied = apply_after_run(app, &instance, &work, &session.pack_name)?;
    emit_log(
        app,
        "info",
        &format!(
            "{}備份位置：{}",
            engine::apply_notice::reapply_log_line(&applied, "修復"),
            backup_status(&applied)
        ),
    );
    let sibling_instance_warning = detect_sibling_instance_warning(&instance);
    if let Some(ref w) = sibling_instance_warning {
        emit_warn(app, w);
    }
    emit_progress_stage(app, dev_progress::STAGE_APPLY, Some(100), "修復完成！");

    let repair_translation_line = if use_ai {
        format!("• AI 本次補 {} 條", ai_filled)
    } else {
        "• 本次只使用本機資料與既有翻譯，未使用線上翻譯服務".to_string()
    };
    let player_summary = format!(
        "【翻譯資源包修復完成】\n\
（此功能不處理「載入世界閃退」——那是結構／世界生成問題，與語言包無關。）\n\n\
{}\n\n\
• 中文詞約 {} 條\n\
{}\n\
• 尚可補約 {} 條\n\
• 結果資料夾：\n{}\n\
• zip：\n{}\n\n\
【接下來】\n\
{}\n\
3. 若還有英文 → 按「接續補完」",
        actions
            .iter()
            .map(|a| format!("• {a}"))
            .collect::<Vec<_>>()
            .join("\n"),
        built.keys_total,
        repair_translation_line,
        still_n,
        work.display(),
        built.pack_path,
        engine::apply_notice::after_run_next_steps(&applied)
    );

    Ok(with_apply_notice(OneClickResult {
        apply_result: None,
        interruption: engine::run_interrupt::view(0, count_map(&session.quality_deferred)),
        display_safety: Default::default(),
        run_plan: repair_plan.clone(),
        run_plan_has_overrides: repair_plan.has_overrides(),
        report: empty_report(
            &session.instance_path,
            built.keys_total,
            built.namespaces,
            still_n,
        ),
        pack_path: built.pack_path,
        work_root: work.display().to_string(),
        namespaces: built.namespaces,
        files_written: built.files_written,
        keys_total: built.keys_total,
        ai_filled,
        pending_count: still_n,
        coverage_percent: if built.keys_total.saturating_add(still_n) == 0 {
            100
        } else {
            ((built.keys_total.saturating_mul(100))
                / built.keys_total.saturating_add(still_n))
                .min(100) as u8
        },
        completed_with_pending: still_n > 0,
        minemenu_msg,
        jar_translation,
        player_summary,
        sibling_instance_warning,
        stays_unchanged: 0,
            apply_status: applied.status,
            apply_message: String::new(),
            pending_overwrites: Vec::new(),
        }, &applied))
}

fn rebuild_zh_from_instance(
    app: &AppHandle,
    session: &TranslateSession,
    dict: &std::collections::HashMap<String, String>,
    actions: &mut Vec<String>,
) -> Result<LangMap, String> {
    let inst = PathBuf::from(session.instance_path.trim());
    if !inst.exists() {
        return Err(format!(
            "無法修復：工作階段記錄的遊戲路徑不存在：\n{}\n\
請改「結果存哪」到含「翻譯工作階段.json」的目錄，或重新「開始一鍵翻譯」。",
            session.instance_path
        ));
    }
    actions.push(format!("從遊戲重建：{}", inst.display()));
    let app_scan = app.clone();
    let (zh_scan, _en, _prov, rep) = scan_instance(&inst, dict, true, true, move |pct, msg| {
        let mapped = 15 + (pct as u16 * 20 / 100) as u8;
        emit_progress(&app_scan, mapped.min(38), msg);
    })?;
    actions.push(format!(
        "本地整理完成：模組 {}、中文 {} 條",
        rep.jars_scanned, rep.keys_zh
    ));
    Ok(zh_scan)
}

/// 建議結果根目錄（實例旁「繁中翻譯輸出」；工具會再建立「翻譯結果」子目錄）
/// B5d 審查 1：只算路徑、不建資料夾（選資料夾與改設定時是查詢，G1.36）；資料夾等開始翻譯時由
/// ensure_result_layout 建。背景執行（遊戲資料夾可能在網路磁碟）。
#[tauri::command]
async fn suggest_resourcepacks_dir(instance_path: String) -> Result<String, String> {
    // 保留舊 command 名以免前端炸掉；語意改為建議「結果根」
    let instance = normalize_path_strict(&instance_path)?;
    tauri::async_runtime::spawn_blocking(move || {
        if !instance.exists() {
            return Err("找不到遊戲資料夾。".to_string());
        }
        Ok(engine::suggest_output_base_path(&instance).display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 「模組整合包旁『繁中翻譯輸出』」模式要用的路徑（只回路徑，不建資料夾：選資料夾、改「翻譯結果放哪裡」
/// 都會呼叫它，是查詢；資料夾等開始翻譯時由 ensure_result_layout 建，B5d 審查 1／G5d.20）。
///
/// 這個指令一直存在，但**從來沒有被加進 `generate_handler!` 清單**——前端呼叫必定失敗、
/// 被 `.catch(() => "")` 吞掉，於是設定裡那個選項按了等於沒按，一律靜默落回 AppData 管理模式。
/// 漏註冊比漏寫更難發現，因為程式碼看起來完全正常；`npm run check:ui` 現在會擋這種漏接。
#[tauri::command]
async fn suggest_output_dir(instance_path: String) -> Result<String, String> {
    suggest_resourcepacks_dir(instance_path).await
}

fn empty_report(mc: &str, keys_zh: usize, namespaces: usize, need_ai: usize) -> ScanReport {
    ScanReport {
        minecraft_dir: mc.to_string(),
        jars_scanned: 0,
        resourcepacks_scanned: 0,
        loose_lang_files: 0,
        namespaces,
        keys_zh,
        keys_need_ai: need_ai,
        keys_from_zh_tw: 0,
        keys_from_zh_cn: 0,
        keys_from_zh_hk_hint: 0,
        keys_tw_playable: 0,
        scan_cache_hits: 0,
        errors: vec![],
    }
}

fn needs_s2tw_key(prov: &ProvenanceMap, ns: &str, key: &str) -> bool {
    prov.get(ns)
        .and_then(|m| m.get(key))
        .copied()
        .map(|s| s.needs_s2tw())
        .unwrap_or(false)
}

/// 為尚無來源標記的 key 補上來源（參考包／接續等）。
fn stamp_missing_provenance(prov: &mut ProvenanceMap, zh: &LangMap, source: LangSource) {
    for (ns, map) in zh {
        let slot = prov.entry(ns.clone()).or_default();
        for k in map.keys() {
            slot.entry(k.clone()).or_insert(source);
        }
    }
}

/// 把本次 pending 中已出現在 zh 的 key 標成 AI（若尚未有來源）。
fn stamp_ai_filled(prov: &mut ProvenanceMap, zh: &LangMap, pending: &LangMap) {
    for (ns, pend) in pending {
        let Some(zh_map) = zh.get(ns) else { continue };
        for k in pend.keys() {
            if zh_map.contains_key(k) {
                let slot = prov.entry(ns.clone()).or_default();
                slot.entry(k.clone()).or_insert(LangSource::Ai);
            }
        }
    }
}

#[tauri::command]
fn has_session(output_dir: String) -> bool {
    let out = normalize_path(&output_dir);
    has_session_file(&out)
}

/// 給前端顯示：工作階段是否存在、在哪
#[tauri::command]
fn session_status(output_dir: String) -> serde_json::Value {
    let out = normalize_path(&output_dir);
    if let Some(p) = find_session_file(&out) {
        serde_json::json!({
            "ok": true,
            "path": p.display().to_string(),
            "message": format!("已找到工作階段：{}", p.display())
        })
    } else {
        serde_json::json!({
            "ok": false,
            "path": null,
            "message": format!("此目錄附近找不到「{}」", SESSION_FILE)
        })
    }
}

fn postprocess_lang_values(zh: &mut LangMap, dict: &HashMap<String, String>) {
    for map in zh.values_mut() {
        for v in map.values_mut() {
            *v = post_one(v, dict);
        }
    }
}

fn post_one(text: &str, dict: &HashMap<String, String>) -> String {
    strip_of_suffix_zhi(&apply_phrase_dict(text, dict))
}

fn apply_minemenu_fixed(instance: &Path, out: &Path) -> Option<String> {
    let mc = resolve_minecraft_dir(instance).ok()?;
    match translate_minemenu(&mc, out, false, None, |_, _| {}) {
        Ok(msg) => Some(msg),
        Err(e) => Some(format!("快捷選單：{e}")),
    }
}

#[tauri::command]
fn scan_only(instance_path: String) -> Result<ScanReport, String> {
    let instance = normalize_path(&instance_path);
    let dict = load_phrase_dict(None);
    let (_zh, _en, _prov, report) = scan_instance(&instance, &dict, true, true, |_, _| {})?;
    Ok(report)
}

#[tauri::command]
fn open_path(path: String) -> Result<bool, String> {
    let p = normalize_path(&path);
    if !p.exists() {
        fs::create_dir_all(&p).map_err(|_| "無法建立這個資料夾。".to_string())?;
    }
    open::that(&p).map_err(|_| "暫時無法開啟這個資料夾。".to_string())?;
    Ok(true)
}

/// 覆寫寫入文字檔（執行日誌等）；單檔上限約 512KB。
#[tauri::command]
fn write_text_file(path: String, content: String) -> Result<bool, String> {
    let p = normalize_path(&path);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("無法建立目錄：{e}"))?;
    }
    const MAX: usize = 512 * 1024;
    let bytes = content.as_bytes();
    let slice = if bytes.len() > MAX {
        // 保留檔頭說明 + 尾端
        let head = content
            .lines()
            .take(20)
            .collect::<Vec<_>>()
            .join("\n");
        let mut out = String::new();
        out.push_str(&head);
        out.push_str("\n…（已截斷以控制檔案大小）…\n");
        let tail_start = content.len().saturating_sub(MAX / 2);
        let tail = &content[tail_start..];
        let boundary = tail.find('\n').map(|i| i + 1).unwrap_or(0);
        out.push_str(&tail[boundary..]);
        out
    } else {
        content
    };
    fs::write(&p, slice).map_err(|e| format!("無法寫入：{e}"))?;
    Ok(true)
}

/// 開啟網址（推廣連結／說明外連）— 僅 http(s)
#[tauri::command]
fn open_url(url: String) -> Result<bool, String> {
    let u = validate_open_url(&url)?;
    open::that(u).map_err(|e| e.to_string())?;
    Ok(true)
}

/// 工具自管的隱藏工作目錄（可攜式根優先，見 `default_managed_work_root`）。
/// 僅供偵測／遷移；一鍵翻譯請用 `managed_output_for_instance`。
#[tauri::command]
fn managed_output_base() -> String {
    default_managed_work_root()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn sanitize_pack_folder_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        // 保留 Unicode 字母數字（含中文）與 -_；符號／™／括號等改成底線或略過
        if c.is_alphanumeric() || matches!(c, '-' | '_') {
            if !c.is_control() {
                out.push(c);
            }
        } else if c.is_whitespace()
            || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '(' | ')' | '[' | ']' | '{' | '}')
        {
            if !out.ends_with('_') {
                out.push('_');
            }
        }
    }
    let trimmed = out.trim_matches('_');
    let mut s: String = trimmed.chars().take(48).collect();
    if s.is_empty() {
        s = "pack".into();
    }
    s
}

fn instance_path_key(instance_path: &Path) -> (PathBuf, String, String) {
    use std::hash::{Hash, Hasher};
    let stable = fs::canonicalize(instance_path).unwrap_or_else(|_| instance_path.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    stable.to_string_lossy().to_ascii_lowercase().hash(&mut hasher);
    let hash = hasher.finish();
    let hash16 = format!("{hash:016x}");
    let hash8 = format!("{hash:016x}")[..8].to_string();
    (stable, hash16, hash8)
}

fn default_managed_work_root() -> Option<PathBuf> {
    Some(engine::paths::resolve_file(Path::new("work")))
}

/// 每個整合包獨立結果根：`{work_base}/packs/{安全名}-{hash8}`。
/// 預設 work_base＝`%APPDATA%\modpack-i18n-tool\work`；可改成使用者自訂根。
/// 僅在預設 work 下，若舊路徑 `instance-{hash16}` 已有工作階段／說明檔則優先沿用。
fn managed_output_for_instance_at(instance_path: &Path, work_base: Option<&Path>) -> String {
    let (stable, hash16, hash8) = instance_path_key(instance_path);
    let Some(default_work) = default_managed_work_root() else {
        return String::new();
    };
    let using_default = work_base
        .map(|b| {
            let nb = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
            let nd = fs::canonicalize(&default_work).unwrap_or_else(|_| default_work.clone());
            path_keys_equal(&nb, &nd)
        })
        .unwrap_or(true);
    let work = work_base
        .filter(|b| !b.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or(default_work);

    if using_default {
        let legacy = work.join(format!("instance-{hash16}"));
        let legacy_has_work = legacy.join(SESSION_FILE).is_file()
            || legacy.join(RESULT_DIR_NAME).join(SESSION_FILE).is_file()
            || legacy.join("【請閱讀】輸出說明.txt").is_file()
            || legacy.join(RESULT_DIR_NAME).join("【請閱讀】輸出說明.txt").is_file();
        if legacy_has_work {
            return legacy.display().to_string();
        }
    }

    let name = stable
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("pack");
    let folder = format!("{}-{}", sanitize_pack_folder_name(name), hash8);
    work.join("packs").join(folder).display().to_string()
}

/// 每個整合包獨立結果根：`work/packs/{安全名}-{hash8}`。
/// 若舊路徑 `work/instance-{hash16}` 已有工作階段／說明檔則優先沿用。
#[tauri::command]
fn managed_output_for_instance(instance_path: String) -> String {
    let path = normalize_path(&instance_path);
    managed_output_for_instance_at(&path, None)
}

/// 在指定根目錄下為整合包配置獨立結果資料夾（設定「自訂已翻譯儲存位置」用）。
#[tauri::command]
fn managed_output_for_instance_with_base(instance_path: String, base_dir: String) -> String {
    let path = normalize_path(&instance_path);
    let base = normalize_path(&base_dir);
    if base.as_os_str().is_empty() {
        return managed_output_for_instance_at(&path, None);
    }
    managed_output_for_instance_at(&path, Some(&base))
}

fn path_keys_equal(a: &Path, b: &Path) -> bool {
    let na = fs::canonicalize(a)
        .unwrap_or_else(|_| a.to_path_buf())
        .to_string_lossy()
        .to_ascii_lowercase();
    let nb = fs::canonicalize(b)
        .unwrap_or_else(|_| b.to_path_buf())
        .to_string_lossy()
        .to_ascii_lowercase();
    na.trim_end_matches(['/', '\\']) == nb.trim_end_matches(['/', '\\'])
}

#[cfg(test)]
mod managed_output_tests {
    use super::sanitize_pack_folder_name;

    #[test]
    fn sanitize_keeps_cjk_and_strips_junk() {
        let s = sanitize_pack_folder_name("Prominence™ II- Hasturian Era(1)");
        assert!(s.contains("Prominence"));
        assert!(!s.contains('™'));
        assert!(!s.contains('('));
    }
}

#[cfg(test)]
mod local_cache_probe_tests {
    use super::*;
    use engine::{save_session, TranslateSession};

    fn base_session(instance: &Path, pending_count: usize, keys_zh: usize) -> TranslateSession {
        // 缺口數量現在是從 `pending_en` 實際算出來的，不是讀那個可能過期的
        // `pending_count` 純量——所以測試也要放進對應數量的**真的可翻**的條目，
        // 否則測到的是「空清單」而不是「還有 N 條沒翻」。
        let mut ns: std::collections::HashMap<String, String> = Default::default();
        for i in 0..pending_count {
            ns.insert(format!("item.test{i}"), format!("Untranslated item {i}"));
        }
        let mut pending_en: engine::LangMap = Default::default();
        if pending_count > 0 {
            pending_en.insert("test".into(), ns);
        }
        TranslateSession {
            version: 1,
            review_pass: 0,
            instance_path: instance.display().to_string(),
            output_dir: String::new(),
            pack_name: "測試包".into(),
            pack_path: String::new(),
            pending_en,
            pending_count,
            quality_deferred: Default::default(),
            keys_zh,
            keys_hk_hint: 0,
            note: String::new(),
            target_version: None,
            translation_mode: "append".into(),
            translation_quality: "balanced".into(),
            coverage_tier: "max".into(),
            mods_fingerprint: 0,
            run_preferences: engine::RunPreferences::default(),
            last_run_outcome: engine::RunOutcome::Completed,
        }
    }

    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("mcpl-cache-probe-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let instance = root.join("instance");
        let work = root.join("翻譯結果");
        fs::create_dir_all(&instance).unwrap();
        fs::create_dir_all(&work).unwrap();
        (instance, work)
    }

    /// 這幾條釘死本輪的分級訊息：同樣是「partial」，剩 1% 跟剩 60% 跟剩大半，
    /// 講的話要不一樣，不能一律「尚有約 X 條可接續補翻」。
    #[test]
    fn near_complete_says_almost_done_not_still_pending() {
        let (instance, work) = scratch("near-complete");
        // 99% 完成：keys_zh=9900, pending=100
        save_session(&work, &base_session(&instance, 100, 9900)).unwrap();
        let probe = probe_cache_at(&instance, &work).expect("should find a partial probe");
        assert_eq!(probe.status, "partial");
        assert_eq!(probe.completion_percent, Some(99));
        assert!(probe.message.contains("幾乎全部翻完"), "{}", probe.message);
        assert!(!probe.message.contains("尚有約"), "{}", probe.message);
        let _ = fs::remove_dir_all(work.parent().unwrap());
    }

    #[test]
    fn majority_done_says_completed_most_not_almost_done() {
        let (instance, work) = scratch("majority");
        // 70% 完成
        save_session(&work, &base_session(&instance, 300, 700)).unwrap();
        let probe = probe_cache_at(&instance, &work).unwrap();
        assert_eq!(probe.completion_percent, Some(70));
        assert!(probe.message.contains("已完成大半"), "{}", probe.message);
        assert!(!probe.message.contains("幾乎全部翻完"), "{}", probe.message);
        let _ = fs::remove_dir_all(work.parent().unwrap());
    }

    /// 站長實測的情境：翻譯在 63% 崩潰，工作階段留下 `pendingCount=33233`
    /// 這個「本地整理完成當下」的舊快照。但那一次的日誌顯示其中 32151 條
    /// 早就被共享庫補上了——重開工具卻說「還有三萬多條缺漏」。
    ///
    /// 沒跑完的計數一律不可信，這張卡就不該拿它嚇人。
    #[test]
    fn a_crashed_run_does_not_report_phantom_gaps() {
        let (instance, work) = scratch("crashed");
        let mut session = base_session(&instance, 33233, 75141);
        session.last_run_outcome = engine::RunOutcome::Crashed;
        save_session(&work, &session).unwrap();
        // 有可分享的成品在（跟站長那份一樣）
        let rp = work.join("resourcepacks");
        fs::create_dir_all(&rp).unwrap();
        fs::write(rp.join("模組包翻譯工具+0902+R1.zip"), b"pack").unwrap();

        let probe = probe_cache_at(&instance, &work).unwrap();
        assert_eq!(probe.status, "ready", "有成品就講成品，別拿過期計數判成 partial");
        assert!(
            !probe.message.contains("33233"),
            "不可以把崩潰前的舊數字丟給使用者：{}",
            probe.message
        );
        assert!(
            !probe.message.contains("缺") && !probe.message.contains("還有約"),
            "沒跑完的情況交給接續卡講，這張卡只講有結果可用：{}",
            probe.message
        );
        let _ = fs::remove_dir_all(work.parent().unwrap());
    }

    /// 跑完了、而且剩下的全是「本來就不該翻」的東西 → 就是已完成。
    #[test]
    fn untranslatable_leftovers_do_not_keep_the_card_in_partial_forever() {
        let (instance, work) = scratch("untranslatable");
        let mut session = base_session(&instance, 0, 9000);
        let mut ns: std::collections::HashMap<String, String> = Default::default();
        ns.insert("enchantment.level.9".into(), "IX".into());
        ns.insert("icon.star".into(), "§f".into());
        ns.insert("mod.optifine".into(), "OptiFine".into());
        session.pending_en.insert("test".into(), ns);
        session.pending_count = 3;
        session.last_run_outcome = engine::RunOutcome::Completed;
        save_session(&work, &session).unwrap();
        let rp = work.join("resourcepacks");
        fs::create_dir_all(&rp).unwrap();
        fs::write(rp.join("模組包翻譯工具+0902+R1.zip"), b"pack").unwrap();

        let probe = probe_cache_at(&instance, &work).unwrap();
        assert_eq!(probe.pending_count, 0, "羅馬數字／圖示／品牌名不算缺漏");
        assert_eq!(probe.status, "ready");
        let _ = fs::remove_dir_all(work.parent().unwrap());
    }

    #[test]
    fn low_completion_keeps_original_wording() {
        let (instance, work) = scratch("low");
        // 20% 完成
        save_session(&work, &base_session(&instance, 800, 200)).unwrap();
        let probe = probe_cache_at(&instance, &work).unwrap();
        assert_eq!(probe.completion_percent, Some(20));
        // 這條守的是「分級」：才 20% 完成時不可以講得像快翻完了
        assert!(probe.message.contains("還有約"), "{}", probe.message);
        assert!(!probe.message.contains("幾乎") && !probe.message.contains("大半"), "{}", probe.message);
        let _ = fs::remove_dir_all(work.parent().unwrap());
    }

    #[test]
    fn take_capped_langmap_is_bounded_and_deterministic() {
        let mut src: LangMap = HashMap::new();
        for ns in ["bbb", "aaa", "ccc"] {
            let m = src.entry(ns.into()).or_default();
            for i in 0..10 {
                m.insert(format!("key{i:02}"), format!("val{i}"));
            }
        }
        assert_eq!(count_map(&src), 30);

        let taken = take_capped_langmap(&src, 12);
        assert_eq!(count_map(&taken), 12, "必須剛好取到上限");
        // 同一份輸入要每次取到同一批，使用者連跑兩次看到的數字才一致
        assert_eq!(taken, take_capped_langmap(&src, 12));
        // 依命名空間字典序，前 10 條應該全部來自 aaa
        assert_eq!(taken["aaa"].len(), 10);
        assert_eq!(taken["bbb"].len(), 2);
        assert!(!taken.contains_key("ccc"));

        assert!(take_capped_langmap(&src, 0).is_empty());
        assert_eq!(count_map(&take_capped_langmap(&src, 999)), 30);
    }

    #[test]
    fn different_mods_under_same_path_is_not_treated_as_cached() {
        // 這條釘死上一輪加的防呆：同一個 instance 路徑，換了完全不同的 mods，
        // 不該被當成「同一包」而顯示已有翻譯。
        let (instance, work) = scratch("mods-changed");
        let mods = instance.join("mods");
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("jei.jar"), vec![0u8; 1000]).unwrap();
        let mut session = base_session(&instance, 100, 900);
        session.mods_fingerprint = engine::mods_fingerprint(&instance);
        save_session(&work, &session).unwrap();

        // 換掉整批 mods（同一個 instance 路徑）
        let _ = fs::remove_dir_all(&mods);
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("totally-different-pack.jar"), vec![0u8; 9999]).unwrap();

        // B5d：不再回 None（那會讓畫面像沒翻過），改回「有變動」；但仍不是可用的結果
        let probe = probe_cache_at(&instance, &work).expect("同一個遊戲資料夾 mods 變了要照實回報");
        assert_eq!(probe.status, "changed");
        assert!(probe.mods_changed && !probe.matched && !probe.shareable && !probe.applyable, "{probe:?}");
        let _ = fs::remove_dir_all(work.parent().unwrap());
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteResultFolderResult {
    deleted: bool,
    path: String,
    player_summary: String,
}

fn result_work_root(path: &Path) -> PathBuf {
    if path.file_name().and_then(|name| name.to_str()) == Some(RESULT_DIR_NAME) {
        path.to_path_buf()
    } else {
        path.join(RESULT_DIR_NAME)
    }
}

/// 刪除工具建立的整個翻譯結果工作根，不會刪除使用者選的上層資料夾。
#[tauri::command]
fn delete_result_folder_cmd(output_dir: String) -> Result<DeleteResultFolderResult, String> {
    let base = normalize_path_strict(&output_dir)?;
    let target = result_work_root(&base);
    if target.parent().and_then(Path::parent).is_none() {
        return Err("這個位置太接近磁碟根目錄，為了安全不能刪除。".into());
    }
    if !target.exists() {
        return Ok(DeleteResultFolderResult {
            deleted: false,
            path: target.display().to_string(),
            player_summary: "沒有找到可刪除的翻譯結果資料夾。".into(),
        });
    }
    if !target.is_dir() {
        return Err("翻譯結果位置不是資料夾，無法刪除。".into());
    }
    let looks_like_result = target.join(SESSION_FILE).is_file()
        || target.join("【請閱讀】輸出說明.txt").is_file()
        || target.join("resourcepacks").is_dir();
    if !looks_like_result {
        return Ok(DeleteResultFolderResult {
            deleted: false,
            path: target.display().to_string(),
            player_summary: "這個位置沒有本工具的翻譯結果，沒有刪除任何檔案。".into(),
        });
    }
    fs::remove_dir_all(&target).map_err(|e| format!("刪除翻譯結果資料夾失敗：{e}"))?;
    Ok(DeleteResultFolderResult {
        deleted: true,
        path: target.display().to_string(),
        player_summary: format!("已完整刪除翻譯結果資料夾：{}", target.display()),
    })
}

/// 找一個還沒被用過的結果資料夾（`{原本位置}-2`、`-3`…），供「另存一份新的」。
///
/// 使用者想比較兩次翻譯（例如換了 AI 來源）時，舊結果必須完整保留。
/// 從 2 開始找，最多找到 99；都被占用就回錯誤讓呼叫端退回覆蓋行為。
#[tauri::command]
fn next_result_dir_cmd(output_dir: String) -> Result<String, String> {
    let base = normalize_path_strict(&output_dir)?;
    let parent = base
        .parent()
        .ok_or_else(|| "這個位置太接近磁碟根目錄，無法另存新資料夾。".to_string())?;
    let stem = base
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "無法解析結果資料夾名稱。".to_string())?;
    for n in 2..=99u32 {
        let candidate = parent.join(format!("{stem}-{n}"));
        if !candidate.exists() {
            fs::create_dir_all(&candidate)
                .map_err(|e| format!("無法建立新的結果資料夾：{e}"))?;
            return Ok(candidate.display().to_string());
        }
    }
    Err("已經有太多份結果資料夾（-2 到 -99 都被占用），請先整理舊的。".into())
}

/// 寫入本次執行紀錄（不覆寫舊的），同時維持舊的 `執行日誌.txt` 相容行為。
///
/// 使用者實測遇到「翻到一半被關掉、重開續翻，前一小時的紀錄整個被蓋掉」——
/// 執行紀錄改成每次一個檔，出問題時才有東西可查。
#[tauri::command]
fn write_run_journal_cmd(
    work_root: String,
    stamp: String,
    content: String,
) -> Result<String, String> {
    let root = normalize_path(&work_root);
    let safe_stamp: String = stamp
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(32)
        .collect();
    let stamp = if safe_stamp.is_empty() {
        "run".to_string()
    } else {
        safe_stamp
    };
    engine::write_run_log(&root, &stamp, &content).map(|p| p.display().to_string())
}

/// 列出既有的執行紀錄（新到舊），供診斷與回報取用。
#[tauri::command]
fn list_run_journals_cmd(work_root: String) -> Vec<String> {
    engine::list_runs(&normalize_path(&work_root))
        .into_iter()
        .map(|p| p.display().to_string())
        .collect()
}

/// 讀出待補項目的 CSV 文字，供「複製失敗項目」直接進剪貼簿。
///
/// 使用者反映：想把沒翻到的拿去線上 AI 翻，但只能一個一個開檔案複製「有點慘」。
#[tauri::command]
fn failed_items_csv_cmd(output_dir: String) -> Result<String, String> {
    let work = result_work_root(&normalize_path_strict(&output_dir)?);
    let (session, _) = load_session(&work)?;
    Ok(engine::build_failed_items_csv(
        &session.pending_en,
        "尚未翻譯或品質未通過",
    ))
}

/// 把使用者在線上翻好、貼回來的內容併入翻譯結果並重建資源包。
///
/// 一定會逐條驗證佔位符——線上 AI 很容易把 `%s`／`§a` 弄丟，直接寫進遊戲
/// 會讓文字格式錯亂。沒過的原樣退回並列出來，不靜默吞掉。
#[tauri::command]
async fn import_translations_cmd(
    app: AppHandle,
    output_dir: String,
    text: String,
) -> Result<engine::ImportReport, String> {
    // B2：每輪開始先清空上一輪的退回紀錄
    engine::begin_guard_run();
    let out = normalize_path_strict(&output_dir)?;
    tauri::async_runtime::spawn_blocking(move || {
        let work = result_work_root(&out);
        let (mut session, _) = load_session(&work)?;
        let entries = engine::parse_import_text(&text);
        if entries.is_empty() {
            return Err("看不出可匯入的內容。請貼「鍵<Tab>譯文」每行一條，或直接貼回匯出的那張表。".into());
        }
        let mut zh = load_pack_zh(&work).unwrap_or_default();
        // B2：建包前載入英文原文全表，重新寫出的舊譯文也完整檢查
        let catalog = engine::source_catalog::prepare_build_sources(&work, Path::new(session.instance_path.trim()));
        if let Some(note) = catalog.player_note() {
            emit_log(&app, "info", note);
        }
        let report = engine::merge_imported(&mut zh, &session.pending_en, &entries);
        if report.accepted == 0 {
            return Ok(report);
        }
        // 併入後把已完成的從待補移除，並重建資源包
        for (ns, map) in &zh {
            if let Some(pending_ns) = session.pending_en.get_mut(ns) {
                for key in map.keys() {
                    pending_ns.remove(key);
                }
            }
        }
        session.pending_en.retain(|_, m| !m.is_empty());
        session.pending_count = count_map(&session.pending_en);
        let pack_format = session_pack_format(&session);
        let built = build_resource_pack(
            &zh,
            &BuildOptions {
                pack_folder_name: session.pack_name.clone(),
                pack_description: "台灣用語繁體中文翻譯資源包".into(),
                output_dir: work.display().to_string(),
                pack_format,
                target_version: session.target_version.clone(),
            },
        )?;
        session.keys_zh = built.keys_total;
        let _ = save_session(&work, &session);
        emit_log(
            &app,
            "info",
            &format!("匯入完成：{}。資源包已重建，請重新套用或啟動遊戲。", report.summary),
        );
        Ok(report)
    })
    .await
    .map_err(|e| format!("匯入工作中斷：{e}"))?
}

/// 檢查這個實例的資源包清單是否健康（是否有檔案在但沒啟用、或整個清單空掉）。
#[tauri::command]
fn verify_resource_packs_cmd(instance_path: String) -> Result<engine::PackHealthReport, String> {
    let instance = normalize_path_strict(&instance_path)?;
    let mc = resolve_minecraft_dir(&instance).unwrap_or(instance);
    Ok(engine::check_pack_health(&mc))
}

/// 把資料夾裡有、但 options.txt 沒啟用的資源包加回清單。
///
/// 使用者的實例被清成 `resourcePacks:[]` 之後遊戲直接閃退（字體找不到材質 →
/// 資源重載失敗 → 模型沒烘焙 → 標題畫面空指標）。這個指令把它救回來。
#[tauri::command]
fn repair_resource_packs_cmd(instance_path: String) -> Result<serde_json::Value, String> {
    let instance = normalize_path_strict(&instance_path)?;
    let mc = resolve_minecraft_dir(&instance).unwrap_or(instance);
    let outcome = engine::repair_pack_list(&mc)?;
    let added = outcome.added;
    let after = engine::check_pack_health(&mc);
    let summary = if added == 0 {
        "資源包清單本來就是完整的，沒有做任何修改。".to_string()
    } else {
        format!("已把 {added} 個資源包加回清單，請重新啟動遊戲確認。")
    };
    // 識別碼認回的說明（部分檔案被整合包更新改過）放在最前面，玩家才看得到
    let summary = match outcome.notice {
        Some(notice) => format!("{notice}\n\n{summary}"),
        None => summary,
    };
    Ok(serde_json::json!({
        "added": added,
        "enabledCount": after.enabled_count,
        "summary": summary,
    }))
}

/// 目前資料存放位置的實況，供設定頁顯示與判斷要不要提供搬移。
#[tauri::command]
fn data_root_info_cmd() -> serde_json::Value {
    let portable = engine::paths::portable_root();
    let legacy = engine::paths::legacy_roaming_root();
    let active = engine::paths::active_root();
    let using_portable = engine::paths::is_using_portable_root();
    serde_json::json!({
        "activeRoot": active.display().to_string(),
        "portableRoot": portable.display().to_string(),
        "legacyRoot": legacy.display().to_string(),
        "usingPortable": using_portable,
        // 只有「還在用舊位置、而且舊位置真的有東西」時才值得提供搬移
        "canMigrate": !using_portable && legacy.is_dir(),
        "legacySizeBytes": engine::paths::dir_size_bytes(&legacy),
    })
}

/// 把舊資料（%APPDATA%）複製到工具旁的資料夾。舊的原地保留當備份。
#[tauri::command]
async fn migrate_data_root_cmd() -> Result<serde_json::Value, String> {
    // B5a-2：翻譯中不搬（正在寫翻譯記憶與紀錄）。設定視窗也會先停用按鈕，這裡是最後一道
    if TRANSLATION_ACTIVE.load(Ordering::Relaxed) {
        return Err("正在翻譯，翻完才能搬移工具資料。".into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        engine::paths::migrate_legacy_to_portable().map(|(files, bytes)| {
            serde_json::json!({
                "files": files,
                "bytes": bytes,
                "newRoot": engine::paths::portable_root().display().to_string(),
                "oldRoot": engine::paths::legacy_roaming_root().display().to_string(),
            })
        })
    })
    .await
    .map_err(|e| format!("搬移工作中斷：{e}"))?
}

/// 設定「要不要把 API 金鑰存到這台電腦」。
///
/// 預設 false＝不落地：金鑰只放在記憶體，工具關掉就沒了，下次要重新輸入。
/// 使用者要求「本地永不儲存 apikey」，但既有已存金鑰的人直接改掉會突然不能用，
/// 所以做成可選，由前端在啟動時把設定推下來。
#[tauri::command]
fn set_remember_api_key_cmd(remember: bool) -> bool {
    engine::set_remember_api_key(remember);
    engine::remember_api_key()
}

/// 讀取工具設定檔。沒有檔案／檔案壞掉都回 null（不是錯誤），
/// 讓前端知道要走 localStorage 遷移路徑。
#[tauri::command]
fn read_app_settings_cmd() -> serde_json::Value {
    engine::read_settings()
}

/// 讀取工具設定檔，附帶健康狀態。
///
/// 回 `{ settings, status, path, backup, detail }`。`status` 為 `corrupt`／`unreadable`
/// 時前端必須明講——舊版讀壞檔只回 null，接著就被當成「還沒有設定檔」把偏好全部
/// 蓋回預設值，使用者完全不知道發生什麼事。
#[tauri::command]
fn read_app_settings_report_cmd() -> serde_json::Value {
    engine::read_settings_report()
}

/// 依路徑合併寫入工具設定檔：`[{ path, value }]` 設值、`[{ path, delete: true }]` 刪除。
/// 值是 null 的項目不會寫入。回 `{ path, settings }`（合併後的整份設定）。
#[tauri::command]
fn patch_app_settings_cmd(ops: Vec<engine::SettingsPatchOp>) -> Result<serde_json::Value, String> {
    let (path, settings) = engine::patch_settings(&ops)?;
    Ok(serde_json::json!({ "path": path.display().to_string(), "settings": settings }))
}

/// 清除已記住的自訂 API 金鑰（記憶體與設定檔都清）。
#[tauri::command]
fn clear_api_key_cmd() -> Result<(), String> {
    engine::clear_api_key()
}

/// 設定檔路徑（即使檔案還不存在也回傳預期位置，供設定頁顯示與「開啟資料夾」）。
#[tauri::command]
fn app_settings_path_cmd() -> String {
    engine::settings_path().display().to_string()
}

/// 這個遊戲資料夾寫得進去嗎？**選完資料夾就問**，不要等翻完三小時才失敗。
#[tauri::command(async)]
fn check_write_access_cmd(instance_path: String) -> serde_json::Value {
    let path = normalize_path(&instance_path);
    serde_json::to_value(engine::check_write_access(&path))
        .unwrap_or_else(|_| serde_json::json!({ "writable": true, "needsAdmin": false }))
}

/// 以系統管理員身分重新開啟工具。
///
/// 回 `{ relaunching }`：`false` 代表使用者在 UAC 按了取消——**那不是錯誤**，
/// 前端只要回到原畫面，讓他改選別的資料夾就好。
#[tauri::command]
fn relaunch_as_admin_cmd(app: AppHandle, instance_path: String) -> Result<serde_json::Value, String> {
    let relaunching = engine::relaunch_as_admin(&instance_path)?;
    if relaunching {
        shutdown_side_processes();
        let exit_app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            exit_app.exit(0);
        });
    }
    Ok(serde_json::json!({ "relaunching": relaunching }))
}

/// 開發人員測試模式的現況。
///
/// 回 `{ eligible, enabled, logPath }`。`eligible` 為 false 時前端**完全不顯示**
/// 這個選項——詳細紀錄會拖慢速度、產生大量檔案，對一般玩家有害無益。
#[tauri::command]
fn dev_mode_status_cmd() -> serde_json::Value {
    serde_json::json!({
        "eligible": engine::dev_mode_eligible(),
        "enabled": engine::dev_mode_enabled(),
        "logPath": engine::dev_mode_log_path().display().to_string(),
    })
}

/// 開關開發人員測試模式。沒有資格時回錯誤，不是靜默忽略。
#[tauri::command]
fn dev_mode_set_cmd(enabled: bool) -> Result<serde_json::Value, String> {
    let on = engine::dev_mode_set_enabled(enabled)?;
    Ok(serde_json::json!({
        "enabled": on,
        "logPath": engine::dev_mode_log_path().display().to_string(),
    }))
}

/// 檢查選取的位置是不是一個可直接安裝的遊戲實例（找得到 minecraft 目錄）。
/// 回 { ok, mcDir, hasResourcepacks }，讓前端決定要不要走「直接覆蓋安裝、不建資料夾」。
#[tauri::command(async)]
fn check_install_target(instance_path: String) -> serde_json::Value {
    match resolve_minecraft_dir(&PathBuf::from(&instance_path)) {
        Ok(mc) => {
            let has_rp = mc.join("resourcepacks").is_dir();
            serde_json::json!({
                "ok": true,
                "mcDir": mc.display().to_string(),
                "hasResourcepacks": has_rp
            })
        }
        Err(e) => serde_json::json!({ "ok": false, "error": e }),
    }
}

/// 嚴格驗證遊戲資料夾是否可開始翻譯（mods＋遊戲資料夾特徵）；選資料夾與開始翻譯共用。
/// B5d：背景執行並設上限（斷線的網路磁碟不卡畫面），逾時回「連不到這個資料夾」。
#[tauri::command]
async fn validate_instance_cmd(instance_path: String) -> Result<InstanceValidation, String> {
    let path = normalize_user_path(&instance_path)?;
    let network = engine::folder_check::is_network_path(&path);
    let verdict = tauri::async_runtime::spawn_blocking(move || {
        engine::folder_check::run_with_timeout(engine::folder_check::INSPECT_TIMEOUT, move || validate_instance_path(&path))
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(verdict.unwrap_or_else(|| InstanceValidation::unreachable(network)))
}

/// B5d 選資料夾就判定（瀏覽、手動輸入、上次共用）：驗證、資料夾形狀（選到 mods、啟動器清單、伺服器）、
/// 寫入檢查。全部只讀（G1.36），背景執行並設上限；逾時回 reachable=false。
#[tauri::command]
async fn inspect_folder_cmd(instance_path: String) -> Result<engine::folder_check::FolderInspection, String> {
    let path = normalize_user_path(&instance_path)?;
    let job_path = path.clone();
    let verdict = tauri::async_runtime::spawn_blocking(move || {
        engine::folder_check::run_with_timeout(engine::folder_check::INSPECT_TIMEOUT, move || {
            engine::folder_check::inspect_folder(&job_path)
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(verdict.unwrap_or_else(|| engine::folder_check::unreachable_inspection(&path)))
}

/// B5d 選資料夾時的身分判斷（規格 S05–S07）：標記壞、紀錄壞、整份複製來的、原位置連不到。
/// 只讀（不建 `.mcpl`、不認回、不改紀錄）；原位置檢查放背景、設上限。整體逾時回 unknown（照舊放行，套用前仍會檢查）。
#[tauri::command]
async fn inspect_instance_identity_cmd(instance_path: String) -> Result<engine::folder_identity::IdentityCheck, String> {
    let path = normalize_user_path(&instance_path)?;
    let verdict = tauri::async_runtime::spawn_blocking(move || {
        engine::folder_check::run_with_timeout(engine::folder_check::INSPECT_TIMEOUT * 2, move || {
            let mc = resolve_minecraft_dir(&path).unwrap_or(path);
            engine::folder_identity::inspect_identity(&mc, engine::folder_check::INSPECT_TIMEOUT)
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(verdict.unwrap_or_else(engine::folder_identity::IdentityCheck::unknown))
}

/// 瀏覽視窗沒有上次路徑時的起始位置：偵測到的常見啟動器資料夾（只讀；找不到回 null）。
#[tauri::command(async)]
fn common_launcher_dir_cmd() -> Option<String> {
    engine::folder_check::common_launcher_dir().map(|p| p.display().to_string())
}

/// 把「翻譯結果」打包成單一 zip，供使用者手動分享整包翻譯檔。
/// 只有勾「建立打包檔案」才會用到；預設一鍵流程是直接覆蓋安裝進遊戲、不打包。
/// `work_root`＝翻譯結果資料夾（通常是工作階段的 output_dir）。
#[tauri::command]
fn create_share_package(work_root: String, dest_dir: String, name: String) -> Result<String, String> {
    let zip = package_translation(&PathBuf::from(&work_root), &PathBuf::from(&dest_dir), &name)?;
    Ok(zip.display().to_string())
}

#[tauri::command]
fn has_shareable_translation_cmd(work_root: String) -> Result<bool, String> {
    let work = normalize_path_strict(&work_root)?;
    if !has_shareable_content(&work) {
        return Ok(false);
    }
    // 有工具資源包時須能解析出 canonical zip，避免分享檔夾帶多版本。
    let rp = work.join("resourcepacks");
    if rp.is_dir() {
        let has_tool = fs::read_dir(&rp)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .any(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let stem = name
                    .trim_end_matches(".zip")
                    .trim_end_matches(".ZIP");
                is_tool_resource_pack(stem)
            });
        if has_tool && resolve_canonical_tool_zip(&work).is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalPackCacheProbe {
    /// ready＝可直接打開／套用／分享；partial＝有工作階段可補翻；none＝沒找到
    status: String,
    matched: bool,
    /// B5d：同一個遊戲資料夾、但上次翻譯後 mods 變了（status＝"changed"）。這時不是「已有可用結果」。
    mods_changed: bool,
    output_dir: String,
    work_root: String,
    session_path: Option<String>,
    pending_count: usize,
    /// 已翻譯／（已翻譯＋待補）的粗估百分比；算不出來（沒有工作階段）時是 None。
    completion_percent: Option<u8>,
    shareable: bool,
    applyable: bool,
    updated_at_ms: Option<u64>,
    message: String,
    pack_name: Option<String>,
    canonical_zip: Option<String>,
    /// B5c 審查 3a：條數與比例可信嗎（上一輪沒跑完＝Aborted 時不可信，前端不寫數字）
    counts_trusted: bool,
    /// B5c 審查 3a：最新一輪結果有沒有套用到這個遊戲資料夾（唯讀比對；不知道＝None）
    last_applied: Option<bool>,
}

fn file_mtime_ms(path: &Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(dur.as_millis() as u64)
}

fn probe_cache_at(instance: &Path, output_dir: &Path) -> Option<LocalPackCacheProbe> {
    if output_dir.as_os_str().is_empty() {
        return None;
    }
    let work = result_work_root(output_dir);
    let session_path = find_session_file(&work).or_else(|| find_session_file(output_dir));
    let canonical = resolve_canonical_tool_zip(&work);
    let canonical_zip = canonical.as_ref().and_then(|p| {
        p.file_name().map(|s| {
            let n = s.to_string_lossy();
            if p.is_dir() {
                format!("{n}.zip")
            } else {
                n.to_string()
            }
        })
    });
    let mut pack_name = None::<String>;
    if let Ok((session, _)) = load_session(&work) {
        let name = session.pack_name.trim();
        if !name.is_empty() {
            pack_name = Some(name.to_string());
        }
    }
    let shareable = has_shareable_content(&work) || has_shareable_content(output_dir);
    // 上一次到底有沒有跑完？沒跑完的計數一律不可信（見 gap_model 的說明）。
    let mut counts_fresh = false;
    let mut mods_changed_here = false;
    let (matched, pending_count, session_path_str, updated_at_ms, keys_zh) =
        if let Some(ref sp) = session_path {
            match load_session(sp.parent().unwrap_or(&work)) {
                Ok((session, path)) => {
                    let path_matched = path_keys_equal(
                        Path::new(session.instance_path.trim()),
                        instance,
                    );
                    // 路徑相同不代表還是同一包：啟動器常見操作是沿用同一個 instance
                    // 資料夾、換掉整個 mods/。只在兩邊都「有記錄」時才拿來否決——
                    // 0 代表舊工作階段（遷移前）或當下讀不到 mods/，一律不擋。
                    let live_fingerprint = engine::mods_fingerprint(instance);
                    let mods_changed = session.mods_fingerprint != 0
                        && live_fingerprint != 0
                        && session.mods_fingerprint != live_fingerprint;
                    let matched = path_matched && !mods_changed;
                    mods_changed_here = path_matched && mods_changed;
                    counts_fresh = session.last_run_outcome.counts_are_trustworthy();
                    // 只算「補得動」的缺口：羅馬數字、圖示、單位、品牌名本來就
                    // 不該翻，算進去的話使用者永遠看到一個補不完的數字。
                    let actionable = engine::count_gaps(&session.pending_en).actionable;
                    (
                        matched,
                        actionable,
                        Some(path.display().to_string()),
                        file_mtime_ms(&path),
                        session.keys_zh,
                    )
                }
                Err(_) => (false, 0, Some(sp.display().to_string()), file_mtime_ms(sp), 0),
            }
        } else {
            // 沒有工作階段但仍有可分享產物：視為同路徑結果（輸出根對得上）
            (shareable, 0, None, file_mtime_ms(&work), 0)
        };
    // 完成度＝已翻譯／（已翻譯＋待補）。0＝算不出來（沒有工作階段），前端不顯示百分比。
    let completion_percent: Option<u8> = if keys_zh + pending_count > 0 {
        Some(((keys_zh * 100) / (keys_zh + pending_count)).min(100) as u8)
    } else {
        None
    };

    // B5d：同一個遊戲資料夾、mods 變了 → 照實回「有變動」（舊版回 None，畫面看起來像沒翻過）。
    // 這不是可用的結果：不可分享、不可直接套用，兩個消費端（開始翻譯的三選一、本機已有翻譯卡）都不當成已有結果。
    if mods_changed_here {
        return Some(LocalPackCacheProbe {
            status: "changed".into(),
            matched: false,
            mods_changed: true,
            output_dir: output_dir.display().to_string(),
            work_root: work.display().to_string(),
            session_path: session_path_str,
            pending_count: 0,
            completion_percent: None,
            shareable: false,
            applyable: false,
            updated_at_ms,
            message: "上次翻譯後模組整合包有變動，要重新翻譯。".into(),
            pack_name,
            canonical_zip,
            counts_trusted: false,
            last_applied: None,
        });
    }
    if !matched && !shareable {
        return None;
    }
    // 工作階段屬於別的實例 → 略過
    if session_path.is_some() && !matched {
        return None;
    }

    let applyable = shareable
        || work.join("resourcepacks").is_dir()
        || output_dir.join(RESULT_DIR_NAME).join("resourcepacks").is_dir();
    // 沒跑完的那一次留下的計數是過期的（AI 與共享庫都還沒把它補掉），
    // 拿來對使用者講缺漏就會出現「還有三萬多條」這種假數字。
    // 那種情況交給「上次的翻譯沒有做完」接續卡處理，這張卡只講「有結果可以用」。
    let trust_counts = counts_fresh || session_path.is_none();
    let status = if shareable && (pending_count == 0 || !trust_counts) {
        "ready"
    } else if shareable || session_path.is_some() {
        "partial"
    } else {
        "none"
    };
    if status == "none" {
        return None;
    }
    // 訊息依完成度分級：同樣是「partial」，剩 5 條跟剩 5000 條給使用者的感受完全不同——
    // 舊版一律講「尚有約 X 條可接續補翻」，99% 完成時這句話讀起來像還差很多。
    let message = match status {
        "ready" if shareable && !trust_counts => {
            // 有可用的成品，但計數不新鮮：講成品，不講數字
            "這個整合包在你的電腦已經有翻譯結果，可以直接套用、打開或打包分享。".into()
        }
        "ready" => "這個整合包在你的電腦已經有翻譯結果，可以直接套用、打開或打包分享，不必重跑一次。"
            .into(),
        "partial" if pending_count > 0 => match completion_percent {
            Some(p) if p >= 98 => format!(
                "幾乎全部翻完了（約 {p}%），只剩約 {pending_count} 條可以再補一點細節。"
            ),
            Some(p) if p >= 50 => format!(
                "已完成大半（約 {p}%），還有約 {pending_count} 條可接續補翻。"
            ),
            _ => format!(
                "已經有部分翻譯結果，還有約 {pending_count} 條可以接著補；也可以先打開結果或分享已完成的部分。"
            ),
        },
        "partial" => "已經有部分翻譯結果，可以打開結果資料夾或接著補翻。".into(),
        _ => "未找到可用的本機翻譯快取。".into(),
    };
    Some(LocalPackCacheProbe {
        status: status.into(),
        matched,
        mods_changed: false,
        output_dir: output_dir.display().to_string(),
        work_root: work.display().to_string(),
        session_path: session_path_str,
        pending_count,
        completion_percent,
        shareable,
        applyable,
        updated_at_ms,
        message,
        pack_name,
        canonical_zip,
        counts_trusted: trust_counts,
        last_applied: engine::result_owner::latest_applied(instance, &work),
    })
}

/// 探測同整合包本機是否已有翻譯結果／工作階段，避免重開工具只為分享又重翻一次。
#[tauri::command(async)]
fn probe_local_pack_cache_cmd(
    instance_path: String,
    output_dir: Option<String>,
    custom_base_dir: Option<String>,
) -> Result<LocalPackCacheProbe, String> {
    let instance = normalize_path_strict(&instance_path)?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    let push = |v: &mut Vec<PathBuf>, p: PathBuf| {
        if p.as_os_str().is_empty() {
            return;
        }
        if !v.iter().any(|x| path_keys_equal(x, &p)) {
            v.push(p);
        }
    };

    if let Some(hint) = output_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        push(&mut candidates, normalize_path(hint));
    }
    let managed = managed_output_for_instance_at(&instance, None);
    if !managed.is_empty() {
        push(&mut candidates, PathBuf::from(managed));
    }
    if let Some(base) = custom_base_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let custom = managed_output_for_instance_at(&instance, Some(&normalize_path(base)));
        if !custom.is_empty() {
            push(&mut candidates, PathBuf::from(custom));
        }
    }
    // 只算路徑不建資料夾（B5d：查詢零寫入，G1.36；舊版在遊戲資料夾建了空的「繁中翻譯輸出」）
    let beside = engine::suggest_output_base_path(&instance);
    if beside.is_dir() {
        push(&mut candidates, beside);
    }

    let mut best: Option<LocalPackCacheProbe> = None;
    for cand in candidates {
        if let Some(probe) = probe_cache_at(&instance, &cand) {
            let take = match (&best, probe.status.as_str()) {
                (None, _) => true,
                (Some(prev), "ready") if prev.status != "ready" => true,
                (Some(prev), "ready") if prev.status == "ready" => {
                    probe.updated_at_ms.unwrap_or(0) > prev.updated_at_ms.unwrap_or(0)
                }
                // B5d 審查 4：「有變動」不是可用結果，後面可接續的 partial 要能取代它
                (Some(prev), "partial") if prev.status == "none" || prev.status == "changed" => true,
                (Some(prev), "partial") if prev.status == "partial" => {
                    probe.updated_at_ms.unwrap_or(0) > prev.updated_at_ms.unwrap_or(0)
                }
                _ => false,
            };
            if take {
                best = Some(probe);
            }
        }
    }

    Ok(best.unwrap_or(LocalPackCacheProbe {
        status: "none".into(),
        matched: false,
        mods_changed: false,
        output_dir: String::new(),
        work_root: String::new(),
        session_path: None,
        pending_count: 0,
        completion_percent: None,
        shareable: false,
        applyable: false,
        updated_at_ms: None,
        message: "此整合包尚未找到本機翻譯結果。完成一次翻譯後，重開工具即可直接分享。".into(),
        pack_name: None,
        canonical_zip: None,
        counts_trusted: false,
        last_applied: None,
    }))
}

#[tauri::command]
async fn upload_share_package_cmd(
    work_root: String,
    name: String,
) -> Result<ShareUploadResult, String> {
    let work = normalize_path_strict(&work_root)?;
    tauri::async_runtime::spawn_blocking(move || upload_share_package(&work, &name))
        .await
        .map_err(|e| format!("分享工作中斷：{e}"))?
}

/// 檢查是否需要遊戲內任務翻譯輔助模組；不相容時只回報並跳過。
#[tauri::command]
fn inspect_translation_helper_cmd(
    instance_path: String,
    output_dir: Option<String>,
) -> Result<TranslationHelperStatus, String> {
    let instance = normalize_user_path(&instance_path)?;
    let output = output_dir
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(normalize_user_path)
        .transpose()?;
    inspect_translation_helper(&instance, output.as_deref())
}

/// 只在使用者主動要求時準備一個相容的任務翻譯輔助模組。
#[tauri::command]
async fn prepare_translation_helper_cmd(
    instance_path: String,
    output_dir: String,
) -> Result<TranslationHelperStatus, String> {
    let instance = normalize_user_path(&instance_path)?;
    let output = normalize_user_path(&output_dir)?;
    tauri::async_runtime::spawn_blocking(move || prepare_translation_helper(&instance, &output))
        .await
        .map_err(|e| format!("準備輔助模組工作中斷：{e}"))?
}

/// 刪除工具自己下載的暫時輔助模組；玩家原本的同類模組不會被刪除。
#[tauri::command]
fn cleanup_translation_helper_cmd(
    instance_path: String,
    output_dir: String,
) -> Result<TranslationHelperStatus, String> {
    let instance = normalize_user_path(&instance_path)?;
    let output = normalize_user_path(&output_dir)?;
    cleanup_translation_helper(&instance, &output)
}

/// 使用說明與免責條款和設定共用同一個獨立視窗的「說明」分頁。
#[tauri::command]
async fn open_guide_window(app: AppHandle) -> Result<(), String> {
    open_settings_window(app, Some("help".into()), None).await
}

/// 設定／使用說明——真正的第二個系統視窗。本包選項留在主工具的 modal，不跨視窗。
///
///
/// ZeitFrei 的穩定做法是讓第二個 WebView 載入專用的靜態頁，而非再載入完整工作台
/// `index.html`。後者會再次初始化工作台、事件和 overlay；任一初始化問題都能讓獨立
/// 視窗白畫面。`settings.html` 的 HTML 先天可讀，JS 只負責互動，因此 JS 失敗也不會
/// 把整個設定頁變空白。
#[tauri::command]
async fn open_settings_window(
    app: AppHandle,
    pane: Option<String>,
    theme: Option<String>,
) -> Result<(), String> {
    // 只接受已知的分頁名，不讓外面的字串直接流進 UI
    let pane = match pane.as_deref().unwrap_or("general") {
        "help" | "guide" | "legal" => "help",
        _ => "general",
    };
    let theme = if theme.as_deref() == Some("light") {
        "light"
    } else {
        "dark"
    };
    if let Some(w) = app.get_webview_window("settings") {
        // 已經開著就聚焦，不開第二個
        let _ = w.emit("settings-pane", serde_json::json!({
            "pane": pane,
            "theme": theme,
        }));
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let boot = serde_json::json!({ "pane": pane, "theme": theme }).to_string();
    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
        .title("設定")
        .inner_size(900.0, 780.0)
        .min_inner_size(600.0, 500.0)
        .decorations(true)
        .theme(Some(if theme == "light" {
            tauri::Theme::Light
        } else {
            tauri::Theme::Dark
        }))
        .initialization_script(&format!("window.__MCPL_SETTINGS_BOOT={boot};"))
        .center()
        .build()
        .map_err(|e| format!("無法開啟設定視窗：{e}"))?;
    Ok(())
}

/// 獨立設定頁要求回到主工具調整 AI 時，只聚焦主視窗；實際的 AI 選項仍在主工作台，
/// 避免「每個整合包的翻譯選項」被拆到不帶整合包狀態的第二個視窗。
#[tauri::command]
fn focus_main_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到主工具視窗。".to_string())?;
    let _ = window.unminimize();
    let _ = window.show();
    window
        .set_focus()
        .map_err(|e| format!("無法聚焦主工具視窗：{e}"))
}

/// 用你喜歡的字體檔建立遊戲字體資源包
#[tauri::command]
fn read_font_file_base64(font_path: String) -> Result<String, String> {
    read_font_preview_base64(&font_path)
}

/// 用你喜歡的字體檔建立遊戲字體資源包
#[tauri::command]
async fn create_font_pack(
    font_path: String,
    output_dir: String,
    pack_name: String,
    pack_desc: String,
    font_options: Option<FontPackOptions>,
    pack_format: Option<u16>,
    target_version: Option<String>,
) -> Result<FontPackResult, String> {
    let font = normalize_path_strict(&font_path)?;
    let out = normalize_path_strict(&output_dir)?;
    // 空名稱交給 font_pack 用「繁體中文遊戲字體」，勿先 sanitize 成翻譯包預設名
    let name = pack_name;
    // 字體包 ≈ 複製一份字體檔；要求字體大小 + 50MB 餘裕
    let font_bytes = std::fs::metadata(&font).map(|m| m.len()).unwrap_or(0);
    ensure_space(&out, font_bytes + 50 * 1024 * 1024)?;
    let options = font_options.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        build_font_pack_str_with_options(
            &font.display().to_string(),
            &out.display().to_string(),
            &name,
            &pack_desc,
            &options,
            pack_format,
            target_version.as_deref(),
        )
    })
    .await
    .map_err(|e| describe_worker_failure(&e))?
}

#[tauri::command]
async fn apply_font_pack_to_current_instance(
    app: AppHandle,
    instance_path: String,
    font_pack_path: String,
) -> Result<FontPackApplyResult, String> {
    let instance = normalize_path_strict(&instance_path)?;
    let pack = normalize_path_strict(&font_pack_path)?;
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        emit_progress(&app2, 20, "字體包：正在套用到目前實例 resourcepacks…");
        let result = apply_font_pack_to_instance(&instance, &pack);
        match &result {
            Ok(ok) => {
                if let Some(backup) = &ok.backup_path {
                    emit_log(&app2, "info", &format!("字體包同名備份：{backup}"));
                }
                emit_progress(&app2, 100, "字體包套用完成");
            }
            Err(error) => {
                emit_error(&app2, error);
                emit_progress(&app2, 0, "字體包套用失敗");
            }
        }
        result
    })
    .await
    .map_err(|e| describe_worker_failure(&e))?;
    result
}

/// 移除字體包：只拿掉字體工具裝進遊戲的字體包（翻譯不動）。
#[tauri::command]
async fn remove_font_pack_cmd(instance_path: String) -> Result<String, String> {
    let instance = normalize_path_strict(&instance_path)?;
    tauri::async_runtime::spawn_blocking(move || engine::font_restore::remove_font_pack_in(&instance))
        .await
        .map_err(|e| describe_worker_failure(&e))?
}

#[tauri::command]
fn save_api_key(key: String) -> Result<String, String> {
    let cur = get_api_settings_public();
    save_api_settings(&key, &cur.base_url)?;
    Ok("已儲存".into())
}

#[tauri::command]
fn save_api_settings_cmd(
    api_key: String,
    base_url: String,
    provider: String,
    model: String,
) -> Result<String, String> {
    save_api_settings_with_provider(&api_key, &base_url, &provider, &model)?;
    Ok("已儲存進階設定".into())
}

/// 嚴格探測已儲存的自訂 API 金鑰（僅 custom 模式）。
#[tauri::command]
async fn test_custom_api_key_cmd() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(verify_custom_api)
        .await
        .map_err(|e| format!("探測執行緒失敗：{e}"))?
        .map(|_| "金鑰有效，可連線到你的 API。".into())
}

#[tauri::command]
async fn set_ai_mode_cmd(ai_mode: String) -> Result<String, String> {
    let mode = tauri::async_runtime::spawn_blocking(move || set_ai_mode(&ai_mode))
        .await
        .map_err(|e| format!("切換執行緒失敗：{e}"))??;
    Ok(match mode.as_str() {
        "custom" => "已切換為自訂 API".into(),
        "gpt" => "已切換為 GPT".into(),
        "local" => "已切換為本地模型".into(),
        _ => "已切換 AI 來源".into(),
    })
}

#[tauri::command]
async fn discord_login(app: AppHandle) -> serde_json::Value {
    let result = tauri::async_runtime::spawn_blocking(move || login_discord_blocking(app))
        .await
        .unwrap_or_else(|_| serde_json::json!({ "ok": false, "error": "登入流程發生問題" }));
    if result.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        clear_turnstile_proof();
    }
    result
}

#[tauri::command]
fn cancel_discord_login_cmd() -> bool {
    cancel_discord_login();
    true
}

#[tauri::command]
async fn gpt_login(app: AppHandle) -> serde_json::Value {
    tauri::async_runtime::spawn_blocking(move || gpt_login_blocking(app))
        .await
        .unwrap_or_else(|_| serde_json::json!({ "ok": false, "error": "GPT 登入流程發生問題" }))
}

#[tauri::command]
fn cancel_gpt_login_cmd() -> bool {
    cancel_gpt_login();
    true
}

#[tauri::command]
async fn gpt_auth_status_cmd() -> Result<GptAuthStatus, String> {
    tauri::async_runtime::spawn_blocking(gpt_auth_status)
        .await
        .map_err(|e| format!("GPT 登入狀態檢查中斷：{e}"))
}

#[tauri::command]
fn gpt_logout_cmd() -> Result<String, String> {
    gpt_logout()?;
    Ok("已登出 GPT".into())
}

#[tauri::command]
fn get_gpt_model_cmd() -> String {
    get_gpt_model()
}

#[tauri::command]
fn set_gpt_model_cmd(model: String) -> Result<String, String> {
    let model = set_gpt_model(&model)?;
    Ok(format!("已切換 GPT 模型：{model}"))
}

#[tauri::command]
async fn discord_auth_status() -> Result<DiscordAuthStatus, String> {
    tauri::async_runtime::spawn_blocking(check_discord_auth_status)
        .await
        .map_err(|e| format!("登入狀態檢查中斷：{e}"))
}

#[tauri::command]
fn discord_logout() -> Result<String, String> {
    logout_discord()?;
    clear_turnstile_proof();
    Ok("已登出 Discord".into())
}

#[tauri::command]
async fn turnstile_verify(app: AppHandle) -> serde_json::Value {
    tauri::async_runtime::spawn_blocking(move || verify_turnstile_blocking(app))
        .await
        .unwrap_or_else(|_| serde_json::json!({ "ok": false, "error": "安全驗證流程發生問題" }))
}

#[tauri::command]
fn cancel_turnstile_verification_cmd() -> bool {
    cancel_turnstile_verification();
    true
}

/// 是否已儲存自訂 API 金鑰。
#[tauri::command]
fn has_api_key() -> bool {
    get_api_settings_public().has_key
}

/// 給 UI 顯示 AI 來源狀態。自訂 API／GPT 都要 Discord 會籍才能翻譯。
#[tauri::command]
async fn ai_status() -> serde_json::Value {
    let settings = get_api_settings_public();
    let mode = get_ai_mode();
    let discord = tauri::async_runtime::spawn_blocking(check_discord_auth_status)
        .await
        .ok();
    let logged_in = discord.as_ref().map(|s| s.logged_in).unwrap_or(false);
    let in_guild = discord.as_ref().map(|s| s.in_guild).unwrap_or(false);
    let service_available = discord
        .as_ref()
        .map(|s| s.service_available)
        .unwrap_or(false);
    let discord_message = discord
        .as_ref()
        .map(|s| s.message.clone())
        .unwrap_or_else(|| "目前無法確認 Discord 登入狀態。".into());
    let discord_ready = logged_in && in_guild && service_available;
    let discord_display = discord
        .as_ref()
        .map(|s| s.nickname.clone())
        .unwrap_or_default();

    let mut payload = if mode == "custom" {
        serde_json::json!({
            "aiMode": "custom",
            "usingOwnKey": settings.has_key,
            "providerReady": settings.has_key,
            "message": if settings.has_key {
                "自訂 API 金鑰已存本機（畫面 # 只是遮罩），翻譯時會用真金鑰連線。"
            } else {
                "尚未儲存自訂 API 金鑰。"
            }
        })
    } else if mode == "gpt" {
        let status = tauri::async_runtime::spawn_blocking(gpt_auth_status)
            .await
            .ok();
        let gpt_logged_in = status.as_ref().map(|s| s.logged_in).unwrap_or(false);
        let expired = status.as_ref().map(|s| s.expired).unwrap_or(true);
        let email = status
            .as_ref()
            .map(|s| s.email.clone())
            .unwrap_or_default();
        let account_id = status
            .as_ref()
            .map(|s| s.account_id.clone())
            .unwrap_or_default();
        let gpt_message = status
            .as_ref()
            .map(|s| s.message.clone())
            .unwrap_or_else(|| "目前無法確認 GPT 登入狀態。".into());
        serde_json::json!({
            "aiMode": "gpt",
            "usingOwnKey": false,
            "providerReady": gpt_logged_in,
            "gptLoggedIn": gpt_logged_in,
            "expired": expired,
            "email": email.clone(),
            "accountId": account_id,
            "model": get_gpt_model(),
            "displayName": email,
            "message": gpt_message,
        })
    } else if mode == "local" {
        let local = engine::local_llm::status_view();
        let installed = local.get("installed").and_then(|v| v.as_bool()).unwrap_or(false);
        let local_ready = local.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
        serde_json::json!({
            "aiMode": "local",
            "usingOwnKey": false,
            "providerReady": local_ready,
            "localInstalled": installed,
            "localReady": local_ready,
            "localDirMissing": local.get("installDirMissing").and_then(|v| v.as_bool()).unwrap_or(false),
            "installDir": local.get("installDir").cloned().unwrap_or(serde_json::Value::String(String::new())),
            "message": local.get("message").and_then(|v| v.as_str()).unwrap_or("尚未安裝本地模型。"),
        })
    } else {
        serde_json::json!({
            "aiMode": "custom",
            "usingOwnKey": false,
            "providerReady": false,
            "message": "請選擇自訂 API、GPT 或本地模型。",
        })
    };

    let provider_ready = payload
        .get("providerReady")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let ready = discord_ready && provider_ready;
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("ready".into(), serde_json::json!(ready));
        obj.insert("discordReady".into(), serde_json::json!(discord_ready));
        obj.insert("managedFree".into(), serde_json::json!(false));
        obj.insert("loggedIn".into(), serde_json::json!(logged_in));
        obj.insert("inGuild".into(), serde_json::json!(in_guild));
        obj.insert("serviceAvailable".into(), serde_json::json!(service_available));
        obj.insert("inviteUrl".into(), serde_json::json!(DISCORD_INVITE_URL));
        obj.insert("discordDisplayName".into(), serde_json::json!(discord_display));
        obj.insert("discordMessage".into(), serde_json::json!(discord_message));
    }
    payload
}

#[tauri::command]
fn get_api_settings() -> ApiSettingsPublic {
    get_api_settings_public()
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalLlmProgressPayload {
    percent: u8,
    message: String,
}

#[tauri::command]
async fn local_llm_probe_cmd(install_dir: Option<String>) -> Result<engine::local_llm::ProbeView, String> {
    let dir = install_dir;
    tauri::async_runtime::spawn_blocking(move || {
        engine::local_llm::probe_install(dir.as_deref())
    })
    .await
    .map_err(|e| format!("偵測執行緒失敗：{e}"))?
}

#[tauri::command]
async fn local_llm_install_cmd(
    app: AppHandle,
    install_dir: Option<String>,
) -> Result<engine::local_llm::ProbeView, String> {
    reset_cancel();
    let dir = install_dir;
    tauri::async_runtime::spawn_blocking(move || {
        engine::local_llm::install_and_start(dir.as_deref(), &mut |percent, message| {
            let _ = app.emit(
                "local-llm-progress",
                LocalLlmProgressPayload {
                    percent,
                    message: message.to_string(),
                },
            );
        })
    })
    .await
    .map_err(|e| format!("安裝執行緒失敗：{e}"))?
}

#[tauri::command]
fn local_llm_status_cmd() -> serde_json::Value {
    engine::local_llm::status_view()
}

/// 檔案已經裝好、只是服務還沒啟動（工具剛重開）時，直接把服務叫起來，
/// 不必再走一次「同意並偵測 → 開始下載」的完整流程——那套流程是為了「要不要
/// 下載幾 GB 到這台電腦」設計的，跟「已經下載過，重開機器/工具後重新啟動一個
/// 早就在的服務」是完全不同量級的動作，不該共用同一道確認關卡。
/// `ensure_ready_for_translate` 本身已經處理好「檔案不齊就報錯」與「服務已在跑
/// 就直接回傳」，這裡只是把它接上前端。
#[tauri::command]
async fn local_llm_ensure_ready_cmd(install_dir: Option<String>) -> Result<u16, String> {
    tauri::async_runtime::spawn_blocking(move || {
        engine::local_llm::ensure_ready_for_translate(install_dir.as_deref())
    })
    .await
    .map_err(|e| format!("啟動執行緒失敗：{e}"))?
}

/// 使用者對「本地翻不好時改用雲端 AI 補量」的選擇（P0-05）。
///
/// 回 `"enabled"`／`"disabled"`／`"not_chosen"`。前端只在 `not_chosen` 時徵詢一次：
/// 這會把整合包文字送上網並花掉使用者自己的 API 額度，沒問過就不能算同意。
/// 已經選過關的人不該被反覆詢問，所以三種狀態必須分得出來。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CloudTopUpView {
    /// `enabled` / `disabled` / `not_chosen`
    choice: String,
    /// 這次是否真的會用到雲端補量
    enabled: bool,
    /// 是否該跳一次同意詢問。規則放後端，前端不重新推導。
    needs_consent: bool,
}

#[tauri::command]
fn cloud_topup_choice_cmd() -> CloudTopUpView {
    let choice = engine::cloud_topup_choice();
    CloudTopUpView {
        choice: match choice {
            engine::CloudTopUpChoice::Enabled => "enabled".into(),
            engine::CloudTopUpChoice::Disabled => "disabled".into(),
            engine::CloudTopUpChoice::NotChosen => "not_chosen".into(),
        },
        enabled: choice.is_enabled(),
        needs_consent: choice.needs_consent_prompt(),
    }
}

/// B4 #8：本地模型「翻完自動關閉」的輪次編號：每一輪翻譯開始時前端來拿。
#[tauri::command]
fn local_llm_begin_round_cmd() -> u64 {
    engine::local_llm::release::begin_round()
}

/// B4 #8：這一輪結束時關閉本地模型——只有「還是最新一輪、而且沒有翻譯在跑」才關，
/// 翻完立刻按「接續補完」時，舊的關閉不會關掉新一輪要用的模型。
#[tauri::command]
fn local_llm_release_after_run_cmd(round: u64) -> serde_json::Value {
    let (stopped, message) = engine::local_llm::release::release_after_run(
        round,
        TRANSLATION_ACTIVE.load(Ordering::Relaxed),
        engine::local_llm::stop_own_server,
    );
    serde_json::json!({ "stopped": stopped, "message": message })
}

#[tauri::command]
fn local_llm_stop_cmd() {
    engine::local_llm::stop_own_server();
}

/// 手動刪除本地模型檔案（models／runtime 兩個子目錄）。這是同步阻塞的檔案 I/O，
/// 放到背景執行緒跑，避免刪除大檔案時卡住 UI 執行緒。
#[tauri::command]
async fn local_llm_delete_cmd(install_dir: Option<String>) -> Result<String, String> {
    // B5a-2 審查 F1：翻譯或套用中不刪（翻譯正在用本地模型）；設定視窗也會先停用按鈕
    if TRANSLATION_ACTIVE.load(Ordering::Relaxed) {
        return Err("正在翻譯或套用，完成後才能刪除本地模型。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        engine::local_llm::delete_local_model(install_dir.as_deref())
    })
    .await
    .map_err(|e| format!("刪除執行緒失敗：{e}"))?
}

/// 自動尋找本機 CTE2 全翻參考包路徑（給 UI 預填）
#[tauri::command]
fn get_default_reference_pack() -> Option<String> {
    discover_default_reference().map(|p| p.display().to_string())
}

/// 可選：嘗試下載 CFPA 對應 MC 版本 release zip（失敗由前端略過）。
#[tauri::command]
async fn download_cfpa_reference_pack(
    mc_version: String,
    dest_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    let version = mc_version.trim().to_string();
    if version.is_empty() {
        return Err("請先選擇或偵測 Minecraft 版本。".into());
    }
    let dest = if let Some(d) = dest_dir.filter(|s| !s.trim().is_empty()) {
        normalize_path_strict(&d)?
    } else {
        // 跟其餘子系統一樣統一走可攜式根（見 engine::paths）；這裡每次都用帶時間戳的
        // 新檔名下載（見 try_download_cfpa_pack），本來就不重用舊檔，搬預設路徑沒有
        // 殘留資料要顧慮。
        engine::paths::resolve_file(Path::new("cfpa-cache"))
    };
    let path = tauri::async_runtime::spawn_blocking(move || try_download_cfpa_pack(&version, &dest))
        .await
        .map_err(|e| format!("下載任務失敗：{e}"))??;
    Ok(serde_json::json!({
        "path": path.display().to_string(),
        "attribution": "參考來源：CFPAOrg/Minecraft-Mod-Language-Package（多為 CC BY-NC-SA 4.0）；本工具只填缺並轉台灣用語，不上傳至共享 R2。"
    }))
}

#[tauri::command]
fn get_ui_prefs() -> serde_json::Value {
    serde_json::json!({
        "minimizeOnClose": get_minimize_on_close(),
        // 版本唯一真相源＝Cargo.toml。前端不得再硬編碼版本字串。
        "appVersion": env!("CARGO_PKG_VERSION"),
    })
}

/// 套用前的前置檢查：這個實例的遊戲是不是還開著。
///
/// 回傳 `running`＝true 才擋；偵測不出來時 `known`＝false 且 `running`＝false（放行）。
#[tauri::command]
async fn is_game_running_cmd(instance_path: String) -> serde_json::Value {
    let path = std::path::PathBuf::from(instance_path.trim());
    let verdict = tauri::async_runtime::spawn_blocking(move || {
        engine::game_process::is_game_running(&path)
    })
    .await
    .unwrap_or(engine::game_process::GameRunning::Unknown);
    let running = verdict.blocks_apply();
    let (known, message) = match &verdict {
        engine::game_process::GameRunning::Yes { detail } => (true, detail.clone()),
        engine::game_process::GameRunning::No => (true, String::new()),
        engine::game_process::GameRunning::Unknown => (false, String::new()),
    };
    serde_json::json!({ "running": running, "known": known, "message": message })
}

#[tauri::command]
fn set_ui_prefs(minimize_on_close: bool) -> Result<String, String> {
    set_minimize_on_close(minimize_on_close)?;
    MINIMIZE_ON_CLOSE.store(minimize_on_close, Ordering::Relaxed);
    Ok(if minimize_on_close {
        "已設定：關閉視窗時縮小，不結束程式".into()
    } else {
        "已設定：關閉視窗會結束程式".into()
    })
}

/// 真正結束前必須做的收尾。
///
/// llama-server 是我們 spawn 出去的獨立行程，Windows 上不會隨父行程結束。舊版離開工具後
/// 它會帶著數 GB 記憶體／VRAM 常駐，玩家回去玩遊戲會掉幀而且不知道原因。
/// 前端在翻譯開始／結束時回報，讓關閉流程知道現在能不能直接退出。
#[tauri::command]
fn set_translation_active_cmd(active: bool) {
    TRANSLATION_ACTIVE.store(active, Ordering::Relaxed);
}

fn shutdown_side_processes() {
    engine::local_llm::stop_own_server();
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    shutdown_side_processes();
    app.exit(0);
}

/// 檢查更新：回最新版本與是否有更新（連不上回 ok=false，不是錯誤）。
#[tauri::command]
async fn check_update() -> UpdateCheck {
    tauri::async_runtime::spawn_blocking(check_update_engine)
        .await
        .unwrap_or_else(|_| UpdateCheck {
            current: env!("CARGO_PKG_VERSION").to_string(),
            latest: env!("CARGO_PKG_VERSION").to_string(),
            update_available: false,
            url: String::new(),
            notes: String::new(),
            ok: false,
            message: "檢查更新時中斷。".into(),
            test_build: false,
        })
}

/// 下載並驗證新版免安裝 EXE；等待目前工具關閉後替換並重新開啟。
#[tauri::command]
async fn download_update(app: AppHandle) -> Result<serde_json::Value, String> {
    let r = tauri::async_runtime::spawn_blocking(download_and_launch)
        .await
        .map_err(|e| describe_worker_failure(&e))?;
    match r {
        Ok(d) => {
            emit_log(&app, "info", &d.message);
            let should_exit = d.should_exit;
            let response = serde_json::json!({
                "path": d.path,
                "launched": d.launched,
                "automatic": d.automatic,
                "shouldExit": should_exit,
                "alreadyCurrent": d.already_current,
                "current": d.current,
                "latest": d.latest,
                "message": d.message,
            });
            if should_exit {
                UPDATE_EXITING.store(true, Ordering::SeqCst);
                // 更新走 app.exit(0) 會繞過 quit_app，所以收尾要在這裡自己做一次：
                // 不收的話舊版的 llama-server 變孤兒，繼續吃記憶體／顯示記憶體，
                // 新版起來後還會跟它搶連接埠。
                shutdown_side_processes();
                let exit_app = app.clone();
                std::thread::spawn(move || {
                    // ZeitFrei do_update：給 bat／子行程一點時間後 exit
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    exit_app.exit(0);
                });
            }
            Ok(response)
        }
        Err(e) => {
            emit_error(&app, &e);
            Err(e)
        }
    }
}

/// 不需 Discord 登入：匿名送出使用回饋（rating + note + 結構化欄位）。
#[tauri::command]
async fn submit_usage_feedback_cmd(
    client_id: String,
    rating: Option<u8>,
    note: Option<String>,
    pain_point: Option<String>,
    wish: Option<String>,
) -> SubmitUsageFeedbackCmdResult {
    let r = tauri::async_runtime::spawn_blocking(move || {
        submit_usage_feedback_impl(client_id, rating, note, pain_point, wish)
    })
    .await;

    match r {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => SubmitUsageFeedbackCmdResult {
            ok: false,
            error_type: Some("feedback_request_failed".into()),
            message: Some(e),
        },
        Err(e) => SubmitUsageFeedbackCmdResult {
            ok: false,
            error_type: Some("feedback_worker_join_failed".into()),
            message: Some(format!("{e}")),
        },
    }
}

/// 打開自訂術語表，讓玩家把不滿意的譯名改掉（下次翻譯就生效）。
#[tauri::command]
fn open_glossary() -> Result<String, String> {
    let path = ensure_user_glossary_template().unwrap_or_else(user_glossary_path);
    open::that(&path).map_err(|e| format!("無法開啟術語表：{e}"))?;
    Ok(path.display().to_string())
}

/// 選用：把「用詞不一致建議.json」的 preferred 併入使用者術語表（預設不覆蓋既有鍵）。
#[tauri::command]
fn merge_consistency_suggestions_cmd(
    suggestions_path: Option<String>,
    work_root: Option<String>,
    overwrite: Option<bool>,
) -> Result<serde_json::Value, String> {
    let path = if let Some(p) = suggestions_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        PathBuf::from(p)
    } else if let Some(root) = work_root
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        consistency_suggestions_path(Path::new(root))
    } else {
        return Err("請提供建議檔路徑或翻譯結果資料夾。".into());
    };
    if !path.is_file() {
        return Err("找不到用詞不一致建議檔。請先完成一輪翻譯／補翻，讓結果資料夾產生「用詞不一致建議.json」。".into());
    }
    let (added, glossary) =
        merge_consistency_suggestions(&path, overwrite.unwrap_or(false))?;
    Ok(serde_json::json!({
        "added": added,
        "glossaryPath": glossary.display().to_string(),
        "suggestionsPath": path.display().to_string(),
        "message": format!("已併入 {added} 條建議譯名到術語表（未自動改遊戲譯文）。"),
    }))
}

/// 查結果資料夾是否有用詞不一致建議檔（供前端顯示按鈕）。
#[tauri::command]
fn consistency_suggestions_status_cmd(work_root: String) -> Result<serde_json::Value, String> {
    let root = PathBuf::from(work_root.trim());
    if root.as_os_str().is_empty() {
        return Ok(serde_json::json!({ "exists": false, "count": 0, "path": "" }));
    }
    match consistency_suggestions_status(&root) {
        Some((path, count)) => Ok(serde_json::json!({
            "exists": true,
            "count": count,
            "path": path.display().to_string(),
        })),
        None => Ok(serde_json::json!({
            "exists": false,
            "count": 0,
            "path": root.join("用詞不一致建議.json").display().to_string(),
        })),
    }
}

/// 舊版相容的一鍵套用命令：依玩家選擇備份後，再複製翻譯結果內容。
/// 現行前端會在翻譯、補翻與修復流程完成後直接呼叫同一套引擎，不顯示獨立按鈕。
#[tauri::command]
async fn apply_translation_to_game(
    app: AppHandle,
    instance_path: String,
    output_dir: String,
    pack_name: Option<String>,
    overwrite_confirmed: Option<bool>,
) -> Result<ApplyResult, String> {
    let instance = match normalize_path_strict(&instance_path) {
        Ok(p) => p,
        Err(e) => {
            emit_error(&app, &e);
            return Err(e);
        }
    };
    let out = match normalize_path_strict(&output_dir) {
        Ok(p) => p,
        Err(e) => {
            emit_error(&app, &e);
            return Err(e);
        }
    };
    let pack_name = pack_name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        emit_progress(&app2, 10, "套用：請確認遊戲已關閉…");
        emit_log(
            &app2,
            "warn",
            "【警告】請先完全關閉 Minecraft，再套用（避免檔案被鎖）",
        );
        emit_progress(&app2, 40, "套用：把翻譯套用到遊戲（備份照你的設定）…");
        // B5c 審查 1：套用前確認結果屬於這個遊戲資料夾（零寫入）；舊版結果沒有歸屬紀錄時放行並註記
        let owner_note = match engine::result_owner::guard(&instance, &out) {
            Ok(note) => note,
            Err(e) => {
                emit_error(&app2, &e);
                emit_progress(&app2, 0, "套用失敗");
                return Err(e);
            }
        };
        let policy = engine::apply_record::policy_for_run(overwrite_confirmed.unwrap_or(false));
        let mut r = apply_to_instance(&instance, &out, pack_name.as_deref(), policy);
        if let (Some(note), Ok(ok)) = (owner_note, r.as_mut()) {
            ok.warnings.push(note);
        }
        match &r {
            Ok(ok) if !ok.is_applied() => {
                emit_warn(&app2, &ok.player_summary);
                emit_progress(&app2, 0, "還沒套用到遊戲");
            }
            Ok(ok) => {
                for w in &ok.warnings {
                    emit_warn(&app2, w);
                }
                emit_progress(&app2, 100, "套用完成");
            }
            Err(e) => {
                emit_error(&app2, e);
                emit_progress(&app2, 0, "套用失敗");
            }
        }
        r
    })
    .await
    .map_err(|e| describe_worker_failure(&e))?;
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 啟動時載入偏好；清自動更新殘留（ZeitFrei run() 開頭同款）
    MINIMIZE_ON_CLOSE.store(get_minimize_on_close(), Ordering::Relaxed);
    cleanup_update_residuals();
    // 舊版設定鍵升到新結構；可重入，已遷移就跳過，失敗不擋啟動
    engine::migrate::run_startup_migration();
    crate::engine::nanazip_ensure::ensure_in_background();

    let mut builder = tauri::Builder::default();
    // 單實例須最先註冊；第二次啟動會聚焦既有視窗（跨版本互斥由同一 identifier 達成）
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                if UPDATE_EXITING.load(Ordering::Relaxed) {
                    return;
                }
                if MINIMIZE_ON_CLOSE.load(Ordering::Relaxed) {
                    api.prevent_close();
                    // 只在第一次縮到背景時解釋一次。舊版每按一次 X 就彈一個 blocking 對話框，
                    // 使用者按第三次以後只覺得工具在擋路。
                    if !MINIMIZE_HINT_SHOWN.swap(true, Ordering::Relaxed) {
                        let _ = window
                            .dialog()
                            .message("工具已縮到背景，翻譯工作會繼續執行。要完全結束工具，請用畫面右下角的「離開」，或到設定取消勾選「關閉時縮到背景」。\n\n這個提醒只會出現這一次。")
                            .title("模組整合包翻譯工具")
                            .kind(MessageDialogKind::Info)
                            .blocking_show();
                    }
                    let _ = window.minimize();
                } else if TRANSLATION_ACTIVE.load(Ordering::Relaxed) {
                    // 翻譯進行中：不能說關就關。先擋下來，讓前端問使用者，
                    // 並在使用者確認要離開時把進度與紀錄落檔後才呼叫 quit_app。
                    api.prevent_close();
                    let _ = window.emit("close-requested-while-busy", ());
                } else {
                    // 真的要關了：先收掉自己 spawn 的子行程。
                    shutdown_side_processes();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            one_click_translate,
            supplement_translate,
            repair_translation_pack,
            apply_translation_to_game,
            has_session,
            session_status,
            scan_only,
            open_path,
            write_text_file,
            open_url,
            open_guide_window,
            open_settings_window,
            focus_main_window,
        create_share_package,
            has_shareable_translation_cmd,
            probe_local_pack_cache_cmd,
            upload_share_package_cmd,
            inspect_translation_helper_cmd,
            prepare_translation_helper_cmd,
            cleanup_translation_helper_cmd,
            managed_output_base,
        managed_output_for_instance,
        managed_output_for_instance_with_base,
        delete_result_folder_cmd,
        failed_items_csv_cmd,
        import_translations_cmd,
        verify_resource_packs_cmd,
        repair_resource_packs_cmd,
        set_translation_active_cmd,
        set_remember_api_key_cmd,
        write_run_journal_cmd,
        list_run_journals_cmd,
        data_root_info_cmd,
        migrate_data_root_cmd,
        next_result_dir_cmd,
        read_app_settings_cmd,
        read_app_settings_report_cmd,
        patch_app_settings_cmd,
        clear_api_key_cmd,
        app_settings_path_cmd,
        dev_mode_status_cmd,
        dev_mode_set_cmd,
        check_write_access_cmd,
        inspect_folder_cmd,
        inspect_instance_identity_cmd,
        common_launcher_dir_cmd,
        relaunch_as_admin_cmd,
            check_install_target,
            validate_instance_cmd,
            create_font_pack,
            read_font_file_base64,
            apply_font_pack_to_current_instance,
            remove_font_pack_cmd,
            save_api_key,
            save_api_settings_cmd,
            test_custom_api_key_cmd,
            set_ai_mode_cmd,
            has_api_key,
            ai_status,
            local_llm_probe_cmd,
            local_llm_install_cmd,
            local_llm_status_cmd,
            local_llm_ensure_ready_cmd,
            cloud_topup_choice_cmd,
            local_llm_stop_cmd,
            local_llm_begin_round_cmd,
            local_llm_release_after_run_cmd,
            local_llm_delete_cmd,
            is_game_running_cmd,
            suggest_output_dir,
            gpt_login,
            cancel_gpt_login_cmd,
            gpt_auth_status_cmd,
            gpt_logout_cmd,
            get_gpt_model_cmd,
            set_gpt_model_cmd,
            discord_login,
            cancel_discord_login_cmd,
            discord_auth_status,
            discord_logout,
            turnstile_verify,
            cancel_turnstile_verification_cmd,
            get_api_settings,
            get_default_reference_pack,
            download_cfpa_reference_pack,
            get_ui_prefs,
            set_ui_prefs,
            quit_app,
            cancel_task,
        detect_mc_version,
        detect_pack_translation_name,
        inspect_jar_documentation,
            diagnose_launch_failure,
            diagnose_pack_dir_cmd,
            diagnose_error_text,
            restore_last_apply_cmd,
            submit_diagnose_report_cmd,
            submit_issue_report_cmd,
            delete_apply_backups_cmd,
            has_apply_backups_cmd,
            apply_backup_location_cmd,
            reset_apply_record_cmd,
            fork_apply_instance_cmd,
            check_update,
            download_update,
            submit_usage_feedback_cmd,
            open_glossary,
            merge_consistency_suggestions_cmd,
            consistency_suggestions_status_cmd,
            suggest_resourcepacks_dir,
            suggest_output_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
#[path = "lib_b4_tests.rs"]
mod lib_b4_tests;

#[cfg(test)]
#[path = "lib_b5a2_tests.rs"]
mod lib_b5a2_tests;

#[cfg(test)]
#[path = "lib_b5d_tests.rs"]
mod lib_b5d_tests;

#[cfg(test)]
#[path = "lib_b5c_tests.rs"]
mod lib_b5c_tests;
