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

/// 品質閘失敗原因（量測用；判定規則見 `quality_fail_reason`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityFailReason {
    StillEnglish,
    MixedFragment,
    SameAsSource,
}

impl QualityFailReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StillEnglish => "still_english",
            Self::MixedFragment => "mixed_fragment",
            Self::SameAsSource => "same_as_source",
        }
    }

    pub fn label_zh(self) -> &'static str {
        match self {
            Self::StillEnglish => "仍為英文",
            Self::MixedFragment => "中英碎片",
            Self::SameAsSource => "與原文相同",
        }
    }
}

/// CJK 嵌在拉丁詞中間，或 CJK 比例過低卻夾長拉丁詞。
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
    let mut cjk = 0usize;
    let mut visible = 0usize;
    let mut latin_word = String::new();
    let mut long_latin = false;
    for c in stripped.chars() {
        if c.is_whitespace() || is_punct_or_symbol(c) {
            if latin_word.len() >= 4 {
                long_latin = true;
            }
            latin_word.clear();
            continue;
        }
        visible += 1;
        if is_cjk(c) {
            cjk += 1;
            if latin_word.len() >= 4 {
                long_latin = true;
            }
            latin_word.clear();
            continue;
        }
        if c.is_ascii_alphabetic() {
            latin_word.push(c);
        } else if latin_word.len() >= 4 {
            long_latin = true;
            latin_word.clear();
        } else {
            latin_word.clear();
        }
    }
    if latin_word.len() >= 4 {
        long_latin = true;
    }
    // 專有名詞保留（如「Flan 突擊步槍」）：CJK≥2 且佔可見字 ≥30% 視為合格
    if cjk >= 2 && visible > 0 && cjk * 100 >= visible * 30 {
        return false;
    }
    long_latin
}

/// 原文本身就不該被翻譯——羅馬數字、圖示字元、純格式字串、單位符號、品牌名。
///
/// 這類原文即使 AI 回覆與原文一模一樣，也是**正確答案**而非翻譯失敗。
/// 實測依據：某次真實翻譯的 663 條「品質未過」裡有 501 條是 same_as_source，
/// 其中絕大多數是 `enchantment.level.109 = "CIX"`、`icon.star = "§f"`、
/// `"%s FPS"`、`"OptiFine"` 這種本來就該原樣保留的字。過去一律當失敗丟掉，
/// 造成它們永遠留在待補、每輪重送 AI、永遠不會結案。
pub fn source_stays_unchanged(en: &str) -> bool {
    let t = en.trim();
    if t.is_empty() {
        return false;
    }
    is_roman_numeral(t) || is_format_or_icon_only(t) || is_unit_or_axis_label(t) || is_known_brand(t)
}

/// 規範寫法的羅馬數字（附魔等級、藥水效力）。
///
/// 用「嚴格規範驗證」而不是「字元都在 IVXLCDM 內」：後者會誤殺 MILD／CIVIL／
/// LIVID 這類剛好由羅馬字母組成的英文字，而規範驗證會因為 IL／IV+IL 這種
/// 非法減法組合自動排除它們。少數仍能通過的英文字另外列黑名單。
fn is_roman_numeral(t: &str) -> bool {
    if t.len() < 2 || t.len() > 15 {
        return false;
    }
    if !t.chars().all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')) {
        return false;
    }
    // 剛好也是規範羅馬數字的英文字（MIX = 1009）
    const LOOKS_LIKE_WORD: &[&str] = &["MIX", "MID", "DIM"];
    if LOOKS_LIKE_WORD.contains(&t) {
        return false;
    }
    match roman_to_int(t) {
        Some(n) if n > 0 => int_to_roman(n) == t,
        _ => false,
    }
}

fn roman_to_int(t: &str) -> Option<u32> {
    let value = |c: char| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        'M' => 1000,
        _ => 0,
    };
    let chars: Vec<char> = t.chars().collect();
    // 用 i64 累加：減法寫法（IX）第一步會先減，u32 會在此下溢。
    let mut total: i64 = 0;
    for i in 0..chars.len() {
        let v = value(chars[i]);
        if v == 0 {
            return None;
        }
        let next = chars.get(i + 1).map(|c| value(*c)).unwrap_or(0);
        if v < next {
            total -= v as i64;
        } else {
            total += v as i64;
        }
    }
    u32::try_from(total).ok()
}

