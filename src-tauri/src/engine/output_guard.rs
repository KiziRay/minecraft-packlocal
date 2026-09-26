//! B2#4：譯文寫出前的逐條自檢（output guard）。
//!
//! 原則：顯示安全優先於翻譯率。每一條譯文寫檔前都要過這一關；不合格的**只退回那一條英文**，
//! 同檔其他條目照常寫出，並把退回原因記進本輪紀錄，讓完成畫面能列出「退回英文清單」。
//!
//! 逐條檢查（[`check_entry`]）：
//! - 空譯文、亂碼字元（U+FFFD、BOM、控制字元）
//! - 格式碼：`%s`／`%d`／`%1$s`／`{0}`／`$(br)` 數量與種類一致（placeholder），`§` 色碼一致
//! - JSON text component 結構一字不差
//! - 模組字型圖示字（私用區、相容漢字）一個都不能少、不能多
//! - 不新增換行（含字面 `\n`）；手冊分頁標記數量一致
//! - **短欄位**長度：原文單行且顯示寬度 ≤ [`SHORT_FIELD_MAX_WIDTH`] 欄（中文字寬 2、其他 1，
//!   `§x` 色碼不計寬）時，譯文寬度上限＝max(ceil(原文寬度 × 1.5), 4)
//! - 前後空白與原文相同；不做全形／半形轉換
//!
//! 檔案層（[`finish_file`]，見 output_guard_file.rs）：UTF-8 無 BOM、.properties 用 `\u` 跳脫、
//! 寫完重新解析（原檔解析得了，新檔就必須解析得了）。

use std::collections::HashMap;
#[cfg(not(test))]
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use super::cjk::icon_glyphs;
use super::jar_scan::LangMap;

pub use super::output_guard_file::{finish_file, zip_entry_ok};
pub use super::text_component::{split_text_component, ComponentPart};
pub use super::output_guard_sources::{remember_native, remember_reference, remember_sources, snapshot_reference, reset_sources, snapshot_native, snapshot_sources};
use super::output_guard_sources::{native_of, reference_of, source_of};

/// 短欄位的原文寬度上限（欄）。按鈕、物品名、選單項目幾乎都在這個範圍內。
pub const SHORT_FIELD_MAX_WIDTH: usize = 16;
/// 單輪最多保留幾筆退回明細（避免異常情況把結果撐爆；計數照算）。
const MAX_REJECTED_KEPT: usize = 5000;

/// 退回原因代碼。`code()` 給 B8 畫面與日誌用，`player_text()` 是白話。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    Empty,
    BadCharacters,
    FormatCodes,
    ColourCodes,
    TextComponent,
    IconGlyphs,
    NewLine,
    PageBreak,
    TooLong,
    LineCount,
    Edges,
    Encoding,
    Reparse,
    MissingSource,
}

impl RejectReason {
    pub fn code(self) -> &'static str {
        match self {
            RejectReason::Empty => "empty",
            RejectReason::BadCharacters => "bad_characters",
            RejectReason::FormatCodes => "format_codes",
            RejectReason::ColourCodes => "colour_codes",
            RejectReason::TextComponent => "text_component",
            RejectReason::IconGlyphs => "icon_glyphs",
            RejectReason::NewLine => "new_line",
            RejectReason::PageBreak => "page_break",
            RejectReason::TooLong => "too_long",
            RejectReason::LineCount => "line_count",
            RejectReason::Edges => "edges",
            RejectReason::Encoding => "encoding",
            RejectReason::Reparse => "reparse",
            RejectReason::MissingSource => "missing_source",
        }
    }

    pub fn player_text(self) -> &'static str {
        match self {
            RejectReason::Empty => "譯文是空的",
            RejectReason::BadCharacters => "譯文含有亂碼或看不見的控制字元",
            RejectReason::FormatCodes => "譯文的 %s、{0} 這類格式碼和原文對不上，遊戲會顯示錯誤",
            RejectReason::ColourCodes => "譯文的顏色碼和原文對不上，會整段變色",
            RejectReason::TextComponent => "譯文把 JSON 格式弄壞了",
            RejectReason::IconGlyphs => "譯文弄丟或改動了原本的小圖示",
            RejectReason::NewLine => "譯文多了換行，會跑版",
            RejectReason::PageBreak => "譯文的分頁標記和原文不同，手冊會錯頁",
            RejectReason::TooLong => "譯文比原文長太多，可能超出按鈕或欄位",
            RejectReason::LineCount => "譯文的行數和原文不同",
            RejectReason::Edges => "譯文的前後空白和原文不同，拼接時會黏在一起",
            RejectReason::Encoding => "檔案編碼不安全，整檔保留原文",
            RejectReason::Reparse => "寫出後讀不回來，整檔保留原文",
            RejectReason::MissingSource => "找不到英文原文，只做了不需原文的檢查（缺原文未完整檢查）",
        }
    }
}

