//! 本地小模型專用的譯文把關。
//!
//! `translation_quality.rs` 擋的是所有模型都會犯的三種錯（仍是英文、中英碎片、
//! 與原文相同）。但**本地小模型另有一組雲端大模型幾乎不會犯的病**，
//! 這些輸出通通是「看起來像正常中文」，一路通過既有閘門直接寫進遊戲：
//!
//! | 病徵 | 實際輸出長相 | 為什麼既有閘門擋不住 |
//! |---|---|---|
//! | 重複迴圈 | 「造成傷害傷害傷害傷害傷害」 | 是中文、不同於原文、沒有英文碎片 |
//! | 輸出被截斷 | 「使用這把劍可以對敵人造成」 | 同上，只是話沒說完 |
//! | 自問自答 | 「好的，以下是翻譯：鐵劍」 | 同上，只是多了模型的旁白 |
//! | 數字漂移 | `Deals 4 damage` →「造成 8 點傷害」 | 完全合法的中文，但**遊戲資訊是錯的** |
//! | 加油添醋 | 一句話翻成一整段解釋 | 同上 |
//! | 遮罩碎片 | `<Treasurer>` → `{0` | 不是英文、不同於原文，但**是純亂碼** |
//!
//! 這幾種裡最危險的是**數字漂移**：玩家會照著錯的數字做決策，而且永遠不會
//! 發現是翻譯的錯。所以這裡寧可誤判成失敗重翻，也不放行。
//!
//! # 為什麼只給本地模型用
//!
//! 一是雲端大模型基本不犯這些錯，檢查只是白花時間；二是**重試成本完全不同**——
//! 本地模型重翻只花時間不花錢，所以判斷標準可以嚴格得多。

/// 本地模型的退化輸出類型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalIssue {
    /// 同一個字或詞卡住重複（取樣退化的典型症狀）
    RepetitionLoop,
    /// 話沒說完就結束（多半是撞到輸出上限）
    Truncated,
    /// 混進模型自己的旁白（「好的，以下是翻譯：」）
    MetaCommentary,
    /// 原文的數字在譯文裡不見了或被改掉——遊戲數值會因此變成錯的
    NumberDrift,
    /// 一句話膨脹成一整段解釋
    Overrun,
    /// 譯文裡留著壞掉的遮罩碎片（`{0`、`{1` 這種少了右括號的東西）
    ///
    /// 站長實測：原文 `<Treasurer>` 的譯文是 `{0`。那不是「翻得不好」，
    /// 是一段對玩家毫無意義的亂碼——遊戲裡就會直接顯示 `{0`。
    MaskFragment,
}

impl LocalIssue {
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::RepetitionLoop => "模型鬼打牆重複",
            Self::Truncated => "話沒說完",
            Self::MetaCommentary => "混進模型旁白",
            Self::NumberDrift => "數字對不上",
            Self::Overrun => "翻得比原文長太多",
            Self::MaskFragment => "譯文變成亂碼",
        }
    }

    /// 這個問題嚴重到「寧可留英文」嗎？
    ///
    /// 判準是站在**看不懂英文的台灣玩家**那一邊想：
    /// 對他來說，英文等於完全看不懂，所以只有在「這段中文會害到他」的時候，
    /// 退回英文才划算。
    ///
    /// - **會害到他**（擋下）：數字被改掉會讓他照著錯的數值做決定；
    ///   鬼打牆重複是純垃圾，看了也沒有資訊。
    /// - **不會害到他**（照樣採用）：話沒說完、翻得比較囉嗦——
    ///   讀起來不完美，但**資訊還在**，比整句英文有用得多。
    ///
    /// 這條界線很重要：沒有設定雲端 AI 的玩家（多數人）碰到「擋下」就只剩英文。
    /// 把標準訂得太嚴，等於幫他把看得懂的東西換成看不懂的。
    ///
    /// 這支只回答「光看問題類型就能決定」的部分。`Truncated` 不在這裡下定論——
    /// 同樣是話沒說完，「這把劍對不死生物造成額外的」還看得懂，只剩「這把」
    /// 兩個字就沒有資訊了。要分辨得看譯文本身，所以走 [`Self::blocks_output_for`]。
    pub fn blocks_output(self) -> bool {
        match self {
            Self::NumberDrift | Self::RepetitionLoop => true,
            // MetaCommentary 會先被 strip_meta_commentary 清掉，清不掉才會走到這裡
            Self::MetaCommentary => true,
            // 壞掉的遮罩碎片是純亂碼，遊戲裡會直接顯示 `{0`
            Self::MaskFragment => true,
            Self::Truncated | Self::Overrun => false,
        }
    }

    /// 看過原文與譯文之後的最終判定。
    ///
    /// 只有 `Truncated` 需要看內容：截斷到**幾乎沒東西**的譯文，
    /// 留著只會讓玩家看到半個詞，那還不如留英文（至少是完整的一句）。
    ///
    /// 兩個擋下條件（任一成立就擋）：
    /// - 譯文長度不到原文的三成——等於整句只剩開頭幾個字
    /// - 譯文裡一個中日韓字都沒有——那根本不是中文，留著沒意義
    pub fn blocks_output_for(self, source: &str, candidate: &str) -> bool {
        if self != Self::Truncated {
            return self.blocks_output();
        }
        let src_len = source.chars().count();
        let out_len = candidate.chars().count();
        let too_short = src_len > 0 && out_len * 10 < src_len * 3;
        let no_cjk = !candidate.chars().any(is_cjk);
        too_short || no_cjk
    }
}

