//! 格式佔位符保護。
//!
//! Minecraft 的語言字串常帶 `String.format` 佔位符（`%s`、`%d`、`%1$s`）。
//! 機器翻譯很容易吃掉、複製多份或改成全形，結果在遊戲裡是
//! `MissingFormatArgumentException`（紅字錯誤或直接崩潰），而不是「翻得不好」。
//!
//! 因此譯文一律先過本模組：
//! 1. 嘗試**修復**常見的可逆破壞（全形 `％`、佔位符中間被插空白、首尾空白被吃掉）
//! 2. 修不好就**退回原文**——寧可留英文，也不要送出會壞掉的字串
//!
//! 分類：
//! - **positional**（`%s`、`%d`）：順序有意義，序列必須完全一致
//! - **keyed**（`%1$s`、`{0}`、`{player}`、`%player%`、`$(br)`）：可重排，比對多重集合
//! - **soft**（`§a` 色碼、換行）：只回報不否決，缺了頂多是排版走樣

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

/// Java 格式佔位符。**旗標刻意不含空白**，否則 `50% chance` 會被誤判成 `% c`。
const RE_JAVA_SPEC: &str = r"%(?:(\d+)\$)?[-#+0,(]*\d*(?:\.\d+)?[bBhHsScCdoxXeEfgGaAtTn%]";
/// `{0}` / `{player}`
const RE_BRACE: &str = r"\{[A-Za-z_][A-Za-z0-9_]*\}|\{\d+\}";
/// `%player%`（內文 ≥3 字，避免吃到 `%s%`）
const RE_NAMED_PERCENT: &str = r"%[A-Za-z_][A-Za-z0-9_]{2,}%";
/// Patchouli 書本巨集 `$(br)`、`$(l:item)`、`$(/l)`
const RE_PATCHOULI: &str = r"\$\([^)]{0,64}\)";
/// Minecraft item/tag references used by books, scripts and recipe text.
const RE_ITEM_REFERENCE: &str =
    r"(?:<(?:(?:item|tag|fluid|block):[^<>\s]{1,160})>|#[A-Za-z0-9_.-]+:[A-Za-z0-9_./-]+)";
/// 色碼（含少數設定檔用的 `&a`）
const RE_COLOR: &str = r"§.|&[0-9a-fk-orA-FK-OR]";

