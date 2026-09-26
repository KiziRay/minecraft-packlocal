//! B2#2：佔位符的三個顯示安全修補（放在獨立模組，不再加肥 placeholder.rs）。
//!
//! 1. **`%s` 被 AI 重排**：遮罩把 `%s … %s` 送成 `{0} … {1}`，中文語序常把兩者對調成
//!    `{1} … {0}`。舊版照順序還原成 `%s … %s`，結果「A 被 B 擊殺」變成「B 被 A 擊殺」。
//!    還原時若看到順序被換，就改寫成 `%2$s … %1$s`（Java／Minecraft 都支援），語意不變。
//! 2. **色碼數量**：`§` 色碼少一個或多一個，後面整段會變色。只缺結尾 `§r` 補回，其他退回。
//!    `&` 開頭的不算（`R&D` 的 `&D` 不是色碼）。
//! 3. **JSON text component**：`{"text":"Campaign","color":"gold"}` 只把 text 給 AI，
//!    其餘結構遮起來；還原後結構必須一字不差、而且仍是合法 JSON。

use std::sync::OnceLock;

use regex::Regex;

use super::output_guard::{split_text_component, ComponentPart};
use super::placeholder::{extract, RE_JAVA_SPEC};

fn java_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(RE_JAVA_SPEC).expect("java spec regex"))
}

fn index_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{(\d+)\}").expect("unmask regex"))
}

fn json_escape_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\\(?:u[0-9A-Fa-f]{4}|.)").expect("json escape regex"))
}

/// 沒有索引、會被重排影響的 Java 佔位符（`%s`、`%d`、`%.2f`；不含 `%%`、`%n`）。
fn is_plain_spec(tok: &str) -> bool {
    java_re().find(tok).is_some_and(|m| m.as_str() == tok)
        && tok != "%%"
        && !tok.ends_with('n')
        && !tok[1..].contains('$')
}

fn is_indexed_spec(tok: &str) -> bool {
    java_re().find(tok).is_some_and(|m| m.as_str() == tok) && tok[1..].contains('$')
}

/// 還原遮罩；`%s` 類佔位符被 AI 換了順序時改寫成 `%N$s`。
pub fn unmask_reordering(masked: &str, tokens: &[String]) -> String {
    let ranks: Vec<Option<usize>> = {
        let mut next = 0usize;
        tokens
            .iter()
            .map(|t| {
                is_plain_spec(t).then(|| {
                    next += 1;
                    next - 1
                })
            })
            .collect()
    };
    let plain_count = ranks.iter().flatten().count();
    let has_indexed = tokens.iter().any(|t| is_indexed_spec(t));
    let seen: Vec<usize> = index_re()
        .captures_iter(masked)
        .filter_map(|c| c[1].parse::<usize>().ok())
        .filter_map(|i| ranks.get(i).copied().flatten())
        .collect();
    let mut sorted = seen.clone();
    sorted.sort_unstable();
    let is_permutation = sorted == (0..plain_count).collect::<Vec<_>>();
    let reorder = plain_count >= 2 && !has_indexed && is_permutation && seen != sorted;

    index_re()
        .replace_all(masked, |c: &regex::Captures| {
            let Some(i) = c[1].parse::<usize>().ok() else {
                return c[0].to_string();
            };
            match (tokens.get(i), ranks.get(i).copied().flatten()) {
                (Some(tok), Some(rank)) if reorder => format!("%{}${}", rank + 1, &tok[1..]),
                (Some(tok), _) => tok.clone(),
                (None, _) => c[0].to_string(),
            }
        })
        .into_owned()
}

/// 把 Java 佔位符正規化成「第幾個參數＋型別」；混用有索引與無索引時回 None（不判等價）。
fn canonical_specs(s: &str) -> Option<Vec<String>> {
    let mut plain = 0usize;
    let mut indexed = false;
    let mut out = Vec::new();
    for m in java_re().captures_iter(s) {
        let whole = &m[0];
        if whole == "%%" || whole.ends_with('n') {
            out.push(whole.to_string());
        } else if let Some(idx) = m.get(1) {
            indexed = true;
            let rest = &whole[1 + idx.as_str().len() + 1..];
            out.push(format!("{}${rest}", idx.as_str()));
        } else {
            plain += 1;
            out.push(format!("{plain}${}", &whole[1..]));
        }
    }
    if indexed && plain > 0 {
        return None;
    }
    out.sort();
    Some(out)
}

/// `%s … %d` 與 `%2$d … %1$s` 等價（同樣的參數、同樣的型別）。
pub fn specs_equivalent(source: &str, translated: &str) -> bool {
    let (Some(a), Some(b)) = (canonical_specs(source), canonical_specs(translated)) else {
        return false;
    };
    let other = |s: &str| -> Vec<String> {
        extract(s)
            .keyed
            .into_iter()
            .filter(|k| !is_indexed_spec(k))
            .collect()
    };
    a == b && other(source) == other(translated)
}

/// 色碼與其結束位置（位元組）。`§x` 一律算；`&x` 只在 x 是**小寫**色碼字元時才算
/// （設定檔慣例一律小寫；`R&D`、`Rock&Roll`、`& ` 都不是色碼）。
fn colour_codes_at(s: &str) -> Vec<(String, usize)> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    for (i, &(_, c)) in chars.iter().enumerate() {
        let Some(&(next_pos, next)) = chars.get(i + 1) else {
            break;
        };
        let end = next_pos + next.len_utf8();
        if c == '§' {
            out.push((format!("§{}", next.to_ascii_lowercase()), end));
        } else if c == '&' && matches!(next, '0'..='9' | 'a'..='f' | 'k'..='o' | 'r') {
            out.push((format!("&{next}"), end));
        }
    }
    out
}