/// 一筆「退回英文」明細。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejected {
    /// 來源（模組／檔案），給玩家定位用
    pub file: String,
    /// 條目鍵（lang key）；沒有鍵的來源（任務、覆寫文字）為空
    pub key: String,
    pub source: String,
    pub translated: String,
    pub reason: RejectReason,
    pub code: &'static str,
    pub reason_text: &'static str,
}

/// 本輪的顯示安全結果（接在翻譯結果上給 B8 顯示）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplaySafety {
    /// 檢查過的條目數
    pub checked: usize,
    /// 退回英文的條目數（可能大於明細筆數）
    pub rejected_count: usize,
    /// 退回英文清單
    pub rejected: Vec<Rejected>,
    /// 已啟用、會蓋掉預設字型的資源包：字體可能不支援中文（B8 做橫幅）
    pub font_may_not_support_chinese: Vec<String>,
    /// 找不到英文原文、只做了部分檢查就寫出的條目數（缺原文未完整檢查）
    pub unverified_count: usize,
    /// 缺原文未完整檢查的明細（代碼 missing_source）
    pub unverified: Vec<Rejected>,
}

// ─── 本輪紀錄 ───────────────────────────────────────────────

/// 正式執行：整個程式共用一份本輪紀錄（寫檔可能在工作執行緒）。
#[cfg(not(test))]
fn with_log<R>(f: impl FnOnce(&mut DisplaySafety) -> R) -> R {
    static LOG: OnceLock<Mutex<DisplaySafety>> = OnceLock::new();
    let mut guard = LOG
        .get_or_init(|| Mutex::new(DisplaySafety::default()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    f(&mut guard)
}

/// 測試：每個測試執行緒各一份，平行測試清空紀錄時不會互相干擾。
#[cfg(test)]
fn with_log<R>(f: impl FnOnce(&mut DisplaySafety) -> R) -> R {
    thread_local! {
        static LOG: std::cell::RefCell<DisplaySafety> = std::cell::RefCell::new(DisplaySafety::default());
    }
    LOG.with(|log| f(&mut log.borrow_mut()))
}

fn record_checked() {
    with_log(|log| log.checked += 1);
}

pub(crate) fn record_rejection(file: &str, key: &str, source: &str, translated: &str, reason: RejectReason) {
    with_log(|log| {
        log.rejected_count += 1;
        if log.rejected.len() < MAX_REJECTED_KEPT {
            log.rejected.push(Rejected {
                file: file.to_string(),
                key: key.to_string(),
                source: source.to_string(),
                translated: translated.to_string(),
                reason,
                code: reason.code(),
                reason_text: reason.player_text(),
            });
        }
    });
}

fn record_unverified(file: &str, key: &str, translated: &str) {
    with_log(|log| {
        log.unverified_count += 1;
        if log.unverified.len() < MAX_REJECTED_KEPT {
            let reason = RejectReason::MissingSource;
            log.unverified.push(Rejected {
                file: file.to_string(),
                key: key.to_string(),
                source: String::new(),
                translated: translated.to_string(),
                reason,
                code: reason.code(),
                reason_text: reason.player_text(),
            });
        }
    });
}

/// 取出本輪紀錄並清空（翻譯結果收尾時呼叫一次）。同一個檔案裡重複的同一條只列一次。
pub fn take_run_report() -> DisplaySafety {
    let mut report = with_log(std::mem::take);
    let before = report.rejected.len();
    let mut seen = std::collections::HashSet::new();
    report
        .rejected
        .retain(|r| seen.insert((r.file.clone(), r.key.clone(), r.source.clone(), r.translated.clone(), r.code)));
    report.rejected_count -= before - report.rejected.len();
    let before = report.unverified.len();
    let mut seen = std::collections::HashSet::new();
    report.unverified.retain(|r| seen.insert((r.key.clone(), r.translated.clone())));
    report.unverified_count -= before - report.unverified.len();
    report
}

/// 每輪（翻譯、補翻、修復、貼回翻譯）開始時清空，上一輪中途失敗留下的退回不會混進來。
pub fn begin_run() {
    with_log(|log| *log = DisplaySafety::default());
}

/// 翻譯結果收尾用：本輪紀錄＋掃描遊戲資料夾「字體可能不支援中文」的資源包。
pub fn take_run_report_with_fonts(mc: &std::path::Path) -> DisplaySafety {
    let mut report = take_run_report();
    report.font_may_not_support_chinese = super::output_guard_file::font_may_not_support_chinese(mc);
    report
}

/// 測試與除錯：看某個來源目前被退回的明細（不清空）。
#[cfg(test)]
pub fn peek_rejected_for(file: &str) -> Vec<Rejected> {
    with_log(|log| log.rejected.iter().filter(|r| r.file == file).cloned().collect())
}

/// 測試：看某個來源目前「缺原文未完整檢查」的明細。
#[cfg(test)]
pub fn peek_unverified_for(file: &str) -> Vec<Rejected> {
    with_log(|log| log.unverified.iter().filter(|r| r.file == file).cloned().collect())
}

// ─── 逐條檢查 ───────────────────────────────────────────────

const PAGE_MARKERS: &[&str] = &["{@pagebreak}", "$(pagebreak)", "\u{000C}"];

/// 一定是壞掉的字元（亂碼替代字、BOM／零寬字元、控制字元）。沒有原文可對照時只擋這些。
fn is_bad_char(c: char) -> bool {
    matches!(c as u32,
        0xFFFD | 0xFEFF | 0x200B..=0x200F | 0x2060..=0x206F | 0x7F..=0x9F)
        || ((c as u32) < 0x20 && !matches!(c, '\n' | '\t' | '\r'))
}

/// 遊戲字型多半畫不出來（顯示成方框）或會打亂排版的字元：原文沒有而譯文出現就退回。
/// 一般全形標點與常用中文不在此列。
fn is_risky_char(c: char) -> bool {
    is_bad_char(c)
        || c == '\r'
        || matches!(c as u32,
            0xFE00..=0xFE0F          // 異體選擇符
            | 0x2028 | 0x2029        // 行／段分隔
            | 0x2600..=0x27BF        // 雜項符號、裝飾符號（emoji）
            | 0x1F000..=0x1FFFF      // emoji 與符號區
            | 0x20000..=0x3FFFF)     // CJK 擴展 B 以後
}

fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// 顯示寬度：中日韓全形字 2 欄，其他 1 欄；`§x` 色碼與控制字元不計。
pub fn display_width(s: &str) -> usize {
    let mut width = 0usize;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
            continue;
        }
        if (c as u32) < 0x20 {
            continue;
        }
        width += if is_wide(c) { 2 } else { 1 };
    }
    width
}

