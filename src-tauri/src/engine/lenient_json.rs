//! 寬鬆 JSON 解析。
//!
//! Minecraft 用 GSON 的寬鬆模式讀語言／資料檔，所以 `//` 註解、`/* */` 區塊註解、
//! 尾逗號在遊戲裡都能正常載入。`serde_json` 嚴格解析會直接拒收，導致整個檔案**靜默消失**
//! ——該模組全程英文，而且使用者分不清是工具漏翻還是檔案壞了。
//!
//! 這裡先嚴格解析；失敗就把註解與尾逗號清掉（**字串字面值內原樣保留**）再試一次。
//! 真的還是壞（例：模組自己括號寫錯）才回 `Err`，由呼叫端記進錯誤日誌，不再靜默。
//!
//! 技術參考 Koudesuk/Modpack_Translator（MIT）的 `_relax_json`，見 `NOTICE.md`。

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

fn string_literal_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#""(?:[^"\\]|\\.)*""#).expect("string literal regex"))
}

fn block_comment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)/\*.*?\*/").expect("block comment regex"))
}

fn line_comment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"//[^\n]*").expect("line comment regex"))
}

fn trailing_comma_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r",(\s*[}\]])").expect("trailing comma regex"))
}

/// 解析（先嚴格、失敗再寬鬆）。回 `Err` 代表連寬鬆都救不回來。
pub fn parse(text: &str) -> Result<Value, String> {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Ok(v);
    }
    serde_json::from_str::<Value>(&relax(text)).map_err(|e| e.to_string())
}

/// 解析成 `{ key: 字串 }`（語言檔用）。非字串值略過。頂層不是物件則回 `Err`。
pub fn parse_object_strings(text: &str) -> Result<HashMap<String, String>, String> {
    let v = parse(text)?;
    let obj = v.as_object().ok_or_else(|| "JSON 頂層不是物件".to_string())?;
    Ok(obj
        .iter()
        .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
        .collect())
}

/// 移除註解與尾逗號；字串字面值內的內容原樣保留。
fn relax(raw: &str) -> String {
    let str_re = string_literal_re();
    let mut out = String::with_capacity(raw.len());
    let mut last = 0usize;
    for m in str_re.find_iter(raw) {
        out.push_str(&strip_comments(&raw[last..m.start()]));
        out.push_str(m.as_str());
        last = m.end();
    }
    out.push_str(&strip_comments(&raw[last..]));
    trailing_comma_re().replace_all(&out, "$1").into_owned()
}

fn strip_comments(chunk: &str) -> String {
    let no_block = block_comment_re().replace_all(chunk, " ");
    line_comment_re().replace_all(&no_block, "").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_json_still_parses() {
        let v = parse(r#"{"a":"b","n":1}"#).unwrap();
        assert_eq!(v["a"], "b");
    }

    #[test]
    fn recovers_trailing_comma() {
        // Gson 讀得動、serde 嚴格會拒收
        let m = parse_object_strings(r#"{"item.foo":"劍","item.bar":"盾",}"#).unwrap();
        assert_eq!(m.get("item.bar").map(|s| s.as_str()), Some("盾"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn recovers_line_and_block_comments() {
        let raw = "{\n  // 這是註解\n  \"a\": \"1\", /* 區塊 */ \"b\": \"2\"\n}";
        let m = parse_object_strings(raw).unwrap();
        assert_eq!(m.get("a").map(|s| s.as_str()), Some("1"));
        assert_eq!(m.get("b").map(|s| s.as_str()), Some("2"));
    }

    #[test]
    fn does_not_touch_slashes_inside_strings() {
        // 字串內的 // 與 , 不可被當註解／尾逗號處理
        let m = parse_object_strings(r#"{"url":"https://a.com/x","k":"a,b"}"#).unwrap();
        assert_eq!(m.get("url").map(|s| s.as_str()), Some("https://a.com/x"));
        assert_eq!(m.get("k").map(|s| s.as_str()), Some("a,b"));
    }

    #[test]
    fn genuinely_broken_json_is_an_error_not_silent() {
        // 括號沒關——連寬鬆都救不回，必須回 Err（呼叫端才能記錄）
        assert!(parse(r#"{"a": "b" "#).is_err());
    }

    #[test]
    fn non_object_top_level_reports_error() {
        assert!(parse_object_strings(r#"["a","b"]"#).is_err());
    }
}
