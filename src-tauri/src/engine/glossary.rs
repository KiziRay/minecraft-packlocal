//! 台灣繁中術語表。
//!
//! 兩個用途：
//! 1. **免費直翻**：整條字串就是一個已知術語時（`Creeper`、`Diamond Sword`）直接查表，
//!    不必送 AI——省錢、零延遲，而且用的是 Minecraft 官方台灣譯名。
//! 2. **提示 AI**：把該批次出現到的術語附在 prompt 裡，避免同一個 mob 在不同模組
//!    被翻成「爬行者／苦力怕／creeper」三種樣子。
//!
//! 使用者可用 `glossary.json` 覆寫（見 [`user_glossary_path`]），格式：
//! ```json
//! { "Creeper": "苦力怕", "Ancient Debris": "遠古遺骸" }
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::glossary_modpack;
use super::placeholder;

/// prompt-cache 用的固定術語區塊上限。
const MAX_PROMPT_CACHE_TERMS: usize = 150;
/// 單條術語一致性最多檢查幾個匹配，避免長文掃描過重。
const MAX_TERM_ENFORCE_MATCHES: usize = 24;

/// Minecraft 官方繁體中文（台灣）譯名。
/// 只收錄有把握的條目；不確定的寧可留給 AI，也不要硬塞錯譯。
const BUILTIN: &[(&str, &str)] = &[
    // ── 生物 ──────────────────────────────────────────────
    ("Creeper", "苦力怕"),
    ("Enderman", "終界使者"),
    ("Endermite", "終界蟎"),
    ("Ender Dragon", "終界龍"),
    ("Zombie", "殭屍"),
    ("Zombie Villager", "殭屍村民"),
    ("Skeleton", "骷髏"),
    ("Wither Skeleton", "凋零骷髏"),
    ("Stray", "流髑"),
    ("Husk", "屍殼"),
    ("Drowned", "溺屍"),
    ("Spider", "蜘蛛"),
    ("Cave Spider", "洞穴蜘蛛"),
    ("Slime", "史萊姆"),
    ("Magma Cube", "岩漿立方怪"),
    ("Ghast", "地獄幽靈"),
    ("Blaze", "烈焰使者"),
    ("Wither", "凋零"),
    ("Silverfish", "蠹魚"),
    ("Guardian", "深海守衛"),
    ("Elder Guardian", "遠古深海守衛"),
    ("Shulker", "界伏蚌"),
    ("Phantom", "夜魅"),
    ("Vex", "惱鬼"),
    ("Witch", "女巫"),
    ("Vindicator", "衛道士"),
    ("Evoker", "喚魔者"),
    ("Pillager", "掠奪者"),
    ("Ravager", "劫毀獸"),
    ("Illusioner", "幻術師"),
    ("Villager", "村民"),
    ("Wandering Trader", "流浪商人"),
    ("Iron Golem", "鐵傀儡"),
    ("Snow Golem", "雪傀儡"),
    ("Piglin", "豬布林"),
    ("Piglin Brute", "豬布林蠻兵"),
    ("Hoglin", "豬布獸"),
    ("Zoglin", "殭屍豬布獸"),
    ("Strider", "熾足獸"),
    ("Warden", "伏守者"),
    ("Allay", "悅靈"),
    ("Axolotl", "六角恐龍"),
    ("Sniffer", "嗅探獸"),
    ("Armadillo", "犰狳"),
    ("Breeze", "微風使者"),
    ("Bogged", "沼骸"),
    ("Glow Squid", "螢光魷魚"),
    ("Squid", "魷魚"),
    ("Dolphin", "海豚"),
    ("Turtle", "海龜"),
    ("Panda", "熊貓"),
    ("Llama", "羊駝"),
    ("Parrot", "鸚鵡"),
    ("Ocelot", "山貓"),
    ("Polar Bear", "北極熊"),
    ("Mooshroom", "哞菇"),
    ("Pufferfish", "河豚"),
    ("Tropical Fish", "熱帶魚"),
    ("Salmon", "鮭魚"),
    ("Tadpole", "蝌蚪"),
    // ── 維度與世界 ────────────────────────────────────────
    ("Overworld", "主世界"),
    ("The Nether", "地獄"),
    ("Nether", "地獄"),
    ("The End", "終界"),
    ("Biome", "生態域"),
    ("Dimension", "維度"),
    ("Spawn Point", "重生點"),
    ("Bedrock", "基岩"),
    // ── 方塊與物品 ────────────────────────────────────────
    ("Cobblestone", "鵝卵石"),
    ("Deepslate", "深板岩"),
    ("Obsidian", "黑曜石"),
    ("Redstone", "紅石"),
    ("Redstone Dust", "紅石粉"),
    ("Netherite", "獄髓"),
    ("Ancient Debris", "遠古遺骸"),
    ("Diamond", "鑽石"),
    ("Emerald", "綠寶石"),
    ("Lapis Lazuli", "青金石"),
    ("Amethyst", "紫水晶"),
    ("Copper", "銅"),
    ("Tuff", "凝灰岩"),
    ("Calcite", "方解石"),
    ("Sculk", "幽匿"),
    ("Shroomlight", "菌光體"),
    ("Crafting Table", "工作台"),
    ("Furnace", "熔爐"),
    ("Blast Furnace", "高爐"),
    ("Smoker", "煙燻爐"),
    ("Anvil", "鐵砧"),
    ("Enchanting Table", "附魔台"),
    ("Brewing Stand", "釀造台"),
    ("Smithing Table", "鍛造台"),
    ("Fletching Table", "製箭台"),
    ("Cartography Table", "製圖台"),
    ("Stonecutter", "石切機"),
    ("Grindstone", "砂輪"),
    ("Loom", "織布機"),
    ("Composter", "堆肥桶"),
    ("Lectern", "講台"),
    ("Barrel", "木桶"),
    ("Chest", "儲物箱"),
    ("Ender Chest", "終界箱"),
    ("Shulker Box", "界伏蚌盒"),
    ("Hopper", "漏斗"),
    ("Dispenser", "發射器"),
    ("Dropper", "投擲器"),
    ("Observer", "偵測器"),
    ("Piston", "活塞"),
    ("Sticky Piston", "黏性活塞"),
    ("Comparator", "紅石比較器"),
    ("Repeater", "紅石中繼器"),
    ("Beacon", "烽火台"),
    ("Conduit", "導管"),
    ("Lodestone", "磁石"),
    ("Respawn Anchor", "重生錨"),
    ("Scaffolding", "鷹架"),
    ("Campfire", "營火"),
    ("Elytra", "鞘翅"),
    ("Trident", "三叉戟"),
    ("Crossbow", "弩"),
    ("Totem of Undying", "不死圖騰"),
    ("Mace", "錘矛"),
    // ── 附魔 ──────────────────────────────────────────────
    ("Sharpness", "鋒利"),
    ("Smite", "不死剋星"),
    ("Bane of Arthropods", "節肢剋星"),
    ("Knockback", "擊退"),
    ("Fire Aspect", "燃燒"),
    ("Looting", "掠奪"),
    ("Sweeping Edge", "橫掃之刃"),
    ("Efficiency", "效率"),
    ("Silk Touch", "絲綢之觸"),
    ("Unbreaking", "耐久"),
    ("Fortune", "幸運"),
    ("Mending", "修補"),
    ("Power", "力量"),
    ("Punch", "衝擊"),
    ("Flame", "火矢"),
    ("Infinity", "無限"),
    ("Protection", "保護"),
    ("Fire Protection", "火焰保護"),
    ("Blast Protection", "爆炸保護"),
    ("Projectile Protection", "彈射物保護"),
    ("Feather Falling", "輕盈"),
    ("Respiration", "水下呼吸"),
    ("Aqua Affinity", "親水性"),
    ("Thorns", "尖刺"),
    ("Depth Strider", "深海漫遊"),
    ("Frost Walker", "冰霜行者"),
    ("Soul Speed", "靈魂疾行"),
    ("Swift Sneak", "迅捷潛行"),
    ("Curse of Binding", "綁定詛咒"),
    ("Curse of Vanishing", "消失詛咒"),
    ("Luck of the Sea", "海之眷顧"),
    ("Lure", "餌釣"),
    ("Loyalty", "忠誠"),
    ("Impaling", "穿刺"),
    ("Riptide", "波濤洶湧"),
    ("Channeling", "引雷"),
    ("Multishot", "多重箭矢"),
    ("Quick Charge", "快速裝填"),
    ("Piercing", "貫穿"),
    ("Density", "緻密"),
    ("Breach", "破甲"),
    ("Wind Burst", "風爆"),
    // ── 狀態效果 ──────────────────────────────────────────
    ("Regeneration", "回復"),
    ("Haste", "挖掘加速"),
    ("Mining Fatigue", "挖掘疲勞"),
    ("Instant Health", "立即治療"),
    ("Instant Damage", "立即傷害"),
    ("Jump Boost", "跳躍提升"),
    ("Nausea", "反胃"),
    ("Resistance", "抗性提升"),
    ("Fire Resistance", "抗火"),
    ("Water Breathing", "水下呼吸"),
    ("Invisibility", "隱形"),
    ("Night Vision", "夜視"),
    ("Weakness", "虛弱"),
    ("Poison", "中毒"),
    ("Health Boost", "生命提升"),
    ("Absorption", "傷害吸收"),
    ("Saturation", "飽食"),
    ("Glowing", "發光"),
    ("Levitation", "漂浮"),
    ("Slow Falling", "緩降"),
    ("Conduit Power", "導管能量"),
    ("Dolphin's Grace", "海豚的恩惠"),
    ("Bad Omen", "不祥之兆"),
    ("Hero of the Village", "村莊英雄"),
    ("Darkness", "黑暗"),
    // ── 介面與屬性 ────────────────────────────────────────
    ("Durability", "耐久度"),
    ("Attack Damage", "攻擊傷害"),
    ("Attack Speed", "攻擊速度"),
    ("Armor Toughness", "盔甲韌性"),
    ("Knockback Resistance", "擊退抗性"),
    ("Movement Speed", "移動速度"),
    ("Inventory", "物品欄"),
    ("Advancement", "進度"),
    ("Experience", "經驗"),
    ("Enchantment", "附魔"),
    ("Cooldown", "冷卻時間"),
    ("Recipe", "合成表"),
    ("Crafting", "合成"),
    ("Smelting", "熔煉"),
    ("Fuel", "燃料"),
    ("Tooltip", "提示文字"),
    ("Hardcore", "極限模式"),
    ("Creative", "創造模式"),
    ("Survival", "生存模式"),
    ("Spectator", "旁觀者模式"),
];

