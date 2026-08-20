//! 簡體／異體 → 台灣正體轉換。
//!
//! **內建轉換表（`zhconv`，純 Rust）**，不需要玩家安裝 Python 或 OpenCC。
//! 相較舊版呼叫外部 `python -c "opencc..."`：
//! - 不再每 800 條開一次子程序（整合包動輒數十萬條字串）
//! - 不再有 Windows 黑窗閃爍
//! - 不再有「本機 OpenCC 不可用 → 交出簡中」的失敗模式
//!
//! 轉換目標為 `zh-Hant-TW`：除了字形（简→簡），也含台灣慣用詞彙
//! （軟件→軟體、質量→品質…），等價於 OpenCC 的 `s2twp`。

use std::collections::HashMap;

use zhconv::{zhconv, Variant};

use super::jar_scan::LangMap;

const LANGMAP_PROGRESS_EVERY: u64 = 4096;

/// 單條字串轉台灣正體。已是正體則幾乎原樣返回（冪等）。
pub fn convert_s2tw(text: &str) -> String {
    if !needs_conversion(text) {
        return text.to_string();
    }
    zhconv(text, Variant::ZhTW)
}

/// 批次轉換；保證輸出長度與輸入相同。
pub fn convert_s2tw_batch(texts: &[String]) -> Vec<String> {
    texts.iter().map(|t| convert_s2tw(t)).collect()
}

/// 沒有任何 CJK 字元就不必進轉換表（絕大多數待譯英文走這條捷徑）。
fn needs_conversion(s: &str) -> bool {
    s.chars().any(is_cjk)
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3400}'..='\u{4dbf}'   // 擴展 A
        | '\u{4e00}'..='\u{9fff}' // 基本區
        | '\u{f900}'..='\u{faff}' // 相容漢字
    )
}

/// 對整張 LangMap 做台灣正體轉換（AI 補譯後必跑，避免簡中混入）。
#[allow(dead_code)]
pub fn convert_langmap_s2tw(zh: &mut LangMap) {
    convert_langmap_s2tw_with_progress(zh, None);
}

/// 只轉換 predicate 為 true 的 (ns, key)，避免對原生 zh_tw 再跑 s2twp。
pub fn convert_langmap_s2tw_selective(
    zh: &mut LangMap,
    should_convert: &dyn Fn(&str, &str) -> bool,
) {
    for (ns, map) in zh.iter_mut() {
        for (k, v) in map.iter_mut() {
            if should_convert(ns, k) && needs_conversion(v) {
                *v = zhconv(v, Variant::ZhTW);
            }
        }
    }
}

pub fn convert_langmap_s2tw_with_progress(
    zh: &mut LangMap,
    mut on_progress: Option<&mut dyn FnMut(u64, u64)>,
) {
    let total = if on_progress.is_some() {
        zh.values().map(|map| map.len() as u64).sum()
    } else {
        0
    };
    let mut done = 0u64;
    let mut next_tick = LANGMAP_PROGRESS_EVERY;
    if let Some(cb) = on_progress.as_deref_mut() {
        cb(0, total);
    }
    for map in zh.values_mut() {
        for v in map.values_mut() {
            if needs_conversion(v) {
                *v = zhconv(v, Variant::ZhTW);
            }
            done += 1;
            if let Some(cb) = on_progress.as_deref_mut() {
                if done >= next_tick || done == total {
                    cb(done, total);
                    next_tick = done.saturating_add(LANGMAP_PROGRESS_EVERY);
                }
            }
        }
    }
}

pub fn apply_phrase_dict(text: &str, dict: &HashMap<String, String>) -> String {
    use super::translation_quality::{is_mixed_fragment, is_still_english};
    if dict.is_empty() {
        return text.to_string();
    }
    // 純英／混雜碎片不做片段置換，留給整句 AI
    if is_still_english(text) || is_mixed_fragment(text) {
        return text.to_string();
    }
    let mut keys: Vec<&String> = dict.keys().collect();
    keys.sort_by(|a, b| b.len().cmp(&a.len()));
    let mut out = text.to_string();
    for k in keys {
        if let Some(v) = dict.get(k) {
            out = replace_with_word_boundary(&out, k, v);
        }
    }
    out
}