/// 檢查結果：問題類型，以及**清理過的譯文**（可能沒變）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCheck {
    pub issue: Option<LocalIssue>,
    /// 修得掉的問題（例如模型旁白）已經清乾淨；`None` 代表原文可直接採用。
    pub cleaned: Option<String>,
}

impl LocalCheck {
    fn ok() -> Self {
        Self { issue: None, cleaned: None }
    }
    fn flag(issue: LocalIssue) -> Self {
        Self { issue: Some(issue), cleaned: None }
    }
}

/// 本地模型輸出的總把關。
///
/// **先修再判**：模型旁白（「好的，以下是翻譯：鐵劍」）這種問題是可以直接清掉的，
/// 清完剩下的往往就是正確答案。能救回來就不要浪費一次重翻。
pub fn check(source: &str, candidate: &str) -> LocalCheck {
    let text = candidate.trim();
    if text.is_empty() {
        return LocalCheck::ok(); // 空字串交給既有閘門判（那是 StillEnglish）
    }
    // 1) 先把救得回來的問題修掉
    if let Some(stripped) = strip_meta_commentary(text) {
        // 清掉旁白之後還有內容，就用清乾淨的版本繼續判
        let rest = stripped.trim();
        if rest.is_empty() {
            return LocalCheck::flag(LocalIssue::MetaCommentary);
        }
        let mut out = judge(source, rest);
        out.cleaned = Some(rest.to_string());
        return out;
    }
    // 2) 修不掉的照原樣判
    judge(source, text)
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
        || ('\u{3400}'..='\u{4dbf}').contains(&c)
        || ('\u{f900}'..='\u{faff}').contains(&c)
}

/// 譯文裡有沒有壞掉的遮罩碎片。
///
/// 翻譯前會把 `<Treasurer>`、`%s` 這類東西換成 `{0}`、`{1}` 再送給模型，
/// 翻完再換回來。模型少打一個右括號（輸出 `{0` 而不是 `{0}`）時，
/// 換回來的步驟找不到完整的 `{0}`，那個碎片就原封不動留在譯文裡，
/// 玩家在遊戲裡看到的就是 `{0`。
///
/// 只認**沒有配對右括號**的 `{數字`。完整的 `{0}` 是正常的遮罩，不能誤判。
fn has_broken_mask_fragment(text: &str) -> bool {
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '{' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        // `{` 後面至少要有一個數字才像遮罩；`{}` 或 `{abc` 是別的東西
        if j > i + 1 && (j >= bytes.len() || bytes[j] != '}') {
            return true;
        }
        i = j.max(i + 1);
    }
    false
}

/// 純判斷，不做清理。順序照「證據強度」排：數字對不上是硬事實，排前面。
fn judge(source: &str, text: &str) -> LocalCheck {
    // 遮罩碎片排最前面：那不是「翻得不好」，是譯文變成亂碼，沒有討論空間。
    if has_broken_mask_fragment(text) && !has_broken_mask_fragment(source) {
        return LocalCheck::flag(LocalIssue::MaskFragment);
    }
    if drops_numbers(source, text) {
        return LocalCheck::flag(LocalIssue::NumberDrift);
    }
    if has_repetition_loop(text) {
        return LocalCheck::flag(LocalIssue::RepetitionLoop);
    }
    if looks_truncated(source, text) {
        return LocalCheck::flag(LocalIssue::Truncated);
    }
    if is_overrun(source, text) {
        return LocalCheck::flag(LocalIssue::Overrun);
    }
    LocalCheck::ok()
}