fn edges(s: &str) -> (&str, &str) {
    (&s[..s.len() - s.trim_start().len()], &s[s.trim_end().len()..])
}

fn sorted(mut v: Vec<char>) -> Vec<char> {
    v.sort_unstable();
    v
}

/// 為什麼 placeholder 修不好：依序判斷最具體的原因。
fn placeholder_failure(source: &str, translated: &str) -> RejectReason {
    if !super::placeholder_fix::text_component_ok(source, translated) {
        return RejectReason::TextComponent;
    }
    if super::placeholder::is_compatible(source, translated)
        && super::placeholder_fix::repair_colour_codes(source, translated).is_none()
    {
        return RejectReason::ColourCodes;
    }
    if source.trim().matches('\n').count() > translated.trim().matches('\n').count() {
        return RejectReason::LineCount;
    }
    RejectReason::FormatCodes
}

/// 檢查一條譯文。回傳可以寫出的譯文（可能經過安全修復，例如補回結尾 `§r`、首尾空白），
/// 或退回原因（呼叫端寫原文）。
pub fn check_entry(source: &str, translated: &str) -> Result<String, RejectReason> {
    check_entry_with(source, translated, false)
}

/// `allow_long`：參考包／CFPA 人工譯文只免長度檢查，其他檢查照常。
fn check_entry_with(source: &str, translated: &str, allow_long: bool) -> Result<String, RejectReason> {
    if translated == source {
        return Ok(translated.to_string());
    }
    if translated.trim().is_empty() {
        return Err(RejectReason::Empty);
    }
    if translated.chars().any(|c| is_risky_char(c) && !source.contains(c)) {
        return Err(RejectReason::BadCharacters);
    }
    let fixed = super::placeholder::validate_and_repair(source, translated)
        .ok_or_else(|| placeholder_failure(source, translated))?;
    if sorted(icon_glyphs(source)) != sorted(icon_glyphs(&fixed)) {
        return Err(RejectReason::IconGlyphs);
    }
    let single_line = !source.trim().contains('\n');
    if (single_line && fixed.trim().contains('\n'))
        || fixed.matches("\\n").count() > source.matches("\\n").count()
    {
        return Err(RejectReason::NewLine);
    }
    if PAGE_MARKERS
        .iter()
        .any(|m| fixed.matches(m).count() != source.matches(m).count())
    {
        return Err(RejectReason::PageBreak);
    }
    let src_width = display_width(source.trim());
    if !allow_long && single_line && src_width <= SHORT_FIELD_MAX_WIDTH {
        let limit = (src_width * 3).div_ceil(2).max(4);
        if display_width(fixed.trim()) > limit {
            return Err(RejectReason::TooLong);
        }
    }
    if edges(source) != edges(&fixed) {
        return Err(RejectReason::Edges);
    }
    Ok(fixed)
}

