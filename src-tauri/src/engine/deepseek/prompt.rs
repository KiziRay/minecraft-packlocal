//! 提示詞組裝、批次規劃與縮批。
//!
//! T1：自原 deepseek.rs 原樣搬出，邏輯、常數、字串不變。

use super::*;

pub(super) fn build_system_prompt(gloss: &Glossary, texts: &[String], local: bool) -> String {
    let term_cap = if local {
        LOCAL_PROMPT_GLOSSARY_TERMS
    } else {
        MAX_PROMPT_GLOSSARY_TERMS
    };
    let fixed_terms = gloss
        .prompt_cache_terms(texts)
        .into_iter()
        .take(term_cap)
        .collect::<Vec<_>>();
    // 固定前綴必須跨批 byte-identical，且夠長以觸發上游 Context Caching（≥64 token）。
    // 動態內容只允許「本回合固定譯名」區塊；勿插入時間戳／批號／句數。
    let mut prompt = String::with_capacity(8192);
    if local {
        // 本地小模型走精簡版：規則越多、指令遵循越差，而且 8192 的視窗是
        // prompt 與輸出共用的，system 佔掉的每一個 token 都從可翻譯的內容裡扣。
        // 雲端那份有 10 條規則加上百餘個台灣用語範例，對小模型是反效果。
        prompt.push_str(LOCAL_SYSTEM_PREFIX);
    } else {
        prompt.push_str(STABLE_SYSTEM_PREFIX);
    }
    prompt.push_str("固定譯名（英文=中文；原文出現時優先使用）：\n");
    if fixed_terms.is_empty() {
        if local {
            prompt.push_str("(這批沒有固定術語。)\n");
        } else {
            // 維持穩定長度：即使用不到術語，也輸出固定占位，避免前綴過短無法進 cache
            prompt.push_str(STABLE_EMPTY_TERMS_BLOCK);
        }
    } else {
        for (en, zh) in &fixed_terms {
            prompt.push_str("- ");
            prompt.push_str(en);
            prompt.push('=');
            prompt.push_str(zh);
            prompt.push('\n');
        }
    }
    prompt.push_str(STABLE_SYSTEM_SUFFIX);
    prompt
}

/// 本地小模型的 prompt 裡最多放幾條固定術語。
///
/// 雲端可以放 150 條（有 context caching，成本幾乎不變）；本地的 8192 視窗是
/// prompt 與輸出**共用**的，術語表塞太多就等於把可翻譯的空間吃掉。
pub(super) const LOCAL_PROMPT_GLOSSARY_TERMS: usize = 40;

/// 本地小模型專用的精簡 system prompt。
///
/// 設計取捨：只留**違反就會壞掉**的規則（佔位符、只輸出 JSON、不要旁白），
/// 其餘品質期待（語氣、簡潔度、術語一致）改由譯後把關處理——
/// 小模型記不住十條規則，但譯後檢查一定會執行。
///
/// 特別加了「不要重複同一個詞」與「不要加解釋」兩條，因為那正是
/// `local_quality.rs` 抓到最多的兩種退化輸出，能在源頭少發生一次是一次。
pub(super) const LOCAL_SYSTEM_PREFIX: &str = "\
你是 Minecraft 模組的台灣繁體中文譯者。把每筆 t 譯成自然的台灣繁體中文。\n\
1. {0} {name} %s %1$s <item:...> $(br) 這類 token 原封不動保留，不可增刪或改字。\n\
2. 數字與單位照抄，不可改動。\n\
3. 只輸出譯文本身：不要加解釋、不要寫「以下是翻譯」、不要重複同一個詞。\n\
4. 純代號、路徑、網址照原樣輸出。\n\
5. 譯文長度應與原文相當，不要擴寫。\n\
";

