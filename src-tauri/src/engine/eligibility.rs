//! 集中的「這句話該不該翻」判定（P0-03）。
//!
//! 舊行為：唯一的守門是 `deepseek::looks_untranslatable`，而它**只看 value**。
//! 於是語言 key 被當成待譯字串送進 AI——真實輸出裡就有
//! `botania.entry.bcIntegration`、`book.minecells.…`、`htp_metadata_credits` 這些東西。
//! 代價有四層：白花 AI 費用、品質檢查把它們判成翻不好、覆蓋報告出現假缺口、
//! 而且這些垃圾會被寫進翻譯記憶與社群共享庫，污染是永久的。
//!
//! 判定必須同時看三件事，缺一就會漏：
//! - `source_kind`：來自 lang、書本、任務還是 advancement
//! - `logical_key` / schema path：`pages.0.type` 是結構欄位，`pages.0.text` 是顯示文字
//! - `text`：值本身長什麼樣
//!
//! ## 三種結果
//! `Keep` 是「本來就不該翻」，**不是**待補缺口；把它算進 pending 會製造假缺口。
//! `Review` 是「我不確定」，一律不送 AI 也不寫進資料庫，留給人看。
//! 只有 `Translate` 才可以進 AI／TM／共享庫。

use super::mech_tokens::is_resource_path_token;

/// 判定結果。`Keep` 與 `Review` 都帶原因碼，報告裡要能解釋為什麼沒翻。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eligibility {
    Translate,
    Keep(KeepReason),
    Review(ReviewReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    Empty,
    LanguageKeyAsValue,
    MetadataField,
    ResourceLocation,
    ResourcePath,
    Url,
    Command,
    SchemaField,
    UnitSymbol,
    ProperNoun,
    NoLetters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewReason {
    ControlCharacter,
    RawUnparsedBlob,
}

impl KeepReason {
    /// 給玩家看的白話。報告裡的「安全略過」要說得出為什麼。
    pub fn player_reason(self) -> &'static str {
        match self {
            KeepReason::Empty => "這一項本來就是空的",
            KeepReason::LanguageKeyAsValue => "這是模組內部的語言代號，不是玩家看得到的文字",
            KeepReason::MetadataField => "這是模組的版本／作者等資料欄位，翻了沒有意義",
            KeepReason::ResourceLocation => "這是物品或方塊的識別碼，翻了會讓配方與 JEI 對不上",
            KeepReason::ResourcePath => "這是檔案路徑，翻了遊戲會找不到檔案",
            KeepReason::Url => "這是網址",
            KeepReason::Command => "這是遊戲指令，翻了就打不出來",
            KeepReason::SchemaField => "這是結構欄位（型別／連結／條件），翻了功能會失效",
            KeepReason::UnitSymbol => "這是單位符號，維持原樣才看得懂",
            KeepReason::ProperNoun => "這是模組名或作者名，慣例保留原文",
            KeepReason::NoLetters => "這一項沒有可翻譯的文字",
        }
    }
}

impl ReviewReason {
    pub fn player_reason(self) -> &'static str {
        match self {
            ReviewReason::ControlCharacter => "這段文字含有異常字元，需要人工確認才安全",
            ReviewReason::RawUnparsedBlob => "這個檔案沒能正常解析，需要人工確認",
        }
    }
}

impl Eligibility {
    pub fn is_translatable(&self) -> bool {
        matches!(self, Eligibility::Translate)
    }
    /// 可以進 AI／翻譯記憶／社群共享庫嗎？只有 Translate 可以。
    /// Review 刻意也不行——不確定的東西不該被寫進會被別人重用的資料庫。
    pub fn may_send_to_ai(&self) -> bool {
        self.is_translatable()
    }
    pub fn may_store_in_shared_data(&self) -> bool {
        self.is_translatable()
    }
    /// 這一項算不算「待補缺口」。Keep 不算——它本來就不該翻。
    pub fn counts_as_pending(&self) -> bool {
        matches!(self, Eligibility::Translate | Eligibility::Review(_))
    }
    pub fn player_reason(&self) -> Option<&'static str> {
        match self {
            Eligibility::Translate => None,
            Eligibility::Keep(r) => Some(r.player_reason()),
            Eligibility::Review(r) => Some(r.player_reason()),
        }
    }
}