/// 檢查並記錄一條譯文；回傳要寫出的文字（不合格時就是原文）。
pub fn check(file: &str, key: &str, source: &str, translated: &str) -> String {
    check_with(file, key, source, translated, false)
}

fn check_with(file: &str, key: &str, source: &str, translated: &str, allow_long: bool) -> String {
    record_checked();
    match check_entry_with(source, translated, allow_long) {
        Ok(safe) => safe,
        Err(reason) => {
            record_rejection(file, key, source, translated, reason);
            source.to_string()
        }
    }
}

/// 原文→譯文對照表（任務、覆寫文字、腳本、Origins…）：不合格的條目從表中拿掉，
/// 寫檔時那一條就維持原文；合格的換成安全修復後的譯文。
pub fn guard_map(file: &str, map: &mut HashMap<String, String>) {
    map.retain(|source, translated| {
        let safe = check(file, "", source, translated);
        if safe == *source && translated != source {
            return false;
        }
        *translated = safe;
        true
    });
}

/// 語言表條目（資源包、JAR 語言檔、參考包、貼回翻譯）：有英文原文就完整檢查；
/// 查不到原文時只做不需要原文的檢查（空譯文、亂碼字元）。
/// 回傳 `Some(要寫出的譯文)`，或 `None`＝這一條不要寫（遊戲會顯示英文）。
pub fn lang_entry(file: &str, ns: &str, key: &str, translated: &str, english: Option<&str>) -> Option<String> {
    // 模組作者自己的 zh_tw 原樣寫出：不是工具產生的譯文，不套長度等檢查
    if native_of(ns, key).as_deref() == Some(translated) {
        record_checked();
        return Some(translated.to_string());
    }
    let known = english.map(str::to_string).or_else(|| source_of(ns, key));
    match known {
        Some(source) => {
            // 參考包人工譯文：只免長度（output_guard_sources::reference）
            let is_reference = reference_of(ns, key).as_deref() == Some(translated);
            let safe = check_with(file, key, &source, translated, is_reference);
            (safe != source || translated == source).then_some(safe)
        }
        None => {
            // 查不到原文：只能做不需原文的檢查（空白、會變方框的字元、結尾停在色碼上），
            // 通過的照寫，但逐條記「缺原文未完整檢查」
            record_checked();
            let reason = if translated.trim().is_empty() {
                Some(RejectReason::Empty)
            } else if translated.chars().any(is_risky_char) {
                Some(RejectReason::BadCharacters)
            } else if super::placeholder_fix::ends_on_unreset_colour(translated) {
                Some(RejectReason::ColourCodes)
            } else {
                None
            };
            match reason {
                Some(r) => {
                    record_rejection(file, key, "", translated, r);
                    None
                }
                None => {
                    record_unverified(file, key, translated);
                    Some(translated.to_string())
                }
            }
        }
    }
}

/// 整張語言表逐條過 guard（貼回翻譯、參考包合併）；不合格的條目移除（＝維持英文）。
pub fn guard_langmap(file: &str, zh: &mut LangMap, english: Option<&LangMap>) {
    for (ns, entries) in zh.iter_mut() {
        let en_ns = english.and_then(|e| e.get(ns));
        entries.retain(|key, value| {
            let en = en_ns.and_then(|m| m.get(key)).map(String::as_str);
            match lang_entry(file, ns, key, value, en) {
                Some(safe) => {
                    *value = safe;
                    true
                }
                None => false,
            }
        });
    }
}

/// 翻譯記憶／共享庫寫入前用：這條譯文過不過得了 guard（不記錄、不計數）。
pub fn passes(source: &str, translated: &str) -> bool {
    check_entry(source, translated).is_ok()
}

#[cfg(test)]
#[path = "output_guard_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "output_guard_coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "output_guard_wiring_tests.rs"]
mod wiring_tests;

#[cfg(test)]
#[path = "output_guard_feedback_tests.rs"]
mod feedback_tests;

#[cfg(test)]
#[path = "output_guard_rate_tests.rs"]
mod rate_tests;
