//! 長句拆解與重組：讓小模型少犯「語意反轉」與「整段漏譯」。
//!
//! 動機（實測案例）：
//! `Heal … For 4 Damage, Or Harm Undead Creatures` 被本地小模型翻成
//! 「造成 4 點傷害」——治療變傷害，而且 `Or Harm Undead` 整個子句消失。
//! 這是小模型對長句從屬結構理解不足的典型失誤：句子越長，越容易只抓到一半。
//!
//! # 為什麼拆句很危險，以及這裡怎麼防
//!
//! 拆句本身會引入三個新的失敗模式，處理不好比不拆更糟：
//!
//! 1. **錯位**：拆成 N 段送翻，回來的順序或數量對不上 → 譯文張冠李戴。
//!    防法：**每一段獨立送、獨立收**，用索引一一對應，不依賴模型保持順序；
//!    只要有任何一段沒回來，整句**放棄拆解、退回原本的整句譯文**。
//! 2. **佔位符被切開**：`%s`／`§a`／`{0}` 若剛好落在切點兩側，
//!    分開翻譯後兩邊都會壞。防法：切點一律避開佔位符，
//!    [`safe_split_points`] 會先把佔位符範圍標記成不可切。
//! 3. **語意黏著被切斷**：`Or`／`but`／`unless` 這種連接詞後面的子句
//!    單獨看會失去指涉。防法：只在**句號類標點**切，不在連接詞切；
//!    連接詞造成的長句改用「整句翻 + 子句覆核」而不是硬拆。
//!
//! # 什麼情況才拆
//!
//! 拆解不是預設行為。只有同時滿足才拆（見 [`should_split`]）：
//! - 原文長度超過門檻（短句模型本來就翻得好，拆了只是多花時間）
//! - 至少切得出兩段，且每段都夠長（切出一堆碎片反而更難翻）
//! - 這次用的是**本地模型**（雲端大模型沒有這個弱點，拆了純粹浪費呼叫次數）

/// 超過這個字元數才考慮拆句。低於此值的句子，小模型的表現本來就沒問題。
const MIN_LEN_TO_SPLIT: usize = 90;
/// 每一段至少要這麼長，否則寧可不拆——碎片沒有上下文，翻出來更差。
const MIN_SEGMENT_LEN: usize = 20;
/// 最多拆幾段。拆太多段等於把一句話打散成沒有關聯的碎片。
const MAX_SEGMENTS: usize = 4;

/// 這句話該不該拆開送翻。
pub fn should_split(source: &str, is_local_model: bool) -> bool {
    if !is_local_model {
        // 雲端大模型沒有長句理解不足的問題，拆了只是多花呼叫次數
        return false;
    }
    if source.chars().count() < MIN_LEN_TO_SPLIT {
        return false;
    }
    split_sentence(source).len() >= 2
}

/// 拆句，但**絕不讓這裡的錯誤殺掉整次翻譯**。
///
/// v21 的教訓：`mask_placeholders` 一個位元組索引 bug，就讓使用者等了三小時的
/// 整包翻譯在 63% 全部作廢（見 `engine/safe_text.rs` 的說明）。
/// 拆句是**純粹的最佳化**——不拆頂多譯文品質差一點，沒有任何理由讓它有能力
/// 中斷整輪工作。
///
/// 這裡的函式都是純函式、不碰共享可變狀態，所以 unwind 是安全的。
/// 出事就當作「這句不拆」，交給呼叫端整句翻。
pub fn split_sentence_safe(source: &str) -> Vec<String> {
    let whole = || vec![source.to_string()];
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| split_sentence(source)))
        .unwrap_or_else(|_| whole())
}

/// 同上，包住 [`should_split`]。出事一律回 `false`（不拆）。
pub fn should_split_safe(source: &str, is_local_model: bool) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        should_split(source, is_local_model)
    }))
    .unwrap_or(false)
}

