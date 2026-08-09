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

/// 單一批次最多附幾條術語提示（控制 prompt 長度）。
const MAX_HINTS_PER_BATCH: usize = 60;

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

#[derive(Debug, Clone, Default)]
pub struct Glossary {
    /// 小寫英文 → 台灣譯名
    exact: HashMap<String, String>,
    /// 供提示 AI 用的（原始大小寫英文, 譯名），已依長度由長到短排序
    terms: Vec<(String, String)>,
    /// 使用者自訂條目數（回報用）
    pub user_entries: usize,
}

impl Glossary {
    /// 整條字串剛好是已知術語 → 直接給官方譯名，不用送 AI。
    pub fn exact(&self, english: &str) -> Option<&str> {
        self.exact
            .get(english.trim().to_ascii_lowercase().as_str())
            .map(|s| s.as_str())
    }

    /// 挑出這批文字裡出現到的術語，作為 prompt 的用詞約束。
    pub fn hints_for(&self, texts: &[String]) -> Vec<(String, String)> {
        let haystack: String = texts.join("\n").to_ascii_lowercase();
        let mut out = Vec::new();
        for (en, zh) in &self.terms {
            if out.len() >= MAX_HINTS_PER_BATCH {
                break;
            }
            if contains_word(&haystack, &en.to_ascii_lowercase()) {
                out.push((en.clone(), zh.clone()));
            }
        }
        out
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

/// 使用者自訂術語表路徑：`%APPDATA%\modpack-i18n-tool\glossary.json`
pub fn user_glossary_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("modpack-i18n-tool")
        .join("glossary.json")
}

/// 官方繁中術語表（1945 條，Minecraft 香草物品／方塊／生物…）。
/// 資料來自 Koudesuk/Modpack_Translator（MIT），見專案 NOTICE。
const BUNDLED_GLOSSARY: &str = include_str!("../../assets/minecraft_glossary_zh_tw.json");

/// 解析一次就快取——1945 條，避免每個 AI 階段重解析。
fn bundled() -> &'static HashMap<String, String> {
    static CACHE: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| serde_json::from_str(BUNDLED_GLOSSARY).unwrap_or_default())
}

/// 載入術語表。優先序：使用者檔 > 內建精選 > 官方大表。
///
/// `extra` 供結果目錄內的專案級術語表（`翻譯結果/自訂詞典.json`）使用。
pub fn load(extra: Option<&Path>) -> Glossary {
    let mut exact: HashMap<String, String> = HashMap::new();
    let mut display: HashMap<String, String> = HashMap::new();

    // 第 1 層：官方大表（最底層，可被覆寫）
    for (en, zh) in bundled() {
        let key = en.to_ascii_lowercase();
        exact.insert(key.clone(), zh.clone());
        display.insert(key, en.clone());
    }

    // 第 2 層：內建精選（含苦力怕等台灣慣用譯名，覆寫官方大表）
    for (en, zh) in BUILTIN {
        exact.insert(en.to_ascii_lowercase(), (*zh).to_string());
        display.insert(en.to_ascii_lowercase(), (*en).to_string());
    }

    let mut user_entries = 0usize;
    for path in [Some(user_glossary_path()), extra.map(|p| p.to_path_buf())]
        .into_iter()
        .flatten()
    {
        for (en, zh) in read_json_map(&path) {
            let key = en.trim().to_ascii_lowercase();
            if key.is_empty() || zh.trim().is_empty() {
                continue;
            }
            exact.insert(key.clone(), zh.trim().to_string());
            display.insert(key, en.trim().to_string());
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

    Glossary {
        exact,
        terms,
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
    fn hints_only_include_terms_present_in_batch() {
        let g = load(None);
        let batch = vec!["A Creeper explodes".to_string(), "Sharpness V".to_string()];
        let hints = g.hints_for(&batch);
        let ens: Vec<&str> = hints.iter().map(|(e, _)| e.as_str()).collect();
        assert!(ens.contains(&"Creeper"));
        assert!(ens.contains(&"Sharpness"));
        assert!(!ens.contains(&"Axolotl"));
    }

    #[test]
    fn hints_respect_word_boundaries() {
        let g = load(None);
        // "Powered" 不該命中術語 "Power"
        let hints = g.hints_for(&["Powered Rail".to_string()]);
        assert!(!hints.iter().any(|(e, _)| e == "Power"));
    }

    #[test]
    fn hints_are_capped() {
        let g = load(None);
        let all: Vec<String> = BUILTIN.iter().map(|(e, _)| (*e).to_string()).collect();
        assert!(g.hints_for(&all).len() <= MAX_HINTS_PER_BATCH);
    }

    #[test]
    fn longer_terms_are_offered_first() {
        let g = load(None);
        let hints = g.hints_for(&["A Cave Spider bit me".to_string()]);
        let first = hints.first().map(|(e, _)| e.clone()).unwrap_or_default();
        assert_eq!(first, "Cave Spider");
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
    fn bundled_official_glossary_parses_and_is_large() {
        // 內建的 include_str! JSON 必須解析成功且量夠（否則等於沒帶到）
        assert!(bundled().len() > 1500, "官方術語表載入異常：{}", bundled().len());
    }

    #[test]
    fn bundled_glossary_is_available_via_lookup() {
        let g = load(None);
        // 官方大表裡有、但我的精選沒有的條目
        assert_eq!(g.exact("Acacia Boat"), Some("相思木船"));
    }

    #[test]
    fn curated_names_override_official_table() {
        let g = load(None);
        // Creeper 在精選層固定為台灣慣用「苦力怕」，不被官方大表蓋掉
        assert_eq!(g.exact("Creeper"), Some("苦力怕"));
    }

    #[test]
    fn phrase_dict_contains_builtin_suffix_rules() {
        let d = load_phrase_dict(None);
        assert_eq!(d.get("of Wealth").map(|s| s.as_str()), Some("財富"));
    }
}
