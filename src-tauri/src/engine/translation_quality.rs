//! 譯文品質閘門：假 zh（仍英／中英碎片）不得鎖死 pending。

/// 字串是否幾乎全為拉丁顯示（保留 §、格式佔位符後判斷）。
pub fn is_still_english(text: &str) -> bool {
    let stripped = strip_format_noise(text);
    if stripped.is_empty() {
        return false;
    }
    let mut letters = 0usize;
    let mut latin = 0usize;
    let mut cjk = 0usize;
    for c in stripped.chars() {
        if c.is_whitespace() || is_punct_or_symbol(c) {
            continue;
        }
        if is_cjk(c) {
            cjk += 1;
            continue;
        }
        if c.is_ascii_alphabetic() {
            letters += 1;
            latin += 1;
        } else if c.is_alphanumeric() {
            letters += 1;
        }
    }
    if cjk > 0 {
        return false;
    }
    latin >= 2 && latin * 10 >= letters.max(1) * 7
}

/// CJK 與較長拉丁詞並存，或 CJK 嵌在拉丁詞中間。
pub fn is_mixed_fragment(text: &str) -> bool {
    let stripped = strip_format_noise(text);
    if stripped.is_empty() {
        return false;
    }
    let has_cjk = stripped.chars().any(is_cjk);
    if !has_cjk {
        return false;
    }
    // CJK 夾在兩個 ASCII 字母之間：smoldering煉獄ember
    let chars: Vec<char> = stripped.chars().collect();
    for i in 1..chars.len().saturating_sub(1) {
        if is_cjk(chars[i])
            && chars[i - 1].is_ascii_alphabetic()
            && chars[i + 1].is_ascii_alphabetic()
        {
            return true;
        }
    }
    // 同時有 CJK 與長度≥4 的拉丁詞
    let mut latin_word = String::new();
    let mut long_latin = false;
    for c in stripped.chars() {
        if c.is_ascii_alphabetic() {
            latin_word.push(c);
        } else {
            if latin_word.len() >= 4 {
                long_latin = true;
            }
            latin_word.clear();
        }
    }
    if latin_word.len() >= 4 {
        long_latin = true;
    }
    long_latin
}

/// 譯文是否可當「已完成中文」：非空、≠原文、非仍英、非混雜碎片。
pub fn is_usable_zh(en: &str, zh: &str) -> bool {
    let zh = zh.trim();
    if zh.is_empty() {
        return false;
    }
    let en_n = normalize_cmp(en);
    let zh_n = normalize_cmp(zh);
    if !en_n.is_empty() && en_n == zh_n {
        return false;
    }
    if is_still_english(zh) {
        return false;
    }
    if is_mixed_fragment(zh) {
        return false;
    }
    true
}

/// 有效中文比例：有 en 對照時用閘門；無 en 則要求非 still_english。
#[allow(dead_code)]
pub fn usable_ratio(zh_map: &std::collections::HashMap<String, String>, en_map: Option<&std::collections::HashMap<String, String>>) -> f32 {
    if zh_map.is_empty() {
        return 0.0;
    }
    let mut ok = 0usize;
    for (k, v) in zh_map {
        let en = en_map.and_then(|m| m.get(k)).map(|s| s.as_str()).unwrap_or("");
        if is_usable_zh(en, v) {
            ok += 1;
        }
    }
    ok as f32 / zh_map.len() as f32
}

fn strip_format_noise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '§' && i + 1 < chars.len() {
            i += 2;
            continue;
        }
        // 簡單略過 %s / %d / %1$s / {0}
        if c == '%' {
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '$') {
                i += 1;
            }
            if i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            continue;
        }
        if c == '{' {
            i += 1;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn normalize_cmp(s: &str) -> String {
    strip_format_noise(s)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
        || ('\u{3400}'..='\u{4dbf}').contains(&c)
        || ('\u{f900}'..='\u{faff}').contains(&c)
}

fn is_punct_or_symbol(c: char) -> bool {
    c.is_ascii_punctuation() || matches!(c, '·' | '…' | '—' | '–' | '「' | '」' | '『' | '』' | '（' | '）')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn still_english_samples() {
        assert!(is_still_english("Timewood Banister"));
        assert!(is_still_english("Blue Journal"));
        assert!(is_still_english("Acoustic Guitar (Nylon)"));
        assert!(!is_still_english("傳送門珍珠"));
        assert!(!is_still_english("蔚藍旅記"));
    }

    #[test]
    fn mixed_fragment_samples() {
        assert!(is_mixed_fragment("黑色Argillite Brick 階梯"));
        assert!(is_mixed_fragment("smoldering煉獄ember"));
        assert!(is_mixed_fragment("Transmuting Easter 蛋糕"));
        assert!(is_mixed_fragment("transformingeaster蛋糕"));
        assert!(!is_mixed_fragment("傳送門珍珠"));
        assert!(!is_mixed_fragment("蔚藍旅記"));
    }

    #[test]
    fn usable_zh_rejects_fake_and_mixed() {
        assert!(!is_usable_zh("Timewood Banister", "Timewood Banister"));
        assert!(!is_usable_zh("Blue Journal", "Blue Journal"));
        assert!(!is_usable_zh("x", "黑色Argillite Brick 階梯"));
        assert!(is_usable_zh("Blue Journal", "蔚藍旅記"));
        assert!(is_usable_zh("Gate Pearl", "傳送門珍珠"));
    }
}