fn colour_codes(s: &str) -> Vec<String> {
    let mut out: Vec<String> = colour_codes_at(s).into_iter().map(|(c, _)| c).collect();
    out.sort();
    out
}

/// 字串（去掉結尾空白後）是否停在一個色碼上；回傳那個色碼。
fn trailing_colour(s: &str) -> Option<String> {
    let t = s.trim_end();
    colour_codes_at(t)
        .into_iter()
        .last()
        .filter(|(_, end)| *end == t.len())
        .map(|(c, _)| c)
}

/// 字串結尾停在一個非重設的色碼上（顏色會漏到後面拼接的文字）。
pub(crate) fn ends_on_unreset_colour(s: &str) -> bool {
    trailing_colour(s).is_some_and(|c| !c.ends_with('r'))
}

/// 色碼（`§` 與 `&`）數量與種類要一致；只缺原文結尾那個重設碼時補回；
/// 原文結尾沒停在色碼上時，譯文結尾不得停在非重設的色碼（顏色會漏到後面拼接的文字）。
/// 其餘回 None（退回原文）。
pub fn repair_colour_codes(source: &str, translated: &str) -> Option<String> {
    let src = colour_codes(source);
    let dst = colour_codes(translated);
    let fixed = if src == dst {
        translated.to_string()
    } else {
        let reset = trailing_colour(source).filter(|c| c.ends_with('r'))?;
        let mut with_reset = dst.clone();
        with_reset.push(reset.clone());
        with_reset.sort();
        if with_reset != src {
            return None;
        }
        let core = translated.trim_end();
        let tail = &translated[core.len()..];
        format!("{core}{reset}{tail}")
    };
    if trailing_colour(source).is_none()
        && trailing_colour(&fixed).is_some_and(|c| !c.ends_with('r'))
    {
        return None;
    }
    // 原文以重設碼結尾：譯文結尾也必須是重設碼。AI 把它移到句中時，把句中最後一個
    // 同樣的重設碼搬回結尾（數量不變，再檢查一次結果相同）。
    if let Some(reset) = trailing_colour(source).filter(|c| c.ends_with('r')) {
        if !trailing_colour(&fixed).is_some_and(|c| c.ends_with('r')) {
            let at = colour_codes_at(&fixed)
                .into_iter()
                .filter(|(c, _)| *c == reset)
                .map(|(_, end)| end)
                .last()?;
            let start = at - fixed[..at].chars().rev().take(2).map(char::len_utf8).sum::<usize>();
            let without = format!("{}{}", &fixed[..start], &fixed[at..]);
            let core = without.trim_end();
            let tail = &without[core.len()..];
            let moved = format!("{core}{reset}{tail}");
            // 搬移後重算：拼出新碼（例如落單的 § 接上重設碼）或數量變了就退回
            return (colour_codes(&moved) == colour_codes(source)).then_some(moved);
        }
    }
    Some(fixed)
}

/// 原文是 JSON text component 時，譯文必須仍是合法 JSON 且結構一字不差。
pub fn text_component_ok(source: &str, translated: &str) -> bool {
    let Some(src) = split_text_component(source) else {
        return true;
    };
    let Some(dst) = split_text_component(translated) else {
        return false;
    };
    let skeleton = |parts: &[ComponentPart]| -> Vec<String> {
        parts
            .iter()
            .map(|p| match p {
                ComponentPart::Skeleton(s) => s.clone(),
                ComponentPart::Text(_) => "\u{0}".into(),
            })
            .collect()
    };
    skeleton(&src) == skeleton(&dst)
}

/// 遮罩 JSON text component：結構與 JSON 跳脫字元變成 token，只有 text 給 AI。
/// 不是 component 時回 None，由呼叫端走一般遮罩。
pub fn mask_text_component(text: &str, mask_re: &Regex) -> Option<(String, Vec<String>)> {
    let parts = split_text_component(text)?;
    let mut tokens: Vec<String> = Vec::new();
    let mut masked = String::new();
    fn push_token(tok: &str, tokens: &mut Vec<String>, masked: &mut String) {
        masked.push_str(&format!("{{{}}}", tokens.len()));
        tokens.push(tok.to_string());
    }
    for part in parts {
        match part {
            ComponentPart::Skeleton(s) => push_token(&s, &mut tokens, &mut masked),
            ComponentPart::Text(t) => {
                let mut last = 0usize;
                let mut chunks: Vec<(bool, &str)> = Vec::new();
                for m in json_escape_re().find_iter(&t) {
                    chunks.push((false, &t[last..m.start()]));
                    chunks.push((true, m.as_str()));
                    last = m.end();
                }
                chunks.push((false, &t[last..]));
                for (is_escape, chunk) in chunks {
                    if is_escape {
                        push_token(chunk, &mut tokens, &mut masked);
                        continue;
                    }
                    let mut pos = 0usize;
                    for m in mask_re.find_iter(chunk) {
                        masked.push_str(&chunk[pos..m.start()]);
                        push_token(m.as_str(), &mut tokens, &mut masked);
                        pos = m.end();
                    }
                    masked.push_str(&chunk[pos..]);
                }
            }
        }
    }
    Some((masked, tokens))
}
