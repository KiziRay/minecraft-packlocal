//! retry_policy 的單元測試（純函式，不碰網路）。

use super::*;
use serde_json::json;
use std::sync::Arc;

#[test]
fn context_length_errors_are_split_not_quota() {
    // 舊版 looks_like_quota_or_auth_error 看到 exceed 就判額度用完，整輪 AI 被停掉
    for body in [
        r#"{"error":{"message":"This model's maximum context length is 8192 tokens","code":"context_length_exceeded"}}"#,
        "the request exceeds the available context size, try increasing it",
        "Request too large for model",
    ] {
        assert_eq!(classify_http(400, body), FailureClass::TooLarge, "{body}");
        assert!(!is_quota_or_auth(body), "{body}");
    }
    assert_eq!(classify_http(413, ""), FailureClass::TooLarge);
}

#[test]
fn rate_limit_exceeded_is_retryable_not_quota() {
    let body = r#"{"error":{"message":"Rate limit exceeded, please slow down"}}"#;
    assert_eq!(classify_http(429, body), FailureClass::RateLimited);
    assert!(FailureClass::RateLimited.retry_same_batch());
    assert!(!is_quota_or_auth("第 3 批失敗：Rate limit exceeded"));
}

#[test]
fn quota_and_auth_stop_ai_instead_of_spinning() {
    assert_eq!(classify_http(402, "Insufficient Balance"), FailureClass::QuotaExhausted);
    assert_eq!(
        classify_http(429, r#"{"error":{"type":"insufficient_quota","message":"You exceeded your current quota"}}"#),
        FailureClass::QuotaExhausted
    );
    assert_eq!(classify_http(401, "invalid api key"), FailureClass::AuthInvalid);
    assert!(FailureClass::QuotaExhausted.stops_ai());
    assert!(FailureClass::AuthInvalid.stops_ai());
    assert!(!FailureClass::QuotaExhausted.retry_same_batch());
    // GPT 帳號的用量上限（429＋usage limit）也是額度類：重送只會空轉
    assert!(is_quota_or_auth(
        "ChatGPT 暫時不接受翻譯請求，可能是翻譯用量已達上限，或短時間內送出太多次：The usage limit has been reached"
    ));
    assert!(is_quota_or_auth("免費翻譯的當日額度已用完"));
    assert!(is_quota_or_auth("金鑰無效或無權限：xxx"));
}

#[test]
fn load_balancer_errors_are_not_quota() {
    assert_eq!(classify_http(502, "upstream load balancer error"), FailureClass::ServerBusy);
    assert!(!is_quota_or_auth("服務錯誤 502：load balancer"));
}

#[test]
fn network_and_timeout_messages_are_retryable() {
    assert_eq!(classify_message("等待 AI 回應逾時"), FailureClass::Timeout);
    assert_eq!(classify_message("連線失敗（無回應）：error sending request"), FailureClass::Network);
    assert!(classify_message("連線失敗（無回應）：x").retry_same_batch());
    assert_eq!(classify_http(503, ""), FailureClass::ServerBusy);
}

#[test]
fn retry_after_seconds_and_http_date_are_understood() {
    let now = UNIX_EPOCH + Duration::from_secs(784_111_777); // Sun, 06 Nov 1994 08:49:37 GMT
    assert_eq!(parse_retry_after(Some("7"), now), Some(Duration::from_secs(7)));
    assert_eq!(parse_retry_after(Some(" 1.5 "), now), Some(Duration::from_millis(1500)));
    assert_eq!(
        parse_retry_after(Some("Sun, 06 Nov 1994 08:49:47 GMT"), now),
        Some(Duration::from_secs(10))
    );
    // 過去的時間＝不用等
    assert_eq!(
        parse_retry_after(Some("Sun, 06 Nov 1994 08:49:00 GMT"), now),
        Some(Duration::ZERO)
    );
    assert_eq!(parse_retry_after(Some("soon"), now), None);
    assert_eq!(parse_retry_after(None, now), None);
    assert_eq!(parse_retry_after_ms(Some("250")), Some(Duration::from_millis(250)));
}

#[test]
fn backoff_grows_is_capped_and_honours_server_hint() {
    let d0 = backoff_delay(0, None, 0);
    let d3 = backoff_delay(3, None, 0);
    assert!(d3 > d0, "{d0:?} {d3:?}");
    for i in 0..40 {
        for seed in [0u64, 7, u64::MAX] {
            assert!(backoff_delay(i, None, seed) <= Duration::from_millis(BACKOFF_CAP_MS));
        }
    }
    // 伺服器說等 5 秒就等 5 秒（加一點抖動），但不會超過上限
    let hinted = backoff_delay(0, Some(Duration::from_secs(5)), 100);
    assert!(hinted >= Duration::from_secs(5) && hinted < Duration::from_millis(5_300));
    assert!(backoff_delay(0, Some(Duration::from_secs(9_999)), 0) <= RETRY_AFTER_CAP);
    // 抖動：不同種子給不同等待
    assert_ne!(backoff_delay(4, None, 1), backoff_delay(4, None, 97));
}

#[test]
fn row_ids_accept_numbers_and_numeric_strings() {
    assert_eq!(parse_row_id(&json!(12)), Some(12));
    assert_eq!(parse_row_id(&json!("12")), Some(12));
    assert_eq!(parse_row_id(&json!(" 3 ")), Some(3));
    assert_eq!(parse_row_id(&json!(4.0)), Some(4));
    assert_eq!(parse_row_id(&json!(4.5)), None);
    assert_eq!(parse_row_id(&json!(-1)), None);
    assert_eq!(parse_row_id(&json!("x1")), None);
    assert_eq!(parse_row_id(&json!(null)), None);
}

#[test]
fn proportional_cap_scales_with_size() {
    assert_eq!(proportional_cap(30, 25, 50, 2000), 30, "小包：全部都能重試");
    assert_eq!(proportional_cap(1_000, 25, 50, 2000), 250);
    assert_eq!(proportional_cap(100_000, 25, 50, 2000), 2000, "大包有上限");
    assert_eq!(proportional_cap(0, 25, 50, 2000), 0);
}

#[test]
fn poisoned_lock_is_recovered_with_its_data() {
    let shared = Arc::new(Mutex::new(vec![1, 2, 3]));
    let clone = Arc::clone(&shared);
    let _ = std::thread::spawn(move || {
        let _guard = clone.lock().unwrap();
        panic!("worker crashed while holding the lock");
    })
    .join();
    assert!(shared.is_poisoned());
    assert_eq!(*lock_or_recover(&shared), vec![1, 2, 3], "資料不可因為別人當掉而消失");
}

#[test]
fn fix4a_batch_numbers_in_messages_are_not_status_codes() {
    for msg in [
        "第 402 批失敗：服務錯誤 400：bad request",
        "第 429 批失敗：回傳格式不對",
        "第 500 批失敗：批次回應與送出內容對不上",
    ] {
        assert!(!is_quota_or_auth(msg), "{msg}");
        assert_eq!(classify_message(msg), FailureClass::Other, "{msg}");
    }
    assert!(is_quota_or_auth("GPT 回應錯誤 402：payment"));
    assert_eq!(classify_message("HTTP 429 from upstream"), FailureClass::RateLimited);
}

#[test]
fn fix4b_per_minute_limits_are_rate_limits_not_exhausted_quota() {
    let gemini = r#"{"error":{"code":429,"message":"Quota exceeded for metric: generate_content_requests per minute","status":"RESOURCE_EXHAUSTED"}}"#;
    assert_eq!(classify_http(429, gemini), FailureClass::RateLimited);
    assert_eq!(
        classify_http(429, r#"{"error":{"type":"insufficient_quota","message":"You exceeded your current quota, please check your plan and billing details"}}"#),
        FailureClass::QuotaExhausted
    );
}

// ═══ 第二輪（F4、F5）═══

const GEMINI_PER_MINUTE_429: &str = r#"{"error":{"code":429,"message":"You exceeded your current quota, please check your plan and billing details. For more information on this error, head to: https://ai.google.dev/gemini-api/docs/rate-limits. Please retry in 37.4s.","status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"generativelanguage.googleapis.com/generate_content_free_tier_requests","quotaId":"GenerateRequestsPerMinutePerProjectPerModel-FreeTier","quotaDimensions":{"location":"global","model":"gemini-2.0-flash"},"quotaValue":"15"}]},{"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"37s"}]}}"#;
const GEMINI_PER_DAY_429: &str = r#"{"error":{"code":429,"message":"You exceeded your current quota, please check your plan and billing details.","status":"RESOURCE_EXHAUSTED","details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaMetric":"generativelanguage.googleapis.com/generate_content_free_tier_requests","quotaId":"GenerateRequestsPerDayPerProjectPerModel-FreeTier","quotaValue":"1500"}]}]}}"#;
const OPENAI_RATE_429: &str = r#"{"error":{"message":"Rate limit reached for gpt-4o-mini in organization org-xxx on requests per min (RPM): Limit 3, Used 3, Requested 1. Please try again in 20s. Visit https://platform.openai.com/account/rate-limits to learn more. You can increase your rate limit by adding a payment method to your account at https://platform.openai.com/account/billing.","type":"requests","param":null,"code":"rate_limit_exceeded"}}"#;
const OPENAI_QUOTA_429: &str = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details. For more information on this error, read the docs: https://platform.openai.com/docs/guides/error-codes/api-errors.","type":"insufficient_quota","param":null,"code":"insufficient_quota"}}"#;

#[test]
fn fix2_f4_real_world_429_bodies_are_split_into_rate_limit_and_quota() {
    assert_eq!(classify_http(429, GEMINI_PER_MINUTE_429), FailureClass::RateLimited, "Gemini 每分鐘上限：等一下就好");
    assert_eq!(classify_http(429, OPENAI_RATE_429), FailureClass::RateLimited, "OpenAI rate_limit_exceeded 附 billing 連結也是限流");
    assert_eq!(classify_http(429, OPENAI_QUOTA_429), FailureClass::QuotaExhausted);
    assert_eq!(classify_http(429, GEMINI_PER_DAY_429), FailureClass::QuotaExhausted, "每日上限＝今天用完");
    assert_eq!(classify_http(429, r#"{"error":{"message":"余额不足"}}"#), FailureClass::QuotaExhausted);
    // 只提到 billing、沒有額度字樣的一般錯誤不是額度用完
    assert_eq!(classify_http(400, "see billing docs for model access"), FailureClass::Other);
}

#[test]
fn fix2_f5_every_5xx_is_server_busy() {
    for code in [500u16, 520, 522, 524, 529, 599] {
        assert_eq!(classify_http(code, "<html>origin error</html>"), FailureClass::ServerBusy, "{code}");
        assert_eq!(classify_message(&format!("服務錯誤 {code}：origin")), FailureClass::ServerBusy, "{code}");
    }
}