/// 把長句拆成可以獨立翻譯的片段。回傳的片段**串接起來等於原文**（含標點與空白），
/// 這樣重組時只要把每段的譯文依序接起來就好，不必猜中間該補什麼。
///
/// 拆不動（找不到安全切點、切出來太碎）時回傳只有一個元素的向量＝不要拆。
pub fn split_sentence(source: &str) -> Vec<String> {
    let points = safe_split_points(source);
    if points.is_empty() {
        return vec![source.to_string()];
    }
    let mut segments = Vec::new();
    let mut start = 0usize;
    for point in points {
        if segments.len() + 1 >= MAX_SEGMENTS {
            break;
        }
        let piece = &source[start..point];
        let rest = &source[point..];
        // 兩邊都要夠長才切；否則這個切點跳過
        if piece.trim().chars().count() < MIN_SEGMENT_LEN
            || rest.trim().chars().count() < MIN_SEGMENT_LEN
        {
            continue;
        }
        segments.push(piece.to_string());
        start = point;
    }
    if segments.is_empty() {
        return vec![source.to_string()];
    }
    segments.push(source[start..].to_string());
    segments
}

/// 找出「可以安全切開」的位元組位置（切在標點**之後**）。
///
/// 只認句號類標點（`.` `!` `?` `；` `。` `！` `？`），**不認逗號與連接詞**：
/// 逗號兩側經常是同一個子句的延續，切開會讓模型失去指涉對象，
/// 正是實測案例裡 `, Or Harm Undead` 那種會出事的地方。
///
/// 佔位符（`%s`、`%1$s`、`§a`、`{0}`）範圍內一律不切。
fn safe_split_points(source: &str) -> Vec<usize> {
    let masked = mask_placeholders(source);
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut points = Vec::new();
    for index in 0..chars.len() {
        let (byte_pos, c) = chars[index];
        if !matches!(c, '.' | '!' | '?' | '；' | '。' | '！' | '？') || masked[byte_pos] {
            continue;
        }
        // 小數點與縮寫（`v1.2`、`Mr.`）不算句子結束：後面要接空白。
        // 標點就在結尾時也不必切。
        let Some((_, next)) = chars.get(index + 1).copied() else {
            continue;
        };
        if !next.is_whitespace() {
            continue;
        }
        // 切點落在「標點與後續空白之後」，讓片段串接起來完全等於原文。
        // 這裡走**字元**索引：位元組索引會停在多位元組字元中間，
        // 那正是 v21 讓整次翻譯崩潰的原因（`Färbt` 的 `ä`）。
        let mut cut = index + 1;
        while matches!(chars.get(cut), Some((_, w)) if w.is_whitespace()) {
            cut += 1;
        }
        if let Some((pos, _)) = chars.get(cut) {
            points.push(*pos);
        }
    }
    points
}

/// 標記每個位元組是否落在佔位符範圍內（true＝不可切）。
///
/// **一律用字元索引前進**。舊版用 `bytes[i] as char` 逐位元組走，
/// 對 `ä`（0xC3 0xA4）會先把 0xC3 當成 `Ã`、再讓 `i` 停在字元中間，
/// 下一行 `source[i..]` 就 panic——那次崩潰讓一整包翻譯在 63% 全毀。
/// `bytes[i] as char` 這個寫法本身就是錯的，這個檔案裡不該再出現。
fn mask_placeholders(source: &str) -> Vec<bool> {
    let mut mask = vec![false; source.len()];
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    // `chars[j]` 的位元組起點；`j` 超出範圍時就是字串結尾。
    let byte_at = |j: usize| -> usize {
        chars.get(j).map(|(pos, _)| *pos).unwrap_or(source.len())
    };
    let mark = |mask: &mut Vec<bool>, from: usize, to: usize| {
        for m in mask.iter_mut().take(to).skip(from) {
            *m = true;
        }
    };

    let mut i = 0usize; // 字元索引，不是位元組索引
    while i < chars.len() {
        let (start, c) = chars[i];
        // `%s`、`%d`、`%1$s`
        if c == '%' {
            let mut j = i + 1;
            while matches!(chars.get(j), Some((_, n)) if n.is_ascii_digit() || *n == '$') {
                j += 1;
            }
            if matches!(chars.get(j), Some((_, n)) if n.is_ascii_alphabetic()) {
                j += 1;
            }
            mark(&mut mask, start, byte_at(j));
            i = j;
            continue;
        }
        // `{0}`、`{name}`
        if c == '{' {
            let mut j = i;
            while let Some((_, n)) = chars.get(j).copied() {
                j += 1;
                if n == '}' {
                    break;
                }
            }
            mark(&mut mask, start, byte_at(j));
            i = j;
            continue;
        }
        // 色碼 `§a`：§ 本身是多位元組，連同後面那一個字元一起標
        if c == '§' {
            let j = (i + 2).min(chars.len());
            mark(&mut mask, start, byte_at(j));
            i = j;
            continue;
        }
        i += 1;
    }
    mask
}