fn re(src: &'static str) -> &'static Regex {
    static CACHE: OnceLock<std::sync::Mutex<HashMap<&'static str, &'static Regex>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = cache.lock().expect("placeholder regex cache poisoned");
    guard.entry(src).or_insert_with(|| {
        Box::leak(Box::new(
            Regex::new(src).expect("placeholder regex must compile"),
        ))
    })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Placeholders {
    /// 順序敏感：`%s`、`%d`、`%.2f`
    pub positional: Vec<String>,
    /// 順序不敏感：`%1$s`、`{0}`、`%player%`、`$(br)`
    pub keyed: Vec<String>,
    /// 只供回報：色碼
    pub soft: Vec<String>,
}

impl Placeholders {
    pub fn is_empty(&self) -> bool {
        self.positional.is_empty() && self.keyed.is_empty()
    }
}

/// 擷取一段文字裡的所有佔位符。
pub fn extract(s: &str) -> Placeholders {
    let mut out = Placeholders::default();

    for m in re(RE_JAVA_SPEC).captures_iter(s) {
        let whole = m.get(0).map(|x| x.as_str().to_string()).unwrap_or_default();
        if whole == "%%" {
            // 字面百分號：算 positional，數量要守恆
            out.positional.push(whole);
        } else if m.get(1).is_some() {
            // `%1$s` 帶索引 → 可重排
            out.keyed.push(whole);
        } else {
            out.positional.push(whole);
        }
    }
    for pat in [RE_BRACE, RE_NAMED_PERCENT, RE_PATCHOULI, RE_ITEM_REFERENCE] {
        for m in re(pat).find_iter(s) {
            out.keyed.push(m.as_str().to_string());
        }
    }
    for m in re(RE_COLOR).find_iter(s) {
        out.soft.push(m.as_str().to_string());
    }
    out.keyed.sort();
    out.soft.sort();
    out
}

/// 譯文是否保住了原文的佔位符（順序／數量）。
pub fn is_compatible(source: &str, translated: &str) -> bool {
    let a = extract(source);
    let b = extract(translated);
    a.positional == b.positional && a.keyed == b.keyed
}

/// 取原文的首／尾空白（拼接字串常靠它，例如 `"等級： "`）。
fn edge_whitespace(s: &str) -> (&str, &str) {
    let lead = &s[..s.len() - s.trim_start().len()];
    let trail = &s[s.trim_end().len()..];
    (lead, trail)
}

fn restore_edges(source: &str, translated: &str) -> String {
    if translated.trim().is_empty() {
        return translated.to_string();
    }
    let (lead, trail) = edge_whitespace(source);
    let core = translated.trim();
    format!("{lead}{core}{trail}")
}

/// 全形符號還原成半形（AI 用中文輸入法時最常見的破壞）。
fn normalize_fullwidth(s: &str) -> String {
    s.replace('％', "%")
        .replace('＄', "$")
        .replace('｛', "{")
        .replace('｝', "}")
        .replace('＆', "&")
}

/// 佔位符被插了空白：`% s`、`%1 $ s`。
fn squeeze_spec_spaces(s: &str) -> String {
    let pat = re(r"%\s+(\d+\s*\$\s*)?([bBhHsScCdoxXeEfgGaAtTn%])");
    pat.replace_all(s, |c: &regex::Captures| {
        let idx = c
            .get(1)
            .map(|m| m.as_str().replace([' ', '\t'], ""))
            .unwrap_or_default();
        format!("%{}{}", idx, &c[2])
    })
    .into_owned()
}

/// 換行守衛。回傳 `Some(修正後)` 或 `None`（內部換行被壓掉 → 退回原文）。
///
/// 兩個實際踩過的雷（其他工具的案例）：
/// - **原文單行、譯文憑空多出 `\n`**：Origins／FancyMenu 面板本來就自動折行，AI 多塞的換行
///   造成排版不一致。原文沒換行卻多出來 → 收斂成單行。
/// - **原文多行、譯文把行併掉**：常伴隨整個子句消失（"…Or Harm Undead" 被吃掉）。
///   內部換行變少 → 判為不可靠，退回原文由呼叫端處理。
fn fix_newlines(source: &str, translated: &str) -> Option<String> {
    let src_n = source.trim().matches('\n').count();
    let dst_n = translated.trim().matches('\n').count();
    if src_n == 0 {
        if dst_n == 0 {
            return Some(translated.to_string());
        }
        // 原文單行：把譯文多出來的換行收斂成單行（連續空白壓成一格）
        let collapsed = translated
            .replace('\r', " ")
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        return Some(collapsed);
    }
    // 原文多行：譯文行數不得變少（變少＝內容被壓掉）
    if dst_n < src_n {
        return None;
    }
    Some(translated.to_string())
}

/// 驗證並盡量修復譯文。
///
/// 回傳 `Some(可安全使用的譯文)`，或 `None` 代表破壞無法修復——呼叫端應退回原文。
pub fn validate_and_repair(source: &str, translated: &str) -> Option<String> {
    let t = translated.trim();
    if t.is_empty() {
        return None;
    }
    let src_ph = extract(source);

    // 原文沒有佔位符：先過換行守衛，再補回首尾空白
    if src_ph.is_empty() {
        let fixed = fix_newlines(source, translated)?;
        return Some(restore_edges(source, &fixed));
    }

    // 依序嘗試候選修法，第一個通過驗證的就採用（並過換行守衛）
    let c1 = translated.to_string();
    let c2 = normalize_fullwidth(&c1);
    let c3 = squeeze_spec_spaces(&c2);
    for cand in [c1, c2, c3] {
        if is_compatible(source, &cand) {
            let fixed = fix_newlines(source, &cand)?;
            return Some(restore_edges(source, &fixed));
        }
    }
    None
}

/// 一批譯文的把關結果。
#[derive(Debug, Default, Clone)]
pub struct GuardStats {
    pub checked: usize,
    /// 修復後可用
    pub repaired: usize,
    /// 無法修復，已退回原文
    pub rejected: usize,
}

impl GuardStats {
    pub fn note(&self) -> String {
        if self.rejected == 0 && self.repaired == 0 {
            return format!("佔位符檢查：{} 條全數正常", self.checked);
        }
        format!(
            "佔位符檢查：{} 條中修復 {} 條、退回原文 {} 條（避免遊戲內格式錯誤）",
            self.checked, self.repaired, self.rejected
        )
    }
}

/// 把關單條譯文：可用則回傳譯文，不可用回傳 `None`（呼叫端退回原文）。
pub fn guard(source: &str, translated: &str, stats: &mut GuardStats) -> Option<String> {
    stats.checked += 1;
    match validate_and_repair(source, translated) {
        Some(fixed) => {
            if fixed != translated {
                stats.repaired += 1;
            }
            Some(fixed)
        }
        None => {
            stats.rejected += 1;
            None
        }
    }
}

// ═══ 遮罩（mask / unmask）══════════════════════════════════════
//
// 比「請 AI 保留 %s」更強的做法：送 AI 前把所有結構 token 換成 `{0} {1} …`，
// 收回來再還原。模型看到的是乾淨句子加幾個簡單索引——它幾乎不可能弄壞 `{0}`，
// 但很容易弄壞 `%1$s` 或 `§c`。還原後仍會過 `guard` 當最後防線。
//
// token 涵蓋比 `extract` 更廣（Patchouli `$(…)`、MDX/JSX 標籤、markdown 連結、
// `\n`…），因為書本／任務文字這些都會出現，遮罩要一次擋住。
// 技術移植自 Koudesuk/Modpack_Translator（MIT），見專案 NOTICE。

/// 單一 pass 抓出所有「必須原樣保留」的結構 token。
///
/// **不使用環視**（Rust regex 不支援，也用不到）：靠交替順序與明確邊界避免誤吃。
const RE_MASK_TOKENS: &str = concat!(
    r"\$\([^)]*\)",                                   // Patchouli：$(br) $(l:item) $()
    r"|/\$",                                          // Patchouli 簡寫收尾
    r"|\[#\]\([0-9A-Fa-f]*\)",                        // Modonomicon 顏色標記
    r"|\((?:item|entry|category|book|command|https?)://[^)]*\)", // Modonomicon 連結目標
    r"|!\[[A-Za-z0-9_.:#/-]*\]\([^)\s]*\)",           // markdown 圖片（alt 是識別字時）
    r"|\]\([^)\s]*\)",                                // markdown 連結目標
    r"|\\?@[A-Z][A-Z0-9_]*@",                         // 舊版指南標記 @L@、\@PAGE@
    r"|\\n|\\&",                                      // 字面 \n、\&
    r"|[&§][0-9A-FK-ORa-fk-or]",                      // Minecraft 色碼／格式碼
    r"|%\d+\$[sdifcbxo%]",                            // 位置格式：%1$s %2$d
    r"|%[sdifcbxo%]",                                 // 簡單格式：%s %d %f
    r"|\{[^{}]+\}",                                   // 既有大括號佔位符 {key} {0}
    r"|<(?:item|tag|fluid|block):[^<>\s]{1,160}>",     // Minecraft item/tag reference
    r"|#[A-Za-z0-9_.-]+:[A-Za-z0-9_./-]+",              // Minecraft tag reference
    // MDX/JSX 與 HTML 標籤：<ItemLink id="ae2:x" />、<br/>、</Row>（屬性必帶 = 才算標籤）
    r#"|</?[A-Za-z][A-Za-z0-9_.-]*(?::[A-Za-z][A-Za-z0-9_.-]*)?(?:\s+[A-Za-z_:][A-Za-z0-9_:.-]*\s*=\s*(?:"[^"]*"|'[^']*'|\{[^{}]*\}))*\s*/?>"#,
);

fn mask_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(RE_MASK_TOKENS).expect("mask regex must compile"))
}