/// 譯後修飾用的片語規則（沿用舊 `load_phrase_dict` 行為）。
const BUILTIN_PHRASES: &[(&str, &str)] = &[
    ("of Elemental Resistance", "元素抗性"),
    ("of Evasion", "閃避"),
    ("of Wealth", "財富"),
    ("of the Vampire", "吸血鬼"),
];

/// 術語分級：大表只進 prompt；精選／使用者／誤用表才譯後 enforce。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlossaryTier {
    /// T0／T1：可 `enforce_terms` 與 `exact` 直翻
    Enforce,
    /// T2：僅 prompt 提示，避免 Toast→吐司 這類誤傷
    PromptOnly,
}

#[derive(Debug, Clone, Default)]
pub struct Glossary {
    /// 小寫英文 → 台灣譯名（含 T2，供 exact／prompt）
    exact: HashMap<String, String>,
    /// 小寫英文 → 分級
    tiers: HashMap<String, GlossaryTier>,
    /// 供提示 AI 用（全部層級），已依長度由長到短排序
    terms: Vec<(String, String)>,
    /// 僅 Enforce 層，供 `enforce_terms`
    enforce_terms_list: Vec<(String, String)>,
    /// 使用者自訂條目數（回報用）
    pub user_entries: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TermConsistencyStats {
    pub enforced_strings: usize,
    pub kept_ai_strings: usize,
}

impl TermConsistencyStats {
    pub fn note(&self) -> Option<String> {
        if self.enforced_strings == 0 && self.kept_ai_strings == 0 {
            return None;
        }
        Some(format!(
            "術語一致性：補強 {} 條英文殘留；{} 條因格式安全保留 AI 原文",
            self.enforced_strings, self.kept_ai_strings
        ))
    }
}

impl Glossary {
    /// 整條字串剛好是已知術語 → 直接給官方譯名，不用送 AI。
    /// 僅 **Enforce** 層（T0／T1）；T2 大表不直翻，避免誤傷。
    pub fn exact(&self, english: &str) -> Option<&str> {
        let key = english.trim().to_ascii_lowercase();
        if self.tiers.get(&key).copied() != Some(GlossaryTier::Enforce) {
            return None;
        }
        self.exact.get(key.as_str()).map(|s| s.as_str())
    }

