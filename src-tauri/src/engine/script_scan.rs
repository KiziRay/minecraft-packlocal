//! B3#3：腳本裡的顯示字串（KubeJS `.js`、CraftTweaker `.zs`），**依位置**找出、依位置替換。
//!
//! 以前用「整檔字串取代」：`Text.of("Stone")` 翻成中文時，同檔其他地方的 `"Stone"`
//! （例如條件判斷、物品名比對）也被換掉，腳本邏輯就壞了。現在只換顯示 API 參數本身那一段。
//!
//! 支援（只認第一個參數是字面字串、而且就是整個參數）：
//! - KubeJS：`Text.of／string／literal`、`Text.<顏色>`（red、gold…）、`Component.literal`、
//!   `text.literal`、`.displayName(…)`、`.tooltip(…)`、`.tell(…)`、`.setStatusMessage(…)`。
//!   反引號字串：沒有 `${…}` 插值才翻；有插值整段略過（拆字面段會打亂語序，選安全的）。
//! - CraftTweaker：`.addTooltip(…)`、`.addShiftTooltip(…)`、`.setDisplayName(…)`、
//!   `.displayName = …`、`MCTextComponent.createStringTextComponent(…)`、`Component.literal(…)`。
//!
//! 「不上傳共享庫」旗標：伺服器腳本（server_scripts）與 `.tell`／`.setStatusMessage` 的字
//! 常是伺服器公告、玩家名、私訊，標記 `private`，由 share_policy 在上傳入口擋掉。

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    KubeJs,
    CraftTweaker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLiteral {
    /// 字面字串（含引號）在原文的位置
    pub start: usize,
    pub end: usize,
    pub quote: char,
    pub text: String,
    /// 不上傳共享庫
    pub private: bool,
}

const LIT: &str = r#"("(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|`(?:\\.|[^`\\])*`)"#;

const TEXT_METHODS: &str = "of|string|literal|black|darkBlue|darkGreen|darkAqua|darkRed|darkPurple|gold|gray|darkGray|blue|green|aqua|red|lightPurple|yellow|white";

fn js_patterns() -> &'static [(Regex, bool)] {
    static RE: OnceLock<Vec<(Regex, bool)>> = OnceLock::new();
    RE.get_or_init(|| {
        let mk = |prefix: &str, private: bool| {
            (Regex::new(&format!(r"{prefix}\s*\(\s*{LIT}\s*[,)]")).expect("script regex"), private)
        };
        vec![
            mk(&format!(r"\bText\.(?:{TEXT_METHODS})"), false),
            mk(r"\b(?:Component|text)\.literal", false),
            mk(r"\.displayName", false),
            mk(r"\.tooltip", false),
            mk(r"\.tell", true),
            mk(r"\.setStatusMessage", true),
        ]
    })
}

fn zs_patterns() -> &'static [(Regex, bool)] {
    static RE: OnceLock<Vec<(Regex, bool)>> = OnceLock::new();
    RE.get_or_init(|| {
        let call = |prefix: &str| (Regex::new(&format!(r"{prefix}\s*\(\s*{LIT}\s*[,)]")).expect("zs regex"), false);
        vec![
            call(r"\.addTooltip"),
            call(r"\.addShiftTooltip"),
            call(r"\.setDisplayName"),
            call(r"\bMCTextComponent\.createStringTextComponent"),
            call(r"\bComponent\.literal"),
            (Regex::new(&format!(r"\.displayName\s*=\s*{LIT}\s*;")).expect("zs assign"), false),
        ]
    })
}

/// 找出所有可翻的顯示字串（依出現位置排序、不重疊）。`server_side`＝伺服器腳本（整檔標 private）。
pub fn find_literals(src: &str, kind: ScriptKind, server_side: bool) -> Vec<ScriptLiteral> {
    let patterns = match kind {
        ScriptKind::KubeJs => js_patterns(),
        ScriptKind::CraftTweaker => zs_patterns(),
    };
    let mut out: Vec<ScriptLiteral> = Vec::new();
    for (re, private) in patterns {
        for cap in re.captures_iter(src) {
            let Some(m) = cap.get(1) else { continue };
            let token = m.as_str();
            let quote = token.chars().next().unwrap_or('"');
            if kind == ScriptKind::CraftTweaker && quote == '`' {
                continue;
            }
            let Some(text) = decode(token) else { continue };
            if !should_translate(&text) {
                continue;
            }
            out.push(ScriptLiteral { start: m.start(), end: m.end(), quote, text, private: *private || server_side });
        }
    }
    out.sort_by_key(|l| l.start);
    out.dedup_by(|b, a| b.start < a.end);
    // 審查 F6：.tell(…)／.setStatusMessage(…) 參數裡巢狀的字串（例如 tell(Text.of("…"))）一律私有
    if kind == ScriptKind::KubeJs {
        for (open, close) in private_call_spans(src) {
            for lit in out.iter_mut().filter(|l| l.start > open && l.end <= close) {
                lit.private = true;
            }
        }
    }
    out
}

/// `.tell(`／`.setStatusMessage(` 呼叫的括號範圍（跳過字串內的括號）。
fn private_call_spans(src: &str) -> Vec<(usize, usize)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\.(?:tell|setStatusMessage)\s*\(").expect("tell regex"));
    let bytes = src.as_bytes();
    let mut spans = Vec::new();
    for m in re.find_iter(src) {
        let open = m.end() - 1;
        let mut depth = 0i32;
        let mut quote: Option<u8> = None;
        let mut i = open;
        while i < bytes.len() {
            let c = bytes[i];
            if let Some(q) = quote {
                if c == b'\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
                // 行註解：跳到行尾
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            } else if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
                // 區塊註解：跳到 */
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            } else {
                match c {
                    b'"' | b'\'' | b'`' => quote = Some(c),
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            spans.push((open, i + 1));
                            break;
                        }
                    }
                    _ => {}
                }
            }
            i += 1;
        }
    }
    spans
}

/// 依位置替換；`choose` 回 `None` 就保留原字串。
pub fn replace_literals(src: &str, literals: &[ScriptLiteral], mut choose: impl FnMut(&ScriptLiteral) -> Option<String>) -> String {
    let mut out = String::with_capacity(src.len() + src.len() / 4);
    let mut last = 0usize;
    for lit in literals {
        if lit.start < last {
            continue;
        }
        out.push_str(&src[last..lit.start]);
        match choose(lit) {
            Some(value) if value != lit.text => out.push_str(&encode(lit.quote, &value)),
            _ => out.push_str(&src[lit.start..lit.end]),
        }
        last = lit.end;
    }
    out.push_str(&src[last..]);
    out
}

fn should_translate(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty()
        && t.chars().any(|c| c.is_alphabetic())
        && !t.contains("minecraft:")
        && !t.contains("#forge:")
        && !super::mech_tokens::skip_before_ai(t)
}

fn decode(token: &str) -> Option<String> {
    let quote = token.chars().next()?;
    let body = token.get(1..token.len() - 1)?;
    if quote == '`' && body.contains("${") {
        return None;
    }
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            c @ ('"' | '\'' | '`' | '\\' | '$') => out.push(c),
            // 其他跳脫（\x、八進位…）看不懂就整段不翻
            _ => return None,
        }
    }
    Some(out)
}

fn encode(quote: char, value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push(quote);
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            '$' if quote == '`' => out.push_str("\\$"),
            _ => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
#[path = "script_scan_tests.rs"]
mod tests;
