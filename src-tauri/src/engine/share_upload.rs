//! Uploads only the installable translation payload to the isolated share API.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::Duration;

use super::discord_auth::managed_ai_session_cookie;
use super::secrets::MANAGED_BASE_URL;
use super::share_pack::package_translation;
use super::turnstile::{managed_ai_turnstile_proof, MANAGED_AI_PROTOCOL};

const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareUploadResult {
    pub url: String,
    pub expires_at: u64,
}

pub fn upload_share_package(work_root: &Path, name: &str) -> Result<ShareUploadResult, String> {
    let session = managed_ai_session_cookie()?;
    let proof = managed_ai_turnstile_proof()?;
    let temp_root = std::env::temp_dir().join(format!(
        "modpack-i18n-share-{}-{}",
        std::process::id(),
        epoch_seconds()
    ));
    let _ = fs::create_dir_all(&temp_root);
    let result = (|| {
        let zip = package_translation(work_root, &temp_root, name)?;
        let bytes = fs::read(&zip).map_err(|e| format!("讀取分享檔失敗：{e}"))?;
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err("分享檔超過 100 MB，請先減少輸出內容。".into());
        }
        if !bytes.starts_with(b"PK") {
            return Err("分享檔不是有效 ZIP。".into());
        }

        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|e| format!("無法建立分享連線：{e}"))?;
        let response = client
            .post(format!("{}/api/share/upload", MANAGED_BASE_URL.trim_end_matches('/')))
            .header("Content-Type", "application/zip")
            .header("X-Zeitfrei-AI-Protocol", MANAGED_AI_PROTOCOL)
            .header("X-Zeitfrei-Client-Version", env!("CARGO_PKG_VERSION"))
            .header("X-Zeitfrei-Session", session)
            .header("X-Zeitfrei-Turnstile", proof)
            .header("X-Zeitfrei-Pack-Name", encode_header_value(name))
            .body(bytes)
            .send()
            .map_err(|e| format!("分享檔上傳失敗：{e}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 => "分享前請先登入 Discord。".to_string(),
                403 => "分享前請加入 ZeitFrei Discord 伺服器並完成安全驗證。".to_string(),
                413 => "分享檔太大。".to_string(),
                429 => "今天的分享次數已達上限，請明天再試。".to_string(),
                _ => format!("分享服務回應錯誤（HTTP {status}）。"),
            });
        }
        response
            .json::<ShareUploadResult>()
            .map_err(|e| format!("分享服務回應格式錯誤：{e}"))
    })();
    let _ = fs::remove_dir_all(&temp_root);
    result
}

fn epoch_seconds() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn encode_header_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            out.push(*byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}