/// 跨批不變的 system 前半（規則＋台灣用語指引）。長度刻意超過 Context Caching 最小單位。
pub(super) const STABLE_SYSTEM_PREFIX: &str = "\
你是 Minecraft 模組的繁體中文（台灣）在地化譯者。\
輸出必須符合台灣玩家習慣用詞，不要使用中國大陸簡體或陸用詞。\
規則：\n\
1. 文中的 {0} {1} {name} %s %1$s <item:...> #tag $(br) 等結構 token 必須原封不動保留；可以依中文語序移動，但不可增刪、改字或改成全形。\n\
2. 保留原文開頭與結尾的空白。\n\
3. 若輸入物件有 c，代表語境；物品名、方塊名、生物名、附魔名、狀態效果名、飾品名、按鍵綁定名稱要像名稱，簡短不成句。\n\
4. 必須整句翻成自然的台灣繁體中文；禁止只換部分英文詞、禁止中英拼接、禁止刪減資訊。\n\
5. 已經是正確繁中的句子照原樣輸出；純代號、路徑、網址、resource id 照原樣輸出。\n\
6. 完整保留語意、語氣、段落與上下文；不要省略資訊。\n\
7. 介面按鈕與提示用語簡潔清楚；任務與書本敘事可較完整，但仍避免冗長翻譯腔。\n\
8. 數字、單位、座標、指令參數若屬機械意義則保留原文符號與格式。\n\
9. 不要解釋規則，不要輸出除譯文 JSON 以外的說明。\n\
10. 同一英文術語在整包中保持譯名一致；若下方固定譯名有列出，必須優先採用。\n\
台灣用語參考（僅在語意相符時使用，勿生硬套用）：模組、整合包、伺服器、單人、創造模式、生存模式、觀察者模式、終界、地獄、主世界、紅石、活塞、漏斗、箱子、工作台、熔爐、附魔台、鐵砧、經驗值、生命值、飽食度、盔甲、工具、武器、弓、弩、盾牌、鞘翅、傳送門、生怪磚、指令方塊、資料包、資源包、光影、幀數、延遲、區塊、生物群系、村民、掠奪者、守護者、烈焰人、終界使者、苦力怕、殭屍、骷髏、蜘蛛、史萊姆、悅靈、狼、貓、馬、驢、騾、豬、牛、羊、雞。\n\
";

pub(super) const STABLE_EMPTY_TERMS_BLOCK: &str = "\
(本次無額外固定術語；請仍遵守上方台灣用語與佔位符規則。)\n\
(CachePrefixPad) Minecraft modpack Traditional Chinese Taiwan localization stable system prefix for prompt caching across translation batches. Keep this English padding unchanged so the shared prefix stays long enough for disk context cache hits on subsequent identical system messages.\n\
";

pub(super) const STABLE_SYSTEM_SUFFIX: &str =
    "輸出格式：只輸出一個 JSON 物件 {\"r\":[{\"i\":輸入的 i,\"t\":\"譯文\"}]}，不得附帶任何其他文字。";


pub(super) fn build_user_payload(items: &[MaskedItem]) -> String {
    let rows: Vec<Value> = items
        .iter()
        .map(|item| match item.context {
            Some(context) => json!({ "i": item.uid, "t": item.masked, "c": context }),
            None => json!({ "i": item.uid, "t": item.masked }),
        })
        .collect();
    let data = serde_json::to_string(&json!({ "r": rows })).unwrap_or_else(|_| "{\"r\":[]}".into());
    // 官方 JSON Output：system 或 user 必須明確要求 JSON，否則可能空白到 length
    format!(
        "將每筆 t 譯成台灣繁體中文。只回傳一個 JSON 物件，格式為 {{\"r\":[{{\"i\":數字,\"t\":\"譯文\"}}]}}，不要其他文字。\n{data}"
    )
}

/// 本地模型的輸出上限。
///
/// llama-server 的 `-c` 是**整個** KV 視窗（prompt ＋ 輸出共用）。舊版上下文寫死 4096，
/// 這裡卻可能要求 8192 個輸出 token——比整個視窗還大，長文本必定截斷或直接失敗，
/// 使用者看到的是「已安裝但翻譯失敗」。現在上下文最低 8192（見 local_llm/server.rs），
/// 輸出夾在 2048＝視窗的四分之一，剩下四分之三留給 system prompt、術語表與待譯內容。
pub(crate) const LOCAL_LLM_MAX_COMPLETION_TOKENS: usize = 2048;
/// 本地模型一批最多幾條（Name／Ui）。本地伺服器的上下文依這個數字回推（local_llm/sizing.rs）。
pub(crate) const LOCAL_MAX_BATCH_ITEMS: usize = 12;