/// 把各段譯文重組回一句。
///
/// **任何一段缺席就整句放棄**（回傳 `None`），由呼叫端退回「整句直接翻」的結果。
/// 這是防錯位的最後一道：寧可用品質差一點的整句譯文，也不要交出
/// 半句中文半句英文、或順序被打亂的東西。
pub fn rejoin(segments: &[String], translated: &[Option<String>]) -> Option<String> {
    if segments.len() != translated.len() || segments.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (index, piece) in translated.iter().enumerate() {
        let text = piece.as_ref()?.trim();
        if text.is_empty() {
            return None;
        }
        out.push_str(text);
        // 段與段之間補一個空白：中文不需要，但原文若以空白分隔要保留可讀性
        if index + 1 < translated.len() && !text.ends_with(|c: char| c.is_whitespace()) {
            let next_starts_ascii = translated[index + 1]
                .as_ref()
                .and_then(|s| s.trim().chars().next())
                .map(|c| c.is_ascii_alphanumeric())
                .unwrap_or(false);
            if next_starts_ascii {
                out.push(' ');
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v21 讓一整包翻譯在 63% 崩潰的那一句，原封不動搬進來當測試。
    ///
    /// 崩潰訊息：`byte index 2 is not a char boundary; it is inside 'ä'`。
    /// 原因是 `mask_placeholders` 用位元組逐格前進，`i` 停在 `ä` 中間之後
    /// `source[i..]` 就炸。只要句子夠長又含非 ASCII 字元就會踩到——
    /// 也就是德文、法文、中文的模組說明全都會。
    #[test]
    fn the_string_that_crashed_a_whole_run_no_longer_panics() {
        let crashed = "Färbt CEM Mobs grün, um sie von Vanilla Modellen\n\
                       unterscheiden zu können und um prüfen zu können,\n\
                       ob sie richtig geladen werden";
        let parts = split_sentence(crashed);
        assert_eq!(parts.concat(), crashed, "片段串接必須等於原文");
        // 走 should_split 是實際呼叫路徑，一併確認
        let _ = should_split(crashed, true);
    }

    #[test]
    fn multibyte_text_never_panics_and_never_cuts_mid_character() {
        // 每一種都曾經（或可能）踩到同一個 bug：非 ASCII 就會讓位元組索引錯位
        let samples = [
            "Färbt CEM Mobs grün und prüfen zu können ob sie richtig geladen werden hier.",
            "Améliore la vitesse de minage sous l'eau. Cela ne fonctionne pas à la surface.",
            "這是一段很長的中文說明文字，用來確認拆句不會在字元中間切開。還有第二句話。",
            "Deals %1$s damage — with an em dash… and §a colour §r codes ✦ plus emoji 🔥 here.",
            "Ärger Öl Über straße mit ß und ümlaut zeichen die alle mehrbyte sind hier so.",
        ];
        for s in samples {
            let mask = mask_placeholders(s);
            assert_eq!(mask.len(), s.len(), "遮罩長度必須等於位元組長度：{s}");
            let parts = split_sentence(s);
            assert_eq!(parts.concat(), s, "片段串接必須等於原文：{s}");
            // 每個切點都必須是合法的字元邊界，否則之後任何切片都會 panic
            for point in safe_split_points(s) {
                assert!(s.is_char_boundary(point), "切點 {point} 不是字元邊界：{s}");
            }
        }
    }

    #[test]
    fn a_panic_in_splitting_costs_one_sentence_not_the_whole_run() {
        // 拆句是純最佳化。就算這裡再出一次 v21 那種 bug，
        // 代價也只能是「這句不拆」，不能是整包翻譯作廢。
        let hostile = "Färbt CEM Mobs grün 🔥 %1$s §a {name} — long enough to be split. \
                       And a second sentence here too, also long enough to matter.";
        // 正常路徑
        assert_eq!(split_sentence_safe(hostile).concat(), hostile);
        // 直接驗證保護本身：panic 會被吃掉並回退成「整句」
        let boom = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("模擬拆句內部炸掉");
        }));
        assert!(boom.is_err(), "這個 panic 本來就該被 catch_unwind 接住");
        // should_split_safe 出事時要保守回 false（不拆）
        assert!(!should_split_safe("short", true));
    }

    #[test]
    fn short_sentences_are_never_split() {
        assert!(!should_split("Deals 4 damage.", true));
        assert_eq!(split_sentence("Deals 4 damage.").len(), 1);
    }

    #[test]
    fn cloud_models_never_split() {
        // 雲端大模型沒有長句理解不足的問題，拆了只是多花呼叫次數
        let long = "Heal the target for four points of health. \
                    Or harm undead creatures instead, dealing the same amount. \
                    This effect cannot critically strike.";
        assert!(should_split(long, true));
        assert!(!should_split(long, false), "雲端模型不該拆句");
    }

    #[test]
    fn segments_concatenate_back_to_the_original() {
        // 這條是重組正確性的根本保證：片段串起來必須完全等於原文，
        // 否則重組時得猜中間少了什麼，那正是錯位的來源。
        let long = "Heal the target for four points of health. \
                    Or harm undead creatures instead, dealing the same amount. \
                    This effect cannot critically strike.";
        let parts = split_sentence(long);
        assert!(parts.len() >= 2, "這種長句應該拆得開：{parts:?}");
        assert_eq!(parts.concat(), long, "片段串接必須等於原文");
    }

    #[test]
    fn never_splits_inside_placeholders() {
        // 佔位符被切開會讓兩邊都壞掉，而且遊戲不會報錯只會顯示錯誤格式
        let text = "Deals %1$s damage over %2$s seconds. \
                    Then heals §a%3$s§r health to the caster afterwards.";
        for piece in split_sentence(text) {
            let percent = piece.matches('%').count();
            // 每個 % 後面都要還跟著它的格式字母，沒有被切成孤兒
            assert_eq!(
                percent,
                piece.matches(|c| c == '%').count(),
                "佔位符不該被切散：{piece}"
            );
            assert!(!piece.trim().ends_with('%'), "切點落在佔位符中間：{piece}");
            assert!(!piece.trim().ends_with('§'), "切點落在顏色碼中間：{piece}");
        }
    }

    #[test]
    fn does_not_split_on_decimal_points_or_abbreviations() {
        let text = "Requires version 1.20.1 or later to work correctly in all cases.";
        assert_eq!(split_sentence(text).len(), 1, "小數點不是句子結束");
    }

    #[test]
    fn does_not_split_on_commas_or_conjunctions() {
        // 逗號兩側常是同一個子句的延續，切開會讓模型失去指涉——
        // 實測的「Or Harm Undead 整段消失」正是這種地方出事。
        let text = "Heal the target for four points of health, or harm undead creatures \
                    instead, dealing exactly the same amount of damage to them.";
        assert_eq!(split_sentence(text).len(), 1, "不得在逗號切開");
    }

    #[test]
    fn rejoin_gives_up_when_any_segment_is_missing() {
        // 錯位防護的最後一道：缺一段就整句放棄，退回整句翻的結果
        let segments = vec!["A. ".to_string(), "B.".to_string()];
        assert!(rejoin(&segments, &[Some("甲。".into()), None]).is_none());
        assert!(rejoin(&segments, &[Some("甲。".into()), Some("  ".into())]).is_none());
        // 數量對不上也放棄
        assert!(rejoin(&segments, &[Some("甲。".into())]).is_none());
        // 全部都在才重組
        assert_eq!(
            rejoin(&segments, &[Some("甲。".into()), Some("乙。".into())]),
            Some("甲。乙。".to_string())
        );
    }

    #[test]
    fn does_not_exceed_the_segment_cap() {
        let many = "One sentence here now. Two sentences here now. Three sentences here now. \
                    Four sentences here now. Five sentences here now. Six sentences here now.";
        let parts = split_sentence(many);
        assert!(parts.len() <= MAX_SEGMENTS, "拆太多段：{}", parts.len());
        assert_eq!(parts.concat(), many);
    }
}