    /// `exact` + `placeholder::guard`：表內缺 `%s` 等毒譯文則拒絕。
    pub fn exact_safe(&self, english: &str) -> Option<String> {
        let zh = self.exact(english)?;
        let mut guard = placeholder::GuardStats::default();
        placeholder::guard(english, zh, &mut guard)
    }

    #[allow(dead_code)]
    pub fn tier_of(&self, english: &str) -> Option<GlossaryTier> {
        self.tiers
            .get(english.trim().to_ascii_lowercase().as_str())
            .copied()
    }

    /// 針對整次翻譯挑出高頻固定譯名，做成可快取的 system prompt 區塊。
    pub fn prompt_cache_terms(&self, texts: &[String]) -> Vec<(String, String)> {
        let lower_texts: Vec<String> = texts.iter().map(|text| text.to_ascii_lowercase()).collect();
        let mut scored: Vec<(usize, usize, String, String)> = Vec::new();
        for (en, zh) in &self.terms {
            if is_short_token(en) {
                continue;
            }
            let en_l = en.to_ascii_lowercase();
            let hits = lower_texts
                .iter()
                .filter(|text| contains_word(text, &en_l))
                .count();
            if hits > 0 {
                scored.push((hits, en.len(), en.clone(), zh.clone()));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
        scored
            .into_iter()
            .take(MAX_PROMPT_CACHE_TERMS)
            .map(|(_, _, en, zh)| (en, zh))
            .collect()
    }

    /// 譯後把仍殘留的固定英文術語補回統一譯名；若可能破壞格式則保留 AI 原文。
    pub fn enforce_terms(
        &self,
        source: &str,
        translated: &str,
        stats: &mut TermConsistencyStats,
    ) -> String {
        let matched = self.matched_terms(source, MAX_TERM_ENFORCE_MATCHES);
        if matched.is_empty() {
            return translated.to_string();
        }

        let (mut masked, tokens) = placeholder::mask(translated);
        let mut changed = false;
        for (en, zh) in matched {
            let (next, replaced) = replace_ascii_word_case_insensitive(&masked, &en, &zh);
            if replaced > 0 {
                masked = next;
                changed = true;
            }
        }
        if !changed {
            return translated.to_string();
        }

        let candidate = placeholder::unmask(&masked, &tokens);
        if placeholder::is_compatible(source, &candidate) {
            stats.enforced_strings += 1;
            candidate
        } else {
            stats.kept_ai_strings += 1;
            translated.to_string()
        }
    }

    fn matched_terms(&self, text: &str, limit: usize) -> Vec<(String, String)> {
        let lower = text.to_ascii_lowercase();
        self.enforce_terms_list
            .iter()
            .filter(|(en, _)| !is_short_token(en))
            .filter(|(en, _)| contains_word(&lower, &en.to_ascii_lowercase()))
            .take(limit)
            .cloned()
            .collect()
    }
}

/// 英文詞需落在字界上，避免 `Power` 命中 `Powered Rail` 之類的誤報。
fn contains_word(haystack_lower: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return false;
    }
    let bytes = haystack_lower.as_bytes();
    let n = needle_lower.len();
    let mut from = 0usize;
    while let Some(rel) = haystack_lower[from..].find(needle_lower) {
        let start = from + rel;
        let end = start + n;
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_word_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
        if from >= haystack_lower.len() {
            break;
        }
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_short_token(term: &str) -> bool {
    !term.contains(' ') && term.chars().count() < 5
}

fn replace_ascii_word_case_insensitive(text: &str, needle: &str, replacement: &str) -> (String, usize) {
    let haystack_lower = text.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    if needle_lower.is_empty() {
        return (text.to_string(), 0);
    }
    let bytes = haystack_lower.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    let mut from = 0usize;
    let mut count = 0usize;
    while let Some(rel) = haystack_lower[from..].find(&needle_lower) {
        let start = from + rel;
        let end = start + needle_lower.len();
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_word_byte(bytes[end]);
        if before_ok && after_ok {
            out.push_str(&text[last..start]);
            out.push_str(replacement);
            last = end;
            count += 1;
        }
        from = start + 1;
        if from >= haystack_lower.len() {
            break;
        }
    }
    if count == 0 {
        return (text.to_string(), 0);
    }
    out.push_str(&text[last..]);
    (out, count)
}

/// 使用者自訂術語表路徑：`%APPDATA%\modpack-i18n-tool\glossary.json`
pub fn user_glossary_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("modpack-i18n-tool")
        .join("glossary.json")
}

/// 官方繁中術語表（1945 條，Minecraft 香草物品／方塊／生物…）。
/// 資料來自 Koudesuk/Modpack_Translator（MIT），見專案 NOTICE。**T2 PromptOnly**。
const BUNDLED_GLOSSARY: &str = include_str!("../../assets/minecraft_glossary_zh_tw.json");

/// 官方用語易誤用小表（T0 Enforce）。見 `docs/支援範圍與免責聲明.md`。
const VANILLA_MISUSE: &str = include_str!("../../assets/vanilla_misuse_zh_tw.json");

/// 解析一次就快取——1945 條，避免每個 AI 階段重解析。
fn bundled() -> &'static HashMap<String, String> {
    static CACHE: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| serde_json::from_str(BUNDLED_GLOSSARY).unwrap_or_default())
}

fn vanilla_misuse() -> &'static HashMap<String, String> {
    static CACHE: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        serde_json::from_str::<HashMap<String, serde_json::Value>>(VANILLA_MISUSE)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| {
                if k.starts_with('_') {
                    return None;
                }
                let zh = v.as_str()?.trim();
                if zh.is_empty() {
                    None
                } else {
                    Some((k, zh.to_string()))
                }
            })
            .collect()
    })
}