/// 把一批「刻意保留」的項目整理成玩家看得懂的分類統計。
///
/// 報告不能只說「原樣保留 N 項」——使用者無法判斷那 N 項是工具在保護他，
/// 還是工具漏翻了。逐類說明才對得起「完成要有覆蓋範圍說明」這條不變式。
/// 同時涵蓋「刻意保留」與「需人工確認」——兩者都不是漏翻，但意義不同，
/// 使用者要能分別看到各有多少、各是為什麼。
pub fn summarize_keep_reasons(items: &[(String, String)], source_kind: &str) -> Vec<(String, usize)> {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for (key, text) in items {
        let verdict = classify(Candidate { source_kind, logical_key: key, text });
        if let Some(label) = verdict.player_reason() {
            match counts.iter_mut().find(|(l, _)| *l == label) {
                Some((_, n)) => *n += 1,
                None => counts.push((label, 1)),
            }
        }
    }
    // 多的排前面，讓使用者先看到主要成因
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    counts.into_iter().map(|(l, n)| (l.to_string(), n)).collect()
}

/// 一個待判定的候選。刻意要求呼叫端一起給 key 與來源，
/// 因為只看 value 就是 P0-03 的成因。
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    pub source_kind: &'a str,
    pub logical_key: &'a str,
    pub text: &'a str,
}

/// 結構欄位：**key 以這些結尾**時，底下放的是機制識別字而不是顯示文字。
///
/// 只比對結尾，不比對「包含」。`jei.tooltip.recipe.by` 的最後一段是 `by`，
/// 那是顯示文案；如果用「包含 `.recipe.`」判定就會把它誤殺——實際語料抓到過這個錯。
const SCHEMA_KEY_SUFFIXES: &[&str] = &[
    ".type", ".frame", ".parent", ".recipe", ".translate", ".linked_page", ".advancement",
    ".condition", ".action", ".modifier", ".predicate", ".filter", ".icon", ".trigger",
];

/// Origins/Apoli 的機制節點：這些**路徑段**底下一律不是顯示文字（AGENTS.md 不變式 17）。
/// 與上面不同，這些要比對「包含」，因為機制節點會再往下巢狀。
const MECHANIC_PATH_SEGMENTS: &[&str] = &[
    ".condition.", ".action.", ".modifier.", ".predicate.", ".filter.",
];

/// 這些 key 一律是模組自述資料。
const METADATA_KEY_MARKERS: &[&str] = &["htp_metadata", "_comment"];

/// 慣例保留原文的欄位（模組名、作者名）。
const PROPER_NOUN_KEY_MARKERS: &[&str] = &["nametranslation", "authorname", "credits"];

