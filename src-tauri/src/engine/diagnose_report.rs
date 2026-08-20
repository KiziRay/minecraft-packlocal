//! 診斷多類回報：打包白名單檔 → MPU 上傳 SHARES `reports/v1/`。
//! Webhook 只由 Worker 送連結；客戶端永不持有 webhook URL。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::discord_auth::managed_ai_session_cookie;
use super::jar_scan::resolve_minecraft_dir;
use super::out_layout::RESULT_DIR_NAME;
use super::secrets::MANAGED_BASE_URL;
use super::session::SESSION_FILE;
use super::turnstile::MANAGED_AI_PROTOCOL;

pub const REPORT_MAX_BYTES: usize = 100 * 1024 * 1024;
const REPORT_FILE_CAP: usize = 2 * 1024 * 1024;
const REPORT_MPU_PART: usize = 8 * 1024 * 1024;

pub const REPORT_CATEGORIES: &[&str] = &[
    "crash_after_apply",
    "crash_on_world",
    "crash_on_book_quest",
    "ui_mojibake",
    "ui_tofu",
    "still_english",
    "bad_translation",
    "shared_lib_suspect",
    "placeholder_broken",
    "pack_unsupported",
    "pack_partial_support",
    "loader_version",
    "missing_source",
    "tool_crash",
    "tool_one_click_fail",
    "tool_apply_fail",
    "tool_restore_fail",
    "tool_update_fail",
    "tool_share_fail",
    "tool_ai_managed",
    "tool_ai_custom",
    "tool_ui",
    "other_feature",
    "other_docs",
    "other_privacy",
    "other",
];

