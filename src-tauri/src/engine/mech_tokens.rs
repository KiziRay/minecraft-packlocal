//! 機制 id／ASCII enum token 共用判斷（overlay、FTB、診斷共用，避免兩套規則漂移）。

/// 全小寫 snake／kebab-case 單 token（無空白、純 ASCII）：不當顯示文翻。
/// 例：`goal`、`crafting`、`has_iron`、`mid-left`、`bottom-right`。
pub fn is_ascii_enum_token(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.contains(' ') || !t.is_ascii() {
        return false;
    }
    t.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && t.chars().any(|c| c.is_ascii_alphabetic())
}

/// 遊戲資源副檔名（檔名／相對路徑會進 ResourceLocation）。
const RESOURCE_PATH_EXTS: &[&str] = &[
    ".txt", ".json", ".json5", ".snbt", ".nbt", ".mcmeta", ".properties",
    ".png", ".jpg", ".jpeg", ".webp", ".gif",
    ".ogg", ".wav", ".mp3",
    ".ttf", ".otf", ".zip",
];

/// 檔名或相對資源路徑（無空白、僅 RL 字元），不當顯示文翻。
///
/// 例：`root.txt`、`alligator.json`、`book/animal_dictionary/root`。
/// 純英文字 `hello`（無 `/`、無副檔名）不是路徑。
pub fn is_resource_path_token(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if !lower
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '/' | '.' | '_' | '-'))
    {
        return false;
    }
    if !lower.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    lower.contains('/') || RESOURCE_PATH_EXTS.iter().any(|ext| lower.ends_with(ext))
}

fn contains_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        ('\u{4e00}'..='\u{9fff}').contains(&c)
            || ('\u{3400}'..='\u{4dbf}').contains(&c)
            || ('\u{f900}'..='\u{faff}').contains(&c)
    })
}

fn is_legal_resource_location_chars(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && !t.contains(char::is_whitespace)
        && t.chars().all(|c| {
            c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || matches!(c, '/' | '.' | '_' | '-' | ':')
        })
}

/// FancyMenu／機制 meta 區塊（`[groups:]`、`[executables:]` 等）不當顯示文。
pub fn is_bracket_meta_token(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    (lower.starts_with('[') && lower.contains(']'))
        && (lower.contains("[groups:")
            || lower.contains("[instances:")
            || lower.contains("[executables:")
            || lower.contains("[executable:")
            || lower.contains("[loading_requirement"))
}

/// FancyMenu layout 僅允許翻這些鍵（其餘一律當結構）。
/// `source`：`text_v2` 正文；實際值還要過 [`is_fancymenu_translatable_source`]。
pub fn is_fancymenu_display_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "label"
            | "description"
            | "tooltip"
            | "text"
            | "hoverlabel"
            | "buttontext"
            | "title"
            | "source"
    )
}

/// FancyMenu `source`／類似鍵的值：只翻玩家可讀句，擋資源路徑與媒體。
pub fn is_fancymenu_translatable_source(value: &str) -> bool {
    let t = value.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("null") {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("[source:") {
        return false;
    }
    for suf in [
        ".png", ".jpg", ".jpeg", ".webp", ".gif", ".ogg", ".wav", ".mp3", ".json", ".mcmeta",
        ".ttf", ".otf", ".nbt", ".zip", ".fma",
    ] {
        // 純路徑／檔名才擋；句子結尾偶有副檔名字樣仍允許（少見）
        if lower.ends_with(suf) && !t.contains(' ') && !lower.contains("%n%") {
            return false;
        }
        if lower.contains(suf) && (lower.contains('/') || lower.contains('\\') || lower.contains(':'))
        {
            return false;
        }
    }
    if is_ascii_enum_token(t) || is_bracket_meta_token(t) || is_resource_path_token(t) {
        return false;
    }
    // 短按鈕文案（Wiki／Discord）也要翻：有字母即可；長句／%n% 一定收
    if lower.contains("%n%") {
        return true;
    }
    if t.contains(' ') || looks_like_display_sentence(t) {
        return true;
    }
    t.chars().any(|c| c.is_ascii_alphabetic()) && t.len() >= 2
}

/// 原文像機制 token，譯文卻含 CJK／被翻壞 → 拒用 TM／共享庫／寫檔／貢獻。
pub fn is_poisoned_mech_translation(source: &str, translated: &str) -> bool {
    let src = source.trim();
    let zh = translated.trim();
    if src.is_empty() || zh.is_empty() || src == zh {
        return false;
    }
    if is_bracket_meta_token(src) {
        return true;
    }
    if is_resource_path_token(src) {
        return contains_cjk(zh) || !is_legal_resource_location_chars(zh);
    }
    if !is_ascii_enum_token(src) {
        return false;
    }
    contains_cjk(zh)
}

/// Aggressive 欄位（如 `category`）值「像句子」才翻：含空白、CJK、或常見標點。
pub fn looks_like_display_sentence(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || is_resource_path_token(t) {
        return false;
    }
    if t.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        return true;
    }
    if t.contains(' ') || t.contains('\n') || t.contains('\t') {
        return true;
    }
    t.chars().any(|c| {
        matches!(
            c,
            '.' | '!' | '?' | ',' | ';' | ':' | '。' | '！' | '？' | '，' | '、' | '；' | '：'
        )
    })
}

/// Aggressive 鍵中「值常是 id、僅句子才翻」的欄位。
pub fn is_sentence_only_aggressive_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("category")
}

