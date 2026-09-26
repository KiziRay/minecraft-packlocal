//! B2#2：JSON text component 拆解（從 output_guard.rs 拆出，控制檔案大小）。

use serde_json::Value;


/// JSON text component 拆開後的片段：`Skeleton` 是原樣保留的結構，`Text` 是要翻的顯示文字
/// （保持原 JSON 字串裡的跳脫寫法）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentPart {
    Skeleton(String),
    Text(String),
}

fn has_text_field(v: &Value) -> bool {
    match v {
        Value::Object(map) => {
            map.get("text").is_some_and(Value::is_string)
                || map
                    .values()
                    .any(|x| matches!(x, Value::Array(_) | Value::Object(_)) && has_text_field(x))
        }
        Value::Array(items) => items.iter().any(has_text_field),
        _ => false,
    }
}

/// 把 JSON text component 拆成結構與 `"text"` 值。不是 component（或沒有任何非空 text）回 None。
pub fn split_text_component(s: &str) -> Option<Vec<ComponentPart>> {
    let t = s.trim();
    if !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    let value: Value = serde_json::from_str(t).ok()?;
    if !has_text_field(&value) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut skeleton_start = 0usize;
    let mut last_key: Option<&str> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && bytes[j] != b'"' {
            j += if bytes[j] == b'\\' { 2 } else { 1 };
        }
        if j >= bytes.len() {
            return None;
        }
        let raw = &s[start..j];
        let next = s[j + 1..].trim_start().chars().next();
        if next == Some(':') {
            last_key = Some(raw);
        } else {
            if last_key == Some("text") && !raw.trim().is_empty() {
                parts.push(ComponentPart::Skeleton(s[skeleton_start..start].to_string()));
                parts.push(ComponentPart::Text(raw.to_string()));
                skeleton_start = j;
            }
            last_key = None;
        }
        i = j + 1;
    }
    if !parts.iter().any(|p| matches!(p, ComponentPart::Text(_))) {
        return None;
    }
    parts.push(ComponentPart::Skeleton(s[skeleton_start..].to_string()));
    Some(parts)
}