const ALLOWED_EXACT: &[&str] = &[
    "manifest.json",
    "user_note.txt",
    "diagnosis.json",
    "crash.txt",
    "latest.log",
    "翻譯工作階段.json",
    "覆蓋範圍說明.txt",
    "翻譯錯誤日誌.txt",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseReportRequest {
    pub report_category: String,
    #[serde(default)]
    pub pack_name: Option<String>,
    #[serde(default)]
    pub pack_unrelated: bool,
    #[serde(default)]
    pub pack_version: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub user_note: Option<String>,
    #[serde(default)]
    pub instance_path: Option<String>,
    #[serde(default)]
    pub output_dir: Option<String>,
    #[serde(default)]
    pub diagnosis_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseReportResult {
    pub expires_at: u64,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MpuCreateResponse {
    token: String,
    key: String,
    upload_id: String,
    #[serde(default)]
    part_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MpuPartResponse {
    part_number: u32,
    etag: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MpuCompletePart {
    part_number: u32,
    etag: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MpuCompleteResponse {
    #[serde(default)]
    expires_at: u64,
    #[serde(default)]
    message: String,
}

pub fn is_report_category(value: &str) -> bool {
    REPORT_CATEGORIES.contains(&value)
}

pub fn is_allowed_report_name(name: &str) -> bool {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    if ALLOWED_EXACT.contains(&name) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.starts_with("crash") && lower.ends_with(".txt")
}

pub fn redact_session_json(raw: &str) -> String {
    let mut value: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return json!({ "note": "<unparseable>", "redacted": true }).to_string(),
    };
    if let Some(obj) = value.as_object_mut() {
        for key in [
            "instancePath",
            "instance_path",
            "outputDir",
            "output_dir",
            "packPath",
            "pack_path",
        ] {
            if obj.contains_key(key) {
                obj.insert(key.to_string(), json!("<redacted>"));
            }
        }
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{\"redacted\":true}".into())
}

pub fn submit_diagnose_report(req: &DiagnoseReportRequest) -> Result<DiagnoseReportResult, String> {
    if !is_report_category(req.report_category.trim()) {
        return Err("請選擇回報類別。".into());
    }
    let pack_unrelated = req.pack_unrelated;
    let pack_name = req.pack_name.clone().unwrap_or_default();
    if !pack_unrelated && pack_name.trim().is_empty() {
        return Err("請填整合包名稱，或勾選「與包無關」。".into());
    }

    let files = collect_report_files(req)?;
    let bytes = build_report_zip(&files)?;
    if bytes.len() < 4 || bytes[0] != 0x50 || bytes[1] != 0x4b {
        return Err("回報打包失敗（不是有效 zip）。".into());
    }
    if bytes.len() > REPORT_MAX_BYTES {
        return Err("回報檔超過 100MB，請減少附檔後再試。".into());
    }
    upload_report_zip(req, &bytes)
}

fn collect_report_files(req: &DiagnoseReportRequest) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0usize;
    let mut push = |name: String, data: Vec<u8>| -> Result<(), String> {
        if !is_allowed_report_name(&name) {
            return Ok(());
        }
        if data.is_empty() {
            return Ok(());
        }
        if total.saturating_add(data.len()) > REPORT_MAX_BYTES {
            return Err("回報內容超過 100MB。".into());
        }
        total += data.len();
        files.push((name, data));
        Ok(())
    };

    let manifest = json!({
        "schemaVersion": 1,
        "reportCategory": req.report_category,
        "packName": if req.pack_unrelated { "與包無關" } else { req.pack_name.as_deref().unwrap_or("") },
        "packUnrelated": req.pack_unrelated,
        "packVersion": req.pack_version,
        "errorCode": req.error_code,
        "toolVersion": env!("CARGO_PKG_VERSION"),
    });
    push(
        "manifest.json".into(),
        serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    )?;

    if let Some(note) = req.user_note.as_deref() {
        let trimmed = note.trim();
        if !trimmed.is_empty() {
            push(
                "user_note.txt".into(),
                trimmed.chars().take(2000).collect::<String>().into_bytes(),
            )?;
        }
    }
    if let Some(diag) = req.diagnosis_json.as_deref() {
        let trimmed = diag.trim();
        if !trimmed.is_empty() {
            let capped = if trimmed.len() > REPORT_FILE_CAP {
                &trimmed[..REPORT_FILE_CAP]
            } else {
                trimmed
            };
            push("diagnosis.json".into(), capped.as_bytes().to_vec())?;
        }
    }

    if let Some(out) = req.output_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let root = PathBuf::from(out);
        let work = [root.clone(), root.join(RESULT_DIR_NAME)]
            .into_iter()
            .find(|p| p.join(SESSION_FILE).is_file() || p.join("覆蓋範圍說明.txt").is_file())
            .unwrap_or(root);
        add_work_file(&work, SESSION_FILE, &mut push)?;
        add_work_file(&work, "覆蓋範圍說明.txt", &mut push)?;
        add_work_file(&work, "翻譯錯誤日誌.txt", &mut push)?;
    }

    if let Some(inst) = req.instance_path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let inst_path = PathBuf::from(inst);
        let mc = resolve_minecraft_dir(&inst_path).unwrap_or(inst_path);
        add_capped_file(mc.join("logs").join("latest.log"), "latest.log", &mut push)?;
        add_crash_reports(&mc.join("crash-reports"), &mut push)?;
    }

    if files.len() < 2 {
        return Err("沒有可附帶的診斷資料。請先分析錯誤，或選整合包／結果資料夾。".into());
    }
    Ok(files)
}

fn add_work_file(
    work: &Path,
    name: &str,
    push: &mut impl FnMut(String, Vec<u8>) -> Result<(), String>,
) -> Result<(), String> {
    add_capped_file(work.join(name), name, push)
}

fn add_capped_file(
    path: PathBuf,
    name: &str,
    push: &mut impl FnMut(String, Vec<u8>) -> Result<(), String>,
) -> Result<(), String> {
    if !path.is_file() {
        return Ok(());
    }
    let mut data = fs::read(&path).unwrap_or_default();
    if name == SESSION_FILE {
        let raw = String::from_utf8_lossy(&data).into_owned();
        data = redact_session_json(&raw).into_bytes();
    }
    if data.len() > REPORT_FILE_CAP {
        data.truncate(REPORT_FILE_CAP);
    }
    push(name.to_string(), data)
}

fn add_crash_reports(
    dir: &Path,
    push: &mut impl FnMut(String, Vec<u8>) -> Result<(), String>,
) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|s| s.to_str())
                    .map(is_allowed_report_name)
                    .unwrap_or(false)
        })
        .collect();
    entries.sort();
    entries.reverse();
    for (i, path) in entries.into_iter().take(3).enumerate() {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("crash-{i}.txt"));
        add_capped_file(path, &name, push)?;
    }
    Ok(())
}

fn build_report_zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, data) in files {
            zip.start_file(name, opts)
                .map_err(|e| format!("打包回報失敗：{e}"))?;
            zip.write_all(data)
                .map_err(|e| format!("寫入回報失敗：{e}"))?;
        }
        zip.finish().map_err(|e| format!("完成回報 zip 失敗：{e}"))?;
    }
    Ok(cursor.into_inner())
}