fn replace_with_word_boundary(haystack: &str, needle: &str, rep: &str) -> String {
    if needle.is_empty() || !haystack.contains(needle) {
        return haystack.to_string();
    }
    // 非 ASCII 片語：不做拉丁字界，直接最長優先置換（中文詞無空格字界）
    if !needle.is_ascii() {
        return haystack.replace(needle, rep);
    }
    let lower_h = haystack.to_ascii_lowercase();
    let lower_n = needle.to_ascii_lowercase();
    let h_bytes = haystack.as_bytes();
    let mut out = String::with_capacity(haystack.len());
    let mut from = 0usize;
    let mut search_from = 0usize;
    while let Some(rel) = lower_h[search_from..].find(&lower_n) {
        let start = search_from + rel;
        let end = start + lower_n.len();
        if !(haystack.is_char_boundary(start) && haystack.is_char_boundary(end)) {
            search_from = start.saturating_add(1);
            while search_from < lower_h.len() && !haystack.is_char_boundary(search_from) {
                search_from += 1;
            }
            if search_from >= lower_h.len() {
                break;
            }
            continue;
        }
        let before_ok =
            start == 0 || !is_word_byte(h_bytes.get(start.wrapping_sub(1)).copied().unwrap_or(0));
        let after_ok = end >= h_bytes.len() || !is_word_byte(h_bytes[end]);
        if before_ok && after_ok {
            out.push_str(&haystack[from..start]);
            out.push_str(rep);
            from = end;
        }
        search_from = end;
        if search_from >= lower_h.len() {
            break;
        }
    }
    out.push_str(&haystack[from..]);
    out
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 詞典替換後常見的斷尾「…之」（例：`of Wealth` → `財富之`）。
pub fn strip_of_suffix_zhi(s: &str) -> String {
    const KEEP: &[&str] = &[
        "之主", "之力", "之心", "之影", "之王", "之怒", "之眼", "之手", "之盾", "之劍", "之書",
        "之塔", "之地",
    ];
    if !s.ends_with('之') {
        return s.to_string();
    }
    for k in KEEP {
        if s.ends_with(k) {
            return s.to_string();
        }
    }
    s.trim_end_matches('之').to_string()
}

/// 給使用者看的轉換引擎名稱。
pub fn converter_name() -> &'static str {
    "內建台灣正體轉換表（zh-Hant-TW）"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_simplified_to_traditional() {
        assert_eq!(convert_s2tw("钻石剑"), "鑽石劍");
        assert_eq!(convert_s2tw("红石中继器"), "紅石中繼器");
    }

    #[test]
    fn applies_taiwan_vocabulary() {
        // s2twp 等級：詞彙也要台灣化，不只是字形
        let out = convert_s2tw("软件质量");
        assert!(out.contains("軟體"), "應轉成台灣用語，實得：{out}");
    }

    #[test]
    fn traditional_input_is_stable() {
        // 已是正體：不得被破壞（冪等）
        for s in ["鑽石劍", "終界使者", "苦力怕"] {
            assert_eq!(convert_s2tw(s), s);
        }
    }

    #[test]
    fn ascii_passes_through_untouched() {
        let s = "Diamond Sword %s §a\n";
        assert_eq!(convert_s2tw(s), s);
    }

    #[test]
    fn batch_preserves_length_and_order() {
        let input: Vec<String> = vec!["钻石".into(), "Sword".into(), "红石".into()];
        let out = convert_s2tw_batch(&input);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1], "Sword");
        assert_eq!(out[0], "鑽石");
    }

    #[test]
    fn strips_dangling_zhi_but_keeps_idioms() {
        assert_eq!(strip_of_suffix_zhi("財富之"), "財富");
        assert_eq!(strip_of_suffix_zhi("烈焰之心"), "烈焰之心");
    }

    #[test]
    fn phrase_dict_prefers_longer_keys() {
        let mut d = HashMap::new();
        d.insert("of the Vampire".to_string(), "吸血鬼".to_string());
        d.insert("of".to_string(), "的".to_string());
        // 純英文跳過片段置換（避免「Ring 吸血鬼」混雜）
        assert_eq!(apply_phrase_dict("Ring of the Vampire", &d), "Ring of the Vampire");
        // 已有中文、且無長拉丁詞：才套用片語；較長鍵優先
        let mut d2 = HashMap::new();
        d2.insert("烈焰之".to_string(), "火焰".to_string());
        d2.insert("之".to_string(), "的".to_string());
        assert_eq!(apply_phrase_dict("烈焰之劍", &d2), "火焰劍");
    }

    #[test]
    fn phrase_dict_skips_mixed_fragments() {
        let mut d = HashMap::new();
        d.insert("ember".to_string(), "餘燼".to_string());
        assert_eq!(
            apply_phrase_dict("smoldering煉獄ember", &d),
            "smoldering煉獄ember"
        );
    }

    #[test]
    fn langmap_progress_reports_start_and_finish() {
        let mut zh = LangMap::new();
        zh.entry("minecraft".into()).or_default().insert("a".into(), "钻石".into());
        zh.entry("minecraft".into()).or_default().insert("b".into(), "紅石".into());
        let mut marks = Vec::new();
        convert_langmap_s2tw_with_progress(&mut zh, Some(&mut |done, total| {
            marks.push((done, total));
        }));
        assert!(!marks.is_empty());
        assert_eq!(marks.first().copied(), Some((0, 2)));
        assert_eq!(marks.last().copied(), Some((2, 2)));
        assert_eq!(
            zh.get("minecraft").and_then(|m| m.get("a")).map(String::as_str),
            Some("鑽石")
        );
    }
}
