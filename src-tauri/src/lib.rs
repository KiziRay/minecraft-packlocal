//! 模組包一鍵繁中 — 後端
//! 進度以事件即時推送，重工作在背景執行緒，避免視窗假死。

mod engine;

use engine::{
    apply_to_instance, restore_last_apply, diagnose_launch, LaunchDiagnosis, RestoreResult,
    build_font_pack_str_with_options, build_resource_pack, check_cancelled,
    convert_langmap_s2tw, converter_name, count_map, detect_minecraft_version, detect_pack_format,
    pack_format_for_version,
    discover_default_reference, ensure_result_layout, ensure_space, ensure_user_glossary_template,
    MIN_FREE_BYTES,
    fill_missing_with_ai, find_pack_near, find_session_file, fix_minemenu_unicode_escapes,
    get_api_settings_public, get_minimize_on_close, has_session_file, load_deepseek_key, managed_ai_available,
    load_pack_zh, load_phrase_dict, load_reference_zh_tw, load_session, merge_fill_missing,
    normalize_user_path, remaining_pending, request_cancel, reset_cancel, resolve_minecraft_dir,
    save_api_settings, save_session, scan_instance, sanitize_folder_name, set_minimize_on_close,
    subtract_covered, suggest_output_base, translate_ftbquests, translate_origins,
    translate_quests_books, translate_text_overlays,
    user_glossary_path, validate_open_url, write_coverage_report, ApiSettingsPublic,
    ApplyResult, BuildOptions, CoverageStats, FontPackOptions, FontPackResult, LangMap, ScanReport,
    TranslateSession, UpdateCheck, CANCEL_MESSAGE, RESULT_DIR_NAME, SESSION_FILE,
};
use engine::{check_update_engine, download_and_launch};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
// append_error_file uses fs
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

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
    let header = format!(
        "\n======== {} ========\n",
        chrono_like_now()
    );
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
    minemenu_msg: Option<String>,
    player_summary: String,
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

