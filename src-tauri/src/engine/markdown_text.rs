//! B3#4：GuideME（AE2 指南）／Lavender 的 Markdown 手冊。
//!
//! 只翻「散文」：標題、清單項、段落、引言的文字部分，以及開頭設定區（frontmatter）的 `title:`。
//! 不動：程式碼區塊、MDX 標籤行（`<ItemImage id="…"/>`）、含連結／圖片語法 `](` 的行、
//! 含 `{…}` 的行、frontmatter 其他欄位（`parent:`、`icon:`、`item_ids:` 都是 id）。
//! 寧可留英文，也不讓指南壞掉。
//!
//! 輸出位置（進主資源包，見 pack_assets）：
//! - GuideME（`ae2guide/`）：`ae2guide/_zh_tw/…`（GuideME 以 `_<語系>` 子資料夾放翻譯頁）。
//!   已有 `_zh_tw` 的頁不覆蓋；`_zh_cn` 的頁轉台灣繁優先於英文送 AI。
//! - Lavender（`lavender/entries/`）：同路徑覆蓋（資源包層級，原 JAR 不動）。
//!   Lavender 是否支援分語系資料夾未查證（推測），同路徑覆蓋確定看得到中文。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// JAR 裡要抽出來翻的手冊頁（小寫、正斜線）。
pub fn is_guide_md_entry(lower: &str) -> bool {
    lower.starts_with("assets/") && lower.ends_with(".md") && (lower.contains("/ae2guide/") || lower.contains("/lavender/entries/"))
}

/// 遊戲資料夾／暫存區裡的手冊頁。
pub fn is_guide_md_path(path: &Path) -> bool {
    let lower = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    lower.ends_with(".md") && (lower.contains("/ae2guide/") || lower.contains("/lavender/entries/")) && lower.contains("/assets/")
}

/// 手冊頁的 zh_tw 位置與優先度（數字小者優先，同 book_locale_priority）；`None`＝不是手冊頁或已是 zh_tw。
pub fn guide_zh_tw_rel(rel: &Path) -> Option<(PathBuf, u8)> {
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let lower: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    if !lower.last().is_some_and(|l| l.ends_with(".md")) {
        return None;
    }
    if let Some(i) = lower.iter().position(|p| p == "ae2guide") {
        let mut out: Vec<String> = parts[..=i].to_vec();
        let rest = &parts[i + 1..];
        let (priority, rest) = match rest.first().map(|s| s.to_ascii_lowercase()) {
            Some(ref s) if s == "_zh_tw" => return None,
            Some(ref s) if s == "_zh_cn" => (0, &rest[1..]),
            Some(ref s) if s == "_zh_hk" => (1, &rest[1..]),
            Some(ref s) if s.starts_with('_') && s.contains('_') && s.len() <= 7 => (3, &rest[1..]),
            _ => (2, rest),
        };
        out.push("_zh_tw".into());
        out.extend(rest.iter().cloned());
        return Some((out.iter().collect(), priority));
    }
    lower.windows(2).any(|w| w[0] == "lavender" && w[1] == "entries").then(|| (rel.to_path_buf(), 2))
}

enum Line<'a> {
    Keep(&'a str),
    /// (前綴, 文字, 後綴)
    Text(&'a str, &'a str, &'a str),
}

fn classify(raw: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut in_code = false;
    let mut in_front = false;
    // 審查 F8：多行 MDX 標籤（`<ItemImage` 換行寫屬性，直到 `>` 才結束）
    let mut in_tag = false;
    for (n, line) in raw.split_inclusive('\n').enumerate() {
        let body = line.trim_end_matches(['\n', '\r']);
        let eol = &line[body.len()..];
        let t = body.trim();
        if n == 0 && t == "---" {
            in_front = true;
            out.push(Line::Keep(line));
            continue;
        }
        if in_front {
            if t == "---" {
                in_front = false;
                out.push(Line::Keep(line));
                continue;
            }
            match body.find("title:") {
                Some(i) if body[..i].trim().is_empty() => {
                    let value = &body[i + 6..];
                    let lead = value.len() - value.trim_start().len();
                    let text = value.trim();
                    let quoted = text.len() >= 2 && ((text.starts_with('"') && text.ends_with('"')) || (text.starts_with('\'') && text.ends_with('\'')));
                    if !quoted && is_prose(text) {
                        let start = i + 6 + lead;
                        out.push(Line::Text(&line[..start], &line[start..start + text.len()], &line[start + text.len()..]));
                        continue;
                    }
                    out.push(Line::Keep(line));
                }
                _ => out.push(Line::Keep(line)),
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
            out.push(Line::Keep(line));
            continue;
        }
        if in_tag {
            if t.contains('>') {
                in_tag = false;
            }
            out.push(Line::Keep(line));
            continue;
        }
        if t.starts_with('<') && !t.contains('>') {
            in_tag = true;
            out.push(Line::Keep(line));
            continue;
        }
        // 審查 F8：行內程式碼（反引號）、縮排 4 格／Tab 的程式碼區塊不翻
        let indented_code = body.starts_with("    ") || body.starts_with('\t');
        if in_code || indented_code || t.contains('`') || t.is_empty() || t.starts_with('<') || t.contains("](") || t.contains('{') || t.contains('}') || (t.contains('<') && t.contains('>')) || t.starts_with('|') {
            out.push(Line::Keep(line));
            continue;
        }
        let lead_ws = body.len() - body.trim_start().len();
        let rest = &body[lead_ws..];
        let marker = markdown_marker(rest);
        let start = lead_ws + marker;
        let text = body[start..].trim_end();
        if is_prose(text) {
            out.push(Line::Text(&line[..start], &line[start..start + text.len()], &line[start + text.len()..body.len()]));
            if !eol.is_empty() {
                out.push(Line::Keep(eol));
            }
        } else {
            out.push(Line::Keep(line));
        }
    }
    out
}

/// 行首的 Markdown 記號長度（`## `、`- `、`* `、`1. `、`> `）。
fn markdown_marker(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b'#' || bytes[i] == b'>') {
        i += 1;
    }
    if i > 0 {
        return i + s[i..].len() - s[i..].trim_start().len();
    }
    if s.starts_with("- ") || s.starts_with("* ") || s.starts_with("+ ") {
        return 2;
    }
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && s[digits..].starts_with(". ") {
        return digits + 2;
    }
    0
}

fn is_prose(t: &str) -> bool {
    t.chars().any(|c| c.is_alphabetic()) && !super::mech_tokens::skip_before_ai(t)
}

/// 可翻的文字片段。
pub fn segments(raw: &str) -> Vec<String> {
    classify(raw)
        .into_iter()
        .filter_map(|l| match l {
            Line::Text(_, t, _) => Some(t.to_string()),
            Line::Keep(_) => None,
        })
        .collect()
}

/// 套用譯文；沒有任何變更回 `None`。譯文含換行時不用（會打亂 Markdown 結構）。
pub fn apply(raw: &str, map: &HashMap<String, String>) -> Option<String> {
    let mut changed = false;
    let mut out = String::with_capacity(raw.len());
    for line in classify(raw) {
        match line {
            Line::Keep(s) => out.push_str(s),
            Line::Text(pre, text, post) => {
                out.push_str(pre);
                match map.get(text) {
                    Some(zh) if zh != text && !zh.contains('\n') => {
                        out.push_str(zh);
                        changed = true;
                    }
                    _ => out.push_str(text),
                }
                out.push_str(post);
            }
        }
    }
    changed.then_some(out)
}

#[cfg(test)]
#[path = "markdown_text_tests.rs"]
mod tests;
