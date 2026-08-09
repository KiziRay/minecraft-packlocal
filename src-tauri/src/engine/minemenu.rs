use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn fix_minemenu_unicode_escapes(menu_json: &Path) -> Result<String, String> {
    if !menu_json.is_file() {
        return Err(format!("找不到快捷選單設定：{}", menu_json.display()));
    }
    let text = fs::read_to_string(menu_json).map_err(|e| e.to_string())?;
    let data: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let final_s = to_ascii_json(&data);
    let _: Value = serde_json::from_str(&final_s).map_err(|e| e.to_string())?;
    let bak = menu_json.with_extension("json.bak");
    if !bak.exists() {
        let _ = fs::copy(menu_json, &bak);
    }
    fs::write(menu_json, final_s.as_bytes()).map_err(|e| e.to_string())?;
    Ok("已修正快捷選單中文顯示（避免亂碼）".into())
}

fn to_ascii_json(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{}\"", escape_str(s)),
        Value::Array(a) => {
            let inner: Vec<String> = a.iter().map(to_ascii_json).collect();
            format!("[\n  {}\n]", inner.join(",\n  "))
        }
        Value::Object(m) => {
            let mut parts = Vec::new();
            for (k, val) in m {
                parts.push(format!("\"{}\": {}", escape_str(k), to_ascii_json(val)));
            }
            format!("{{\n  {}\n}}", parts.join(",\n  "))
        }
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) > 0x7f => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