pub fn classify(c: Candidate<'_>) -> Eligibility {
    let text = c.text;
    let key_lower = c.logical_key.to_ascii_lowercase();

    // ── 先看「不確定」：不確定的東西連判成 Keep 都不對 ──
    if c.logical_key == "__raw__" {
        return Eligibility::Review(ReviewReason::RawUnparsedBlob);
    }
    if text.chars().any(|ch| (ch as u32) < 0x20 && ch != '\n' && ch != '\t' && ch != '\r') {
        return Eligibility::Review(ReviewReason::ControlCharacter);
    }

    // ── 空值 ──
    if text.trim().is_empty() {
        return Eligibility::Keep(KeepReason::Empty);
    }

    // ── 由 key／schema path 判定（value 看不出來的那一半）──
    if METADATA_KEY_MARKERS.iter().any(|m| key_lower.contains(m)) {
        return Eligibility::Keep(KeepReason::MetadataField);
    }
    // schema 規則只套在**結構化來源**（書本／任務／advancement／資料檔）。
    //
    // lang 檔的 key 是模組作者自由命名的扁平字串，`jei.tooltip.recipe.by` 的最後一段
    // 剛好撞到結構欄位名，但它是顯示文案。結構化來源的 key 是真正的 JSON 路徑
    // （`pages.0.type`），撞名的機率極低。用來源區分，兩邊都不會被誤判。
    let structured_source = !matches!(c.source_kind, "lang" | "");
    if structured_source
        && (SCHEMA_KEY_SUFFIXES.iter().any(|s| key_lower.ends_with(s))
            || MECHANIC_PATH_SEGMENTS.iter().any(|s| key_lower.contains(s)))
    {
        return Eligibility::Keep(KeepReason::SchemaField);
    }
    if PROPER_NOUN_KEY_MARKERS.iter().any(|m| key_lower.contains(m)) {
        return Eligibility::Keep(KeepReason::ProperNoun);
    }

    // ── 由 value 判定 ──
    if !text.chars().any(|ch| ch.is_alphabetic()) {
        return Eligibility::Keep(KeepReason::NoLetters);
    }
    if text.starts_with("http://") || text.starts_with("https://") {
        return Eligibility::Keep(KeepReason::Url);
    }
    if text.contains("://") && !text.contains(char::is_whitespace) {
        return Eligibility::Keep(KeepReason::Url);
    }
    if is_command_literal(text) {
        return Eligibility::Keep(KeepReason::Command);
    }
    // 單位符號要排在路徑判定之前：`EU/t` 有斜線又全是允許字元，
    // 會被 is_resource_path_token 當成路徑。兩者都是 Keep，但報告上的原因
    // 「這是單位符號」與「這是檔案路徑」對使用者的意義完全不同。
    if is_unit_symbol(text) {
        return Eligibility::Keep(KeepReason::UnitSymbol);
    }
    if is_resource_location(text) {
        return Eligibility::Keep(KeepReason::ResourceLocation);
    }
    if is_resource_path_token(text) {
        return Eligibility::Keep(KeepReason::ResourcePath);
    }
    // 這是舊版最大的漏洞：值本身就是一個語言 key。
    if looks_like_language_key(text) || text == c.logical_key {
        return Eligibility::Keep(KeepReason::LanguageKeyAsValue);
    }

    Eligibility::Translate
}

/// `minecraft:stone`、`create:andesite_alloy`。判準嚴格（全小寫、無空白），
/// 避免誤殺 `Warning: fire`、`HP:100`。
fn is_resource_location(t: &str) -> bool {
    if !t.contains(':') || t.contains(char::is_whitespace) {
        return false;
    }
    t.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, ':' | '_' | '/' | '.' | '-')
    })
}

/// `/give`、`/botania-skyblock-spread`。
fn is_command_literal(t: &str) -> bool {
    let trimmed = t.trim();
    trimmed.starts_with('/')
        && trimmed.len() > 1
        && !trimmed.contains(char::is_whitespace)
        && trimmed[1..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '/'))
}

/// 值本身長得像語言 key：`botania.entry.bcIntegration`、`_comment.flasks.tag`。
///
/// 判準要夠嚴才不會誤殺正常句子：
/// - 不能有空白（真正的句子幾乎都有）
/// - 至少兩個點（`Mr.Smith` 這種只有一個點的不算）
/// - 每一段都必須是識別字（`Hello.` 會產生空段落，直接排除）
fn looks_like_language_key(t: &str) -> bool {
    if t.contains(char::is_whitespace) {
        return false;
    }
    let segments: Vec<&str> = t.split('.').collect();
    if segments.len() < 3 {
        return false;
    }
    segments.iter().all(|seg| {
        !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    })
}

