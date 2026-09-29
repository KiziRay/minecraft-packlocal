use regex::Regex;
use serde::{Deserialize, Serialize};

use super::secrets::managed_base_url;

const FEEDBACK_NOTE_MAX_CHARS: usize = 800;
const FEEDBACK_CLIENT_ID_RE: &str = r"^[A-Za-z0-9_-]{8,64}$";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitUsageFeedbackCmdResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerErrorEnvelope {
    error: Option<WorkerError>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerError {
    #[serde(default)]
    message: String,
    #[serde(default)]
    #[serde(rename = "type")]
    error_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerSubmitFeedbackResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

fn worker_url(path: &str) -> String {
    let base = managed_base_url().trim_end_matches('/');
    format!("{base}{path}")
}

fn sanitize_client_id(client_id: &str) -> Result<String, String> {
    let re = Regex::new(FEEDBACK_CLIENT_ID_RE).map_err(|e| e.to_string())?;
    let s = client_id.trim();
    if !re.is_match(s) {
        return Err("clientId 格式不正確。".into());
    }
    Ok(s.to_string())
}

fn sanitize_note(note: Option<String>) -> Result<Option<String>, String> {
    let Some(n) = note else { return Ok(None) };
    let cleaned = n.replace('\0', "");
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let limited: String = trimmed.chars().take(FEEDBACK_NOTE_MAX_CHARS).collect();
    Ok(Some(limited))
}

fn sanitize_feedback_tag(value: Option<String>, max_chars: usize) -> Result<Option<String>, String> {
    let Some(v) = value else { return Ok(None) };
    let cleaned = v.replace('\0', "").trim().to_string();
    if cleaned.is_empty() {
        return Ok(None);
    }
    let limited: String = cleaned.chars().take(max_chars).collect();
    Ok(Some(limited))
}

fn parse_worker_error_type(body_text: &str) -> (Option<String>, Option<String>) {
    // 依目前 worker 設計：{ error: { message, type } } 或 { ok:false, error:"..." }
    if let Ok(env) = serde_json::from_str::<WorkerErrorEnvelope>(body_text) {
        if let Some(err) = env.error {
            let t = if err.error_type.trim().is_empty() {
                None
            } else {
                Some(err.error_type.trim().to_string())
            };
            let m = if err.message.trim().is_empty() {
                None
            } else {
                Some(err.message.trim().to_string())
            };
            return (t, m);
        }
    }

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body_text) {
        let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
        if !ok {
            if let Some(e) = v.get("error").and_then(|x| x.as_str()) {
                return (Some(e.to_string()), None);
            }
        }
    }
    (None, None)
}

pub fn submit_usage_feedback_cmd(
    client_id: String,
    rating: Option<u8>,
    note: Option<String>,
    pain_point: Option<String>,
    wish: Option<String>,
) -> Result<SubmitUsageFeedbackCmdResult, String> {
    let client_id = sanitize_client_id(&client_id)?;
    if let Some(r) = rating {
        if !(1..=5).contains(&r) {
            return Ok(SubmitUsageFeedbackCmdResult {
                ok: false,
                error_type: Some("rating invalid".into()),
                message: None,
            });
        }
    }
    let note = sanitize_note(note)?;
    let pain_point = sanitize_feedback_tag(pain_point, 48)?;
    let wish = sanitize_feedback_tag(wish, 48)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut body = serde_json::Map::new();
    body.insert("clientId".into(), serde_json::Value::String(client_id));
    if let Some(r) = rating {
        body.insert("rating".into(), serde_json::Value::from(r));
    }
    if let Some(n) = note {
        body.insert("note".into(), serde_json::Value::String(n));
    }
    if let Some(p) = pain_point {
        body.insert("painPoint".into(), serde_json::Value::String(p));
    }
    if let Some(w) = wish {
        body.insert("wish".into(), serde_json::Value::String(w));
    }
    body.insert(
        "toolVersion".into(),
        serde_json::Value::String(env!("CARGO_PKG_VERSION").to_string()),
    );

    let resp = client
        .post(worker_url("/api/feedback/submit"))
        .header("content-type", "application/json")
        .body(serde_json::Value::Object(body).to_string())
        .send()
        .map_err(|e| format!("feedback submit 失敗：{e}"))?;

    let status = resp.status();
    let body_text = resp.text().unwrap_or_default();
    if status.is_success() {
        let parsed: WorkerSubmitFeedbackResponse =
            serde_json::from_str(&body_text).unwrap_or(WorkerSubmitFeedbackResponse {
                ok: false,
                error: Some("parse failed".into()),
            });
        if parsed.ok {
            return Ok(SubmitUsageFeedbackCmdResult {
                ok: true,
                error_type: None,
                message: None,
            });
        }
        return Ok(SubmitUsageFeedbackCmdResult {
            ok: false,
            error_type: parsed.error,
            message: None,
        });
    }

    let (error_type, message) = parse_worker_error_type(&body_text);
    Ok(SubmitUsageFeedbackCmdResult {
        ok: false,
        error_type,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_client_id_rejects_bad() {
        assert!(sanitize_client_id(" ").is_err());
        assert!(sanitize_client_id("a").is_err());
        assert!(sanitize_client_id("中文id").is_err());
        assert!(sanitize_client_id("Abc-123_foo").is_ok());
    }

    #[test]
    fn sanitize_note_truncates() {
        let note = "a".repeat(FEEDBACK_NOTE_MAX_CHARS + 10);
        let out = sanitize_note(Some(note)).unwrap().unwrap();
        assert_eq!(out.chars().count(), FEEDBACK_NOTE_MAX_CHARS);
    }

    #[test]
    fn sanitize_note_empty_becomes_none() {
        assert!(sanitize_note(Some("   \0  ".to_string()))
            .unwrap()
            .is_none());
    }

    #[test]
    fn sanitize_feedback_tag_truncates() {
        let tag = "a".repeat(60);
        let out = sanitize_feedback_tag(Some(tag), 48).unwrap().unwrap();
        assert_eq!(out.chars().count(), 48);
    }
}

