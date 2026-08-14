//! 模組包一鍵繁中 — 後端
//! 進度以事件即時推送，重工作在背景執行緒，避免視窗假死。

mod engine;

use engine::{
    apply_font_pack_to_instance, apply_to_instance, build_font_pack_str_with_options,
    build_resource_pack, cancel_discord_login, build_pack_name,
    cancel_turnstile_verification, check_cancelled, check_discord_auth_status,
    clear_turnstile_proof, classify_diagnosis, convert_langmap_s2tw, converter_name, count_map,
    detect_minecraft_version, detect_pack_format, diagnose_launch, discover_default_reference,
    try_download_cfpa_pack,
    cleanup_transient_work, ensure_result_layout, ensure_ready_to_write, ensure_space, ensure_user_glossary_template,
    extract_jar_documentation,
    rewrite_translated_jars, translate_jar_display_texts, translate_jar_patchouli,
    fill_missing_with_mode,
    find_pack_near, find_session_file, fix_minemenu_unicode_escapes, get_ai_mode,
    get_api_settings_public, get_minimize_on_close, has_session_file, load_pack_zh,
    load_phrase_dict, load_reference_zh_tw, load_session, login_discord_blocking, logout_discord,
    is_probably_network_path, managed_ai_available, merge_fill_missing, normalize_user_path,
    package_translation,
    upload_share_package,
    pack_format_for_version,
    probe_apply_targets,
    remaining_pending, request_cancel, reset_cancel, resolve_minecraft_dir, restore_last_apply_in,
    delete_apply_backups_in, has_apply_backups_in,
    save_api_settings, save_api_settings_with_provider, save_session,
    scan_instance, set_ai_mode,
    run_search_pipeline, write_search_artifacts,
    set_minimize_on_close, subtract_covered, suggest_output_base, translate_ftbquests,
    translate_archive_overlays, translate_kubejs_literals, translate_origins,
    translate_quests_books, translate_text_overlays,
    mode_note, skip_complete_namespaces, TranslationMode, TranslationQuality,
    user_glossary_path, validate_instance_path, validate_open_url, verify_turnstile_blocking, write_coverage_report,
    write_gap_summary_file,
    map_stage_progress, CoverageSourceFlags, CoverageTier,
    ApiSettingsPublic, ApplyResult, BuildOptions, PackVersionInfo,
    CoverageStats, DiscordAuthStatus, FontPackApplyResult, FontPackOptions, FontPackResult,
    InstanceValidation, JarDocumentationReport, JarTranslationReport,
    LangMap, LaunchDiagnosis, ShareUploadResult,
    DeleteBackupResult, RestoreResult, ScanReport, TranslateSession, UpdateCheck, CANCEL_MESSAGE, DISCORD_INVITE_URL,
    MIN_FREE_BYTES, RESULT_DIR_NAME, SESSION_FILE,
    TranslationScope,
    TranslationHelperStatus,
    cleanup_translation_helper, inspect_translation_helper, prepare_translation_helper,
};
use engine::{check_update_engine, download_and_launch};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
// append_error_file uses fs
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// 關閉視窗時是否縮小（與 secrets 同步）
static MINIMIZE_ON_CLOSE: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    percent: u8,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogPayload {
    /// info | warn | error
    level: String,
    message: String,
}

fn backup_status(applied: &ApplyResult) -> String {
    if applied.backup_reused {
        format!("沿用既有備份：{}", applied.backup_dir)
    } else if applied.backup_created {
        format!("新建備份：{}", applied.backup_dir)
    } else {
        "未建立備份（依你的選擇）".into()
    }
}

fn emit_progress(app: &AppHandle, percent: u8, message: &str) {
    let _ = app.emit(
        "translate-progress",
        ProgressPayload {
            percent: percent.min(100),
            message: message.to_string(),
        },
    );
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
    jar_translation: JarTranslationReport,
    minemenu_msg: Option<String>,
    player_summary: String,
}

#[derive(Debug, Clone, Copy)]
enum ExtraSourceKind {
    Ftbquests,
    TextOverlay,
    ArchiveOverlay,
    Origins,
    QuestsBooks,
    ScriptLiterals,
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
}

#[derive(Debug, Default)]
struct ExtraSourceSummary {
    notes: Vec<String>,
    skipped: Vec<String>,
    errors: Vec<String>,
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
) -> Result<ExtraSourceOutcome, String> {
    check_cancelled()?;
    let mut outcome = ExtraSourceOutcome::default();
    let label = task.kind.label();
    emit_progress(
        &app,
        task.base,
        &format!("{label}：開始{}", if use_ai { "" } else { "離線" }),
    );
    match task.kind {
        ExtraSourceKind::Ftbquests => {
            let app_q = app.clone();
            let q = translate_ftbquests(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress(&app_q, map_stage_progress(task.base, task.span, pct), msg);
            })?;
            if q.files_written > 0 {
                emit_progress(
                    &app,
                    map_stage_progress(task.base, task.span, 100),
                    &format!("任務已寫出 {} 個檔到「翻譯結果」", q.files_written),
                );
            }
            outcome.note = q.note;
        }
        ExtraSourceKind::TextOverlay => {
            let app_o = app.clone();
            let o = translate_text_overlays(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress(&app_o, map_stage_progress(task.base, task.span, pct), msg);
            })?;
            if o.files_written > 0 {
                emit_progress(
                    &app,
                    map_stage_progress(task.base, task.span, 100),
                    &format!("覆寫文字已寫出 {} 個檔", o.files_written),
                );
            }
            outcome.note = o.note;
        }
        ExtraSourceKind::ArchiveOverlay => {
            let app_a = app.clone();
            let a = translate_archive_overlays(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress(&app_a, map_stage_progress(task.base, task.span, pct), msg);
            })?;
            outcome.note = if a.skipped.is_empty() {
                format!(
                    "ZIP 文字：掃描 {} 個、重建 {} 個、寫入 {} 個項目",
                    a.archives_scanned, a.archives_rewritten, a.entries_rewritten
                )
            } else {
                for skipped in a.skipped {
                    outcome.errors.push(format!("ZIP 文字：{skipped}"));
                }
                format!(
                    "ZIP 文字：掃描 {} 個、重建 {} 個；{} 個略過（詳見錯誤日誌）",
                    a.archives_scanned,
                    a.archives_rewritten,
                    outcome.errors.len()
                )
            };
        }
        ExtraSourceKind::Origins => {
            let app_or = app.clone();
            let o = translate_origins(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress(&app_or, map_stage_progress(task.base, task.span, pct), msg);
            })?;
            if o.files_written > 0 {
                emit_progress(
                    &app,
                    map_stage_progress(task.base, task.span, 100),
                    &format!("Origins 能力已寫出 {} 個檔", o.files_written),
                );
            }
            outcome.note = o.note;
        }
        ExtraSourceKind::QuestsBooks => {
            let app_qb = app.clone();
            let o = translate_quests_books(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress(&app_qb, map_stage_progress(task.base, task.span, pct), msg);
            })?;
            if o.files_written > 0 {
                emit_progress(
                    &app,
                    map_stage_progress(task.base, task.span, 100),
                    &format!("任務／書本已寫出 {} 個檔", o.files_written),
                );
            }
            outcome.note = o.note;
        }
        ExtraSourceKind::ScriptLiterals => {
            let app_s = app.clone();
            let s = translate_kubejs_literals(mc, work, use_ai, scope, move |pct, msg| {
                emit_progress(&app_s, map_stage_progress(task.base, task.span, pct), msg);
            })?;
            outcome.note = s.note;
        }
    }
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

    let mut collect = |task: ExtraSourceTask, result: std::thread::Result<Result<ExtraSourceOutcome, String>>| {
        match result {
            Ok(Ok(outcome)) => {
                if !outcome.note.trim().is_empty() {
                    emit_log(app, "info", &outcome.note);
                    summary.notes.push(outcome.note);
                }
                summary.errors.extend(outcome.errors);
            }
            Ok(Err(error)) => {
                let line = format!("{} 略過／失敗：{error}", task.kind.label());
                emit_error(app, &line);
                summary.notes.push(line.clone());
                summary.errors.push(line);
            }
            Err(_) => {
                let line = format!("{} 略過／失敗：背景工作發生 panic", task.kind.label());
                emit_error(app, &line);
                summary.notes.push(line.clone());
                summary.errors.push(line);
            }
        }
    };

    if use_ai {
        for task in tasks {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_one_extra_source(app.clone(), mc, work, use_ai, scope, task)
            }));
            collect(task, result);
        }
    } else {
        emit_log(app, "info", "未勾選 AI：額外來源最多 3 路並行整理。");
        for chunk in tasks.chunks(3) {
            std::thread::scope(|scope_thread| {
                let mut handles = Vec::new();
                for task in chunk.iter().copied() {
                    let app_task = app.clone();
                    handles.push((
                        task,
                        scope_thread.spawn(move || {
                            run_one_extra_source(app_task, mc, work, use_ai, scope, task)
                        }),
                    ));
                }
                for (task, handle) in handles {
                    collect(task, handle.join());
                }
            });
        }
    }

    summary
}