/// 模型把自己的旁白也一起輸出了——**能清掉就清掉，不要整句丟棄**。
///
/// 「好的，以下是翻譯：鐵劍」把前面那段拿掉就剩「鐵劍」，那是正確答案。
/// 直接判失敗等於白白浪費一次翻譯，對沒有雲端可用的玩家更是直接少一句中文。
///
/// 回 `Some(清乾淨的內容)` 代表有偵測到旁白並清掉了；`None` 代表開頭沒有旁白。
/// 只認**開頭**的固定套語——「翻譯這本書」是正常的遊戲文字，不是旁白。
fn strip_meta_commentary(text: &str) -> Option<String> {
    const PREFIXES: &[&str] = &[
        "好的",
        "以下是",
        "這是翻譯",
        "翻譯：",
        "翻譯:",
        "譯文：",
        "譯文:",
        "中文翻譯",
        "繁體中文：",
        "sure,",
        "sure!",
        "here is",
        "here's",
        "translation:",
        "translated:",
        "```",
    ];
    let head = text.trim_start();
    let lower: String = head.chars().take(24).collect::<String>().to_lowercase();
    if !PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return None;
    }
    // 旁白通常以冒號或換行收尾，真正的譯文在後面。取最後一個分隔點之後的內容。
    let mut rest = head;
    if let Some(pos) = rest.find(['：', ':']) {
        // 冒號後面才是譯文；`：` 是多位元組字元，要用它自己的長度前進
        let width = rest[pos..].chars().next().map(char::len_utf8).unwrap_or(1);
        rest = &rest[pos + width..];
    } else if let Some(pos) = rest.find('\n') {
        rest = &rest[pos + 1..];
    } else {
        // 只有套語、沒有分隔符（例如整句就是 "```"）→ 清完是空的
        return Some(String::new());
    }
    // Markdown 圍欄與收尾的引號一併去掉
    Some(
        rest.trim()
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .trim_matches('"')
            .trim()
            .to_string(),
    )
}

/// 取樣退化：同一個字連續重複，或同一小段詞反覆出現。
///
/// 門檻刻意抓得鬆（要很明顯才算），因為中文本來就有疊字
/// （「漸漸」「輕輕」「一步一步」），誤判會讓正常譯文被丟掉。
fn has_repetition_loop(text: &str) -> bool {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.len() < 8 {
        return false;
    }
    // 同一個字連續 6 次以上：「的的的的的的」
    let mut run = 1usize;
    for i in 1..chars.len() {
        if chars[i] == chars[i - 1] {
            run += 1;
            if run >= 6 {
                return true;
            }
        } else {
            run = 1;
        }
    }
    // 同一段 2～6 字的詞連續重複 4 次以上：「傷害傷害傷害傷害」
    for size in 2..=6usize {
        if chars.len() < size * 4 {
            continue;
        }
        let mut start = 0usize;
        while start + size * 4 <= chars.len() {
            let unit = &chars[start..start + size];
            let mut repeats = 1usize;
            let mut at = start + size;
            while at + size <= chars.len() && &chars[at..at + size] == unit {
                repeats += 1;
                at += size;
            }
            if repeats >= 4 {
                return true;
            }
            start += 1;
        }
    }
    false
}

/// 話沒說完。判準是**結構性的**，不靠長度猜：
/// 括號／引號沒收尾，或以連接性的字結尾（「的」「和」「並且」後面顯然還有話）。
fn looks_truncated(source: &str, text: &str) -> bool {
    // 原文本來就沒收尾的（標題、片語）不適用
    let source_balanced = brackets_balanced(source);
    if source_balanced && !brackets_balanced(text) {
        return true;
    }
    // 以懸空的連接字結尾，且原文明顯是完整句子（有句末標點）
    const DANGLING: &[char] = &['的', '和', '與', '或', '及', '把', '被', '讓', '使', '在', '從', '對'];
    let source_ends_sentence = source
        .trim_end()
        .ends_with(|c| matches!(c, '.' | '!' | '?' | '。' | '！' | '？'));
    if source_ends_sentence {
        if let Some(last) = text.trim_end().chars().last() {
            if DANGLING.contains(&last) {
                return true;
            }
        }
    }
    false
}

