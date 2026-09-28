//! B4 #5：GPT 串流的 incomplete／failed 事件改走拆半（不打真實 GPT，只測解析）。

use super::parse_codex_response;

fn finish_reason(v: &serde_json::Value) -> &str {
    v["choices"][0]["finish_reason"].as_str().unwrap_or("")
}

#[test]
fn incomplete_stream_becomes_a_length_truncation_so_the_batch_is_split() {
    let body = concat!(
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"{\\\"r\\\":[{\\\"i\\\":0,\\\"t\\\":\\\"鐵\"}]},\"output_index\":0}\n",
        "data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"},\"output\":[],\"usage\":{\"input_tokens\":9,\"output_tokens\":64}}}\n"
    );
    let v = parse_codex_response(body).unwrap();
    assert_eq!(finish_reason(&v), "length", "截斷要讓翻譯端拆半，不是原樣重送");
    assert!(v["choices"][0]["message"]["content"].as_str().unwrap().contains("鐵"));
    assert_eq!(v["usage"]["output_tokens"], 64);
}

#[test]
fn generic_failed_event_is_split_not_resent_as_is() {
    let body = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\",\"message\":\"The model failed to generate\"},\"output\":[]}}\n";
    let v = parse_codex_response(body).unwrap();
    assert_eq!(finish_reason(&v), "length");
}

#[test]
fn context_length_failure_is_split() {
    let body = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"context_length_exceeded\",\"message\":\"Your input exceeds the context window\"}}}\n";
    let v = parse_codex_response(body).unwrap();
    assert_eq!(finish_reason(&v), "length");
}

#[test]
fn quota_failure_is_reported_not_split() {
    let body = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"insufficient_quota\",\"message\":\"You exceeded your current quota\"}}}\n";
    assert!(parse_codex_response(body).is_err(), "額度用完拆半也沒用，要照實回報");
}