/// 非同步 command：UI 不會卡住；進度用事件推送
#[tauri::command]
async fn one_click_translate(
    app: AppHandle,
    instance_path: String,
    output_dir: String,
    _pack_name: String,
    use_ai: bool,
    backup_before_apply: bool,
    reference_pack: Option<String>,
    target_version: Option<String>,
    translation_mode: Option<String>,
    translation_quality: Option<String>,
    coverage_tier: Option<String>,
    advanced_unpack: Option<bool>,
) -> Result<OneClickResult, String> {
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
    let (pack_name, pack_version) = build_pack_name(&instance);
    emit_log(
        &app,
        "info",
        &format!(
            "這次資源包版本：{}（{}）\n輸出名稱固定為：{}",
            pack_version.version, pack_version.source, pack_name
        ),
    );
    let use_ai = use_ai;
    let reference_pack = reference_pack
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let target_version = target_version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let advanced_unpack = advanced_unpack.unwrap_or(false);

    reset_cancel();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_one_click(
            &app2,
            instance,
            out,
            pack_name,
            use_ai,
            backup_before_apply,
            reference_pack,
            target_version,
            translation_mode,
            translation_quality,
            coverage_tier,
            advanced_unpack,
        )
    })
    .await
    .map_err(|e| format!("工作中斷：{e}"))?;
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            report_failure(&app, &e);
            Err(e)
        }
    }
}

/// 取消與失敗要分開講：使用者按了停止不該看到滿畫面紅字。
fn report_failure(app: &AppHandle, message: &str) {
    if message == CANCEL_MESSAGE {
        emit_warn(app, message);
        emit_progress(app, 0, "已停止");
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
    let report = rewrite_translated_jars(instance, translated, fallback_english, work)?;
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
#[tauri::command]
fn detect_mc_version(instance_path: String) -> Option<String> {
    let inst = normalize_path(&instance_path);
    let mc = resolve_minecraft_dir(&inst).unwrap_or(inst);
    detect_minecraft_version(&mc)
}

/// Returns the resource-pack version used in the generated pack name.  This
/// intentionally does not expose or reuse the application version.
#[tauri::command]
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
        .map_err(|e| format!("工作中斷：{e}"))
}

#[tauri::command]
fn diagnose_error_text(text: String) -> LaunchDiagnosis {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return classify_diagnosis("", "貼上的錯誤文字");
    }
    classify_diagnosis(trimmed, "使用者貼上的錯誤文字")
}

/// 一鍵還原上次套用（新增的刪掉、覆蓋的還原），用來排除「是不是翻譯造成開不起來」。
#[tauri::command]
async fn restore_last_apply_cmd(
    app: AppHandle,
    instance_path: String,
    output_dir: Option<String>,
) -> Result<RestoreResult, String> {
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
    .map_err(|e| format!("工作中斷：{e}"))?;
    if let Err(e) = &r {
        emit_error(&app, e);
    }
    r
}

