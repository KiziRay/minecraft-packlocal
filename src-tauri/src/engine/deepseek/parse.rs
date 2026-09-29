//! AI 回應解析與批次稽核。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

pub(super) fn extract_message_content(v: &Value) -> String {
    let message = &v["choices"][0]["message"];
    let primary = match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
            .collect::<String>(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    if !primary.trim().is_empty() {
        return primary;
    }
    if let Some(reasoning) = message
        .get("reasoning_content")
        .and_then(|value| value.as_str())
    {
        if looks_like_translation_payload(reasoning) {
            return reasoning.to_string();
        }
    }
    primary
}

/// 解析模型回覆：優先吃 `{"r":[...]}`，但保留陣列回退相容。
pub(super) fn parse_translation_object(content: &str) -> Result<HashMap<usize, String>, String> {
    let body = extract_json_body(strip_code_fence(content));
    let value: Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "回傳格式不對：{e} / {}",
            body.chars().take(120).collect::<String>()
        )
    })?;
    let items: Vec<Value> = match value {
        Value::Object(map) => map
            .get("r")
            .and_then(|rows| rows.as_array())
            .cloned()
            .unwrap_or_default(),
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    Ok(parse_translation_rows(&items).into_iter().collect())
}

/// 抽出原始 (id, 譯文) 序列，**不去重**——去重會抹掉「同一個 id 回兩次」這個訊號。
pub(super) fn parse_translation_rows(items: &[Value]) -> Vec<(usize, String)> {
    let mut rows = Vec::new();
    for item in items {
        // B4：`"i":"12"` 也收（有些模型會把 id 加引號；舊版整批變成「沒回應」）
        let Some(i) = item.get("i").and_then(super::super::retry_policy::parse_row_id) else {
            continue;
        };
        let Some(translated) = item.get("t").and_then(|value| value.as_str()) else {
            continue;
        };
        rows.push((i, translated.to_string()));
    }
    rows
}

/// 解析並對帳。回傳的 map 只含**送出去過而且沒有衝突**的 id：
/// 模型自己編的 id 一律丟掉（不可能寫進任何 key），
/// 同 id 衝突的也丟掉（不知道哪個才對，就不要猜）。
pub fn parse_translation_object_audited(
    content: &str,
    requested: &[usize],
) -> Result<(HashMap<usize, String>, BatchAudit), String> {
    let body = extract_json_body(strip_code_fence(content));
    let value: Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "回傳格式不對：{e} / {}",
            body.chars().take(120).collect::<String>()
        )
    })?;
    let items: Vec<Value> = match value {
        Value::Object(map) => map
            .get("r")
            .and_then(|rows| rows.as_array())
            .cloned()
            .unwrap_or_default(),
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    let rows = parse_translation_rows(&items);
    let audit = audit_batch_response(requested, &rows);

    let requested_set: HashSet<usize> = requested.iter().copied().collect();
    let mut by_i: HashMap<usize, String> = HashMap::new();
    for (id, text) in rows {
        if !requested_set.contains(&id) || audit.conflicting.contains(&id) {
            continue;
        }
        by_i.entry(id).or_insert(text);
    }
    Ok((by_i, audit))
}

/// 一批 AI 回應與「我們送出去的東西」對帳的結果。
///
/// 配對本身早就是靠 id（`{"i":…,"t":…}`）而不是陣列位置，所以不會錯位。
/// 這裡補的是另一半：**回應本身可不可信**。模型少回一筆、同一個 id 回兩次、
/// 回一個我們沒送過的 id——這些都代表這批的輸出不穩定，靜默接受就會變成
/// 「翻譯完成了但少了幾句」，而且沒有任何跡象可查。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BatchAudit {
    pub requested: usize,
    /// 有回、且內容非空的 id 數
    pub resolved: usize,
    /// 送出去但沒回來的
    pub missing: Vec<usize>,
    /// 同一個 id 回了不只一次，且內容彼此不同（純重複同值不算問題）
    pub conflicting: Vec<usize>,
    /// 回了我們沒送過的 id（模型自己編的）
    pub unknown: Vec<usize>,
    /// 有回但譯文是空字串
    pub empty: Vec<usize>,
}