fn brackets_balanced(text: &str) -> bool {
    let mut round = 0i32;
    let mut square = 0i32;
    let mut curly = 0i32;
    for c in text.chars() {
        match c {
            '(' | '（' => round += 1,
            ')' | '）' => round -= 1,
            '[' | '【' => square += 1,
            ']' | '】' => square -= 1,
            '{' => curly += 1,
            '}' => curly -= 1,
            _ => {}
        }
        if round < 0 || square < 0 || curly < 0 {
            return false; // 右括號比左括號早出現也算壞
        }
    }
    round == 0 && square == 0 && curly == 0
}

/// 原文的數字在譯文裡不見了。
///
/// **這是最危險的一種**：`Deals 4 damage` 譯成「造成 8 點傷害」是完全合法的中文，
/// 但玩家會照著錯的數字做決策，而且永遠不會發現是翻譯的錯。
///
/// 防誤判：
/// - 中文數字是合法翻法（`Level 3` →「三級」），所以小整數要一併認中文寫法
/// - 千分位逗號要正規化（`1,000` →「1000」）
/// - 原文數字太多（表格、座標清單）就不判——那種東西本來就不該逐一比對
fn drops_numbers(source: &str, text: &str) -> bool {
    let numbers = digit_runs(source);
    if numbers.is_empty() || numbers.len() > 6 {
        return false;
    }
    let flat: String = text.chars().filter(|c| *c != ',' && *c != '，').collect();
    numbers.iter().any(|n| {
        if flat.contains(n.as_str()) {
            return false;
        }
        // `Level 3` →「三級」也是對的
        match chinese_numeral(n) {
            Some(cn) => !flat.contains(&cn),
            None => true,
        }
    })
}

/// 抽出原文裡的數字（去掉千分位逗號）。小數點與版本號原樣保留。
fn digit_runs(text: &str) -> Vec<String> {
    let cleaned: String = text
        .chars()
        .map(|c| if c == ',' { '\u{0}' } else { c })
        .collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut prev_digit = false;
    for c in cleaned.chars() {
        if c.is_ascii_digit() {
            current.push(c);
            prev_digit = true;
            continue;
        }
        // 數字中間的 `.` 與被清掉的千分位逗號都算數字的一部分
        if prev_digit && (c == '.' || c == '\u{0}') {
            if c == '.' {
                current.push('.');
            }
            continue;
        }
        if !current.is_empty() {
            out.push(current.trim_end_matches('.').to_string());
            current.clear();
        }
        prev_digit = false;
    }
    if !current.is_empty() {
        out.push(current.trim_end_matches('.').to_string());
    }
    out.retain(|n| !n.is_empty());
    out
}