/// 刪除目前實例旁所有由工具建立的翻譯套用備份。
#[tauri::command]
async fn delete_apply_backups_cmd(
    app: AppHandle,
    instance_path: String,
    output_dir: Option<String>,
) -> Result<DeleteBackupResult, String> {
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

/// 檢查目前實例／結果位置是否有可還原的工具備份；只讀取，不會修改檔案。
#[tauri::command]
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

fn run_one_click(
    app: &AppHandle,
    instance: PathBuf,
    out: PathBuf,
    pack_name: String,
    use_ai: bool,
    backup_before_apply: bool,
    reference_pack: Option<String>,
    target_version: Option<String>,
    translation_mode: Option<String>,
    translation_quality: Option<String>,
    coverage_tier: Option<String>,
    advanced_unpack: bool,
) -> Result<OneClickResult, String> {
    emit_progress(app, 2, "檢查資料夾…");
    let mode = TranslationMode::parse(translation_mode.as_deref());
    // 主路徑固定完整挑戰；舊 UI 若仍傳 quick／standard 僅記一筆日誌。
    let requested = CoverageTier::parse(coverage_tier.as_deref());
    let tier = CoverageTier::Max;
    let sources: CoverageSourceFlags = tier.sources();
    emit_log(app, "info", &tier.note());
    if requested != CoverageTier::Max {
        emit_log(
            app,
            "info",
            &format!("已忽略舊完整度選項「{}」，固定使用盡量完整。", requested.label()),
        );
    }
    let quality = if translation_quality
        .as_deref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        tier.default_quality()
    } else {
        TranslationQuality::parse(translation_quality.as_deref())
    };
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
    emit_log(app, "info", &format!("{}", mode_note(mode, 0)));
    emit_log(app, "info", &format!("翻譯品質：{}", quality.label()));
    if advanced_unpack {
        emit_log(
            app,
            "info",
            "已同意進階模式：搜尋會一併納入進階來源（只寫副本，不改原模組檔）。",
        );
    }
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
    emit_progress(app, 3, &format!("結果目錄：{}", work.display()));

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
            "共享翻譯分類：{}",
            if translation_scope.is_known() {
                translation_scope.pack_name.as_str()
            } else {
                "未命名整合包"
            }
        ),
    );

    let dict = load_phrase_dict(None);

    // ═══ 階 4：搜尋系統（分析＋理解＋整合→工作圖；進階同意則含進階來源）═══
    {
        let app_search = app.clone();
        let graph = run_search_pipeline(&instance, advanced_unpack, &mut |pct, msg| {
            emit_progress(&app_search, pct, msg);
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
    }

    // ═══ 階段 A：本機掃模組／資源包語言（不 AI）═══
    let app_scan = app.clone();
    let (mut zh, mut en_only, mut report) =
        scan_instance(&instance, &dict, true, true, move |pct, msg| {
            emit_progress(&app_scan, pct, msg);
        })?;

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

    emit_progress(app, 40, "本地整理：詞典與快捷選單…");
    postprocess_lang_values(&mut zh, &dict);
    let minemenu_msg = apply_minemenu_fixed(&instance, &work);

    // ═══ 階段 A2：本機合併「先前完整繁中參考包」（對齊 CTE2 全翻，不花 AI）═══
    let mut ref_note;
    let ref_path = reference_pack
        .as_ref()
        .map(|s| PathBuf::from(s.trim().trim_matches('"')))
        .filter(|p| p.exists())
        .or_else(discover_default_reference);
    if let Some(ref_p) = ref_path {
        emit_progress(
            app,
            41,
            &format!("本機合併參考翻譯包（不呼叫 AI）：{}", ref_p.display()),
        );
        match load_reference_zh_tw(&ref_p) {
            Ok((ref_zh, files)) => {
                let before = count_map(&zh);
                let filled = merge_fill_missing(&mut zh, &ref_zh);
                subtract_covered(&mut en_only, &zh);
                postprocess_lang_values(&mut zh, &dict);
                convert_langmap_s2tw(&mut zh);
                let after = count_map(&zh);
                ref_note = format!(
                    "參考包 {} 個繁中／簡中語言檔，本機補入 {} 條（{} → {}）",
                    files, filled, before, after
                );
                emit_progress(app, 42, &ref_note);
                report.keys_zh = after;
            }
            Err(e) => {
                ref_note = format!("參考包未合併：{e}");
                emit_progress(app, 42, &ref_note);
            }
        }
    } else {
        ref_note = if use_ai {
            "未找到參考包。可選本機繁中／社群漢化包或 zip，本機合併可大幅減少 AI 用量。".into()
        } else {
            "未找到參考包。可選本機繁中／社群漢化包或 zip，本機合併可補上更多內容。".into()
        };
        emit_progress(app, 42, &ref_note);
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
        if let Some(p) = cand {
            if let Ok((ex_zh, _)) = load_reference_zh_tw(&p) {
                let n = merge_fill_missing(&mut zh, &ex_zh);
                if n > 0 {
                    subtract_covered(&mut en_only, &zh);
                    ref_note = format!("{ref_note}；遊戲內舊包再補 {n} 條");
                    emit_progress(app, 43, &format!("本機合併遊戲內舊翻譯包 +{n} 條"));
                }
            }
        }
    }

    let skipped_complete = if mode == TranslationMode::SkipIfComplete {
        skip_complete_namespaces(&zh, &mut en_only, 90)
    } else {
        0
    };
    if skipped_complete > 0 {
        emit_log(app, "info", &mode_note(mode, skipped_complete));
    }

    // AI 只翻字：待補清單
    let pending_before = remaining_pending(&en_only, &zh);
    let pending_before_n = count_map(&pending_before);
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
            keys_zh: zh.values().map(|m| m.len()).sum(),
            note: if use_ai {
                format!("本地+參考包整理完成，待 AI 僅 {} 條。{} {}", pending_before_n, mode_note(mode, skipped_complete), ref_note)
            } else {
                format!("本地+參考包整理完成，仍待補英文 {} 條。{} {}", pending_before_n, mode_note(mode, skipped_complete), ref_note)
            },
            target_version: target_version.clone(),
            translation_mode: mode.value().into(),
            translation_quality: quality.value().into(),
            coverage_tier: tier.value().into(),
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
    emit_progress(app, 41, &pending_progress);

    // 掃描階段錯誤全部進日誌與錯誤檔
    let mut error_lines: Vec<String> = Vec::new();
    if !report.errors.is_empty() {
        emit_warn(
            app,
            &format!(
                "掃描時有 {} 筆問題（詳見下方與錯誤日誌檔）",
                report.errors.len()
            ),
        );
        for (i, e) in report.errors.iter().enumerate() {
            let line = format!("掃描問題 [{}/{}]：{}", i + 1, report.errors.len(), e);
            emit_error(app, &line);
            error_lines.push(line);
        }
    }

    // ═══ 階段 B：補譯（術語表 → 翻譯記憶 → AI）═══
    // 前兩層不需要網路，所以沒勾 AI 也要跑：玩家至少拿得到官方譯名與先前翻過的內容。
    let mut ai_filled = 0usize;
    let mut glossary_hits = 0usize;
    let mut tm_hits = 0usize;
    let mut shared_hits = 0usize;
    let mut ai_note = String::new();
    {
        if pending_before_n > 0 {
            let app_ai = app.clone();
            let app_ai_err = app.clone();
            match fill_missing_with_mode(&mut zh, &pending_before, use_ai, mode == TranslationMode::Force, quality, Some(&translation_scope), move |pct, msg| {
                emit_progress(&app_ai, map_stage_progress(55, 20, pct), msg);
                // AI 訊息若含失敗／錯誤，同步打錯誤日誌
                if msg.contains("失敗") || msg.contains("錯誤") || msg.contains("略過") {
                    emit_error(&app_ai_err, msg);
                }
            }) {
                Ok(r) => {
                    ai_filled = r.filled;
                    glossary_hits = r.glossary_hits;
                    tm_hits = r.tm_hits;
                    shared_hits = r.shared_hits;
                    ai_note = if use_ai {
                        r.note()
                    } else {
                        format!("本機補譯 {} 條；其餘缺漏保留原文", r.filled)
                    };
                    emit_log(app, "info", &format!("補譯結束：{ai_note}"));
                    if r.rejected > 0 {
                        let line = format!(
                            "有 {} 條譯文的 %s／§ 等格式符號被 AI 破壞，已退回英文原文（保護遊戲不出錯）",
                            r.rejected
                        );
                        emit_warn(app, &line);
                        error_lines.push(line);
                    }
                }
                Err(e) => {
                    let line = format!("AI 翻譯失敗：{e}");
                    emit_error(app, &line);
                    error_lines.push(line.clone());
                    append_error_file(&work, &error_lines);
                    return Err(line);
                }
            }
            postprocess_lang_values(&mut zh, &dict);
            // AI 常吐簡中：強制再轉台灣正體
            emit_progress(app, map_stage_progress(55, 20, 100), "正在把補譯結果轉成台灣正體…");
            convert_langmap_s2tw(&mut zh);
            postprocess_lang_values(&mut zh, &dict);
            report.keys_zh = zh.values().map(|m| m.len()).sum();
            report.keys_need_ai = pending_before_n.saturating_sub(ai_filled);
        } else {
            emit_progress(app, map_stage_progress(55, 20, 100), "沒有需要補譯的文字");
        }
        if !use_ai {
            emit_log(
                app,
                "info",
                "未勾選 AI：只用內建術語表與翻譯記憶補，其餘缺漏保留原文",
            );
        }
    }

    // 再保險：整包強制轉台灣正體（修舊簡中殘留）
    check_cancelled()?;
    emit_progress(app, map_stage_progress(75, 7, 0), "最終台灣正體檢查…");
    convert_langmap_s2tw(&mut zh);

    // ═══ 階段 C：寫出資源包（進度 75–82）═══
    emit_progress(app, map_stage_progress(75, 7, 5), "正在建立翻譯檔與資源包…");
    let jar_translation = rewrite_jars_and_log(&app, &instance, &work, &zh, &en_only)?;
    let jar_patchouli_note = if sources.jar_patchouli {
        match translate_jar_patchouli(&instance, &work, use_ai, Some(&translation_scope), |pct, msg| {
            emit_progress(app, map_stage_progress(75, 7, pct), msg);
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
    if sources.jar_display {
        match translate_jar_display_texts(&instance, &work, use_ai, Some(&translation_scope), |pct, msg| {
            emit_progress(app, map_stage_progress(75, 7, pct), msg);
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
    emit_progress(app, map_stage_progress(75, 7, 100), "正在寫出資源包 zip…");
    let built = build_resource_pack(
        &zh,
        &BuildOptions {
            pack_folder_name: pack_name.clone(),
            pack_description: "台灣用語繁體中文翻譯資源包".into(),
            output_dir: work.display().to_string(),
            pack_format,
            target_version: resolved_version.clone(),
        },
    )?;

    // ═══ 階段 D–E4：獨立額外來源（進度 82–97）═══
    // use_ai=true 時維持序列，避免多個來源同時打 AI；use_ai=false 時最多 3 路並行。
    let mc_for_extra = resolve_minecraft_dir(&instance).unwrap_or_else(|_| instance.clone());
    let extra_summary = run_extra_sources(
        app,
        &mc_for_extra,
        &work,
        use_ai,
        Some(&translation_scope),
        sources,
        82,
        15,
    );
    for note in &extra_summary.skipped {
        skipped_by_tier.push(note.clone());
        emit_log(app, "info", note);
    }
    let mut quest_note = extra_summary.combined_note();
    error_lines.extend(extra_summary.errors);
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
    if sources.write_gap_summary {
        match write_gap_summary_file(&work, &pending, 120) {
            Ok(p) => emit_log(
                app,
                "info",
                &format!("已寫待補缺口摘要（樣本）：{}", p.display()),
            ),
            Err(e) => emit_warn(app, &format!("待補缺口摘要寫入失敗：{e}")),
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
            keys_zh: built.keys_total,
            note: format!(
                "完整流程後產生。可「只補缺漏」續翻。剩餘約 {} 條。{}",
                pending_count, quest_note
            ),
            target_version: resolved_version.clone(),
            translation_mode: mode.value().into(),
            translation_quality: quality.value().into(),
            coverage_tier: tier.value().into(),
        },
    );

    let mut coverage_unsupported = report.errors.clone();
    coverage_unsupported.extend(skipped_by_tier.iter().cloned());

    // 社群誠實原則：寫覆蓋範圍說明
    let _ = write_coverage_report(
        &layout,
        &CoverageStats {
            keys_zh: built.keys_total,
            keys_pending: pending_count,
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
                        "補譯命中：術語表 {}／翻譯記憶 {}／共享庫 {}",
                        glossary_hits, tm_hits, shared_hits
                    ),
                    quest_note.clone(),
                ];
                notes.extend(skipped_by_tier.iter().cloned());
                notes
            },
            unsupported: coverage_unsupported,
            glossary_hits,
            tm_hits,
            shared_hits,
            coverage_tier: tier.value().into(),
        },
    );

    let apply_progress = if backup_before_apply {
        "翻譯檔已建立，正在備份並直接套用到遊戲資料夾…"
    } else {
        "翻譯檔已建立，不建立備份，直接套用到遊戲資料夾…"
    };
    emit_progress(app, map_stage_progress(97, 3, 0), apply_progress);
    emit_log(app, "info", "套用前再確認寫入權限；若遊戲開著請先關閉。");
    probe_apply_targets(&instance)?;
    let applied = apply_to_instance(&instance, &work, Some(&pack_name), backup_before_apply)?;
    emit_log(
        app,
        "info",
        &format!(
            "已直接套用到遊戲：{}；翻譯 JAR {} 個。備份位置：{}",
            applied.zip_copied.as_deref().unwrap_or("已建立其他翻譯檔"),
            applied.jars_copied,
            backup_status(&applied)
        ),
    );
    emit_progress(app, 100, "全部完成！");

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
    let pending_note = if use_ai {
        format!("• 尚可 AI 補約 {} 條", pending_count)
    } else {
        format!("• 尚待本機資料或手動翻譯約 {} 條", pending_count)
    };
    let player_summary = format!(
        "完成！目標＝整合包可遊玩文字→台灣繁中（除圖片）；原始 JAR 只讀，翻譯副本已套用。\n\
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
1. 這次已直接套用到遊戲；請確認 Minecraft 已關閉後再重新啟動\n\
2. 語言繁中（台灣）並啟用資源包\n\
3. 補翻／修復時「結果存哪」選你設的根目錄（會找到「{}」）",
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
        RESULT_DIR_NAME,
    );

    Ok(OneClickResult {
        report,
        pack_path: built.pack_path,
        work_root: work.display().to_string(),
        namespaces: built.namespaces,
        files_written: built.files_written,
        keys_total: built.keys_total,
        ai_filled,
        jar_translation,
        minemenu_msg,
        player_summary,
    })
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

/// 只補缺漏：讀上次工作階段 + 現有資源包，不重掃 mods。AI 是選用功能。
#[tauri::command]
async fn supplement_translate(
    app: AppHandle,
    output_dir: String,
    use_ai: bool,
    backup_before_apply: bool,
) -> Result<OneClickResult, String> {
    let out = normalize_path_strict(&output_dir)?;
    reset_cancel();
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        run_supplement(&app2, out, use_ai, backup_before_apply)
    })
        .await
        .map_err(|e| format!("工作中斷：{e}"))?;
    if let Err(e) = &r {
        report_failure(&app, e);
    }
    r
}

fn run_supplement(
    app: &AppHandle,
    out: PathBuf,
    use_ai: bool,
    backup_before_apply: bool,
) -> Result<OneClickResult, String> {
    emit_progress(app, 5, "正在讀取上次的翻譯工作階段…");
    if !out.exists() {
        return Err("輸出資料夾不存在。請選與上次相同的「結果存哪」。".into());
    }
    ensure_space(&out, MIN_FREE_BYTES)?;
    let layout = ensure_result_layout(&out)?;
    let work = layout.work_root.clone();
    cleanup_transient_work(&work)?;
    let (mut session, session_file) = load_session(&out).or_else(|_| load_session(&work))?;
    let mode = TranslationMode::parse(Some(&session.translation_mode));
    let quality = TranslationQuality::parse(Some(&session.translation_quality));
    let tier = CoverageTier::Max;
    let sources: CoverageSourceFlags = tier.sources();
    session.coverage_tier = tier.value().into();
    emit_log(app, "info", &mode_note(mode, 0));
    emit_log(app, "info", &format!("翻譯品質：{}", quality.label()));
    emit_log(app, "info", &tier.note());
    session.review_pass = session.review_pass.saturating_add(1);
    emit_progress(
        app,
        10,
        &format!("已找到工作階段：{}", session_file.display()),
    );

    let session_home = session_file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| work.clone());

    emit_progress(app, 15, "正在讀取已有的資源包…");
    let dict = load_phrase_dict(None);
    let mut recovered = false;
    let mut zh = match find_pack_near(&work, &session.pack_name, &session.pack_path)
        .or_else(|| find_pack_near(&out, &session.pack_name, &session.pack_path))
        .or_else(|| find_pack_near(&session_home, &session.pack_name, &session.pack_path))
    {
        Some(pack_path) => {
            session.pack_path = pack_path.display().to_string();
            emit_progress(app, 18, &format!("讀取資源包：{}", pack_path.display()));
            load_pack_zh(&pack_path)?
        }
        None => {
            // 資源包遺失：用 session 裡的實例路徑「只本地整理」重建中文底稿，不重跑 AI 全量
            recovered = true;
            emit_progress(
                app,
                16,
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
            let (zh_scan, _en, _rep) = scan_instance(&inst, &dict, true, true, move |pct, msg| {
                // 映射到 16–24
                let mapped = 16 + (pct as u16 * 8 / 100) as u8;
                emit_progress(&app_scan, mapped.min(24), msg);
            })?;
            zh_scan
        }
    };
    postprocess_lang_values(&mut zh, &dict);

    let pending = remaining_pending(&session.pending_en, &zh);
    let need = count_map(&pending);
    if need == 0 {
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
        emit_progress(app, 100, "沒有可再補的缺漏了");
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
        session.pack_path = built.pack_path.clone();
        session.output_dir = work.display().to_string();
        session.keys_zh = built.keys_total;
        session.pending_count = 0;
        let _ = save_session(&work, &session);
        let _ = write_coverage_report(
            &layout,
            &CoverageStats {
                keys_zh: built.keys_total,
                keys_pending: 0,
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
                    let mut notes = vec!["複查：沿用已有資源包與工作階段，沒有新的線上翻譯".into()];
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
                coverage_tier: tier.value().into(),
            },
        );
        let instance = PathBuf::from(session.instance_path.trim());
        let applied = apply_to_instance(
            &instance,
            &work,
            Some(&session.pack_name),
            backup_before_apply,
        )?;
        emit_log(
            app,
            "info",
            &format!(
                "複查後已重新套用；備份位置：{}",
                backup_status(&applied)
            ),
        );
        return Ok(OneClickResult {
            report: empty_report(
                &session.instance_path,
                built.keys_total,
                built.namespaces,
                0,
            ),
            pack_path: built.pack_path.clone(),
            work_root: work.display().to_string(),
            namespaces: built.namespaces,
            files_written: built.files_written,
            keys_total: built.keys_total,
            ai_filled: 0,
            jar_translation,
            minemenu_msg: None,
            player_summary: format!(
                "沒有還能補的缺漏了。\n目前資源包約有 {} 條中文。\n位置：\n{}{}",
                built.keys_total,
                built.pack_path,
                if recovered {
                    "\n（已因遺失資源包而重建 zip）"
                } else {
                    ""
                },
            ),
        });
    }

    // 代管 AI 一律可用，補翻不再需要使用者先設金鑰。
    emit_progress(
        app,
        25,
        &format!(
            "工作階段就緒{}。開始{}補約 {} 條…",
            if recovered {
                "（已恢復中文底稿）"
            } else {
                ""
            },
            if use_ai { "AI " } else { "離線 " },
            need,
        ),
    );
    let app_ai = app.clone();
    let supplement_scope = TranslationScope::from_instance(Path::new(session.instance_path.trim()));
    let ai_report = fill_missing_with_mode(&mut zh, &pending, use_ai, mode == TranslationMode::Force, quality, Some(&supplement_scope), move |pct, msg| {
        let mapped = 25 + (pct as u16 * 65 / 100) as u8;
        emit_progress(&app_ai, mapped.min(90), msg);
    })?;
    let ai_filled = ai_report.filled;
    emit_log(app, "info", &ai_report.note());
    postprocess_lang_values(&mut zh, &dict);
    emit_progress(app, 88, "補翻結果轉台灣正體…");
    convert_langmap_s2tw(&mut zh);
    postprocess_lang_values(&mut zh, &dict);

    let instance = PathBuf::from(session.instance_path.trim());
    let jar_translation = rewrite_jars_and_log(app, &instance, &work, &zh, &session.pending_en)?;
    if sources.jar_patchouli {
        let _ = translate_jar_patchouli(&instance, &work, use_ai, Some(&supplement_scope), |pct, msg| {
            emit_progress(app, 89 + (pct as u16 / 10) as u8, msg);
        });
    } else {
        emit_log(app, "info", "完整度略過：JAR Patchouli");
    }
    if sources.jar_display {
        let _ = translate_jar_display_texts(&instance, &work, use_ai, Some(&supplement_scope), |pct, msg| {
            emit_progress(app, 90 + (pct as u16 / 10) as u8, msg);
        });
    } else {
        emit_log(app, "info", "完整度略過：JAR 顯示文字");
    }

    emit_progress(app, 90, "正在寫回資源包（zip）…");
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

    // 補翻時也重跑任務與文字覆寫；是否使用線上翻譯由玩家選項決定。完整度沿用工作階段。
    let mut skipped_by_tier: Vec<String> = Vec::new();
    let mut source_errors: Vec<String> = Vec::new();
    let inst = PathBuf::from(&session.instance_path);
    let mut quest_note = if let Ok(mc) = resolve_minecraft_dir(&inst) {
        let extra_summary = run_extra_sources(
            app,
            &mc,
            &work,
            use_ai,
            Some(&supplement_scope),
            sources,
            92,
            7,
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

    let still = remaining_pending(&session.pending_en, &zh);
    let still_n = count_map(&still);
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
            ai_filled,
            ai_enabled: use_ai,
            jars_scanned: jar_translation.jars_scanned,
            jars_rewritten: jar_translation.jars_rewritten,
            jar_lang_files: jar_translation.lang_files_written,
            jar_errors: jar_translation.errors.len(),
            quests_note: quest_note.clone(),
            ref_note: "補翻流程".into(),
            pack_path: built.pack_path.clone(),
            pack_format: 15,
            source_notes: {
                let mut notes = vec![
                    format!("完整度：{}（{}）", tier.label(), tier.value()),
                    "補翻：只處理工作階段仍缺少的文字，再重建所有輸出來源".into(),
                    quest_note.clone(),
                ];
                notes.extend(skipped_by_tier.iter().cloned());
                notes
            },
            unsupported: skipped_by_tier.clone(),
            glossary_hits: ai_report.glossary_hits,
            tm_hits: ai_report.tm_hits,
            shared_hits: ai_report.shared_hits,
            coverage_tier: tier.value().into(),
        },
    );

    let instance = PathBuf::from(session.instance_path.trim());
    let applied = apply_to_instance(
        &instance,
        &work,
        Some(&session.pack_name),
        backup_before_apply,
    )?;
    emit_log(
        app,
        "info",
        &format!(
            "複查後已重新套用；備份位置：{}",
            backup_status(&applied)
        ),
    );
    emit_progress(app, 100, "補翻完成！");

    let ai_result_line = if use_ai {
        format!("• 這次 AI 新補 {} 條{}", ai_filled, if recovered {
            "（先前資源包遺失，已重建）"
        } else {
            ""
        })
    } else {
        "• 這次只使用本機資料與既有翻譯，未使用線上翻譯服務".to_string()
    };

    Ok(OneClickResult {
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
        jar_translation,
        minemenu_msg: None,
        player_summary: format!(
            "補翻完成！目標＝整合包可遊玩文字→台灣繁中（除圖片）；原始 JAR 只讀，翻譯副本已套用。\n\
{}\n\
• 資源包現在約 {} 條中文\n\
• 尚可補約 {} 條\n\
• 結果資料夾：\n{}\n\
• zip：\n{}\n\
• {}\n\
• 見「覆蓋範圍說明.txt」；這次已直接套用到遊戲\n\
（任務／覆寫在結果資料夾 config、patchouli_books、kubejs 等）",
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
    })
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
    backup_before_apply: bool,
) -> Result<OneClickResult, String> {
    let out = normalize_path_strict(&output_dir)?;
    reset_cancel();
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        run_repair(&app2, out, use_ai, backup_before_apply)
    })
        .await
        .map_err(|e| format!("工作中斷：{e}"))?;
    if let Err(e) = &r {
        report_failure(&app, e);
    }
    r
}

fn run_repair(
    app: &AppHandle,
    out: PathBuf,
    use_ai: bool,
    backup_before_apply: bool,
) -> Result<OneClickResult, String> {
    emit_progress(app, 3, "修復：尋找工作階段…");
    if !out.exists() {
        return Err("輸出資料夾不存在。".into());
    }
    ensure_space(&out, MIN_FREE_BYTES)?;
    let layout = ensure_result_layout(&out)?;
    let work = layout.work_root.clone();
    cleanup_transient_work(&work)?;
    let (mut session, session_file) = load_session(&out).or_else(|_| load_session(&work))?;
    session.review_pass = session.review_pass.saturating_add(1);
    let session_home = session_file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| work.clone());
    emit_progress(
        app,
        8,
        &format!("修復：工作階段 → {}", session_file.display()),
    );

    let dict = load_phrase_dict(None);
    let mut actions: Vec<String> = Vec::new();
    actions.push(format!("找到工作階段：{}", session_file.display()));
    actions.push(format!("結果資料夾：{}", work.display()));

    // 1) 載入或重建中文
    emit_progress(app, 12, "修復：檢查資源包…");
    let pack_found = find_pack_near(&work, &session.pack_name, &session.pack_path)
        .or_else(|| find_pack_near(&out, &session.pack_name, &session.pack_path))
        .or_else(|| find_pack_near(&session_home, &session.pack_name, &session.pack_path));

    let mut zh = if let Some(ref pack_path) = pack_found {
        actions.push(format!("找到既有資源包：{}", pack_path.display()));
        emit_progress(app, 18, &format!("讀取：{}", pack_path.display()));
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
    let pending = remaining_pending(&session.pending_en, &zh);
    let need = count_map(&pending);
    let repair_mode = TranslationMode::parse(Some(session.translation_mode.as_str()));
    let repair_quality = TranslationQuality::parse(Some(session.translation_quality.as_str()));
    let repair_scope = TranslationScope::from_instance(Path::new(session.instance_path.trim()));
    if use_ai && need > 0 {
        emit_progress(app, 40, &format!("修復＋補翻：約 {} 條…", need));
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
                emit_progress(&app_ai, mapped.min(88), msg);
            },
        )?;
        ai_filled = r.filled;
        postprocess_lang_values(&mut zh, &dict);
        actions.push(r.note());
    } else if need > 0 {
        actions.push(format!(
            "尚有約 {} 條缺漏未補（修復不會連線補譯；可再按「只補缺漏」）。",
            need
        ));
    } else {
        actions.push("沒有待補缺漏。".into());
    }

    // 3) 重產 zip + 對齊 session（寫入「翻譯結果」）
    emit_progress(app, 90, "修復：重產 zip 資源包…");
    let instance = PathBuf::from(session.instance_path.trim());
    let repair_scope = TranslationScope::from_instance(&instance);
    let jar_translation = rewrite_jars_and_log(app, &instance, &work, &zh, &session.pending_en)?;
    let _ = translate_jar_patchouli(&instance, &work, use_ai, Some(&repair_scope), |pct, msg| {
        emit_progress(app, 88 + (pct as u16 / 10) as u8, msg);
    });
    let _ = translate_jar_display_texts(&instance, &work, use_ai, Some(&repair_scope), |pct, msg| {
        emit_progress(app, 89 + (pct as u16 / 10) as u8, msg);
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
    actions.push(format!("已寫入 zip：{}", built.pack_path));

    let still = remaining_pending(&session.pending_en, &zh);
    let still_n = count_map(&still);
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
    let applied = apply_to_instance(
        &instance,
        &work,
        Some(&session.pack_name),
        backup_before_apply,
    )?;
    emit_log(
        app,
        "info",
        &format!(
            "修復後已重新套用；備份位置：{}",
            backup_status(&applied)
        ),
    );
    emit_progress(app, 100, "修復完成！");

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
1. 這次已直接套用到遊戲；請重新啟動 Minecraft\n\
2. 語言選繁中（台灣）並啟用資源包\n\
3. 若還有英文 → 同一根目錄按「只補缺漏」",
        actions
            .iter()
            .map(|a| format!("• {a}"))
            .collect::<Vec<_>>()
            .join("\n"),
        built.keys_total,
        repair_translation_line,
        still_n,
        work.display(),
        built.pack_path
    );

    Ok(OneClickResult {
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
        minemenu_msg,
        jar_translation,
        player_summary,
    })
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
    let (zh_scan, _en, rep) = scan_instance(&inst, dict, true, true, move |pct, msg| {
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
#[tauri::command]
fn suggest_resourcepacks_dir(instance_path: String) -> Result<String, String> {
    // 保留舊 command 名以免前端炸掉；語意改為建議「結果根」
    let instance = normalize_path_strict(&instance_path)?;
    if !instance.exists() {
        return Err("找不到遊戲資料夾。".into());
    }
    let base = suggest_output_base(&instance)?;
    Ok(base.display().to_string())
}

#[tauri::command]
fn suggest_output_dir(instance_path: String) -> Result<String, String> {
    suggest_resourcepacks_dir(instance_path)
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
        scan_cache_hits: 0,
        errors: vec![],
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
    let mut keys: Vec<&String> = dict.keys().collect();
    keys.sort_by(|a, b| b.len().cmp(&a.len()));
    let mut out = text.to_string();
    for k in keys {
        if let Some(rep) = dict.get(k) {
            if out.contains(k.as_str()) {
                out = out.replace(k, rep);
            }
        }
    }
    const KEEP: &[&str] = &[
        "之主", "之力", "之心", "之影", "之王", "之怒", "之眼", "之手", "之盾", "之劍", "之書",
        "之塔", "之地",
    ];
    if out.ends_with('之') {
        let keep = KEEP.iter().any(|k| out.ends_with(k));
        if !keep {
            out = out.trim_end_matches('之').to_string();
        }
    }
    out
}

fn apply_minemenu_fixed(instance: &Path, out: &Path) -> Option<String> {
    let mc = resolve_minecraft_dir(instance).ok()?;
    let menu = mc.join("minemenu").join("menu.json");
    if !menu.is_file() {
        return Some("此整合包沒有快捷選單設定，已跳過（正常）。".into());
    }
    let msg = match fix_minemenu_unicode_escapes(&menu) {
        Ok(m) => m,
        Err(e) => format!("快捷選單：{e}"),
    };
    let out_menu = out.join("minemenu");
    let _ = std::fs::create_dir_all(&out_menu);
    let dest = out_menu.join("menu.json");
    let _ = std::fs::copy(&menu, &dest);
    Some(msg)
}

#[tauri::command]
fn scan_only(instance_path: String) -> Result<ScanReport, String> {
    let instance = normalize_path(&instance_path);
    let dict = load_phrase_dict(None);
    let (_zh, _en, report) = scan_instance(&instance, &dict, true, true, |_, _| {})?;
    Ok(report)
}

#[tauri::command]
fn open_path(path: String) -> Result<bool, String> {
    let p = normalize_path(&path);
    if !p.exists() {
        return Err("路徑不存在（可含空白，請確認有沒有打錯）".into());
    }
    open::that(&p).map_err(|e| e.to_string())?;
    Ok(true)
}

/// 開啟網址（推廣連結／說明外連）— 僅 http(s)
#[tauri::command]
fn open_url(url: String) -> Result<bool, String> {
    let u = validate_open_url(&url)?;
    open::that(u).map_err(|e| e.to_string())?;
    Ok(true)
}

/// 工具自管的隱藏工作目錄（`%APPDATA%\modpack-i18n-tool\work`）。
/// 一鍵流程預設把中繼檔放這裡，使用者選的位置就不會被塞一個「翻譯結果」資料夾。
#[tauri::command]
fn managed_output_base() -> String {
    dirs::data_dir()
        .map(|d| d.join("modpack-i18n-tool").join("work"))
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

#[tauri::command]
fn managed_output_for_instance(instance_path: String) -> String {
    use std::hash::{Hash, Hasher};
    let path = normalize_path(&instance_path);
    let stable = fs::canonicalize(&path).unwrap_or(path);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    stable.to_string_lossy().to_ascii_lowercase().hash(&mut hasher);
    let folder = format!("instance-{:016x}", hasher.finish());
    dirs::data_dir()
        .map(|d| d.join("modpack-i18n-tool").join("work").join(folder))
        .map(|p| p.display().to_string())
        .unwrap_or_default()
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
        return Err("這個資料夾不像是本工具建立的翻譯結果，為了安全沒有刪除。".into());
    }
    fs::remove_dir_all(&target).map_err(|e| format!("刪除翻譯結果資料夾失敗：{e}"))?;
    Ok(DeleteResultFolderResult {
        deleted: true,
        path: target.display().to_string(),
        player_summary: format!("已完整刪除翻譯結果資料夾：{}", target.display()),
    })
}

/// 檢查選取的位置是不是一個可直接安裝的遊戲實例（找得到 minecraft 目錄）。
/// 回 { ok, mcDir, hasResourcepacks }，讓前端決定要不要走「直接覆蓋安裝、不建資料夾」。
#[tauri::command]
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

/// 嚴格驗證實例是否可開始翻譯（mods＋實例特徵）；選路徑與一鍵入口共用。
#[tauri::command]
fn validate_instance_cmd(instance_path: String) -> Result<InstanceValidation, String> {
    let path = normalize_user_path(&instance_path)?;
    Ok(validate_instance_path(&path))
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

/// Notion 風格完整說明（獨立視窗）
#[tauri::command]
fn open_guide_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("guide") {
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "guide", WebviewUrl::App("guide.html".into()))
        .title("使用說明與免責條款")
        .inner_size(720.0, 780.0)
        .min_inner_size(480.0, 520.0)
        .center()
        .build()
        .map_err(|e| format!("無法開啟說明視窗：{e}"))?;
    Ok(())
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
    .map_err(|e| format!("工作中斷：{e}"))?
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
    .map_err(|e| format!("工作中斷：{e}"))?;
    result
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

#[tauri::command]
fn set_ai_mode_cmd(ai_mode: String) -> Result<String, String> {
    let mode = set_ai_mode(&ai_mode)?;
    Ok(if mode == "custom" {
        "已切換為自訂 API".into()
    } else {
        "已切換為開發者代管 AI".into()
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

/// 是否已儲存自訂 API 金鑰；代管模式的可用性由 `ai_status` 判斷。
#[tauri::command]
fn has_api_key() -> bool {
    get_api_settings_public().has_key
}

/// 給 UI 顯示 AI 來源狀態。
/// 自訂 API 只檢查本機是否有金鑰；代管 AI 需要 Discord 會員資格（不再要求 Turnstile）。
#[tauri::command]
async fn ai_status() -> serde_json::Value {
    let settings = get_api_settings_public();
    let mode = get_ai_mode();
    if mode == "custom" {
        return serde_json::json!({
            "ready": settings.has_key,
            "aiMode": "custom",
            "usingOwnKey": settings.has_key,
            "managedFree": false,
            "loggedIn": false,
            "inGuild": false,
            "message": if settings.has_key {
                "自訂 API 已設定，翻譯時會直接連線到你的服務。"
            } else {
                "尚未儲存自訂 API 金鑰。"
            }
        });
    }

    let status = tauri::async_runtime::spawn_blocking(check_discord_auth_status)
        .await
        .ok();
    let logged_in = status.as_ref().map(|s| s.logged_in).unwrap_or(false);
    let in_guild = status.as_ref().map(|s| s.in_guild).unwrap_or(false);
    let service_available = status
        .as_ref()
        .map(|s| s.service_available)
        .unwrap_or(false);
    let message = status
        .as_ref()
        .map(|s| s.message.clone())
        .unwrap_or_else(|| "目前無法確認 Discord 登入狀態。".into());
    let identity_ready = logged_in && in_guild && service_available;
    let ready = managed_ai_available() && identity_ready;
    let status_message = if !service_available {
        message
    } else if identity_ready {
        "免費代管翻譯已可使用。".to_string()
    } else {
        message
    };
    serde_json::json!({
        "ready": ready,
        "aiMode": "managed",
        "usingOwnKey": false,
        "managedFree": true,
        "loggedIn": logged_in,
        "inGuild": in_guild,
        "serviceAvailable": service_available,
        "turnstileRequired": false,
        "turnstileVerified": true,
        "turnstileExpiresAt": 0,
        "turnstileServiceReady": true,
        "turnstileHealthError": null,
        "inviteUrl": DISCORD_INVITE_URL,
        "displayName": status.as_ref().map(|s| s.nickname.clone()).unwrap_or_default(),
        "message": status_message
    })
}

#[tauri::command]
fn get_api_settings() -> ApiSettingsPublic {
    get_api_settings_public()
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
        dirs::data_local_dir()
            .unwrap_or_else(|| std::env::temp_dir())
            .join("modpack-i18n-tool")
            .join("cfpa-cache")
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
        "minimizeOnClose": get_minimize_on_close()
    })
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

#[tauri::command]
fn quit_app(app: AppHandle) {
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
        })
}

/// 下載並驗證新版免安裝 EXE；等待目前工具關閉後替換並重新開啟。
#[tauri::command]
async fn download_update(app: AppHandle) -> Result<serde_json::Value, String> {
    let r = tauri::async_runtime::spawn_blocking(download_and_launch)
        .await
        .map_err(|e| format!("工作中斷：{e}"))?;
    match r {
        Ok(d) => {
            emit_log(&app, "info", &d.message);
            let should_exit = d.should_exit;
            let response = serde_json::json!({
                "path": d.path,
                "launched": d.launched,
                "automatic": d.automatic,
                "message": d.message,
            });
            if should_exit {
                let exit_app = app.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(900));
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

/// 打開自訂術語表，讓玩家把不滿意的譯名改掉（下次翻譯就生效）。
#[tauri::command]
fn open_glossary() -> Result<String, String> {
    let path = ensure_user_glossary_template().unwrap_or_else(user_glossary_path);
    open::that(&path).map_err(|e| format!("無法開啟術語表：{e}"))?;
    Ok(path.display().to_string())
}

/// 舊版相容的一鍵套用命令：依玩家選擇備份後，再複製翻譯結果內容。
/// 現行前端會在翻譯、補翻與修復流程完成後直接呼叫同一套引擎，不顯示獨立按鈕。
#[tauri::command]
async fn apply_translation_to_game(
    app: AppHandle,
    instance_path: String,
    output_dir: String,
    pack_name: Option<String>,
    backup_before_apply: bool,
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
        let message = if backup_before_apply {
            "套用：備份後複製資源包／任務…"
        } else {
            "套用：不建立備份，直接複製資源包／任務…"
        };
        emit_progress(&app2, 40, message);
        let r = apply_to_instance(
            &instance,
            &out,
            pack_name.as_deref(),
            backup_before_apply,
        );
        match &r {
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
    .map_err(|e| format!("工作中斷：{e}"))?;
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 啟動時載入偏好
    MINIMIZE_ON_CLOSE.store(get_minimize_on_close(), Ordering::Relaxed);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                if MINIMIZE_ON_CLOSE.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = window
                        .dialog()
                        .message("工具已縮到背景，翻譯工作會繼續執行。要完全結束工具，請取消勾選「關閉視窗時縮到背景」後再關閉。")
                        .title("模組包翻譯工具")
                        .kind(MessageDialogKind::Info)
                        .blocking_show();
                    let _ = window.minimize();
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
            open_url,
            open_guide_window,
        create_share_package,
            upload_share_package_cmd,
            inspect_translation_helper_cmd,
            prepare_translation_helper_cmd,
            cleanup_translation_helper_cmd,
            managed_output_base,
        managed_output_for_instance,
        delete_result_folder_cmd,
            check_install_target,
            validate_instance_cmd,
            create_font_pack,
            apply_font_pack_to_current_instance,
            save_api_key,
            save_api_settings_cmd,
            set_ai_mode_cmd,
            has_api_key,
            ai_status,
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
            diagnose_error_text,
            restore_last_apply_cmd,
            delete_apply_backups_cmd,
            has_apply_backups_cmd,
            check_update,
            download_update,
            open_glossary,
            suggest_resourcepacks_dir,
            suggest_output_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