fn insert_term(
    exact: &mut HashMap<String, String>,
    display: &mut HashMap<String, String>,
    tiers: &mut HashMap<String, GlossaryTier>,
    en: &str,
    zh: &str,
    tier: GlossaryTier,
) {
    let key = en.trim().to_ascii_lowercase();
    if key.is_empty() || zh.trim().is_empty() {
        return;
    }
    exact.insert(key.clone(), zh.trim().to_string());
    display.insert(key.clone(), en.trim().to_string());
    tiers.insert(key, tier);
}

/// 載入術語表。
///
/// 層級（後者覆寫前者的譯名與分級）：
/// T2 官方大表 → T0 內建精選／模組包／誤用表 → T1 使用者／extra。
///
/// `extra` 供結果目錄內的專案級術語表（`翻譯結果/自訂詞典.json`）使用。
pub fn load(extra: Option<&Path>) -> Glossary {
    let mut exact: HashMap<String, String> = HashMap::new();
    let mut display: HashMap<String, String> = HashMap::new();
    let mut tiers: HashMap<String, GlossaryTier> = HashMap::new();

    // T2：官方大表（只 prompt，不 enforce／exact 直翻）
    for (en, zh) in bundled() {
        insert_term(
            &mut exact,
            &mut display,
            &mut tiers,
            en,
            zh,
            GlossaryTier::PromptOnly,
        );
    }

    // T0：內建精選
    for (en, zh) in BUILTIN {
        insert_term(
            &mut exact,
            &mut display,
            &mut tiers,
            en,
            zh,
            GlossaryTier::Enforce,
        );
    }

    // T0：模組包常見術語
    for (en, zh) in glossary_modpack::TERMS {
        insert_term(
            &mut exact,
            &mut display,
            &mut tiers,
            en,
            zh,
            GlossaryTier::Enforce,
        );
    }

    // T0：官方誤用補強
    for (en, zh) in vanilla_misuse() {
        insert_term(
            &mut exact,
            &mut display,
            &mut tiers,
            en,
            zh,
            GlossaryTier::Enforce,
        );
    }

    let mut user_entries = 0usize;
    for path in [Some(user_glossary_path()), extra.map(|p| p.to_path_buf())]
        .into_iter()
        .flatten()
    {
        for (en, zh) in read_json_map(&path) {
            let key = en.trim().to_ascii_lowercase();
            if key.is_empty() || zh.trim().is_empty() || key.starts_with('_') {
                continue;
            }
            insert_term(
                &mut exact,
                &mut display,
                &mut tiers,
                en.trim(),
                zh.trim(),
                GlossaryTier::Enforce,
            );
            user_entries += 1;
        }
    }

    let mut terms: Vec<(String, String)> = exact
        .iter()
        .map(|(k, zh)| {
            let en = display.get(k).cloned().unwrap_or_else(|| k.clone());
            (en, zh.clone())
        })
        .collect();
    // 長詞優先：先命中 "Cave Spider" 再輪到 "Spider"
    terms.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));

    let mut enforce_terms_list: Vec<(String, String)> = terms
        .iter()
        .filter(|(en, _)| {
            tiers.get(&en.to_ascii_lowercase()).copied() == Some(GlossaryTier::Enforce)
        })
        .cloned()
        .collect();
    enforce_terms_list.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));

    Glossary {
        exact,
        tiers,
        terms,
        enforce_terms_list,
        user_entries,
    }
}

