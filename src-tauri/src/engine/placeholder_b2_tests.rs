//! B2#2：`%s` 被重排、色碼數量、JSON text component 的回歸測試。

use super::*;

#[test]
fn b2_reordered_plain_specs_are_rewritten_to_indexed_form() {
    let src = "%s was slain by %s";
    let (m, t) = mask(src);
    assert_eq!(m, "{0} was slain by {1}");
    // AI 依中文語序把兩個參數對調
    let restored = unmask("{1} 擊殺了 {0}", &t);
    // 主詞受詞不顛倒：第二個參數（兇手）在前
    assert_eq!(restored, "%2$s 擊殺了 %1$s");
    let safe = validate_and_repair(src, &restored).expect("改寫成 %N$s 後應可用");
    assert_eq!(safe, "%2$s 擊殺了 %1$s");
}

#[test]
fn b2_mixed_type_reorder_keeps_conversion() {
    let src = "%s has %d items";
    let (_m, t) = mask(src);
    let restored = unmask("{1} 個物品屬於 {0}", &t);
    assert_eq!(restored, "%2$d 個物品屬於 %1$s");
    assert!(validate_and_repair(src, &restored).is_some());
}

#[test]
fn b2_unreordered_specs_stay_plain() {
    let (_m, t) = mask("Deals %s to %s");
    assert_eq!(unmask("對 {1} 造成 {0}", &t), "對 %2$s 造成 %1$s");
    assert_eq!(unmask("造成 {0} 給 {1}", &t), "造成 %s 給 %s");
}

#[test]
fn b2_raw_reordered_positional_still_rejected() {
    // 沒經過遮罩、無從得知對應關係的原始重排，仍要退回
    assert!(validate_and_repair("%s has %d items", "%d 個物品屬於 %s").is_none());
}

#[test]
fn b2_colour_code_count_mismatch_is_rejected() {
    // 少了一個色碼：後面整段會變色
    assert!(validate_and_repair("§aGreen§r and §cRed§r", "§a綠色§r和§c紅色").is_some(),
        "只缺結尾 §r 可以補回");
    assert!(validate_and_repair("§aGreen§r and §cRed§r text", "§a綠色和§c紅色§r文字").is_none(),
        "中間少一個 §r 修不好，退回");
    assert!(validate_and_repair("§aGreen", "§a§l綠色").is_none(), "多出色碼也退回");
}

#[test]
fn b2_trailing_reset_is_repaired() {
    let out = validate_and_repair("§eGold Ingot§r", "§e金錠");
    assert_eq!(out.as_deref(), Some("§e金錠§r"));
}

#[test]
fn b2_ampersand_is_not_treated_as_colour_code() {
    // R&D 的 &D 不是色碼，翻掉不能被當成色碼遺失
    assert!(validate_and_repair("R&D Lab", "研發實驗室").is_some());
}

