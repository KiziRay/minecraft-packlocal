use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

use super::codex_auth::{ensure_fresh_access_token, refresh_access_token_force, GptAccessContext};

pub const CODEX_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

const CODEX_USER_AGENT: &str =
    "codex-tui/0.135.0 (Mac OS 26.5.0; arm64) iTerm.app/3.6.10 (codex-tui; 0.135.0)";
const CODEX_ORIGINATOR: &str = "codex-tui";

#[derive(Debug, Clone)]
pub struct CodexChatError {
    pub status_code: Option<u16>,
    pub message: String,
    /// B4：伺服器要求等多久再試（`Retry-After`）；沒給就是 `None`
    pub retry_after: Option<Duration>,
}

pub fn complete_chat(
    client: &reqwest::blocking::Client,
    model: &str,
    request: &Value,
    timeout: Duration,
) -> Result<Value, CodexChatError> {
    let payload = build_codex_request(model, request);
    let auth = ensure_fresh_access_token().map_err(auth_error)?;
    match send_once(client, &auth, &payload, timeout) {
        Err(error) if error.status_code == Some(401) => {
            let refreshed = refresh_access_token_force().map_err(auth_error)?;
            send_once(client, &refreshed, &payload, timeout)
        }
        result => result,
    }
}

fn auth_error(message: String) -> CodexChatError {
    CodexChatError {
        status_code: Some(401),
        message,
        retry_after: None,
    }
}

fn build_codex_request(model: &str, request: &Value) -> Value {
    let mut input = Vec::new();
    if let Some(messages) = request.get("messages").and_then(Value::as_array) {
        for message in messages {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .trim();
            if role.is_empty() {
                continue;
            }
            let content = message_text_content(message.get("content").unwrap_or(&Value::Null));
            if content.trim().is_empty() {
                continue;
            }
            let mapped_role = if role.eq_ignore_ascii_case("system") {
                "developer"
            } else {
                role
            };
            let part_type = if mapped_role.eq_ignore_ascii_case("assistant") {
                "output_text"
            } else {
                "input_text"
            };
            input.push(json!({
                "type": "message",
                "role": mapped_role,
                "content": [
                    {
                        "type": part_type,
                        "text": content,
                    }
                ],
            }));
        }
    }
    json!({
        "model": model,
        "stream": true,
        "store": false,
        "instructions": "",
        "input": input,
    })
}

fn message_text_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<String>(),
        Value::Object(map) => map
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

fn send_once(
    client: &reqwest::blocking::Client,
    auth: &GptAccessContext,
    payload: &Value,
    timeout: Duration,
) -> Result<Value, CodexChatError> {
    let response = client
        .post(CODEX_RESPONSES_URL)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("Authorization", format!("Bearer {}", auth.access_token))
        .header("Chatgpt-Account-Id", auth.account_id.as_str())
        .header("Originator", CODEX_ORIGINATOR)
        .header("User-Agent", CODEX_USER_AGENT)
        .header("Connection", "Keep-Alive")
        .timeout(timeout)
        .json(payload)
        .send()
        .map_err(|error| CodexChatError {
            status_code: None,
            message: if error.is_timeout() {
                "等待 GPT 回應逾時".into()
            } else {
                format!("無法連線到 GPT：{error}")
            },
            retry_after: None,
        })?;
    let status = response.status();
    let retry_after = super::retry_policy::parse_retry_after(
        response.headers().get("retry-after").and_then(|v| v.to_str().ok()),
        std::time::SystemTime::now(),
    );
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        let mut error = status_error(status.as_u16(), &body);
        error.retry_after = retry_after;
        return Err(error);
    }
    parse_codex_response(&body)
}

fn status_error(status: u16, body: &str) -> CodexChatError {
    let message = parse_error_message(body);
    CodexChatError {
        status_code: Some(status),
        retry_after: None,
        message: match status {
            401 | 403 => format!("GPT 驗證失敗：{message}"),
            // 429 只證明這個 Codex Responses 端點拒絕了本次請求；它不能證明
            // ChatGPT 對話帳號的整體額度已經用完。
            429 => format!("ChatGPT 暫時不接受翻譯請求，可能是翻譯用量已達上限，或短時間內送出太多次：{message}"),
            500..=599 => format!("GPT 服務錯誤 {status}：{message}"),
            _ => format!("GPT 回應錯誤 {status}：{message}"),
        },
    }
}