fn read_json_map(path: &Path) -> HashMap<String, String> {
    if !path.is_file() {
        return HashMap::new();
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
        .unwrap_or_default()
}

/// 譯後片語修飾規則（內建 + 使用者 `phrases.json`）。
pub fn load_phrase_dict(extra: Option<&Path>) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = BUILTIN_PHRASES
        .iter()
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect();
    let user = user_glossary_path().with_file_name("phrases.json");
    m.extend(read_json_map(&user));
    if let Some(p) = extra {
        m.extend(read_json_map(p));
    }
    m
}

/// 首次使用時寫出一份帶說明的範本，讓玩家知道可以自己改詞。
pub fn ensure_user_glossary_template() -> Option<PathBuf> {
    let path = user_glossary_path();
    if path.is_file() {
        return Some(path);
    }
    let parent = path.parent()?;
    fs::create_dir_all(parent).ok()?;
    let sample = serde_json::json!({
        "_說明": "這裡填你想固定的譯名：英文原文 → 你要的中文。存檔後重跑翻譯即生效。",
        "_範例": "把下面兩行改成你要的，或直接新增",
        "Creeper": "苦力怕",
        "Ancient Debris": "遠古遺骸"
    });
    fs::write(&path, serde_json::to_string_pretty(&sample).ok()? + "\n").ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_lookup_is_case_insensitive() {
        let g = load(None);
        assert_eq!(g.exact("Creeper"), Some("苦力怕"));
        assert_eq!(g.exact("creeper"), Some("苦力怕"));
        assert_eq!(g.exact("  CREEPER  "), Some("苦力怕"));
    }

    #[test]
    fn unknown_terms_return_none() {
        let g = load(None);
        assert!(g.exact("Thermal Dynamo").is_none());
    }

    #[test]
    fn prompt_cache_terms_use_pack_frequency_order() {
        let g = load(None);
        let texts = vec![
            "Creeper Head".to_string(),
            "Creeper Banner Pattern".to_string(),
            "Use Enderman Pearl".to_string(),
        ];
        let terms = g.prompt_cache_terms(&texts);
        let first = terms.first().map(|(en, _)| en.as_str());
        assert_eq!(first, Some("Creeper"));
    }

    #[test]
    fn prompt_cache_terms_respect_word_boundaries() {
        let g = load(None);
        let terms = g.prompt_cache_terms(&["Powered Rail".to_string()]);
        assert!(!terms.iter().any(|(en, _)| en == "Power"));
    }

    #[test]
    fn builtin_has_no_duplicate_keys() {
        let mut seen = std::collections::HashSet::new();
        for (en, _) in BUILTIN {
            let k = en.to_ascii_lowercase();
            assert!(seen.insert(k), "術語表重複條目：{en}");
        }
    }

    #[test]
    fn modpack_terms_are_unique_and_loaded() {
        let mut seen = std::collections::HashSet::new();
        for (en, _) in glossary_modpack::TERMS {
            let k = en.to_ascii_lowercase();
            assert!(seen.insert(k), "模組包術語重複條目：{en}");
        }
        let g = load(None);
        assert_eq!(g.exact("Modpack"), Some("模組包"));
        assert_eq!(g.exact("Mechanical Press"), Some("機械壓床"));
        assert_eq!(g.exact("Quest Chapter"), Some("任務章節"));
        assert_eq!(g.exact("Storage Bus"), Some("儲存匯流排"));
    }

    #[test]
    fn bundled_official_glossary_parses_and_is_large() {
        // 內建的 include_str! JSON 必須解析成功且量夠（否則等於沒帶到）
        assert!(bundled().len() > 1500, "官方術語表載入異常：{}", bundled().len());
    }

    #[test]
    fn bundled_glossary_is_prompt_only_not_exact() {
        let g = load(None);
        // 官方大表裡有、精選沒有的條目：只進 prompt，不 exact 直翻（E8 類）
        assert_eq!(g.tier_of("Acacia Boat"), Some(GlossaryTier::PromptOnly));
        assert!(g.exact("Acacia Boat").is_none());
        assert!(g.exact("Apple").is_none()); // Apple 僅大表
        let prompt = g.prompt_cache_terms(&["Acacia Boat ready".to_string()]);
        assert!(prompt.iter().any(|(en, _)| en == "Acacia Boat"));
    }

    #[test]
    fn t2_toast_like_term_not_enforced_in_ui_string() {
        let g = load(None);
        // Bread 僅 T2；enforce 不可把 UI 字串裡的英文換成食物譯名
        assert_eq!(g.tier_of("Bread"), Some(GlossaryTier::PromptOnly));
        let mut stats = TermConsistencyStats::default();
        let src = "show_error_toast Bread";
        let ai = "show_error_toast Bread";
        let out = g.enforce_terms(src, ai, &mut stats);
        assert_eq!(out, ai);
        assert_eq!(stats.enforced_strings, 0);
    }

    #[test]
    fn exact_safe_rejects_poisoned_glossary_entry() {
        let mut g = load(None);
        // 模擬使用者寫入缺 %s 的毒譯文（T1 Enforce）
        g.exact
            .insert("hit %s please".to_string(), "打中目標".to_string());
        g.tiers
            .insert("hit %s please".to_string(), GlossaryTier::Enforce);
        assert!(g.exact_safe("Hit %s please").is_none());
        assert_eq!(g.exact_safe("Creeper"), Some("苦力怕".to_string()));
    }

    #[test]
    fn curated_names_override_official_table() {
        let g = load(None);
        // Creeper 在精選層固定為台灣慣用「苦力怕」，不被官方大表蓋掉
        assert_eq!(g.exact("Creeper"), Some("苦力怕"));
        assert_eq!(g.tier_of("Creeper"), Some(GlossaryTier::Enforce));
    }

    #[test]
    fn phrase_dict_contains_builtin_suffix_rules() {
        let d = load_phrase_dict(None);
        assert_eq!(d.get("of Wealth").map(|s| s.as_str()), Some("財富"));
    }

    #[test]
    fn term_enforcement_replaces_english_leftovers() {
        let g = load(None);
        let mut stats = TermConsistencyStats::default();
        let out = g.enforce_terms("A Creeper Head", "A Creeper 頭顱", &mut stats);
        assert_eq!(out, "A 苦力怕 頭顱");
        assert_eq!(stats.enforced_strings, 1);
    }

    #[test]
    fn term_consistency_does_not_break_placeholders() {
        let g = load(None);
        let mut stats = TermConsistencyStats::default();
        let src = "Use <item:minecraft:diamond_sword> on Creeper %s";
        let out = g.enforce_terms(src, "對 Creeper 使用 <item:minecraft:diamond_sword> %s", &mut stats);
        assert_eq!(out, "對 苦力怕 使用 <item:minecraft:diamond_sword> %s");
        assert!(placeholder::is_compatible(src, &out));
        assert_eq!(stats.kept_ai_strings, 0);
    }
}