pub(super) fn clamp_completion_tokens_for(input_chars: usize, item_count: usize, local: bool) -> usize {
    let from_chars = input_chars.saturating_mul(8).saturating_div(10);
    let from_items = item_count
        .saturating_mul(TOKENS_PER_ITEM)
        .max(MIN_COMPLETION_TOKENS);
    let ceiling = if local {
        LOCAL_LLM_MAX_COMPLETION_TOKENS
    } else {
        MAX_COMPLETION_TOKENS
    };
    from_chars
        .max(from_items)
        .clamp(MIN_COMPLETION_TOKENS.min(ceiling), ceiling)
}

pub(super) fn classify_track(source: &str, context: Option<&str>) -> BatchTrackKind {
    let len = source.chars().count();
    if len > SOLO_MIN_CHARS {
        return BatchTrackKind::Solo;
    }
    if len > STORY_MIN_CHARS || is_story_like(context, source) {
        return BatchTrackKind::Story;
    }
    if is_nameish_context(context) && len <= NAME_MAX_CHARS {
        return BatchTrackKind::Name;
    }
    if len <= UI_MAX_CHARS {
        BatchTrackKind::Ui
    } else {
        BatchTrackKind::Story
    }
}

pub(super) fn is_nameish_context(context: Option<&str>) -> bool {
    matches!(
        context,
        Some(
            "物品名"
                | "方塊名"
                | "液體名"
                | "生物名"
                | "生態域名"
                | "狀態效果名"
                | "附魔名"
                | "飾品名"
                | "按鍵綁定名稱"
        )
    )
}

pub(super) fn is_story_like(context: Option<&str>, source: &str) -> bool {
    matches!(context, Some("任務文字" | "進度名稱或說明"))
        || (matches!(context, Some("提示說明" | "設定項" | "訊息"))
            && (source.contains('\n') || source.chars().count() > 120))
}

pub(super) fn plan_track_batches_for(
    items: &[MaskedItem],
    strict_single: bool,
    local: bool,
) -> Vec<BatchPlan> {
    if strict_single {
        return items
            .iter()
            .cloned()
            .map(|item| BatchPlan {
                track: classify_track(&item.source, item.context),
                items: vec![item],
            })
            .collect();
    }

    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| {
        let track_a = classify_track(&a.source, a.context);
        let track_b = classify_track(&b.source, b.context);
        track_a
            .cmp(&track_b)
            .then_with(|| a.source.chars().count().cmp(&b.source.chars().count()))
            .then_with(|| a.source.to_ascii_lowercase().cmp(&b.source.to_ascii_lowercase()))
            .then_with(|| a.uid.cmp(&b.uid))
    });

    let mut plans = Vec::new();
    let mut index = 0usize;
    while index < sorted.len() {
        let track = classify_track(&sorted[index].source, sorted[index].context);
        let batch_size = track.batch_size_for(false, local);
        let mut bucket = Vec::new();
        while index < sorted.len()
            && classify_track(&sorted[index].source, sorted[index].context) == track
            && bucket.len() < batch_size
        {
            bucket.push(sorted[index].clone());
            index += 1;
        }
        plans.push(BatchPlan { track, items: bucket });
    }
    plans
}

/// B4：沒回應的句子「同一輪縮小批次重送」——把每批再切成 `divisor` 份。
pub(super) fn shrink_plans(plans: Vec<BatchPlan>, divisor: usize) -> Vec<BatchPlan> {
    if divisor <= 1 {
        return plans;
    }
    let mut out = Vec::new();
    for plan in plans {
        let size = plan.items.len().div_ceil(divisor).max(1);
        for chunk in plan.items.chunks(size) {
            out.push(BatchPlan {
                track: plan.track,
                items: chunk.to_vec(),
            });
        }
    }
    out
}

/// 進度取「批次進度」與「句數進度」較高者，避免久卡同一個數字像當機。
pub(super) fn by_batch_or_strings(
    done_batches: usize,
    total_batches: usize,
    got: usize,
    total_unique: usize,
    base: u8,
    span: u8,
) -> u8 {
    let ratio = |a: usize, b: usize| -> u32 {
        if b == 0 {
            0
        } else {
            (a as u32 * span as u32 / b as u32).min(span as u32)
        }
    };
    let p = base as u32 + ratio(done_batches, total_batches).max(ratio(got, total_unique));
    p.min((base as u32 + span as u32).min(100)) as u8
}