fn parse_error_message(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(message) = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
        {
            let trimmed = message.trim();
            if !trimmed.is_empty() {
                return trimmed.chars().take(220).collect();
            }
        }
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            let trimmed = message.trim();
            if !trimmed.is_empty() {
                return trimmed.chars().take(220).collect();
            }
        }
        // Codex 後端把錯誤放在 `detail`，不是 `error.message` 或 `message`。
        // 少了這一條，站長看到的是一整段原始 JSON：
        // 「GPT 回應錯誤 400：{"detail":"The 'gpt-5.6-sol' model is not
        //   supported when using Codex with a ChatGPT account."}」
        if let Some(message) = value.get("detail").and_then(Value::as_str) {
            let trimmed = message.trim();
            if !trimmed.is_empty() {
                return trimmed.chars().take(220).collect();
            }
        }
    }
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = compact.trim();
    if trimmed.is_empty() {
        "空回應".into()
    } else {
        trimmed.chars().take(220).collect()
    }
}

fn stream_error(message: impl Into<String>) -> CodexChatError {
    CodexChatError {
        status_code: Some(200),
        message: message.into(),
        retry_after: None,
    }
}

fn parse_codex_response(body: &str) -> Result<Value, CodexChatError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(stream_error("GPT 沒有回傳任何內容。"));
    }
    if trimmed.starts_with('{') {
        let value: Value = serde_json::from_str(trimmed)
            .map_err(|error| stream_error(format!("GPT 回應無法解析：{error}")))?;
        if value.get("response").is_some() {
            return normalize_completed_event(&value, &BTreeMap::new(), &[]).map_err(stream_error);
        }
        return normalize_nonstream_response(&value).map_err(stream_error);
    }
    parse_codex_sse(trimmed)
}

