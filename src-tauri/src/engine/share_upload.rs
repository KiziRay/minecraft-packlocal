//! Uploads the NanaZip self-extracting share package to the isolated share API.
//!
//! 一律走 R2 multipart（create → part×N → complete），繞過 Worker 單次 body ≈100MB。
//! 軟頂見 [`SHARE_MAX_UPLOAD_BYTES`]（4GiB）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::Duration;

use super::discord_auth::managed_ai_session_cookie;
use super::secrets::MANAGED_BASE_URL;
use super::share_pack::{package_translation_sfx, SHARE_MAX_UPLOAD_BYTES, SHARE_MPU_PART_BYTES};
use super::turnstile::MANAGED_AI_PROTOCOL;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareUploadResult {
    pub url: String,
    pub expires_at: u64,
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

pub fn upload_share_package(work_root: &Path, name: &str) -> Result<ShareUploadResult, String> {
    let session = managed_ai_session_cookie()?;
    let temp_root = std::env::temp_dir().join(format!(
        "modpack-i18n-share-{}-{}",
        std::process::id(),
        epoch_seconds()
    ));
    let _ = fs::create_dir_all(&temp_root);
    let result = (|| {
        let sfx = package_translation_sfx(work_root, &temp_root, name)?;
        let bytes = fs::read(&sfx).map_err(|e| format!("讀取分享檔失敗：{e}"))?;
        if bytes.len() < 64 || !(bytes.starts_with(b"MZ") || bytes.starts_with(b"PK")) {
            return Err("分享檔不是有效的自解 exe。".into());
        }
        if bytes.len() as u64 > SHARE_MAX_UPLOAD_BYTES {
            let gb = bytes.len() as f64 / (1024.0 * 1024.0 * 1024.0);
            return Err(format!(
                "分享檔約 {gb:.1} GB，超過雲端軟頂 4 GB。請精簡翻譯結果後再試；本機套用不受影響。"
            ));
        }

        let base = MANAGED_BASE_URL.trim_end_matches('/');
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| format!("無法建立分享連線：{e}"))?;

        let create = client
            .post(format!("{base}/api/share/mpu-create"))
            .header("Content-Type", "application/json")
            .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
            .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
            .header("X-Zeitfrei-Session", &session)
            .header("X-Zeitfrei-Pack-Name", encode_header_value(name))
            .header("X-Zeitfrei-Share-Kind", "sfx-exe")
            .json(&serde_json::json!({
                "name": name,
                "kind": "sfx-exe",
                "size": bytes.len(),
                "contentType": "application/vnd.microsoft.portable-executable",
            }))
            .send()
            .map_err(|e| format!("分享上傳初始化失敗：{e}"))?;
        let create_status = create.status();
        if !create_status.is_success() {
            return Err(map_share_http_error(create_status.as_u16(), "初始化"));
        }
        let created: MpuCreateResponse = create
            .json()
            .map_err(|e| format!("分享上傳初始化回應錯誤：{e}"))?;
        if created.token.is_empty() || created.key.is_empty() || created.upload_id.is_empty() {
            return Err("分享上傳初始化缺少 token／key／uploadId。".into());
        }

        let part_size = created
            .part_size
            .unwrap_or(SHARE_MPU_PART_BYTES as u64)
            .clamp(5 * 1024 * 1024, 90 * 1024 * 1024) as usize;
        let mut uploaded_parts: Vec<MpuCompletePart> = Vec::new();
        let mut part_number: u32 = 1;
        let mut offset = 0usize;
        while offset < bytes.len() {
            let end = (offset + part_size).min(bytes.len());
            let chunk = &bytes[offset..end];
            let part_url = format!(
                "{base}/api/share/mpu-part?key={}&uploadId={}&partNumber={part_number}",
                encode_header_value(&created.key),
                encode_header_value(&created.upload_id),
            );
            let part_resp = client
                .put(&part_url)
                .header(
                    "Content-Type",
                    "application/vnd.microsoft.portable-executable",
                )
                .header("Content-Length", chunk.len().to_string())
                .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
                .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
                .header("X-Zeitfrei-Session", &session)
                .body(chunk.to_vec())
                .send()
                .map_err(|e| format!("分享分塊上傳失敗（第 {part_number} 塊）：{e}"))?;
            let part_status = part_resp.status();
            if !part_status.is_success() {
                return Err(map_share_http_error(
                    part_status.as_u16(),
                    &format!("分塊 {part_number}"),
                ));
            }
            let part: MpuPartResponse = part_resp
                .json()
                .map_err(|e| format!("分享分塊回應錯誤：{e}"))?;
            uploaded_parts.push(MpuCompletePart {
                part_number: part.part_number.max(part_number),
                etag: part.etag,
            });
            offset = end;
            part_number = part_number.saturating_add(1);
        }

        let complete = client
            .post(format!("{base}/api/share/mpu-complete"))
            .header("Content-Type", "application/json")
            .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
            .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
            .header("X-Zeitfrei-Session", &session)
            .json(&serde_json::json!({
                "token": created.token,
                "key": created.key,
                "uploadId": created.upload_id,
                "parts": uploaded_parts,
            }))
            .send()
            .map_err(|e| format!("分享上傳完成失敗：{e}"))?;
        let complete_status = complete.status();
        if !complete_status.is_success() {
            return Err(map_share_http_error(complete_status.as_u16(), "完成"));
        }
        complete
            .json::<ShareUploadResult>()
            .map_err(|e| format!("分享服務回應格式錯誤：{e}"))
    })();
    let _ = fs::remove_dir_all(&temp_root);
    result
}

fn map_share_http_error(code: u16, stage: &str) -> String {
    match code {
        401 => "分享前請先登入 Discord。".to_string(),
        403 => "分享前請加入 ZeitFrei Discord 伺服器。".to_string(),
        413 => "分享檔太大（雲端軟頂約 4 GB）。".to_string(),
        429 => "今天的分享次數已達上限，請明天再試。".to_string(),
        _ => format!("分享服務回應錯誤（{stage}，HTTP {code}）。"),
    }
}

fn epoch_seconds() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn encode_header_value(value: &str) -> String {
    urlencoding_encode(value)
}

fn urlencoding_encode(value: &str) -> String {
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
