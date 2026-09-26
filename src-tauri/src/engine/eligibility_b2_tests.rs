//! B2#1：「該不該翻」判定的回歸測試（單位白名單、按鈕字、全小寫單字）。

use super::*;
use crate::engine::mech_tokens::{
    is_identifier_token, is_poisoned_mech_translation, skip_before_ai,
};

fn lang(key: &str, text: &str) -> Eligibility {
    classify(Candidate { source_kind: "lang", logical_key: key, text })
}

#[test]
fn b2_button_words_are_translated_not_kept_as_units() {
    // 舊判定「短＋有大寫＝單位」把按鈕字全吃掉了
    for text in ["Yes", "No", "On", "Off", "Axe", "Done", "Cancel", "OK", "Back", "Hoe", "Map"] {
        let got = lang("gui.example", text);
        assert!(got.is_translatable(), "「{text}」是按鈕／物品字，要翻，實際：{got:?}");
    }
}

#[test]
fn b2_real_unit_symbols_stay_kept_by_whitelist() {
    for text in ["EU", "EU/t", "RF", "RF/t", "FE", "FE/t", "mB", "mB/t", "AE", "kJ", "MW", "Hz", "SU", "RPM", "°C"] {
        assert_eq!(
            lang("unit.example", text),
            Eligibility::Keep(KeepReason::UnitSymbol),
            "「{text}」是單位符號，要保留"
        );
    }
}

#[test]
fn b2_lowercase_single_words_go_to_ai_but_ids_do_not() {
    // 一般英文單字（全小寫）不再默默略過
    for word in ["axe", "on", "off", "enabled", "north", "goal"] {
        assert!(!skip_before_ai(word), "「{word}」是一般單字，要送翻");
        assert!(!is_identifier_token(word));
        assert!(lang("gui.example", word).is_translatable());
    }
    // 真正的機制 id／路徑／meta 仍不送 AI
    for token in ["has_iron", "mid-left", "strawberry_crate", "tier2", "root.txt", "book/root", "[groups:]"] {
        assert!(skip_before_ai(token), "「{token}」是機制代號，不送 AI");
    }
}

#[test]
fn b2_translating_a_plain_lowercase_word_is_not_poison() {
    assert!(!is_poisoned_mech_translation("axe", "斧頭"));
    assert!(!is_poisoned_mech_translation("on", "開"));
    // 機制 id 被翻成中文仍是毒譯文
    assert!(is_poisoned_mech_translation("has_iron", "有鐵"));
    assert!(is_poisoned_mech_translation("mid-left", "中左"));
}