/// 單位符號白名單（B2：改白名單）。
///
/// 舊判定「短、無空白、有大寫」把 `Yes`／`No`／`On`／`Off`／`Axe` 這種按鈕與物品字
/// 全當成單位保留，按鈕永遠是英文。現在只認得明確列出的單位，
/// 可再接 `/t`、`/s`、`/tick` 這類「每單位時間」後綴。大小寫必須完全一致（`mB` ≠ `MB`）。
const UNIT_SYMBOLS: &[&str] = &[
    "EU", "RF", "FE", "kFE", "MFE", "GFE", "AE", "J", "kJ", "MJ", "GJ", "W", "kW", "MW", "GW",
    "mB", "B", "kB", "Hz", "kHz", "MHz", "V", "kV", "A", "mA", "Ω", "SU", "RPM", "TPS", "FPS",
    "XP", "HP", "MP", "°C", "°F", "K", "ms", "kg", "g", "cm", "mm", "km", "m", "L", "mL",
];

fn is_unit_symbol(t: &str) -> bool {
    let trimmed = t.trim();
    let base = ["/t", "/s", "/tick", "/sec"]
        .iter()
        .find_map(|suffix| trimmed.strip_suffix(suffix))
        .unwrap_or(trimmed);
    UNIT_SYMBOLS.contains(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c<'a>(kind: &'a str, key: &'a str, text: &'a str) -> Candidate<'a> {
        Candidate { source_kind: kind, logical_key: key, text }
    }

    #[test]
    fn language_keys_as_values_are_kept_not_translated() {
        // 這些全部出現在真實輸出的待補清單裡，而且真的被送進過 AI。
        for (key, text) in [
            ("botania.entry.bcIntegration", "botania.entry.bcIntegration"),
            ("_comment.corporeaBlock1.tag", "_comment.corporeaBlock1.tag"),
            ("botania.subtitle.way", "botania.subtitle.way"),
            ("book.minecells.entry.x", "book.minecells.entry.prisoners_quarters"),
            ("sortilege.limit", "sortilege.enchantments.limit.0.10"),
        ] {
            let got = classify(c("lang", key, text));
            assert!(
                matches!(got, Eligibility::Keep(_)),
                "{text} 應該保留原樣，實際：{got:?}"
            );
            assert!(!got.may_send_to_ai(), "{text} 不得送 AI");
            assert!(!got.may_store_in_shared_data(), "{text} 不得寫進共享庫");
            assert!(!got.counts_as_pending(), "{text} 不該被算成待補缺口");
        }
    }

    #[test]
    fn old_value_only_check_would_have_missed_these() {
        // 對照組：舊的 looks_untranslatable 只看 value，這些一個都擋不掉。
        // 它們沒有冒號、有字母、長度夠、不是 URL——每一項都會通過舊守門送進 AI。
        for text in ["botania.entry.bcIntegration", "book.minecells.entry.prisoners_quarters"] {
            assert!(text.len() > 1);
            assert!(text.chars().any(|ch| ch.is_alphabetic()));
            assert!(!text.contains(':'));
            // 新判定擋得下來
            assert!(matches!(classify(c("lang", "k", text)), Eligibility::Keep(_)));
        }
    }

    #[test]
    fn normal_game_text_is_still_translated() {
        // 守住反方向：不能為了擋 key 就把正常文案也擋掉。
        for text in [
            "Iron Sword",
            "Are you sure you want to discard these changes?",
            "The forge burns hottest just before dawn.",
            "%1$s was slain by %2$s",
            "Recipe by %s",
            "Reward: {0} x {1}",
            "§9Mana: §f%s§7/§f%s",
            "Press $(k:key.use) to activate the $(item)Mana Pool$().",
            "{\"text\":\"Campaign\"}",
            "Welcome to the Lexica Botania, a guide to the mystical arts.",
            "Aquatic Breathing",
            "Crystal Blade",
            "Ancient Tome",
            "Warning: fire",
            "HP:100",
        ] {
            let got = classify(c("lang", "item.example.name", text));
            assert!(got.is_translatable(), "「{text}」應該要翻，實際：{got:?}");
        }
    }

    #[test]
    fn schema_fields_are_kept_by_key_not_by_value() {
        // 「challenge」這個值本身完全像正常英文字，只有從 key 才看得出是結構欄位。
        assert_eq!(
            classify(c("advancement", "display.frame", "challenge")),
            Eligibility::Keep(KeepReason::SchemaField)
        );
        assert_eq!(
            classify(c("book", "pages.0.type", "patchouli:text")),
            Eligibility::Keep(KeepReason::SchemaField)
        );
        assert_eq!(
            classify(c("ftbquests", "quest.translate", "ftbquests.chapter.campaign")),
            Eligibility::Keep(KeepReason::SchemaField)
        );
        assert_eq!(
            classify(c("origins", "powers.0.condition.type", "origins:submerged_in")),
            Eligibility::Keep(KeepReason::SchemaField)
        );
        // 同一個模組的顯示欄位仍然要翻
        assert!(classify(c("origins", "powers.0.name", "Aquatic Breathing")).is_translatable());
        assert!(classify(c("book", "pages.0.text", "Iron ingots are useful.")).is_translatable());
    }

    #[test]
    fn resource_ids_paths_urls_and_commands_are_kept() {
        assert_eq!(
            classify(c("lang", "block.x", "minecraft:stone")),
            Eligibility::Keep(KeepReason::ResourceLocation)
        );
        assert_eq!(
            classify(c("book", "pages.0.text", "zh_tw/root.txt")),
            Eligibility::Keep(KeepReason::ResourcePath)
        );
        assert_eq!(
            classify(c("lang", "modmenu.website", "https://example.invalid/docs")),
            Eligibility::Keep(KeepReason::Url)
        );
        assert_eq!(
            classify(c("lang", "cmd.help", "/botania-skyblock-spread")),
            Eligibility::Keep(KeepReason::Command)
        );
    }

    #[test]
    fn metadata_and_proper_nouns_are_kept() {
        assert_eq!(
            classify(c("lang", "htp_metadata_credits", "htp_metadata_credits")),
            Eligibility::Keep(KeepReason::MetadataField)
        );
        assert_eq!(
            classify(c("lang", "modmenu.nameTranslation.examplemod", "Botania")),
            Eligibility::Keep(KeepReason::ProperNoun)
        );
        assert_eq!(
            classify(c("lang", "modmenu.authorName", "Vazkii")),
            Eligibility::Keep(KeepReason::ProperNoun)
        );
    }

    #[test]
    fn case_sensitive_unit_symbols_survive() {
        // text.modern_industrialization.Eu 與 ...eu 是實際輸出裡的一對，值是單位符號。
        assert_eq!(
            classify(c("lang", "text.modern_industrialization.Eu", "EU")),
            Eligibility::Keep(KeepReason::UnitSymbol)
        );
        assert_eq!(
            classify(c("lang", "text.modern_industrialization.eu", "EU/t")),
            Eligibility::Keep(KeepReason::UnitSymbol)
        );
        // 但真的單字不能被當成單位吃掉
        assert!(classify(c("lang", "item.x", "Iron")).is_translatable());
        assert!(classify(c("lang", "item.x", "Gold")).is_translatable());
    }

    #[test]
    fn uncertain_input_goes_to_review_not_keep_and_never_to_ai() {
        let raw = classify(c("lang", "__raw__", "{\"a\": \"b\", }"));
        assert_eq!(raw, Eligibility::Review(ReviewReason::RawUnparsedBlob));
        let ctrl = classify(c("lang", "item.x", "Broken\u{0007}Bell"));
        assert_eq!(ctrl, Eligibility::Review(ReviewReason::ControlCharacter));

        for got in [raw, ctrl] {
            // 不確定的東西不送 AI、不寫資料庫，但要算進待處理讓人看得到
            assert!(!got.may_send_to_ai());
            assert!(!got.may_store_in_shared_data());
            assert!(got.counts_as_pending());
            assert!(got.player_reason().is_some());
        }
    }

    #[test]
    fn empty_values_are_kept_and_not_counted_as_gaps() {
        for text in ["", "   "] {
            let got = classify(c("lang", "item.x", text));
            assert_eq!(got, Eligibility::Keep(KeepReason::Empty));
            assert!(!got.counts_as_pending());
        }
    }

    #[test]
    fn inherited_regressions_from_the_old_value_only_gate() {
        // 這些判準原本釘在 deepseek::looks_untranslatable 的測試裡。
        // 那個函式已被本模組取代，但它守住的行為一條都不能掉。
        for text in [
            "https://example.com",
            "minecraft:stone_sword",
            "create:andesite_alloy",
            "123",
            "root.txt",
            "alligator.json",
        ] {
            assert!(
                !classify(c("lang", "item.x", text)).is_translatable(),
                "「{text}」不該被送去翻"
            );
        }
        assert!(classify(c("lang", "item.x", "Diamond Sword")).is_translatable());

        // 有冒號但確實要翻的句子，不能被 id 判斷誤殺
        for text in ["Warning: fire", "HP:100", "Tier: Advanced"] {
            assert!(
                classify(c("lang", "item.x", text)).is_translatable(),
                "「{text}」是給玩家看的，要翻"
            );
        }
    }

    #[test]
    fn report_summary_groups_by_reason_and_excludes_translatable_items() {
        let items: Vec<(String, String)> = vec![
            ("a.b.c".into(), "botania.entry.one".into()),   // 語言代號
            ("d.e.f".into(), "botania.entry.two".into()),   // 語言代號
            ("g".into(), "minecraft:stone".into()),          // 資源識別碼
            ("htp_metadata_version".into(), "1.0".into()),   // 資料欄位
            ("item.x".into(), "Iron Sword".into()),          // 這個要翻，不該進統計
            ("item.y".into(), "Broken\u{0007}Bell".into()),  // 需人工確認
        ];
        let summary = summarize_keep_reasons(&items, "lang");

        let total: usize = summary.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 5, "只有 Iron Sword 該被排除，實際：{summary:?}");

        // 最多的那類排最前面，使用者先看到主因
        assert_eq!(summary[0].1, 2);
        assert!(summary[0].0.contains("語言代號"));

        // 需人工確認也要出現，且與「刻意保留」用不同說法
        assert!(
            summary.iter().any(|(r, _)| r.contains("人工確認")),
            "需人工確認的項目也要列進報告：{summary:?}"
        );
        // 報告文字全部是白話，不得出現內部代號
        for (reason, _) in &summary {
            assert!(!reason.contains("Reason") && !reason.contains("Keep"), "{reason}");
        }
    }

    #[test]
    fn every_keep_and_review_reason_has_player_facing_text() {
        for r in [
            KeepReason::Empty,
            KeepReason::LanguageKeyAsValue,
            KeepReason::MetadataField,
            KeepReason::ResourceLocation,
            KeepReason::ResourcePath,
            KeepReason::Url,
            KeepReason::Command,
            KeepReason::SchemaField,
            KeepReason::UnitSymbol,
            KeepReason::ProperNoun,
            KeepReason::NoLetters,
        ] {
            let text = r.player_reason();
            assert!(!text.is_empty(), "{r:?} 缺少玩家可讀原因");
            assert!(!text.contains("Reason"), "{r:?} 的原因不該是內部代號");
        }
        for r in [ReviewReason::ControlCharacter, ReviewReason::RawUnparsedBlob] {
            assert!(!r.player_reason().is_empty());
        }
    }
}

#[cfg(test)]
#[path = "eligibility_b2_tests.rs"]
mod b2_tests;