impl BatchAudit {
    /// 這批可以整批採信嗎？
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty()
            && self.conflicting.is_empty()
            && self.unknown.is_empty()
            && self.empty.is_empty()
    }

    /// 這批該不該進「拆批重試」流程。
    ///
    /// 判準刻意分兩級：少數幾筆沒回來是常態（下一輪補），
    /// 但**幻覺 id 或同 id 衝突**代表模型沒有照格式走，整批都不該當成可靠輸出。
    pub fn needs_diagnostic_split(&self) -> bool {
        if !self.conflicting.is_empty() || !self.unknown.is_empty() {
            return true;
        }
        // 超過三成沒回來，不是零星漏掉，是這批本身有問題（多半是被截斷）
        self.requested > 0 && self.missing.len() * 10 > self.requested * 3
    }

    /// 給開發者定位用的一行摘要。不含原文，只有數量與 id。
    pub fn summary(&self) -> String {
        format!(
            "批次對帳：送出 {}、可用 {}、未回 {}、衝突 {}、未知 {}、空譯文 {}",
            self.requested,
            self.resolved,
            self.missing.len(),
            self.conflicting.len(),
            self.unknown.len(),
            self.empty.len()
        )
    }
}

/// 以「我們送出去的 id 集合」為準對帳一批回應。
///
/// `raw_rows` 是還沒去重的原始 (id, 譯文) 序列——必須未去重，
/// 否則偵測不到「同一個 id 回兩次」。
pub fn audit_batch_response(requested: &[usize], raw_rows: &[(usize, String)]) -> BatchAudit {
    use std::collections::BTreeMap;
    let requested_set: HashSet<usize> = requested.iter().copied().collect();

    let mut first_seen: BTreeMap<usize, String> = BTreeMap::new();
    let mut conflicting: Vec<usize> = Vec::new();
    let mut unknown: Vec<usize> = Vec::new();

    for (id, text) in raw_rows {
        if !requested_set.contains(id) {
            if !unknown.contains(id) {
                unknown.push(*id);
            }
            continue;
        }
        match first_seen.get(id) {
            Some(prev) if prev != text => {
                if !conflicting.contains(id) {
                    conflicting.push(*id);
                }
            }
            Some(_) => {} // 同一個 id 回了兩次但內容相同，無害
            None => {
                first_seen.insert(*id, text.clone());
            }
        }
    }

    let mut missing: Vec<usize> = Vec::new();
    let mut empty: Vec<usize> = Vec::new();
    let mut resolved = 0usize;
    for id in requested {
        match first_seen.get(id) {
            None => missing.push(*id),
            Some(text) if text.trim().is_empty() => empty.push(*id),
            Some(_) => resolved += 1,
        }
    }

    BatchAudit {
        requested: requested.len(),
        resolved,
        missing,
        conflicting,
        unknown,
        empty,
    }
}

pub(super) fn parse_usage_totals(response: &Value) -> AiUsageTotals {
    let usage = response.get("usage").unwrap_or(&Value::Null);
    let prompt_cache_hit_tokens = usage
        .get("prompt_cache_hit_tokens")
        .and_then(|value| value.as_u64())
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|value| value.get("cached_tokens"))
                .and_then(|value| value.as_u64())
        })
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|value| value.get("cached_tokens"))
                .and_then(|value| value.as_u64())
        })
        .unwrap_or(0) as usize;
    let prompt_total = usage
        .get("prompt_tokens")
        .and_then(|value| value.as_u64())
        .or_else(|| usage.get("input_tokens").and_then(|value| value.as_u64()))
        .unwrap_or(0) as usize;
    let prompt_cache_miss_tokens = usage
        .get("prompt_cache_miss_tokens")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or_else(|| prompt_total.saturating_sub(prompt_cache_hit_tokens));
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(|value| value.as_u64())
        .or_else(|| usage.get("output_tokens").and_then(|value| value.as_u64()))
        .unwrap_or(0) as usize;
    AiUsageTotals {
        prompt_cache_hit_tokens,
        prompt_cache_miss_tokens,
        completion_tokens,
    }
}

pub(super) fn extract_json_body(s: &str) -> &str {
    let s = s.trim();
    if let (Some(a), Some(b)) = (s.find('{'), s.rfind('}')) {
        if a < b {
            return &s[a..=b];
        }
    }
    if let (Some(a), Some(b)) = (s.find('['), s.rfind(']')) {
        if a < b {
            return &s[a..=b];
        }
    }
    s
}

pub(super) fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim();
    }
    if let Some(rest) = s.strip_prefix("```") {
        return rest.trim_end_matches("```").trim();
    }
    s
}

pub(super) fn mentions_unsupported_params(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    (lower.contains("unsupported") || lower.contains("unknown parameter") || lower.contains("invalid parameter") || lower.contains("not supported"))
        && (lower.contains("response_format")
            || lower.contains("max_tokens")
            || lower.contains("max_completion_tokens")
            || lower.contains("temperature"))
}