/// 幾乎不會有玩家可讀字串的資料包子路徑（跨模組通用）。
pub fn is_mechanism_path_segment(lower_slash_path: &str) -> bool {
    const SKIP: &[&str] = &[
        "/recipes/",
        "/loot_tables/",
        "/tags/",
        "/structures/",
        "/worldgen/",
        "/functions/",
        "/predicates/",
        "/item_modifiers/",
        "/dimension/",
        "/dimension_type/",
        "/biome/",
        "/noise_settings/",
        "/template_pool/",
        "/processor_list/",
        "/configured_feature/",
        "/placed_feature/",
        "/chat_type/",
        "/damage_type/",
        "/mmorpg_value_calc/",
        "/mmorpg_stat_condition/",
        "/mmorpg_stat_effect/",
        "/mmorpg_stat_compat/",
        "/mmorpg_auto_item/",
        "/mmorpg_base_stats/",
        "/mmorpg_game_balance/",
        "/mmorpg_atlas_layout/",
    ];
    SKIP.iter().any(|s| lower_slash_path.contains(s))
}

/// Origins／Apoli 能力樹交給 `origins.rs`，overlay／jar_display 勿雙寫。
pub fn is_origins_powers_path(lower_slash_path: &str) -> bool {
    lower_slash_path.contains("/powers/")
        || lower_slash_path.contains("/origins/")
        || lower_slash_path.contains("/origin_layers/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_token_rejects_snake_ids() {
        assert!(is_ascii_enum_token("goal"));
        assert!(is_ascii_enum_token("strawberry_crate"));
        assert!(is_ascii_enum_token("has_iron"));
        assert!(is_ascii_enum_token("mid-left"));
        assert!(is_ascii_enum_token("bottom-right"));
        assert!(is_ascii_enum_token("mid-centered"));
        assert!(!is_ascii_enum_token("Get rich soil"));
        assert!(!is_ascii_enum_token("Combat Ready"));
        assert!(!is_ascii_enum_token("目標"));
    }

    #[test]
    fn poisoned_mech_rejects_translated_anchors() {
        assert!(is_poisoned_mech_translation("mid-left", "中左"));
        assert!(is_poisoned_mech_translation("bottom-right", "右下"));
        assert!(is_poisoned_mech_translation(
            "[groups:][instances:]",
            "[群組:][實例:]"
        ));
        assert!(!is_poisoned_mech_translation("Multiplayer", "多人遊戲"));
        assert!(!is_poisoned_mech_translation("mid-left", "mid-left"));
        assert!(is_poisoned_mech_translation("root.txt", "根.txt"));
        assert!(is_poisoned_mech_translation("alligator.json", "短吻鱷.json"));
        assert!(!is_poisoned_mech_translation("root.txt", "root.txt"));
        assert!(!is_poisoned_mech_translation("Get rich soil", "獲得肥沃土壤"));
    }

    #[test]
    fn resource_path_token_detects_filenames_and_rel_paths() {
        assert!(is_resource_path_token("root.txt"));
        assert!(is_resource_path_token("alligator.json"));
        assert!(is_resource_path_token("book/animal_dictionary/root"));
        assert!(is_resource_path_token("book/animal_dictionary/root.json"));
        assert!(!is_resource_path_token("hello"));
        assert!(!is_resource_path_token("Get rich soil"));
        assert!(!is_resource_path_token("Welcome to the pack"));
        assert!(!looks_like_display_sentence("root.txt"));
        assert!(looks_like_display_sentence("Get rich soil"));
    }

    #[test]
    fn fancymenu_display_keys() {
        assert!(is_fancymenu_display_key("label"));
        assert!(is_fancymenu_display_key("Tooltip"));
        assert!(is_fancymenu_display_key("source"));
        assert!(!is_fancymenu_display_key("anchor_point"));
        assert!(!is_fancymenu_display_key("element_type"));
    }

    #[test]
    fn fancymenu_source_value_filter() {
        assert!(is_fancymenu_translatable_source(
            "You are a Flameborn, created by S'kellak.%n%%n%You're being sent."
        ));
        assert!(is_fancymenu_translatable_source("The Talent Tree"));
        assert!(is_fancymenu_translatable_source("Check out the Official Wiki"));
        assert!(!is_fancymenu_translatable_source(
            "[source:local]/config/fancymenu/assets/welcome_screen/who_you_are.png"
        ));
        assert!(!is_fancymenu_translatable_source(
            "[source:location]minecraft:textures/item/book.png"
        ));
        assert!(!is_fancymenu_translatable_source("null"));
        assert!(!is_fancymenu_translatable_source("mid-left"));
        assert!(!is_fancymenu_translatable_source("root.txt"));
    }

    #[test]
    fn sentence_detects_space_cjk_punct() {
        assert!(looks_like_display_sentence("Welcome to Combat"));
        assert!(looks_like_display_sentence("戰鬥分類"));
        assert!(looks_like_display_sentence("Hello!"));
        assert!(!looks_like_display_sentence("Combat"));
        assert!(!looks_like_display_sentence("mining"));
    }

    #[test]
    fn origins_path_detection() {
        assert!(is_origins_powers_path("data/origins/powers/foo.json"));
        assert!(is_origins_powers_path("data/mod/origins/human.json"));
        assert!(!is_origins_powers_path("data/mod/quests/chapter.json"));
    }
}
