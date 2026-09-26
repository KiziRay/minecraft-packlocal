//! B2#3：統一的「這段有沒有中文」判斷，以及遊戲圖示字（字型私用區）辨識。
//!
//! 以前 convert、ftbquests、text_overlay、origins、quests_books、mech_tokens 各寫一份，
//! 範圍各不相同（有的含相容漢字 U+F900–FAFF、有的連擴展 A 都沒有）。
//! 相容漢字區在模組字型裡常被拿來放**圖示**（技能樹、按鍵提示），
//! 把它當中文會讓「只有圖示的字串」被當成已翻好，甚至被簡轉繁改成普通漢字。

/// 中文（漢字）字元：基本區、擴展 A、擴展 B 以後。**不含**相容漢字 U+F900–FAFF（見模組說明）。
pub fn is_chinese_char(c: char) -> bool {
    matches!(c,
        '\u{3400}'..='\u{4dbf}'       // 擴展 A
        | '\u{4e00}'..='\u{9fff}'     // 基本區
        | '\u{20000}'..='\u{3134f}'   // 擴展 B 以後（含 C–G）
    )
}

pub fn looks_chinese(s: &str) -> bool {
    s.chars().any(is_chinese_char)
}

/// 模組字型的圖示字：私用區（U+E000–F8FF、補充私用區）與相容漢字 U+F900–FAFF。
pub fn is_icon_glyph(c: char) -> bool {
    matches!(c,
        '\u{e000}'..='\u{f8ff}'
        | '\u{f900}'..='\u{faff}'
        | '\u{f0000}'..='\u{ffffd}'
        | '\u{100000}'..='\u{10fffd}'
    )
}

/// 字串裡的圖示字（依出現順序）。
pub fn icon_glyphs(s: &str) -> Vec<char> {
    s.chars().filter(|c| is_icon_glyph(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b2_chinese_detection_covers_ext_a_and_b_but_not_icons() {
        assert!(looks_chinese("鐵錠"));
        assert!(looks_chinese("\u{3400}"));
        assert!(looks_chinese("\u{20000}"));
        assert!(!looks_chinese("Iron Ingot"));
        assert!(!looks_chinese("\u{F900}"), "相容漢字區在模組字型裡是圖示");
        assert!(!looks_chinese("\u{E001} Mana"));
    }

    #[test]
    fn b2_icon_glyphs_are_recognised() {
        for c in ['\u{E000}', '\u{F8FF}', '\u{F900}', '\u{FAFF}', '\u{F0001}', '\u{100001}'] {
            assert!(is_icon_glyph(c), "{:X}", c as u32);
        }
        assert!(!is_icon_glyph('鐵'));
        assert!(!is_icon_glyph('A'));
    }

    #[test]
    fn b2_no_duplicate_chinese_detection_left() {
        // 統一後，這些檔不得再自帶一份判斷
        for (name, src) in [
            ("convert.rs", include_str!("convert.rs")),
            ("ftbquests.rs", include_str!("ftbquests.rs")),
            ("text_overlay.rs", include_str!("text_overlay.rs")),
            ("origins.rs", include_str!("origins.rs")),
            ("quests_books.rs", include_str!("quests_books.rs")),
            ("mech_tokens.rs", include_str!("mech_tokens.rs")),
        ] {
            for f in ["fn looks_chinese", "fn contains_cjk", "fn is_cjk"] {
                assert!(!src.contains(f), "{name} 仍自帶 {f}");
            }
        }
    }

    #[test]
    fn b2_icon_glyphs_survive_mask_and_unmask() {
        use crate::engine::placeholder::{mask, unmask, validate_and_repair};
        let src = "\u{E001} Mana Pool \u{F900}";
        let (masked, tokens) = mask(src);
        assert!(!masked.contains('\u{E001}') && !masked.contains('\u{F900}'), "圖示不給 AI 看：{masked}");
        let restored = unmask(&masked.replace("Mana Pool", "魔力池"), &tokens);
        assert_eq!(restored, "\u{E001} 魔力池 \u{F900}");
        assert!(validate_and_repair(src, &restored).is_some());
    }

    #[test]
    fn b2_nfc_in_keyhash_never_touches_icons_or_stored_translation() {
        use crate::engine::shared_tm::normalize_source;
        use crate::engine::tm::Tm;
        // 私用區圖示在 NFC 下不變
        assert_eq!(normalize_source("\u{E001} Mana"), "\u{E001} Mana");
        // 相容漢字只會影響「查詢鍵」，存下的譯文原樣取回
        let mut tm = Tm::default();
        tm.insert("\u{F900} Skill Point", "\u{F900} 技能點");
        assert_eq!(tm.get("\u{F900} Skill Point").as_deref(), Some("\u{F900} 技能點"));
    }
}