fn upload_report_zip(req: &DiagnoseReportRequest, bytes: &[u8]) -> Result<DiagnoseReportResult, String> {
    let session = managed_ai_session_cookie()?;
    let base = MANAGED_BASE_URL.trim_end_matches('/');
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("無法建立回報連線：{e}"))?;

    let create = client
        .post(format!("{base}/api/report/mpu-create"))
        .header("Content-Type", "application/json")
        .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
        .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
        .header("X-Zeitfrei-Session", &session)
        .json(&json!({
            "reportCategory": req.report_category,
            "packName": req.pack_name,
            "packUnrelated": req.pack_unrelated,
            "packVersion": req.pack_version,
            "errorCode": req.error_code,
            "toolVersion": env!("CARGO_PKG_VERSION"),
            "size": bytes.len() as u64,
        }))
        .send()
        .map_err(|e| format!("回報建立失敗：{e}"))?;
    let create_status = create.status();
    if !create_status.is_success() {
        return Err(map_report_http_error(create_status.as_u16(), "建立"));
    }
    let created: MpuCreateResponse = create
        .json()
        .map_err(|e| format!("回報服務回應錯誤：{e}"))?;

    let part_size = created
        .part_size
        .unwrap_or(REPORT_MPU_PART as u64)
        .clamp(5 * 1024 * 1024, 90 * 1024 * 1024) as usize;
    let mut uploaded_parts: Vec<MpuCompletePart> = Vec::new();
    let mut part_number: u32 = 1;
    let mut offset = 0usize;
    while offset < bytes.len() {
        let end = (offset + part_size).min(bytes.len());
        let chunk = &bytes[offset..end];
        let part_url = format!(
            "{base}/api/report/mpu-part?key={}&uploadId={}&partNumber={part_number}",
            url_encode(&created.key),
            url_encode(&created.upload_id),
        );
        let part_resp = client
            .put(&part_url)
            .header("Content-Type", "application/zip")
            .header("Content-Length", chunk.len().to_string())
            .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
            .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
            .header("X-Zeitfrei-Session", &session)
            .body(chunk.to_vec())
            .send()
            .map_err(|e| format!("回報分塊上傳失敗（第 {part_number} 塊）：{e}"))?;
        if !part_resp.status().is_success() {
            return Err(map_report_http_error(part_resp.status().as_u16(), "分塊"));
        }
        let part: MpuPartResponse = part_resp
            .json()
            .map_err(|e| format!("回報分塊回應錯誤：{e}"))?;
        uploaded_parts.push(MpuCompletePart {
            part_number: part.part_number.max(part_number),
            etag: part.etag,
        });
        offset = end;
        part_number = part_number.saturating_add(1);
    }

    let complete = client
        .post(format!("{base}/api/report/mpu-complete"))
        .header("Content-Type", "application/json")
        .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
        .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
        .header("X-Zeitfrei-Session", &session)
        .json(&json!({
            "token": created.token,
            "key": created.key,
            "uploadId": created.upload_id,
            "parts": uploaded_parts,
        }))
        .send()
        .map_err(|e| format!("回報完成失敗：{e}"))?;
    if !complete.status().is_success() {
        return Err(map_report_http_error(complete.status().as_u16(), "完成"));
    }
    let done: MpuCompleteResponse = complete
        .json()
        .map_err(|e| format!("回報完成回應錯誤：{e}"))?;
    Ok(DiagnoseReportResult {
        expires_at: done.expires_at,
        message: if done.message.trim().is_empty() {
            "已送出（資料 3 天內刪除）".into()
        } else {
            done.message
        },
    })
}

fn map_report_http_error(code: u16, stage: &str) -> String {
    match code {
        401 => "回報前請先登入 Discord。".into(),
        403 => "回報前請加入 ZeitFrei Discord 伺服器。".into(),
        411 => "回報內容為空。".into(),
        413 => "回報檔太大（上限 100MB）。".into(),
        415 => "回報檔格式不正確。".into(),
        429 => "今天的診斷回報次數已達上限，請明天再試。".into(),
        502 => "回報通知失敗，請稍後再試。".into(),
        503 => "診斷回報尚未設定完成（USAGE 或通知通道）。".into(),
        _ => format!("診斷回報{stage}失敗（{code}）。"),
    }
}

fn url_encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_category_and_path_names() {
        assert!(is_report_category("crash_after_apply"));
        assert!(!is_report_category(""));
        assert!(is_allowed_report_name("latest.log"));
        assert!(is_allowed_report_name("翻譯工作階段.json"));
        assert!(is_allowed_report_name("crash-2026-08-17.txt"));
        assert!(!is_allowed_report_name("../secrets.txt"));
        assert!(!is_allowed_report_name("a/b.txt"));
        assert!(!is_allowed_report_name("api-key.txt"));
    }

    #[test]
    fn session_paths_are_redacted() {
        let raw = r#"{"instancePath":"C:\\Users\\me\\curseforge\\pack","outputDir":"D:\\out","packPath":"D:\\out\\a.zip","pendingCount":3}"#;
        let redacted = redact_session_json(raw);
        assert!(redacted.contains("<redacted>"));
        assert!(!redacted.contains("curseforge"));
        assert!(!redacted.contains("Users"));
    }

    #[test]
    fn zip_under_limit_has_pk_header() {
        let zip = build_report_zip(&[("manifest.json".into(), b"{}".to_vec())]).unwrap();
        assert!(zip.starts_with(b"PK"));
        assert!(zip.len() < REPORT_MAX_BYTES);
    }
}