/// B4：`response.incomplete`（輸出被截斷）與可拆小的 `response.failed` 改成「截斷」回應
/// （`finish_reason = length`，內容可能是一半），讓翻譯端走既有的拆半重送；
/// 舊版把它們都當成「串流在完成前中斷」原樣重送同一批，結果一樣截斷。
fn truncated_response(
    event: &Value,
    indexed_items: &BTreeMap<i64, Value>,
    fallback_items: &[Value],
    why: &str,
) -> Value {
    let mut output = event
        .get("response")
        .and_then(|response| response.get("output"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if output.is_empty() {
        output.extend(indexed_items.values().cloned());
        output.extend(fallback_items.iter().cloned());
    }
    json!({
        "choices": [
            {
                "message": { "content": extract_output_text(&output) },
                "finish_reason": "length",
                "codex_event": why,
            }
        ],
        "usage": event
            .get("response")
            .and_then(|response| response.get("usage"))
            .cloned()
            .unwrap_or_else(|| json!({})),
    })
}

/// `response.failed`／`error` 事件：額度與限流照實回錯（不拆、不空轉）；
/// 內容太長與其他失敗改走拆半。
fn failed_event(
    event: &Value,
    indexed_items: &BTreeMap<i64, Value>,
    fallback_items: &[Value],
) -> Result<Value, CodexChatError> {
    let error = event
        .get("response")
        .and_then(|response| response.get("error"))
        .or_else(|| event.get("error"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let code = error.get("code").and_then(Value::as_str).unwrap_or("").to_string();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(220)
        .collect::<String>();
    let lower = format!("{code} {message}").to_lowercase();
    if lower.contains("insufficient_quota") || lower.contains("usage_limit") || lower.contains("usage limit") {
        return Err(CodexChatError {
            status_code: Some(429),
            message: format!("ChatGPT 暫時不接受翻譯請求（用量上限）：{code} {message}").trim().to_string(),
            retry_after: None,
        });
    }
    if lower.contains("rate_limit") || lower.contains("rate limit") {
        return Err(CodexChatError {
            status_code: Some(429),
            message: format!("GPT 請求太頻繁：{message}"),
            retry_after: None,
        });
    }
    Ok(truncated_response(event, indexed_items, fallback_items, "failed"))
}

fn parse_codex_sse(body: &str) -> Result<Value, CodexChatError> {
    let mut indexed_items = BTreeMap::new();
    let mut fallback_items = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(payload)
            .map_err(|error| stream_error(format!("GPT 串流事件無法解析：{error}")))?;
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_item.done" => {
                if let Some(item) = value.get("item").cloned() {
                    if let Some(index) = value.get("output_index").and_then(Value::as_i64) {
                        indexed_items.insert(index, item);
                    } else {
                        fallback_items.push(item);
                    }
                }
            }
            "response.completed" => {
                return normalize_completed_event(&value, &indexed_items, &fallback_items)
                    .map_err(stream_error);
            }
            "response.incomplete" => {
                return Ok(truncated_response(&value, &indexed_items, &fallback_items, "incomplete"));
            }
            "response.failed" | "error" => {
                return failed_event(&value, &indexed_items, &fallback_items);
            }
            _ => {}
        }
    }
    Err(stream_error("GPT 串流在完成前中斷。"))
}

fn normalize_completed_event(
    event: &Value,
    indexed_items: &BTreeMap<i64, Value>,
    fallback_items: &[Value],
) -> Result<Value, String> {
    let mut output = event
        .get("response")
        .and_then(|response| response.get("output"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if output.is_empty() && (!indexed_items.is_empty() || !fallback_items.is_empty()) {
        output.extend(indexed_items.values().cloned());
        output.extend(fallback_items.iter().cloned());
    }
    let assistant_text = extract_output_text(&output);
    if assistant_text.trim().is_empty() {
        return Err("GPT 有回應，但沒有可用的 assistant text。".into());
    }
    let usage = event
        .get("response")
        .and_then(|response| response.get("usage"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    Ok(json!({
        "choices": [
            {
                "message": {
                    "content": assistant_text,
                }
            }
        ],
        "usage": usage,
    }))
}

fn normalize_nonstream_response(value: &Value) -> Result<Value, String> {
    let output = value
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let assistant_text = extract_output_text(&output);
    if assistant_text.trim().is_empty() {
        return Err("GPT 有回應，但沒有可用的 assistant text。".into());
    }
    Ok(json!({
        "choices": [
            {
                "message": {
                    "content": assistant_text,
                }
            }
        ],
        "usage": value.get("usage").cloned().unwrap_or_else(|| json!({})),
    }))
}

fn extract_output_text(output: &[Value]) -> String {
    let mut text = String::new();
    for item in output {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("assistant");
        if role != "assistant" {
            continue;
        }
        if let Some(content) = item.get("content").and_then(Value::as_array) {
            for part in content {
                let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
                if !matches!(part_type, "output_text" | "input_text" | "text") {
                    continue;
                }
                if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                    text.push_str(part_text);
                }
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{
        build_codex_request, extract_output_text, message_text_content, parse_codex_response,
        parse_error_message, status_error,
    };
    use serde_json::json;

    #[test]
    fn message_text_content_joins_array_parts() {
        let content = json!([
            {"type": "text", "text": "Hello"},
            {"type": "text", "text": " world"},
        ]);
        assert_eq!(message_text_content(&content), "Hello world");
    }

    #[test]
    fn build_request_maps_system_to_developer() {
        let request = json!({
            "messages": [
                {"role": "system", "content": "be strict"},
                {"role": "user", "content": "hi"},
            ]
        });
        let payload = build_codex_request("gpt-5.4", &request);
        assert_eq!(payload["input"][0]["role"], "developer");
        assert_eq!(payload["input"][0]["content"][0]["type"], "input_text");
    }

    #[test]
    fn extract_output_text_reads_assistant_message_parts() {
        let output = vec![json!({
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "output_text", "text": "{\"r\":["},
                {"type": "output_text", "text": "]}"},
            ]
        })];
        assert_eq!(extract_output_text(&output), "{\"r\":[]}");
    }

    #[test]
    fn parse_response_uses_output_item_fallback_when_completed_output_empty() {
        let body = concat!(
            "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"{\\\"r\\\":[]}\"}]},\"output_index\":0}\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":5}}}\n"
        );
        let normalized = parse_codex_response(body).unwrap();
        assert_eq!(normalized["choices"][0]["message"]["content"], "{\"r\":[]}");
        assert_eq!(normalized["usage"]["input_tokens"], 3);
    }

    #[test]
    fn parse_error_message_prefers_json_error_message() {
        let body = r#"{"error":{"message":"invalid token"}}"#;
        assert_eq!(parse_error_message(body), "invalid token");
    }

    #[test]
    fn parse_error_message_understands_codex_detail_field() {
        // 站長 2026-09-02 看到的原文：整段 JSON 直接被當成錯誤訊息印出來。
        let body = r#"{"detail":"The 'gpt-5.6-sol' model is not supported when using Codex with a ChatGPT account."}"#;
        assert_eq!(
            parse_error_message(body),
            "The 'gpt-5.6-sol' model is not supported when using Codex with a ChatGPT account."
        );
        // error.message 仍然優先（那是 OpenAI 標準格式）
        let both = r#"{"error":{"message":"first"},"detail":"second"}"#;
        assert_eq!(parse_error_message(both), "first");
    }

    #[test]
    fn rate_limit_does_not_claim_the_chatgpt_account_has_no_quota() {
        let error = status_error(429, r#"{"detail":"The usage limit has been reached"}"#);
        assert_eq!(error.status_code, Some(429));
        assert!(error.message.contains("ChatGPT 暫時不接受翻譯請求"));
        assert!(error.message.contains("The usage limit has been reached"));
        assert!(!error.message.contains("帳號額度"), "不可斷定帳號額度已用完");
        for word in ["Codex", "端點", "429"] {
            assert!(!error.message.contains(word), "玩家看得到的文字不可出現「{word}」");
        }
    }
}

#[cfg(test)]
#[path = "codex_chat_b4_tests.rs"]
mod b4_tests;