fn unmask_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{(\d+)\}").expect("unmask regex must compile"))
}

/// 把結構 token 換成 `{0} {1} …`，回傳（遮罩後文字, token 表）。
pub fn mask(text: &str) -> (String, Vec<String>) {
    let mut tokens: Vec<String> = Vec::new();
    let masked = mask_re()
        .replace_all(text, |c: &regex::Captures| {
            let idx = tokens.len();
            tokens.push(c[0].to_string());
            format!("{{{idx}}}")
        })
        .into_owned();
    (masked, tokens)
}

/// 還原遮罩：`{N}` 換回第 N 個 token；索引越界就原樣留著（交給 guard 判）。
pub fn unmask(masked: &str, tokens: &[String]) -> String {
    unmask_re()
        .replace_all(masked, |c: &regex::Captures| {
            match c[1].parse::<usize>().ok().and_then(|i| tokens.get(i)) {
                Some(t) => t.clone(),
                None => c[0].to_string(),
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(s: &str) -> Vec<String> {
        extract(s).positional
    }

    #[test]
    fn extracts_java_specs() {
        assert_eq!(pos("Deals %s damage to %s"), vec!["%s", "%s"]);
        assert_eq!(pos("%d/%d"), vec!["%d", "%d"]);
        assert_eq!(pos("%.2f%%"), vec!["%.2f", "%%"]);
    }

    #[test]
    fn percent_in_plain_prose_is_not_a_spec() {
        // 舊寫法會把「% c」當成佔位符，導致大量誤判
        assert!(pos("50% chance to drop").is_empty());
        assert!(pos("100% complete").is_empty());
    }

    #[test]
    fn indexed_specs_are_keyed_not_positional() {
        let p = extract("%1$s gave %2$s an item");
        assert!(p.positional.is_empty());
        assert_eq!(p.keyed, vec!["%1$s", "%2$s"]);
    }

    #[test]
    fn collects_brace_named_and_patchouli_tokens() {
        let p = extract("Hi {player}, {0} $(br)done %player%");
        assert!(p.keyed.contains(&"{player}".to_string()));
        assert!(p.keyed.contains(&"{0}".to_string()));
        assert!(p.keyed.contains(&"$(br)".to_string()));
        assert!(p.keyed.contains(&"%player%".to_string()));
    }

    #[test]
    fn protects_item_and_tag_references() {
        let p = extract("Use <item:minecraft:diamond> with #forge:ingots/iron");
        assert!(p.keyed.contains(&"<item:minecraft:diamond>".to_string()));
        assert!(p.keyed.contains(&"#forge:ingots/iron".to_string()));
        assert!(is_compatible(
            "Use <item:minecraft:diamond> with #forge:ingots/iron",
            "使用 <item:minecraft:diamond> 搭配 #forge:ingots/iron"
        ));
    }

    #[test]
    fn colour_codes_are_soft_only() {
        let p = extract("§aGreen§r text");
        assert!(p.is_empty());
        assert_eq!(p.soft.len(), 2);
    }

    #[test]
    fn rejects_translation_that_drops_a_spec() {
        assert!(validate_and_repair("Deals %s damage", "造成傷害").is_none());
    }

    #[test]
    fn rejects_translation_that_duplicates_a_spec() {
        assert!(validate_and_repair("Deals %s damage", "造成 %s %s 傷害").is_none());
    }

    #[test]
    fn rejects_reordered_positional_specs() {
        // %s %d 換成 %d %s 會讓 Java 丟型別例外
        assert!(validate_and_repair("%s has %d items", "%d 個物品屬於 %s").is_none());
    }

    #[test]
    fn allows_reordered_indexed_specs() {
        let out = validate_and_repair("%1$s gave %2$s", "%2$s 收到 %1$s");
        assert_eq!(out.as_deref(), Some("%2$s 收到 %1$s"));
    }

    #[test]
    fn repairs_fullwidth_percent() {
        let out = validate_and_repair("Deals %s damage", "造成 ％s 傷害");
        assert_eq!(out.as_deref(), Some("造成 %s 傷害"));
    }

    #[test]
    fn repairs_space_injected_into_spec() {
        let out = validate_and_repair("Deals %s damage", "造成 % s 傷害");
        assert_eq!(out.as_deref(), Some("造成 %s 傷害"));
    }

    #[test]
    fn restores_edge_whitespace() {
        // 原文尾端的空白是拼接用的，AI 吃掉要補回來
        let out = validate_and_repair("Level: ", "等級：");
        assert_eq!(out.as_deref(), Some("等級： "));
        let out2 = validate_and_repair(" Level ", "等級");
        assert_eq!(out2.as_deref(), Some(" 等級 "));
    }

    #[test]
    fn empty_translation_is_rejected() {
        assert!(validate_and_repair("Anything", "   ").is_none());
    }

    #[test]
    fn guard_counts_outcomes() {
        let mut st = GuardStats::default();
        assert!(guard("Deals %s damage", "造成 %s 傷害", &mut st).is_some());
        assert!(guard("Deals %s damage", "造成 ％s 傷害", &mut st).is_some());
        assert!(guard("Deals %s damage", "造成傷害", &mut st).is_none());
        assert_eq!(st.checked, 3);
        assert_eq!(st.repaired, 1);
        assert_eq!(st.rejected, 1);
    }

    // ── 遮罩 ──────────────────────────────────────────────
    fn roundtrip(src: &str) -> String {
        let (masked, tokens) = mask(src);
        unmask(&masked, &tokens)
    }

    #[test]
    fn mask_replaces_specs_with_indices() {
        let (m, t) = mask("Deals %s to %s");
        assert_eq!(m, "Deals {0} to {1}");
        assert_eq!(t, vec!["%s", "%s"]);
    }

    #[test]
    fn mask_roundtrips_every_token_family() {
        for s in [
            "Deals %s damage",
            "%1$s gave %2$s",
            "Level {level} of {0}",
            "§aGreen§r and &cred",
            "Press $(l:item)here$(/l) now",
            "Use <item:minecraft:diamond> and #forge:ingots/iron",
            "See ](./page.md#anchor) link",
            "escaped\\nnewline",
            r#"<ItemLink id="ae2:controller" /> table"#,
        ] {
            assert_eq!(roundtrip(s), s, "round-trip failed for: {s}");
        }
    }

    #[test]
    fn patchouli_link_fixture_survives_mask_unmask() {
        let src = "Open $(l:entries/tools/hammer)Hammer Guide$(/l)$(br)Then craft $(item)Iron Plate$()";
        let (masked, tokens) = mask(src);
        assert_ne!(masked, src);
        assert_eq!(unmask(&masked, &tokens), src);
    }

    #[test]
    fn item_and_tag_fixture_survives_translated_sentence() {
        let src = "Use <item:create:wrench> on #forge:storage_blocks/brass";
        let (masked, tokens) = mask(src);
        let restored = unmask(
            &masked
                .replace("Use", "使用")
                .replace("on", "對準"),
            &tokens,
        );
        assert!(is_compatible(src, &restored));
    }

    #[test]
    fn model_can_reorder_but_not_break_masked_tokens() {
        let (m, t) = mask("%1$s gave %2$s an item");
        // 模型回傳：語序調換、索引保留
        let model_out = m.replace("{0}", "TMP").replace("{1}", "{0}").replace("TMP", "{1}");
        let restored = unmask(&model_out, &t);
        // 還原後兩個 token 都在，guard 視為相容（keyed 可重排）
        assert!(is_compatible("%1$s gave %2$s an item", &restored));
    }

    #[test]
    fn dropped_masked_index_is_caught_by_guard() {
        let (_m, t) = mask("Deals %s damage");
        // 模型把 {0} 吃掉了
        let restored = unmask("造成傷害", &t);
        assert!(!is_compatible("Deals %s damage", &restored));
    }

    #[test]
    fn out_of_range_index_is_left_literal() {
        // 模型幻想出一個 {9}：還原時原樣留著，交給 guard 擋
        assert_eq!(unmask("造成 {9} 傷害", &["%s".to_string()]), "造成 {9} 傷害");
    }

    #[test]
    fn text_without_tokens_is_unchanged() {
        let (m, t) = mask("Diamond Sword");
        assert_eq!(m, "Diamond Sword");
        assert!(t.is_empty());
    }

    // ── 換行守衛 ──────────────────────────────────────────
    #[test]
    fn collapses_spurious_newlines_when_source_is_single_line() {
        // 原文單行，AI 憑空加了換行 → 收斂成單行
        let out = validate_and_repair("Grants brief resistance", "獲得\n短暫的\n抗性");
        assert_eq!(out.as_deref(), Some("獲得 短暫的 抗性"));
    }

    #[test]
    fn rejects_when_multiline_source_gets_compressed() {
        // 原文兩行、譯文併成一行（常伴隨子句消失）→ 退回原文
        let src = "Heals 4 HP\nOr harms undead";
        assert!(validate_and_repair(src, "治療 4 點生命，或傷害不死生物").is_none());
    }

    #[test]
    fn keeps_matching_multiline_translation() {
        let src = "Line one\nLine two";
        let out = validate_and_repair(src, "第一行\n第二行");
        assert_eq!(out.as_deref(), Some("第一行\n第二行"));
    }

    #[test]
    fn newline_guard_still_preserves_placeholders() {
        // 單行含 %s、AI 多加換行 → 收斂後 %s 仍在
        let out = validate_and_repair("Deals %s damage", "造成\n%s\n傷害");
        assert_eq!(out.as_deref(), Some("造成 %s 傷害"));
    }
}