fn int_to_roman(mut n: u32) -> String {
    const TABLE: &[(u32, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (v, s) in TABLE {
        while n >= *v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// 去掉格式碼與佔位符後沒有任何文數字：`§f`、`✔`、`↓%s`、`[⌛ %s]`、`...... `。
fn is_format_or_icon_only(t: &str) -> bool {
    let stripped = strip_format_noise(t);
    !stripped.chars().any(|c| c.is_alphanumeric())
}

/// 單位符號或座標軸標籤：`%s RPM`、`%s FPS`、`ms`、`su`、`X: %s`、`240 ˚T`。
fn is_unit_or_axis_label(t: &str) -> bool {
    // 這些短字雖然短，但確實該翻，不能被單位規則吃掉。
    const REAL_WORDS: &[&str] = &["ok", "no", "yes", "on", "off", "up", "all", "new", "add"];
    if REAL_WORDS.contains(&t.trim().to_ascii_lowercase().as_str()) {
        return false;
    }
    let stripped = strip_format_noise(t);
    let alpha: String = stripped.chars().filter(|c| c.is_alphabetic()).collect();
    if !stripped.chars().any(|c| c.is_alphanumeric()) {
        // 交給 is_format_or_icon_only 判
        return false;
    }
    if alpha.is_empty() {
        // 去掉佔位符後只剩數字：`240`、`0`
        return true;
    }
    if alpha.chars().count() > 4 {
        return false;
    }
    // 全大寫短符號（RPM／FPS／LF／GPS）或 1–2 個字母（ms／su／X／Y／Z）
    alpha.chars().all(|c| c.is_ascii_uppercase()) || alpha.chars().count() <= 2
}

/// 品牌／平台名：翻成中文反而讓玩家找不到對應的網站或模組。
fn is_known_brand(t: &str) -> bool {
    const BRANDS: &[&str] = &[
        "optifine",
        "curseforge",
        "modrinth",
        "patchouli",
        "discord",
        "github",
        "youtube",
        "twitch",
        "kofi",
        "ko-fi",
        "patreon",
        "minecraft",
        "forge",
        "fabric",
        "quilt",
        "neoforge",
        "iris",
        "sodium",
        "jei",
        "emi",
        "rei",
    ];
    let cleaned = t
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim();
    BRANDS.contains(&cleaned.to_ascii_lowercase().as_str())
}

/// 回傳品質失敗原因；`Ok(())` 表示可當已完成中文。
pub fn quality_fail_reason(en: &str, zh: &str) -> Result<(), QualityFailReason> {
    let zh = zh.trim();
    if zh.is_empty() {
        return Err(QualityFailReason::StillEnglish);
    }
    let en_n = normalize_cmp(en);
    let zh_n = normalize_cmp(zh);
    if !en_n.is_empty() && en_n == zh_n {
        // 原文本來就不該翻（羅馬數字／圖示／單位／品牌）→ 原樣保留是正確結果，
        // 不是翻譯失敗；讓它結案，不要永遠卡在待補並每輪重送 AI。
        if source_stays_unchanged(en) {
            return Ok(());
        }
        return Err(QualityFailReason::SameAsSource);
    }
    if is_still_english(zh) {
        return Err(QualityFailReason::StillEnglish);
    }
    if is_mixed_fragment(zh) {
        return Err(QualityFailReason::MixedFragment);
    }
    Ok(())
}

/// 譯文是否可當「已完成中文」：非空、≠原文、非仍英、非混雜碎片。
pub fn is_usable_zh(en: &str, zh: &str) -> bool {
    quality_fail_reason(en, zh).is_ok()
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
        // 專有名詞＋足夠中文：不再當碎片
        assert!(!is_mixed_fragment("Flan 突擊步槍"));
        assert!(!is_mixed_fragment("Mekanism 能量單元"));
    }

    #[test]
    fn quality_fail_reason_three_kinds() {
        assert_eq!(
            quality_fail_reason("Diamond Sword", "Diamond Sword"),
            Err(QualityFailReason::SameAsSource)
        );
        assert_eq!(
            QualityFailReason::SameAsSource.as_str(),
            "same_as_source"
        );
        assert_eq!(QualityFailReason::StillEnglish.label_zh(), "仍為英文");
        assert_eq!(
            quality_fail_reason("Blue Journal", "Blue Journal leftover"),
            Err(QualityFailReason::StillEnglish)
        );
        assert_eq!(
            quality_fail_reason("x", "黑色Argillite Brick 階梯"),
            Err(QualityFailReason::MixedFragment)
        );
        assert!(quality_fail_reason("Flan Assault Rifle", "Flan 突擊步槍").is_ok());
        assert!(quality_fail_reason("Gate Pearl", "傳送門珍珠").is_ok());
    }

    #[test]
    fn roman_numerals_are_recognized_but_english_words_are_not() {
        // 實測資料裡 279 條待補是這種附魔等級
        for s in ["II", "IX", "XC", "XLIX", "CIX", "LXXIII", "CVI"] {
            assert!(is_roman_numeral(s), "{s} 應該被認成羅馬數字");
        }
        // 剛好由羅馬字母組成的英文字不能誤殺
        for s in ["MILD", "CIVIL", "LIVID", "VIVID", "MILL", "DILL", "MIDI", "MIX", "DIM"] {
            assert!(!is_roman_numeral(s), "{s} 是英文字，不該當羅馬數字");
        }
        // 非規範寫法（IIII 應寫成 IV）也不算
        assert!(!is_roman_numeral("IIII"));
        assert!(!is_roman_numeral("VV"));
        assert!(!is_roman_numeral("I"), "單字元交給既有的長度規則處理");
    }

    #[test]
    fn format_and_icon_only_strings_are_recognized() {
        // 實測資料裡 79 條待補是這種字型圖示
        assert!(is_format_or_icon_only("§f"));
        assert!(is_format_or_icon_only("✔"));
        assert!(is_format_or_icon_only("↓%s"));
        assert!(is_format_or_icon_only("[⌛ %s]"));
        assert!(is_format_or_icon_only("%s / %s"));
        assert!(is_format_or_icon_only("...... "));
        // 有實字的不算
        assert!(!is_format_or_icon_only("§aDiamond Sword"));
        assert!(!is_format_or_icon_only("Deals %s damage"));
    }

    #[test]
    fn unit_and_axis_labels_are_recognized() {
        for s in ["%s RPM", "%s FPS", "%s LF", "ms", "su", "GPS", "X: %s", "240 ˚T"] {
            assert!(is_unit_or_axis_label(s), "{s} 應該被當成單位／座標標籤");
        }
        // 真正該翻的短字不能被吃掉
        for s in ["OK", "Yes", "No", "Off", "All", "New"] {
            assert!(!is_unit_or_axis_label(s), "{s} 該翻，不能當單位");
        }
        // 完整句子不能被吃掉
        assert!(!is_unit_or_axis_label("Exit Minecraft"));
        assert!(!is_unit_or_axis_label("Mipmap Levels"));
    }

    #[test]
    fn same_as_source_is_accepted_only_for_untranslatable_sources() {
        // 這是本輪的核心修正：AI 對這些字回覆原文是正確答案，不是失敗
        assert!(
            quality_fail_reason("CIX", "CIX").is_ok(),
            "羅馬數字原樣保留應該算完成"
        );
        assert!(quality_fail_reason("§f", "§f").is_ok());
        assert!(quality_fail_reason("%s FPS", "%s FPS").is_ok());
        assert!(quality_fail_reason("OptiFine", "OptiFine").is_ok());
        assert!(quality_fail_reason("[CurseForge]", "[CurseForge]").is_ok());

        // 完整句子原樣回來，仍然是 AI 沒翻 → 維持失敗
        assert_eq!(
            quality_fail_reason("Diamond Sword", "Diamond Sword"),
            Err(QualityFailReason::SameAsSource)
        );
        assert_eq!(
            quality_fail_reason("Exit Minecraft", "Exit Minecraft"),
            Err(QualityFailReason::SameAsSource)
        );
        assert_eq!(
            quality_fail_reason("OK", "OK"),
            Err(QualityFailReason::SameAsSource),
            "OK 該翻成確定，不能因為短就放行"
        );
    }

    #[test]
    fn usable_zh_rejects_fake_and_mixed() {
        assert!(!is_usable_zh("Timewood Banister", "Timewood Banister"));
        assert!(!is_usable_zh("Blue Journal", "Blue Journal"));
        assert!(!is_usable_zh("x", "黑色Argillite Brick 階梯"));
        assert!(is_usable_zh("Blue Journal", "蔚藍旅記"));
        assert!(is_usable_zh("Gate Pearl", "傳送門珍珠"));
        assert!(is_usable_zh("Flan Assault Rifle", "Flan 突擊步槍"));
    }
}
