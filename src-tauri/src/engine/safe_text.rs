//! 字串切片的邊界安全工具。
//!
//! # 為什麼需要這個檔案
//!
//! Rust 的 `&s[a..b]` 用的是**位元組**索引，但索引落在多位元組字元中間時
//! **直接 panic**。而我們處理的全是使用者的文字：德文的 `ä`、法文的 `é`、
//! 中文、破折號、emoji——沒有一個是單位元組。
//!
//! 這不是理論風險。v21 就是因為
//! [`super::sentence_split::mask_placeholders`] 用位元組逐格前進，
//! 在 `Färbt CEM Mobs grün…` 的 `ä` 中間切下去，讓一整包整合包的翻譯
//! **在 63% 全部作廢**。使用者等了三小時，拿到的是一行 Rust 內部錯誤訊息。
//!
//! # 規則
//!
//! 處理使用者文字時：
//! - **一律用 `char_indices()` 前進**，不要用 `bytes[i]` 配 `i += 1`
//! - **絕對不要寫 `bytes[i] as char`**——那不是「取第 i 個字元」，
//!   對 `ä` 會得到 `Ã`（0xC3），既算錯又讓索引錯位
//! - 索引來源不受控時（例如往回掃、算出來的位移），用這裡的函式夾一下

/// 把位元組索引往**前**移到最近的字元邊界（含自己）。
///
/// 索引超過字串長度時回傳字串長度——那本來就是合法的切片端點。
pub fn floor_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut i = index;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 把位元組索引往**後**移到最近的字元邊界（含自己）。
pub fn ceil_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut i = index;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// 邊界安全的切片：端點會被夾到合法字元邊界，永遠不會 panic。
///
/// 起點往前夾、終點往後夾——寧可多含一個完整字元，也不要切壞一個字元。
pub fn safe_slice(text: &str, start: usize, end: usize) -> &str {
    let start = floor_boundary(text, start.min(text.len()));
    let end = ceil_boundary(text, end.min(text.len()));
    if start >= end {
        return "";
    }
    &text[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 這一串就是 v21 崩潰現場的字：`F` 一個位元組，`ä` 佔 1..3。
    const CRASHED: &str = "Färbt";

    #[test]
    fn boundaries_snap_out_of_the_middle_of_a_character() {
        // byte 2 落在 ä 中間——直接 &s[..2] 會 panic
        assert!(!CRASHED.is_char_boundary(2));
        assert_eq!(floor_boundary(CRASHED, 2), 1, "往前夾到 ä 的開頭");
        assert_eq!(ceil_boundary(CRASHED, 2), 3, "往後夾到 ä 的結尾");
        // 本來就是邊界的不動
        assert_eq!(floor_boundary(CRASHED, 1), 1);
        assert_eq!(ceil_boundary(CRASHED, 3), 3);
    }

    #[test]
    fn safe_slice_never_panics_on_any_index() {
        // 每一組索引組合都不能 panic——包含落在字元中間與超出長度的
        let samples = ["Färbt", "中文字串", "🔥emoji🔥", "", "plain ascii"];
        for s in samples {
            for start in 0..=s.len() + 2 {
                for end in 0..=s.len() + 2 {
                    let out = safe_slice(s, start, end);
                    assert!(s.contains(out) || out.is_empty());
                }
            }
        }
    }

    #[test]
    fn safe_slice_keeps_whole_characters() {
        assert_eq!(safe_slice(CRASHED, 0, 2), "Fä", "切在 ä 中間要含進整個 ä");
        assert_eq!(safe_slice(CRASHED, 2, 5), "ärb", "起點在 ä 中間要退回 ä 開頭");
        assert_eq!(safe_slice("中文字串", 1, 4), "中文");
        assert_eq!(safe_slice("abc", 2, 1), "", "start >= end 回空字串，不 panic");
        assert_eq!(safe_slice("abc", 0, 99), "abc", "超出長度夾到結尾");
    }
}