/// 非同步 command：UI 不會卡住；進度用事件推送
#[tauri::command]
async fn one_click_translate(
    app: AppHandle,
    instance_path: String,
    output_dir: String,
    pack_name: String,
    use_ai: bool,
    reference_pack: Option<String>,
    target_version: Option<String>,
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
    let pack_name = match sanitize_folder_name(&pack_name) {
        Ok(p) => p,
        Err(e) => {
            emit_error(&app, &e);
            return Err(e);
        }
    };
    let use_ai = use_ai;
    let reference_pack = reference_pack
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let target_version = target_version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    reset_cancel();
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_one_click(&app2, instance, out, pack_name, use_ai, reference_pack, target_version)
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

/// 診斷「遊戲／世界開不起來」：讀當機報告與 log，判斷是缺模組還是我們的檔。
#[tauri::command]
async fn diagnose_launch_failure(instance_path: String) -> Result<LaunchDiagnosis, String> {
    let inst = normalize_path_strict(&instance_path)?;
    tauri::async_runtime::spawn_blocking(move || diagnose_launch(&inst))
        .await
        .map_err(|e| format!("工作中斷：{e}"))
}

/// 一鍵還原上次套用（新增的刪掉、覆蓋的還原），用來排除「是不是翻譯造成開不起來」。
#[tauri::command]
async fn restore_last_apply_cmd(
    app: AppHandle,
    instance_path: String,
) -> Result<RestoreResult, String> {
    let inst = normalize_path_strict(&instance_path)?;
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        emit_progress(&app2, 20, "還原：讀取上次套用的備份…");
        let r = restore_last_apply(&inst);
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

fn run_one_click(
    app: &AppHandle,
    instance: PathBuf,
    out: PathBuf,
    pack_name: String,
    use_ai: bool,
    reference_pack: Option<String>,
    target_version: Option<String>,
) -> Result<OneClickResult, String> {
    emit_progress(app, 2, "準備中（此階段不呼叫 AI）…");
    if !instance.exists() {
        return Err("找不到這個資料夾，請重新選擇或檢查路徑是否正確（可用空白字元）。".into());
    }
    // 第一次執行時放一份可編輯的術語表範本，讓玩家知道譯名可以自己改
    if let Some(p) = ensure_user_glossary_template() {
        emit_log(
            app,
            "info",
            &format!("想固定某些譯名可編輯：{}", p.display()),
        );
    }
    // 空間不足先擋，避免寫到一半失敗留半成品
    ensure_space(&out, MIN_FREE_BYTES)?;
    // 使用者只選根目錄；工具建立 翻譯結果/ 與子目錄
    let layout = ensure_result_layout(&out)?;
    let work = layout.work_root.clone();
    emit_progress(
        app,
        3,
        &format!("結果目錄：{}", work.display()),
    );

    let pack_name = if pack_name.is_empty() {
        "繁體中文翻譯".to_string()
    } else {
        pack_name
    };

    let dict = load_phrase_dict(None);

    // ═══ 階段 A：本機掃模組／資源包語言（不 AI）═══
    let app_scan = app.clone();
    let (mut zh, mut en_only, mut report) = scan_instance(
        &instance,
        &dict,
        true,
        true,
        move |pct, msg| {
            emit_progress(&app_scan, pct, msg);
        },
    )?;

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
                    "參考包 {} 個 zh_tw 檔，本機補入 {} 條（{} → {}）",
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
        ref_note =
            "未找到參考包。建議選 CTE2 TW 全翻 zip，本機合併可大幅減少 AI 用量。".into();
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

    // AI 只翻字：待補清單
    let pending_before = remaining_pending(&en_only, &zh);
    let pending_before_n = count_map(&pending_before);
    let _ = save_pending_manifest(&work, &pending_before, pending_before_n);
    let _ = save_session(
        &work,
        &TranslateSession {
            version: 1,
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
            note: format!(
                "本地+參考包整理完成，待 AI 僅 {} 條。{}",
                pending_before_n, ref_note
            ),
            target_version: target_version.clone(),
        },
    );

    emit_progress(
        app,
        41,
        &format!(
            "本地全部整理完成（模組 {}／資源包 {}／鬆散 {}；中文 {}；待 AI {}）",
            report.jars_scanned,
            report.resourcepacks_scanned,
            report.loose_lang_files,
            report.keys_zh,
            pending_before_n
        ),
    );

    // 掃描階段錯誤全部進日誌與錯誤檔
    let mut error_lines: Vec<String> = Vec::new();
    if !report.errors.is_empty() {
        emit_warn(
            app,
            &format!("掃描時有 {} 筆問題（詳見下方與錯誤日誌檔）", report.errors.len()),
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
    let mut ai_note = String::new();
    {
        if pending_before_n > 0 {
            let app_ai = app.clone();
            let app_ai_err = app.clone();
            match fill_missing_with_ai(&mut zh, &pending_before, use_ai, move |pct, msg| {
                emit_progress(&app_ai, pct, msg);
                // AI 訊息若含失敗／錯誤，同步打錯誤日誌
                if msg.contains("失敗") || msg.contains("錯誤") || msg.contains("略過") {
                    emit_error(&app_ai_err, msg);
                }
            }) {
                Ok(r) => {
                    ai_filled = r.filled;
                    ai_note = r.note();
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
            emit_progress(app, 89, "正在把補譯結果轉成台灣正體…");
            convert_langmap_s2tw(&mut zh);
            postprocess_lang_values(&mut zh, &dict);
            report.keys_zh = zh.values().map(|m| m.len()).sum();
            report.keys_need_ai = pending_before_n.saturating_sub(ai_filled);
        } else {
            emit_progress(app, 88, "沒有需要補譯的文字");
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
    emit_progress(app, 90, "最終台灣正體檢查…");
    convert_langmap_s2tw(&mut zh);

    // ═══ 階段 C：寫出資源包 ═══
    emit_progress(app, 91, "正在建立翻譯檔與資源包…");
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

    // ═══ 階段 D：FTB Quests 任務／劇情（舊流程完全沒做）═══
    let mut quest_note;
    let mc_for_extra = resolve_minecraft_dir(&instance).unwrap_or_else(|_| instance.clone());
    {
        let app_q = app.clone();
        match translate_ftbquests(&mc_for_extra, &work, use_ai, move |pct, msg| {
            let mapped = 93 + (pct as u16 * 1 / 100) as u8;
            emit_progress(&app_q, mapped.min(94), msg);
        }) {
            Ok(q) => {
                quest_note = q.note;
                if q.files_written > 0 {
                    emit_progress(
                        app,
                        94,
                        &format!("任務已寫出 {} 個檔到「翻譯結果」", q.files_written),
                    );
                }
            }
            Err(e) => {
                quest_note = format!("任務翻譯略過／失敗：{e}");
                emit_error(app, &quest_note);
                error_lines.push(quest_note.clone());
                emit_progress(app, 94, &quest_note);
            }
        }
    }

    // ═══ 階段 E：文字覆寫（patchouli／openloader／kubejs／datapacks／fancymenu）═══
    let overlay_note;
    {
        let app_o = app.clone();
        match translate_text_overlays(&mc_for_extra, &work, use_ai, move |pct, msg| {
            let mapped = 94 + (pct as u16 * 4 / 100) as u8;
            emit_progress(&app_o, mapped.min(98), msg);
        }) {
            Ok(o) => {
                overlay_note = o.note;
                if o.files_written > 0 {
                    emit_progress(
                        app,
                        98,
                        &format!("覆寫文字已寫出 {} 個檔", o.files_written),
                    );
                } else {
                    emit_progress(app, 98, &overlay_note);
                }
            }
            Err(e) => {
                overlay_note = format!("覆寫文字略過／失敗：{e}");
                emit_error(app, &overlay_note);
                error_lines.push(overlay_note.clone());
                emit_progress(app, 98, &overlay_note);
            }
        }
    }
    // ═══ 階段 E2：Origins/Apoli 能力文字（data/powers、origins）═══
    let origins_note;
    {
        let app_or = app.clone();
        match translate_origins(&mc_for_extra, &work, use_ai, move |pct, msg| {
            let mapped = 98 + (pct as u16 * 1 / 100) as u8;
            emit_progress(&app_or, mapped.min(99), msg);
        }) {
            Ok(o) => {
                origins_note = o.note;
                if o.files_written > 0 {
                    emit_progress(app, 99, &format!("Origins 能力已寫出 {} 個檔", o.files_written));
                }
            }
            Err(e) => {
                origins_note = format!("Origins 能力略過／失敗：{e}");
                emit_error(app, &origins_note);
                error_lines.push(origins_note.clone());
            }
        }
    }
    // ═══ 階段 E3：任務／書本（Better Questing／HQM／Heracles／Modonomicon）═══
    let qb_note;
    {
        let app_qb = app.clone();
        match translate_quests_books(&mc_for_extra, &work, use_ai, move |pct, msg| {
            let mapped = 99 + (pct as u16 * 1 / 100) as u8;
            emit_progress(&app_qb, mapped.min(99), msg);
        }) {
            Ok(o) => {
                qb_note = o.note;
                if o.files_written > 0 {
                    emit_progress(app, 99, &format!("任務／書本已寫出 {} 個檔", o.files_written));
                }
            }
            Err(e) => {
                qb_note = format!("任務／書本略過／失敗：{e}");
                emit_error(app, &qb_note);
                error_lines.push(qb_note.clone());
            }
        }
    }

    // 覆蓋報告把 overlay 併入 quests_note
    if !overlay_note.is_empty() {
        if quest_note.is_empty() {
            quest_note = overlay_note.clone();
        } else {
            quest_note = format!("{quest_note}；{overlay_note}");
        }
    }
    if !origins_note.is_empty() {
        quest_note = if quest_note.is_empty() {
            origins_note.clone()
        } else {
            format!("{quest_note}；{origins_note}")
        };
    }
    if !qb_note.is_empty() {
        quest_note = if quest_note.is_empty() {
            qb_note.clone()
        } else {
            format!("{quest_note}；{qb_note}")
        };
    }
    if !ai_note.is_empty() {
        quest_note = if quest_note.is_empty() {
            ai_note.clone()
        } else {
            format!("{quest_note}；{ai_note}")
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
    let _ = save_session(
        &work,
        &TranslateSession {
            version: 1,
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
        },
    );

    // 社群誠實原則：寫覆蓋範圍說明
    let _ = write_coverage_report(
        &layout,
        &CoverageStats {
            keys_zh: built.keys_total,
            keys_pending: pending_count,
            ai_filled,
            jars_scanned: report.jars_scanned,
            quests_note: quest_note.clone(),
            ref_note: ref_note.clone(),
            pack_path: built.pack_path.clone(),
            pack_format,
        },
    );

    emit_progress(app, 100, "全部完成！");

    let player_summary = format!(
        "完成！目標＝整合包可遊玩文字→台灣繁中（除圖片）；不改 jar。\n\
（整理＝本機，AI＝只翻譯缺漏英文；不宣稱 100%）\n\
• 中文總計約 {} 條（AI 新補 {}）\n\
• 尚可 AI 補約 {} 條\n\
• {}\n\
• {}\n\
• 台灣正體轉換：{}\n\
• pack_format：{}\n\
• 結果資料夾（工具自動建立）：\n{}\n\
• 資源包 zip：\n{}\n\
• 詳見「覆蓋範圍說明.txt」\n\n\
【請你】\n\
1. 關遊戲後按「一鍵套用到遊戲」（會先備份）或手動複製 zip／任務／覆寫文字\n\
2. 語言繁中（台灣）並啟用資源包\n\
3. 補翻／修復時「結果存哪」選你設的根目錄（會找到「{}」）",
        built.keys_total,
        ai_filled,
        pending_count,
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

/// 寫出待譯清單（人類可讀），證明 AI 前已整理完
fn save_pending_manifest(out: &Path, pending: &LangMap, count: usize) -> Result<(), String> {
    let p = out.join("待翻譯清單-本地整理完成.json");
    let obj = serde_json::json!({
        "note": "此檔在呼叫 AI 之前寫入。AI 只翻譯這裡的英文，不再掃描 jar／資源包。",
        "pendingCount": count,
        "namespaces": pending.len(),
    });
    std::fs::write(p, serde_json::to_string_pretty(&obj).unwrap_or_default() + "\n")
        .map_err(|e| e.to_string())
}

/// 只補缺漏：讀上次工作階段 + 現有資源包，不重掃 mods。必須有 AI 金鑰。
#[tauri::command]
async fn supplement_translate(
    app: AppHandle,
    output_dir: String,
) -> Result<OneClickResult, String> {
    let out = normalize_path_strict(&output_dir)?;
    reset_cancel();
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || run_supplement(&app2, out))
        .await
        .map_err(|e| format!("工作中斷：{e}"))?;
    if let Err(e) = &r {
        report_failure(&app, e);
    }
    r
}

fn run_supplement(app: &AppHandle, out: PathBuf) -> Result<OneClickResult, String> {
    emit_progress(app, 5, "正在讀取上次的翻譯工作階段…");
    if !out.exists() {
        return Err("輸出資料夾不存在。請選與上次相同的「結果存哪」。".into());
    }
    ensure_space(&out, MIN_FREE_BYTES)?;
    let layout = ensure_result_layout(&out)?;
    let work = layout.work_root.clone();
    let (mut session, session_file) = load_session(&out)
        .or_else(|_| load_session(&work))?;
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
            let (zh_scan, _en, _rep) = scan_instance(
                &inst,
                &dict,
                true,
                true,
                move |pct, msg| {
                    // 映射到 16–24
                    let mapped = 16 + (pct as u16 * 8 / 100) as u8;
                    emit_progress(&app_scan, mapped.min(24), msg);
                },
            )?;
            zh_scan
        }
    };
    postprocess_lang_values(&mut zh, &dict);

    let pending = remaining_pending(&session.pending_en, &zh);
    let need = count_map(&pending);
    if need == 0 {
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
        return Ok(OneClickResult {
            report: empty_report(&session.instance_path, built.keys_total, built.namespaces, 0),
            pack_path: built.pack_path.clone(),
            work_root: work.display().to_string(),
            namespaces: built.namespaces,
            files_written: built.files_written,
            keys_total: built.keys_total,
            ai_filled: 0,
            minemenu_msg: None,
            player_summary: format!(
                "沒有還能補的缺漏了。\n目前資源包約有 {} 條中文。\n位置：\n{}{}",
                built.keys_total,
                built.pack_path,
                if recovered {
                    "\n（已因遺失資源包而重建 zip）"
                } else {
                    ""
                }
            ),
        });
    }

    // 代管 AI 一律可用，補翻不再需要使用者先設金鑰。
    emit_progress(
        app,
        25,
        &format!(
            "工作階段就緒{}。開始純 AI 補約 {} 條…",
            if recovered {
                "（已恢復中文底稿）"
            } else {
                ""
            },
            need
        ),
    );
    let app_ai = app.clone();
    let ai_report = fill_missing_with_ai(&mut zh, &pending, true, move |pct, msg| {
        let mapped = 25 + (pct as u16 * 65 / 100) as u8;
        emit_progress(&app_ai, mapped.min(90), msg);
    })?;
    let ai_filled = ai_report.filled;
    emit_log(app, "info", &ai_report.note());
    postprocess_lang_values(&mut zh, &dict);
    emit_progress(app, 88, "補翻結果轉台灣正體…");
    convert_langmap_s2tw(&mut zh);
    postprocess_lang_values(&mut zh, &dict);

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

    // 補翻時也重跑任務與文字覆寫（代管 AI 一律可用）
    let mut quest_note = String::new();
    let mut overlay_note = String::new();
    {
        let inst = PathBuf::from(&session.instance_path);
        if let Ok(mc) = resolve_minecraft_dir(&inst) {
            let app_q = app.clone();
            if let Ok(q) = translate_ftbquests(&mc, &work, true, move |pct, msg| {
                let mapped = 92 + (pct as u16 * 2 / 100) as u8;
                emit_progress(&app_q, mapped.min(94), msg);
            }) {
                quest_note = q.note;
            }
            let app_o = app.clone();
            if let Ok(o) = translate_text_overlays(&mc, &work, true, move |pct, msg| {
                let mapped = 94 + (pct as u16 * 3 / 100) as u8;
                emit_progress(&app_o, mapped.min(97), msg);
            }) {
                overlay_note = o.note;
            }
            let app_or = app.clone();
            if let Ok(o) = translate_origins(&mc, &work, true, move |pct, msg| {
                let mapped = 97 + (pct as u16 * 1 / 100) as u8;
                emit_progress(&app_or, mapped.min(98), msg);
            }) {
                if !o.note.is_empty() {
                    overlay_note = if overlay_note.is_empty() {
                        o.note
                    } else {
                        format!("{overlay_note}；{}", o.note)
                    };
                }
            }
            let app_qb = app.clone();
            if let Ok(o) = translate_quests_books(&mc, &work, true, move |pct, msg| {
                let mapped = 98 + (pct as u16 * 1 / 100) as u8;
                emit_progress(&app_qb, mapped.min(99), msg);
            }) {
                if !o.note.is_empty() {
                    overlay_note = if overlay_note.is_empty() {
                        o.note
                    } else {
                        format!("{overlay_note}；{}", o.note)
                    };
                }
            }
        }
    }
    if !overlay_note.is_empty() {
        if quest_note.is_empty() {
            quest_note = overlay_note;
        } else {
            quest_note = format!("{quest_note}；{overlay_note}");
        }
    }

    let still = remaining_pending(&session.pending_en, &zh);
    let still_n = count_map(&still);
    session.pending_en = still;
    session.pending_count = still_n;
    session.keys_zh = built.keys_total;
    session.pack_path = built.pack_path.clone();
    session.output_dir = work.display().to_string();
    session.note = format!("補翻後剩餘約 {} 條可再補。{}", still_n, quest_note);
    let _ = save_session(&work, &session);

    let _ = write_coverage_report(
        &layout,
        &CoverageStats {
            keys_zh: built.keys_total,
            keys_pending: still_n,
            ai_filled,
            jars_scanned: 0,
            quests_note: quest_note.clone(),
            ref_note: "補翻流程".into(),
            pack_path: built.pack_path.clone(),
            pack_format: 15,
        },
    );

    emit_progress(app, 100, "補翻完成！");

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
        minemenu_msg: None,
        player_summary: format!(
            "補翻完成！目標＝整合包可遊玩文字→台灣繁中（除圖片）；不改 jar。\n\
• 這次 AI 新補 {} 條{}\n\
• 資源包現在約 {} 條中文\n\
• 尚可補約 {} 條\n\
• 結果資料夾：\n{}\n\
• zip：\n{}\n\
• {}\n\
• 見「覆蓋範圍說明.txt」；可用「一鍵套用到遊戲」\n\
（任務／覆寫在結果資料夾 config、patchouli_books、kubejs 等）",
            ai_filled,
            if recovered {
                "（先前資源包遺失，已重建）"
            } else {
                ""
            },
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
) -> Result<OneClickResult, String> {
    let out = normalize_path_strict(&output_dir)?;
    reset_cancel();
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || run_repair(&app2, out, use_ai))
        .await
        .map_err(|e| format!("工作中斷：{e}"))?;
    if let Err(e) = &r {
        report_failure(&app, e);
    }
    r
}

fn run_repair(app: &AppHandle, out: PathBuf, use_ai: bool) -> Result<OneClickResult, String> {
    emit_progress(app, 3, "修復：尋找工作階段…");
    if !out.exists() {
        return Err("輸出資料夾不存在。".into());
    }
    ensure_space(&out, MIN_FREE_BYTES)?;
    let layout = ensure_result_layout(&out)?;
    let work = layout.work_root.clone();
    let (mut session, session_file) = load_session(&out)
        .or_else(|_| load_session(&work))?;
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
    if use_ai && need > 0 {
        emit_progress(app, 40, &format!("修復＋補翻：約 {} 條…", need));
        let app_ai = app.clone();
        let r = fill_missing_with_ai(&mut zh, &pending, true, move |pct, msg| {
            let mapped = 40 + (pct as u16 * 45 / 100) as u8;
            emit_progress(&app_ai, mapped.min(88), msg);
        })?;
        ai_filled = r.filled;
        postprocess_lang_values(&mut zh, &dict);
        actions.push(r.note());
    } else if need > 0 {
        actions.push(format!(
            "尚有約 {} 條缺漏未補（修復本身不強制 AI；可再按「只補缺漏」）。",
            need
        ));
    } else {
        actions.push("沒有待補缺漏。".into());
    }

    // 3) 重產 zip + 對齊 session（寫入「翻譯結果」）
    emit_progress(app, 90, "修復：重產 zip 資源包…");
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

    emit_progress(app, 100, "修復完成！");

    let player_summary = format!(
        "【翻譯資源包修復完成】\n\
（此功能不處理「載入世界閃退」——那是結構／世界生成問題，與語言包無關。）\n\n\
{}\n\n\
• 中文詞約 {} 條\n\
• AI 本次補 {} 條\n\
• 尚可補約 {} 條\n\
• 結果資料夾：\n{}\n\
• zip：\n{}\n\n\
【接下來】\n\
1. 把 zip 複製到遊戲 resourcepacks 並啟用\n\
2. 語言選繁中（台灣）\n\
3. 若還有英文 → 同一根目錄按「只補缺漏」",
        actions
            .iter()
            .map(|a| format!("• {a}"))
            .collect::<Vec<_>>()
            .join("\n"),
        built.keys_total,
        ai_filled,
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
    let (zh_scan, _en, rep) = scan_instance(
        &inst,
        dict,
        true,
        true,
        move |pct, msg| {
            let mapped = 15 + (pct as u16 * 20 / 100) as u8;
            emit_progress(&app_scan, mapped.min(38), msg);
        },
    )?;
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
) -> Result<FontPackResult, String> {
    let font = normalize_path_strict(&font_path)?;
    let out = normalize_path_strict(&output_dir)?;
    let name = sanitize_folder_name(&pack_name)?;
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
        )
    })
    .await
    .map_err(|e| format!("工作中斷：{e}"))?
}

#[tauri::command]
fn save_api_key(key: String) -> Result<String, String> {
    let cur = get_api_settings_public();
    save_api_settings(&key, &cur.base_url)?;
    Ok("已儲存".into())
}

#[tauri::command]
fn save_api_settings_cmd(api_key: String, base_url: String) -> Result<String, String> {
    save_api_settings(&api_key, &base_url)?;
    Ok("已儲存進階設定".into())
}

/// AI 是否可用。永遠 true：有使用者金鑰走自己的，沒有就走開發者代管 Worker。
/// 前端據此不再擋「勾了 AI 但沒金鑰」。
#[tauri::command]
fn has_api_key() -> bool {
    true
}

/// 給 UI 顯示 AI 來源狀態。
/// - `ready`：AI 是否能用（一律 true）
/// - `usingOwnKey`：使用者是否填了自己的金鑰
/// - `managedFree`：是否使用開發者免費代管
#[tauri::command]
fn ai_status() -> serde_json::Value {
    let own = load_deepseek_key().is_some();
    serde_json::json!({
        "ready": true,
        "usingOwnKey": own,
        "managedFree": !own && managed_ai_available(),
        "message": if own {
            "AI：使用你自己的金鑰"
        } else {
            "AI：使用開發者免費提供的翻譯（額度有限，用完可自備金鑰或贊助支持）"
        }
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

/// 下載新版安裝檔並開啟（使用者手動點一次完成）。不自動替換執行檔。
#[tauri::command]
async fn download_update(app: AppHandle) -> Result<serde_json::Value, String> {
    let r = tauri::async_runtime::spawn_blocking(download_and_launch)
        .await
        .map_err(|e| format!("工作中斷：{e}"))?;
    match r {
        Ok(d) => {
            emit_log(&app, "info", &d.message);
            Ok(serde_json::json!({
                "path": d.path,
                "launched": d.launched,
                "message": d.message,
            }))
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

/// 一鍵套用到遊戲：先備份再複製 zip／任務／快捷選單（不改 jar）
#[tauri::command]
async fn apply_translation_to_game(
    app: AppHandle,
    instance_path: String,
    output_dir: String,
    pack_name: Option<String>,
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
        emit_progress(&app2, 40, "套用：備份後複製資源包／任務…");
        let r = apply_to_instance(
            &instance,
            &out,
            pack_name.as_deref(),
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
            create_font_pack,
            save_api_key,
            save_api_settings_cmd,
            has_api_key,
            ai_status,
            get_api_settings,
            get_default_reference_pack,
            get_ui_prefs,
            set_ui_prefs,
            quit_app,
            cancel_task,
            detect_mc_version,
            diagnose_launch_failure,
            restore_last_apply_cmd,
            check_update,
            download_update,
            open_glossary,
            suggest_resourcepacks_dir,
            suggest_output_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
