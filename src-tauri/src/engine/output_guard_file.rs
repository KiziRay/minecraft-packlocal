//! B2#4：output guard 的檔案層檢查（編碼、跳脫、寫完重新解析）與字型掃描。
//!
//! 逐條檢查在 output_guard.rs；這裡處理「整個檔」：條目都合格，組出來的檔仍可能壞
//! （跳脫錯、引號沒關、BOM）。檔案層失敗時**整檔保留原文**並記錄，不寫出壞檔。

use std::path::Path;

use super::output_guard::{record_rejection, RejectReason};

fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// `.properties` 以 Java `Properties.load` 讀取時是 ISO-8859-1；非 ASCII 一律寫成 `\uXXXX`。
pub fn escape_properties(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        let mut buf = [0u16; 2];
        for unit in c.encode_utf16(&mut buf) {
            out.push_str(&format!("\\u{:04X}", unit));
        }
    }
    out
}

/// 字串與括號是否成對（字串外的 `{}`、`[]`、`()`）。`quotes` 是這種格式的引號字元。
fn balanced(text: &str, quotes: &[char]) -> bool {
    let mut stack: Vec<char> = Vec::new();
    let mut in_str: Option<char> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if let Some(q) = in_str {
            if c == '\\' {
                chars.next();
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            c if quotes.contains(&c) => in_str = Some(c),
            '{' | '[' | '(' => stack.push(c),
            '}' | ']' | ')' => {
                let want = match c {
                    '}' => '{',
                    ']' => '[',
                    _ => '(',
                };
                if stack.pop() != Some(want) {
                    return false;
                }
            }
            _ => {}
        }
    }
    stack.is_empty() && in_str.is_none()
}

fn parses_json(text: &str) -> bool {
    super::lenient_json::parse(text).is_ok()
}

/// 這種格式「讀得回來」嗎？沒有對應解析器的格式回 true（只做編碼檢查）。
fn reparses(ext: &str, text: &str) -> bool {
    match ext {
        "json" | "json5" | "mcmeta" => parses_json(text),
        "snbt" => balanced(text, &['"', '\'']),
        "js" | "ts" | "zs" => balanced(text, &['"', '\'', '`']),
        _ => true,
    }
}

/// 檔案寫出前的最後一關。`name` 用來判斷格式（副檔名）與記錄；`original` 是原檔內容。
/// 合格回傳要寫的位元組（去掉 BOM、`.properties` 已跳脫）；不合格回傳原檔內容並記錄。
pub fn finish_file(name: &str, original: &[u8], new: Vec<u8>) -> Vec<u8> {
    let ext = ext_of(name);
    let mut bytes = new;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes.drain(..3);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        record_rejection(name, "", "", "", RejectReason::Encoding);
        return original.to_vec();
    };
    let text = if ext == "properties" { escape_properties(&text) } else { text };
    let orig_text = String::from_utf8_lossy(original);
    let orig_text = orig_text.trim_start_matches('\u{FEFF}');
    if reparses(&ext, orig_text) && !reparses(&ext, &text) {
        record_rejection(name, "", "", "", RejectReason::Reparse);
        return original.to_vec();
    }
    text.into_bytes()
}

/// zip 內檔名：UTF-8（Rust 字串本身保證）、正斜線、不含 `..`、不是絕對路徑。
pub fn zip_entry_ok(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && !name.starts_with('/')
        && !name.split('/').any(|seg| seg == "..")
        && !name.contains(':')
}

/// 已啟用、會取代預設字型（含 `font/`）的資源包：字體可能不支援中文。
/// 判斷沿用 apply_instance 既有的 `enabled_packs_covering_font`；本工具的字體包不算。
pub fn font_may_not_support_chinese(mc: &Path) -> Vec<String> {
    let ours = super::font_pack::DEFAULT_FONT_PACK_NAME;
    super::apply_instance::enabled_packs_covering_font(mc, &|name| name.starts_with(ours))
}