/// 0～10 的中文寫法；超過就不認（「一千二百三十四」的寫法太多種，比對不可靠）。
fn chinese_numeral(n: &str) -> Option<String> {
    const CN: &[&str] = &["零", "一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    let value: usize = n.parse().ok()?;
    CN.get(value).map(|s| (*s).to_string())
}

/// 一句話膨脹成一整段。
///
/// 中文比英文緊湊得多，所以譯文**比原文還長**本身就是警訊。
/// 門檻取 1.3 倍，依據是實際量到的比例：正常譯文落在原文長度的
/// 0.27～0.39 倍（「Heal the target for 4 health points」→「治療目標 4 點生命值」是 0.31），
/// 而模型自己加解釋的那種輸出量到 1.55 倍。1.3 對正常譯文有三倍以上的餘裕，
/// 又擋得住加油添醋。
const OVERRUN_NUMERATOR: usize = 13;
const OVERRUN_DENOMINATOR: usize = 10;

fn is_overrun(source: &str, text: &str) -> bool {
    let source_len = source.trim().chars().count();
    let text_len = text.trim().chars().count();
    if source_len < 12 {
        return false; // 短原文的長度比例沒有意義（"OK" → "確定" 就超過兩倍）
    }
    text_len * OVERRUN_DENOMINATOR > source_len * OVERRUN_NUMERATOR
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(source: &str, candidate: &str) -> Option<LocalIssue> {
        check(source, candidate).issue
    }

    #[test]
    fn catches_repetition_loops() {
        // 小模型取樣退化的典型長相：是中文、不同於原文、沒有英文碎片，
        // 所以既有的三道閘門一道都攔不住
        assert_eq!(
            issue("Deals damage to the target", "造成傷害傷害傷害傷害傷害"),
            Some(LocalIssue::RepetitionLoop)
        );
        assert_eq!(
            issue("A very long description here", "這把劍的的的的的的威力"),
            Some(LocalIssue::RepetitionLoop)
        );
    }

    #[test]
    fn normal_chinese_reduplication_is_not_a_loop() {
        // 中文本來就有疊字，誤判會把正常譯文丟掉
        assert_eq!(issue("Slowly approaches the target", "緩緩地靠近目標"), None);
        assert_eq!(issue("Step by step toward the goal", "一步一步走向目標"), None);
        assert_eq!(issue("Sparkling brightly in the dark", "在黑暗中閃閃發光"), None);
    }

    #[test]
    fn catches_number_drift() {
        // 最危險的一種：完全合法的中文，但遊戲數值是錯的
        assert_eq!(
            issue("Heal the target for 4 health points", "治療目標 8 點生命值"),
            Some(LocalIssue::NumberDrift)
        );
        assert_eq!(
            issue("Deals 12 damage over 5 seconds", "造成傷害並持續一段時間"),
            Some(LocalIssue::NumberDrift)
        );
    }

    #[test]
    fn chinese_numerals_are_a_valid_translation() {
        assert_eq!(issue("Requires level 3 to unlock", "需要三級才能解鎖"), None);
        assert_eq!(issue("Heal the target for 4 points", "治療目標四點"), None);
    }

    #[test]
    fn thousands_separators_do_not_trigger_drift() {
        assert_eq!(
            issue("Costs 1,000 experience points to use", "使用需要 1000 點經驗值"),
            None
        );
    }

    #[test]
    fn tables_of_numbers_are_left_alone() {
        // 座標／數值表逐一比對只會製造誤判
        assert_eq!(issue("Coordinates 10 20 30 40 50 60 70 80", "座標清單"), None);
    }

    #[test]
    fn model_chatter_is_stripped_instead_of_thrown_away() {
        // 「好的，以下是翻譯：鐵劍」清掉前面那段就剩「鐵劍」——那是正確答案。
        // 整句丟掉等於白白浪費一次翻譯，對沒有雲端可用的玩家更是直接少一句中文。
        let out = check("Iron Sword", "好的，以下是翻譯：鐵劍");
        assert_eq!(out.issue, None, "清乾淨之後就是好譯文，不該還算失敗");
        assert_eq!(out.cleaned.as_deref(), Some("鐵劍"));

        let en = check("Iron Sword", "Here is the translation: 鐵劍");
        assert_eq!(en.issue, None);
        assert_eq!(en.cleaned.as_deref(), Some("鐵劍"));

        // Markdown 圍欄也要清掉
        let fenced = check("Iron Sword", "```\n鐵劍\n```");
        assert_eq!(fenced.cleaned.as_deref(), Some("鐵劍"));
    }

    #[test]
    fn normal_text_containing_the_word_translate_is_not_chatter() {
        // 「翻譯這本書」是正常的遊戲文字
        let out = check("Translate this book", "翻譯這本書");
        assert_eq!(out.issue, None);
        assert_eq!(out.cleaned, None, "沒有旁白就不該動到譯文");
    }

    #[test]
    fn broken_mask_fragments_are_blocked_not_shipped() {
        // 站長實測：原文 <Treasurer>，本地模型吐出 `{0`。
        // 遮罩換回來的步驟找不到完整的 `{0}`，碎片就原封不動留在譯文裡，
        // 遊戲畫面上顯示的就是 `{0`——那不是「翻得不好」，是亂碼。
        assert_eq!(issue("<Treasurer>", "{0"), Some(LocalIssue::MaskFragment));
        assert_eq!(issue("Deals %s damage", "造成 {0 點傷害"), Some(LocalIssue::MaskFragment));
        assert!(LocalIssue::MaskFragment.blocks_output_for("<Treasurer>", "{0"));

        // 完整的遮罩是正常的，不可以誤判
        assert_eq!(issue("Deals {0} damage", "造成 {0} 點傷害"), None);
        // 原文本來就有碎片時不算模型的錯
        assert_eq!(issue("Use {0 to open", "用 {0 開啟"), None);
        // 不是遮罩的大括號也不算
        assert_eq!(issue("Set {} to empty", "把 {} 設為空"), None);
    }

    #[test]
    fn truncation_is_judged_by_how_much_meaning_is_left() {
        // 「這把劍對不死生物造成額外的」讀起來不完美，但資訊還在——
        // 對看不懂英文的玩家來說，這遠比一整句英文有用，所以照樣採用。
        let src = "This sword deals extra damage to undead.";
        let usable = "這把劍對不死生物造成額外的";
        assert_eq!(issue(src, usable), Some(LocalIssue::Truncated));
        assert!(
            !LocalIssue::Truncated.blocks_output_for(src, usable),
            "還看得懂的截斷句要留著"
        );

        // 只剩兩個字＝沒有資訊，那不如留完整的英文
        assert!(
            LocalIssue::Truncated.blocks_output_for(src, "這把"),
            "截到幾乎沒東西就該擋下並送去補完"
        );
        // 一個中文字都沒有＝根本不是譯文
        assert!(
            LocalIssue::Truncated.blocks_output_for(src, "This sword deals"),
            "沒有中日韓字的輸出不算翻譯"
        );
    }

    #[test]
    fn catches_truncated_output() {
        assert_eq!(
            issue("Use this sword (deals extra damage).", "使用這把劍（造成額外傷害"),
            Some(LocalIssue::Truncated)
        );
        assert_eq!(
            issue("This sword deals extra damage to undead.", "這把劍對不死生物造成額外的"),
            Some(LocalIssue::Truncated)
        );
    }

    #[test]
    fn complete_sentences_are_not_flagged_as_truncated() {
        assert_eq!(
            issue("Use this sword (deals extra damage).", "使用這把劍（造成額外傷害）。"),
            None
        );
    }

    #[test]
    fn catches_explanation_overrun() {
        let source = "Increases mining speed while underwater.";
        let bloated = "提高在水下的挖掘速度。這個效果在你完全浸入水中時才會生效，\
                       而且需要同時裝備適當的裝備才能發揮最大效果，建議搭配水下呼吸使用。";
        assert_eq!(issue(source, bloated), Some(LocalIssue::Overrun));
    }

    #[test]
    fn real_translation_length_ratios_stay_well_below_the_overrun_line() {
        // 門檻的依據：正常譯文實測落在原文長度的 0.27～0.39 倍。
        // 這條測試守住那個餘裕——有人調鬆門檻時會先在這裡失敗。
        for (source, zh) in [
            ("Heal the target for 4 health points", "治療目標 4 點生命值"),
            ("Use this sword (deals extra damage).", "使用這把劍（造成額外傷害）。"),
            ("Costs 1,000 experience points to use", "使用需要 1000 點經驗值"),
            ("Increases mining speed while underwater.", "提高在水下的挖掘速度。"),
        ] {
            let ratio = zh.chars().count() as f32 / source.chars().count() as f32;
            assert!(ratio < 0.5, "{source} → {zh} 的比例是 {ratio}，門檻假設已不成立");
            assert!(!is_overrun(source, zh), "正常譯文被誤判成加油添醋：{zh}");
        }
    }

    #[test]
    fn short_sources_are_never_flagged_as_overrun() {
        assert_eq!(issue("OK", "確定"), None);
        assert_eq!(issue("Mine", "挖掘礦石"), None);
    }

    #[test]
    fn clean_translations_pass() {
        assert_eq!(issue("Iron Sword", "鐵劍"), None);
        assert_eq!(issue("Heal the target for 4 health points", "治療目標 4 點生命值"), None);
        assert_eq!(issue("Increases mining speed while underwater.", "提高在水下的挖掘速度。"), None);
    }

    #[test]
    fn only_actively_harmful_output_falls_back_to_english() {
        // 這是整個模組最重要的一條線。看不懂英文的玩家，看到英文就是完全看不懂；
        // 所以只有「這段中文會害到他」才值得退回英文。
        //
        // 會害到他：數字被改掉（照著錯的數值做決定）、鬼打牆（純垃圾沒有資訊）
        assert!(LocalIssue::NumberDrift.blocks_output());
        assert!(LocalIssue::RepetitionLoop.blocks_output());
        // 不會害到他：讀起來不完美，但資訊還在，比整句英文有用得多
        assert!(!LocalIssue::Truncated.blocks_output(), "話沒說完也比整句英文有用");
        assert!(LocalIssue::MaskFragment.blocks_output(), "亂碼要擋下");
        assert!(!LocalIssue::Overrun.blocks_output(), "翻得囉嗦也比整句英文有用");
    }
}