#[test]
fn b2_text_component_masks_structure_and_translates_text() {
    let src = r#"{"text":"Campaign","color":"gold"}"#;
    let (masked, tokens) = mask(src);
    assert!(masked.contains("Campaign"), "text 要送去翻：{masked}");
    assert!(!masked.contains("color"), "結構不給 AI 看：{masked}");
    let ai = masked.replace("Campaign", "戰役");
    let restored = unmask(&ai, &tokens);
    assert_eq!(restored, r#"{"text":"戰役","color":"gold"}"#);
    assert!(validate_and_repair(src, &restored).is_some());
}

#[test]
fn b2_broken_text_component_is_rejected() {
    let src = r#"{"text":"Campaign","color":"gold"}"#;
    assert!(validate_and_repair(src, r#"{"text":"戰役","color":"金色"}"#).is_none(), "結構被改");
    assert!(validate_and_repair(src, r#"{"text":"戰"役","color":"gold"}"#).is_none(), "JSON 壞掉");
    assert!(validate_and_repair(src, "戰役").is_none(), "整個結構被吃掉");
}

#[test]
fn b2_f1_ampersand_colour_codes_are_counted() {
    // 設定檔常用 & 色碼：少一個一樣會整段變色
    assert!(validate_and_repair("&aGreen &cRed", "&a綠色 紅色").is_none());
    assert!(validate_and_repair("&aGreen &cRed", "&a綠色 &c紅色").is_some());
    assert_eq!(validate_and_repair("&eGold&r", "&e金").as_deref(), Some("&e金&r"));
    // 不是色碼的 &：R&D、& 後接空白、大寫字母
    assert!(validate_and_repair("R&D Lab", "研發實驗室").is_some());
    assert!(validate_and_repair("Salt & Pepper", "鹽和胡椒").is_some());
    assert!(validate_and_repair("Rock&Roll", "搖滾").is_some());
}

#[test]
fn b2_f1_translation_must_not_end_on_a_dangling_colour() {
    // 原文結尾沒有色碼，譯文卻停在色碼上：後面拼接的文字會被染色
    assert!(validate_and_repair("§aGreen§r text", "§a綠色§r文字§a").is_none());
    assert!(validate_and_repair("Hello §bworld", "你好世界§b").is_none());
    // 停在重設碼上沒關係
    assert!(validate_and_repair("§aGreen text§r", "§a綠色文字§r").is_some());
    // 原文本來就停在色碼上：照原文
    assert!(validate_and_repair("Prefix §a", "前綴 §a").is_some());
}

#[test]
fn b2_l1_source_ending_with_reset_keeps_reset_at_the_end() {
    // 原文以重設碼結尾：譯文結尾也要是重設碼，後面拼接的文字才不會被染色
    let out = validate_and_repair("§aGreen§r", "§a綠§r色").unwrap();
    assert!(out.ends_with("§r"), "{out}");
    let out = validate_and_repair("&aGreen&r", "&a綠&r色").unwrap();
    assert!(out.ends_with("&r"), "{out}");
    assert_eq!(validate_and_repair("§eGold§r", "§e金§r").as_deref(), Some("§e金§r"));
}

#[test]
fn b2_fb_mid_sentence_reset_is_moved_not_duplicated() {
    assert_eq!(validate_and_repair("§aGreen§r", "§a綠§r色").as_deref(), Some("§a綠色§r"));
    assert_eq!(validate_and_repair("&aGreen&r", "&a綠&r色").as_deref(), Some("&a綠色&r"));
    assert_eq!(validate_and_repair("§eGold§r ", "§e金§r色 ").as_deref(), Some("§e金色§r "));
}

/// 修復後的結果再檢查一次必須照樣通過且不變（資源包與參考包會重複檢查同一條）。
#[test]
fn b2_fb_repair_is_idempotent() {
    let cases: &[(&str, &str)] = &[
        ("§aGreen§r", "§a綠§r色"),
        ("&aGreen&r", "&a綠&r色"),
        ("§eGold Ingot§r", "§e金錠"),
        ("&eGold&r", "&e金"),
        ("§aGreen§r and §cRed§r", "§a綠色§r和§c紅色"),
        ("§aGreen text§r", "§a綠色文字§r"),
        ("Prefix §a", "前綴 §a"),
        ("&aGreen &cRed", "&a綠色 &c紅色"),
        ("R&D Lab", "研發實驗室"),
        ("Deals %s damage", "造成 ％s 傷害"),
        ("Deals %s damage", "造成 % s 傷害"),
        ("%1$s gave %2$s", "%2$s 收到 %1$s"),
        (" Level ", "等級"),
        ("Level: ", "等級："),
        ("Open the door", "打開\n門"),
        (r#"{"text":"Campaign","color":"gold"}"#, r#"{"text":"戰役","color":"gold"}"#),
        ("\u{E001} Mana Pool \u{F900}", "\u{E001} 魔力池 \u{F900}"),
        ("§9Mana: §f%s§7/§f%s", "§9魔力：§f%s§7/§f%s"),
    ];
    for (src, t) in cases {
        let once = validate_and_repair(src, t).unwrap_or_else(|| panic!("{src} → {t} 第一次應可用"));
        let twice = validate_and_repair(src, &once).unwrap_or_else(|| panic!("{src} → {once} 第二次被退回"));
        assert_eq!(once, twice, "{src}：修復不是冪等");
    }
    // 重排改寫的結果（%N$s）再檢查一次也要照樣通過
    let (_m, tok) = mask("%s was slain by %s");
    let restored = unmask("{1} 擊殺了 {0}", &tok);
    let once = validate_and_repair("%s was slain by %s", &restored).unwrap();
    assert_eq!(validate_and_repair("%s was slain by %s", &once).as_deref(), Some(once.as_str()));
}

#[test]
fn b2_f3_4_moving_reset_never_creates_a_new_code() {
    // 搬移後拼出新碼（§ 接上 §r 的 §）要退回
    assert!(validate_and_repair("§aGreen§r", "§a綠§r色§").is_none());
    // §r§r 結尾：搬最後一個，數量不變
    let out = validate_and_repair("§aGreen§r§r", "§a綠§r§r色").unwrap();
    assert!(out.ends_with("§r") && out.matches("§r").count() == 2, "{out}");
    assert_eq!(validate_and_repair("§aGreen§r§r", &out).as_deref(), Some(out.as_str()));
    // &r 與 §r 混用：搬的是原文結尾那一種
    let out = validate_and_repair("§aGreen§r and &bBlue&r", "§a綠§r和&b藍&r色").unwrap();
    assert!(out.ends_with("&r") && out.contains("§r"), "{out}");
    assert_eq!(validate_and_repair("§aGreen§r and &bBlue&r", &out).as_deref(), Some(out.as_str()));
    // 重排加色碼
    let src = "§a%s§r was slain by §c%s";
    let (_m, tok) = mask(src);
    let restored = unmask("{3}{4}擊殺了{0}{1}{2}", &tok);
    let once = validate_and_repair(src, &restored).expect("重排加色碼應可用");
    assert!(once.contains("%2$s") && once.contains("%1$s"), "{once}");
    assert_eq!(validate_and_repair(src, &once).as_deref(), Some(once.as_str()));
}
