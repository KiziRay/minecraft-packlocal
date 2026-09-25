//! Footer 問題回報：帶 Discord session 打 Worker /api/issue-thread。不直連 Discord。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::secrets::MANAGED_BASE_URL;
use super::turnstile::MANAGED_AI_PROTOCOL;

pub const ISSUE_SUMMARIES: &[&str] = &[
    "翻譯結果不對或沒翻到",
    "套用後遊戲異常",
    "本地模型／AI 無法使用",
    "介面或縮放",
    "分享給其他玩家",
    "其他",
];

pub const ISSUE_CAUSES: &[&str] = &[
    "剛更新工具或整合包",
    "操作後立刻發生",
    "只有特定整合包",
    "看不懂畫面上的說明",
    "不確定",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitIssueReportResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkerIssueResponse {
    ok: Option<bool>,
    #[serde(rename = "caseId", default)]
    case_id: String,
    #[serde(default)]
    delivery: String,
    #[serde(default)]
    message: String,
    error: Option<WorkerIssueError>,
}

#[derive(Debug, Deserialize)]
struct WorkerIssueError {
    #[serde(default)]
    message: String,
    #[serde(default)]
    #[serde(rename = "type")]
    error_type: String,
}

pub fn sanitize_issue_text(raw: &str) -> String {
    let stripped = regex::Regex::new(r"(?i)@(?:everyone|here)")
        .ok()
        .map(|re| re.replace_all(raw, "").into_owned())
        .unwrap_or_else(|| raw.to_string());
    stripped
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn validate_issue_fields(
    summary: &str,
    cause: &str,
    detail: Option<&str>,
) -> Result<(String, String, Option<String>), String> {
    let summary = summary.trim();
    if !ISSUE_SUMMARIES.contains(&summary) {
        return Err("請選擇問題概要".into());
    }
    let cause = cause.trim();
    if !ISSUE_CAUSES.contains(&cause) {
        return Err("請選擇問題原因".into());
    }
    let detail = match detail {
        None => None,
        Some(raw) if raw.trim().is_empty() => None,
        Some(raw) => {
            let cleaned = sanitize_issue_text(raw);
            if cleaned.is_empty() {
                None
            } else if cleaned.chars().count() <= 10 {
                return Err("詳細說明請超過十個字".into());
            } else {
                Some(cleaned.chars().take(500).collect())
            }
        }
    };
    Ok((summary.to_string(), cause.to_string(), detail))
}

fn discord_headers() -> Result<Vec<(String, String)>, String> {
    let session = super::discord_auth::managed_ai_session_cookie()?;
    Ok(vec![
        (
            "X-Zeitfrei-AI-Protocol".into(),
            MANAGED_AI_PROTOCOL.to_string(),
        ),
        ("X-Zeitfrei-Session".into(), session),
    ])
}

/// 上游有給訊息就用上游的，否則退回我們的預設句。
///
/// 罐頭訊息只該當「真的沒有更好的資訊」時的保底，不能用來蓋掉伺服器講的實話。
fn pick_message(upstream: &str, fallback: &str) -> String {
    let trimmed = upstream.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn fail(error_type: &str, message: &str) -> SubmitIssueReportResult {
    SubmitIssueReportResult {
        ok: false,
        case_id: None,
        delivery: None,
        error_type: Some(error_type.into()),
        message: Some(message.into()),
    }
}

static ISSUE_REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 回報視窗在逾時後重按時必須沿用同一把 key，Worker 才能回傳第一次已建立的案件，
/// 而不是又開一條私人討論串。格式與 Worker 的 `[A-Za-z0-9_-]{8,64}` 契約一致。
pub fn issue_idempotency_key(provided: Option<&str>) -> String {
    let raw = provided.unwrap_or("").trim();
    if (8..=64).contains(&raw.len())
        && raw
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return raw.to_string();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let sequence = ISSUE_REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("mcpl-{now:x}-{sequence:x}")
}

pub fn submit_issue_report(
    summary: String,
    cause: String,
    detail: Option<String>,
    idempotency_key: Option<String>,
) -> SubmitIssueReportResult {
    let (summary, cause, detail) = match validate_issue_fields(&summary, &cause, detail.as_deref()) {
        Ok(v) => v,
        Err(m) => return fail("invalid_body", &m),
    };
    let headers = match discord_headers() {
        Ok(h) => h,
        Err(_) => {
            return fail(
                "login_required",
                "請先登入 Discord 並加入官方伺服器，方便維護、收集建議與調整工具。",
            )
        }
    };
    let url = format!(
        "{}/api/issue-thread",
        MANAGED_BASE_URL.trim_end_matches('/')
    );
    let idempotency_key = issue_idempotency_key(idempotency_key.as_deref());
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        // Worker 端需要建串、加入成員與寫入訊息；v30 的最壞路徑超過 25 秒，
        // 造成「私人討論串其實已建立、工具卻說連線失敗」。45 秒涵蓋 Discord
        // 慢回覆，仍不是無限等待。
        .timeout(std::time::Duration::from_secs(45))
        .build()
    {
        Ok(c) => c,
        Err(_) => return fail("request_failed", "暫時無法送出，請稍後再試"),
    };
    let mut req = client.post(&url).json(&serde_json::json!({
        "summary": summary,
        "cause": cause,
        "detail": detail.unwrap_or_default(),
        "toolVersion": env!("CARGO_PKG_VERSION"),
        "idempotencyKey": idempotency_key,
    }));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = match req.send() {
        Ok(r) => r,
        Err(_) => return fail("request_failed", "連不上翻譯雲端，請檢查網路後再試"),
    };
    let status = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    interpret_worker_issue(status, &text)
}

fn interpret_worker_issue(status: u16, text: &str) -> SubmitIssueReportResult {
    if let Ok(parsed) = serde_json::from_str::<WorkerIssueResponse>(text) {
        if parsed.ok == Some(true) && (200..300).contains(&status) {
            return SubmitIssueReportResult {
                ok: true,
                case_id: (!parsed.case_id.trim().is_empty()).then_some(parsed.case_id),
                delivery: (!parsed.delivery.trim().is_empty()).then_some(parsed.delivery),
                error_type: None,
                message: Some(pick_message(
                    &parsed.message,
                    "已送給站長，正在建立討論串。",
                )),
            };
        }
        let et = parsed
            .error
            .as_ref()
            .map(|e| e.error_type.trim())
            .filter(|t| !t.is_empty())
            .unwrap_or("");
        let msg = parsed
            .error
            .as_ref()
            .map(|e| e.message.trim())
            .filter(|m| !m.is_empty())
            .unwrap_or("");
        // 伺服器有給訊息就用它的，不要用我們自己編的蓋掉。
        //
        // 使用者實測：人確實在伺服器裡、bot 的 GUILD_MEMBERS 也開了，卻一直看到
        // 「請先加入官方 Discord 伺服器」。原因就是這裡——不管上游回什麼，
        // 403 一律被改寫成這句罐頭訊息，真正的失敗原因（可能是通道權限、
        // Cloudflare Access 擋下、頻道權限不足…）完全看不到，害人往錯的方向修。
        if status == 401 || et == "login_required" {
            return fail("login_required", &pick_message(msg, "請先登入 Discord。"));
        }
        if status == 403 || et == "guild_required" {
            return fail(
                "guild_required",
                &pick_message(msg, "Discord 端拒絕了這次回報。請直接到官方 Discord 告訴我們。"),
            );
        }
        if status == 429 || et == "rate_limited" {
            return fail("rate_limited", "今天已回報三次，請明天再試。");
        }
        if et == "auth_unavailable" {
            return fail("auth_unavailable", "登入狀態暫時無法確認，請稍後再試。");
        }
        if !msg.is_empty() && !(200..300).contains(&status) {
            return fail(if et.is_empty() { "upstream_unavailable" } else { et }, msg);
        }
    }
    if status == 401 {
        return fail("login_required", "請先登入 Discord。");
    }
    if status == 403 {
        // 連 JSON 都解析不出來的 403：多半不是「沒加入伺服器」，而是中間某一層
        // （反向代理、Cloudflare Access、頻道權限）擋掉了。訊息要講得夠中性，
        // 才不會讓使用者一直去做「重新加入伺服器」這件沒用的事。
        return fail(
            "guild_required",
            "Discord 端拒絕了這次回報（可能是通道權限設定）。請直接到官方 Discord 告訴我們。",
        );
    }
    if status == 429 {
        return fail("rate_limited", "今天已回報三次，請明天再試。");
    }
    if status == 400 {
        return fail("invalid_body", "請檢查回報內容後再試");
    }
    fail(
        "upstream_unavailable",
        "站長聯絡通道暫時離線，請稍後再試，或直接到官方 Discord 告訴我們",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_illegal_options_and_short_detail() {
        assert!(validate_issue_fields("聯絡站長", "不確定", None).is_err());
        assert!(validate_issue_fields("其他", "隨便", None).is_err());
        assert!(validate_issue_fields("其他", "不確定", Some("1234567890")).is_err());
        assert!(validate_issue_fields("其他", "不確定", Some("這段詳細說明超過十個字了")).is_ok());
        assert!(validate_issue_fields("其他", "不確定", None).is_ok());
    }

    #[test]
    fn strips_broadcast_mentions() {
        let cleaned = sanitize_issue_text("@everyone @HERE 這段詳細說明超過十個字了");
        assert!(!cleaned.to_lowercase().contains("everyone"));
        assert!(!cleaned.to_lowercase().contains("here"));
        assert!(validate_issue_fields("其他", "不確定", Some("@everyone 這段詳細說明超過十個字了")).is_ok());
    }

    #[test]
    fn worker_503_bot_message_is_shown_not_generic() {
        let r = interpret_worker_issue(
            503,
            r#"{"ok":false,"error":{"message":"站長聯絡通道暫時離線，請稍後再試，或直接到官方 Discord 告訴我們","type":"bot_unavailable"}}"#,
        );
        assert!(!r.ok);
        assert_eq!(r.error_type.as_deref(), Some("bot_unavailable"));
        assert!(r.message.unwrap_or_default().contains("離線"));
    }

    #[test]
    fn worker_ok_true_is_success() {
        let r = interpret_worker_issue(200, r#"{"ok":true,"caseId":"MCPL-20260904-ABCDEF","delivery":"delivered","message":"已建立私人討論串"}"#);
        assert!(r.ok);
        assert_eq!(r.case_id.as_deref(), Some("MCPL-20260904-ABCDEF"));
        assert_eq!(r.delivery.as_deref(), Some("delivered"));
        assert_eq!(r.message.as_deref(), Some("已建立私人討論串"));
    }

    #[test]
    fn idempotency_key_keeps_valid_ui_key_and_replaces_bad_input() {
        assert_eq!(
            issue_idempotency_key(Some("mcpl-retry_key-123")),
            "mcpl-retry_key-123"
        );
        let generated = issue_idempotency_key(Some("bad key"));
        assert!(generated.starts_with("mcpl-"));
        assert!(generated.len() >= 8);
    }
}
