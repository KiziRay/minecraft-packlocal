//! B3#1：FTB Quests 新格式的任務語言檔 `config/ftbquests/quests/lang/en_us.snbt`
//! （也支援拆成資料夾的 `lang/en_us/**.snbt`）。
//!
//! 新版 FTB Quests 的任務檔只放 id，標題／說明全在語言檔，key 形如 `quest.0A1B….title`
//! （含點）。舊的逐欄位掃描（`[A-Za-z0-9_]+` 的 key）完全看不到這些字，任務書整本英文。
//!
//! 做法：
//! - 英文檔只讀不改；依它的結構原樣產出 `zh_tw.snbt`（key、順序、陣列行數都照英文檔），
//!   只換字串值。沒翻到的值留英文，遊戲照樣能讀。
//! - 遊戲裡已有 `zh_tw.snbt` 且**不是本工具寫的**（見 `tool_products`）→ 當人工翻譯：
//!   同一個 key／行已是中文的照用（轉台灣繁），不送 AI、不覆蓋。
//!   工具以前寫的 zh_tw.snbt 不算人工，照常重翻（翻譯記憶會命中，不多花錢）。
//! - 只翻英文檔裡有的 key；人工檔裡多出來的 key 不寫。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::cjk::looks_chinese;
use super::convert::convert_s2tw;

/// 一個字串值在原文裡的位置（含引號）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub key: String,
    /// 陣列裡第幾行；單一字串為 0
    pub index: usize,
    pub text: String,
    /// 含看不懂的跳脫（例如 \u、\x）：整段不翻、原樣保留（審查 F11）
    pub opaque: bool,
}

/// 一份要產出的語言檔。
pub(crate) struct LangJob {
    /// 英文檔（遊戲裡的路徑；來源指紋用）
    pub en_path: PathBuf,
    /// 實際讀的英文原檔（遊戲裡的原檔或原檔備份）
    pub read_path: PathBuf,
    /// 相對 `config/ftbquests` 的 zh_tw 路徑
    pub zh_rel: PathBuf,
    pub en_text: String,
    pub spans: Vec<Span>,
    /// 人工 zh_tw 裡已是中文的值（已轉台灣繁）：(key, 行) → 值
    pub human: HashMap<(String, usize), String>,
}

/// `rel`（相對 `config/ftbquests`）在任務語言資料夾裡嗎？這些檔由本模組處理，逐欄位掃描要跳過。
pub(crate) fn is_lang_file(rel: &Path) -> bool {
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    parts.windows(2).any(|w| w[0] == "quests" && w[1] == "lang")
}

/// 找出英文語言檔並建立工作。`mc`＝遊戲資料夾；`ftb_root`＝`mc/config/ftbquests`。
pub(crate) fn collect_jobs(mc: &Path, ftb_root: &Path) -> Vec<LangJob> {
    let lang_dir = ftb_root.join("quests").join("lang");
    let mut jobs = Vec::new();
    let index = super::tool_products::ToolIndex::for_game(mc);
    let single = lang_dir.join("en_us.snbt");
    let split = lang_dir.join("en_us");
    // 審查 F10：單檔與資料夾並存時（FTB Lang Splitter 拆過），只寫資料夾版
    if single.is_file() && !split.is_dir() {
        if let Some(job) = make_job(mc, &index, ftb_root, &single, &lang_dir.join("zh_tw.snbt")) {
            jobs.push(job);
        }
    }
    if split.is_dir() {
        for entry in WalkDir::new(&split).max_depth(8).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let is_snbt = path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("snbt"));
            if !path.is_file() || !is_snbt {
                continue;
            }
            let Ok(inner) = path.strip_prefix(&split) else { continue };
            let zh_path = lang_dir.join("zh_tw").join(inner);
            if let Some(job) = make_job(mc, &index, ftb_root, path, &zh_path) {
                jobs.push(job);
            }
        }
    }
    jobs
}

fn make_job(
    mc: &Path,
    index: &super::tool_products::ToolIndex,
    ftb_root: &Path,
    en_path: &Path,
    zh_path: &Path,
) -> Option<LangJob> {
    // 審查 F2：英文來源必須是原檔；遊戲裡是工具內容就讀原檔備份，沒有就不翻並列出
    let read = match index.read_source(en_path) {
        super::tool_products::ReadSource::Use(p) => p,
        super::tool_products::ReadSource::NeedsOriginal => {
            super::output_guard::record_needs_original(&super::apply_record::rel_key(mc, en_path));
            return None;
        }
    };
    let en_text = fs::read_to_string(&read).ok()?;
    let en_text = en_text.trim_start_matches('\u{FEFF}').to_string();
    let spans = parse_spans(&en_text)?;
    let zh_rel = zh_path.strip_prefix(ftb_root).ok()?.to_path_buf();
    let mut human = HashMap::new();
    // 人工 zh_tw：必須確認是原檔（工具寫的、無法確認的都不保留）
    if zh_path.is_file() && index.is_original(zh_path) {
        if let Some(zh_spans) = fs::read_to_string(zh_path).ok().and_then(|t| parse_spans(t.trim_start_matches('\u{FEFF}'))) {
            for span in zh_spans {
                if !span.opaque && looks_chinese(&span.text) {
                    human.insert((span.key, span.index), convert_s2tw(&span.text));
                }
            }
        }
    }
    Some(LangJob { en_path: en_path.to_path_buf(), read_path: read, zh_rel, en_text, spans, human })
}

/// 要翻的顯示字串（人工中文已涵蓋的不送）。
pub(crate) fn candidates(jobs: &[LangJob]) -> Vec<String> {
    let mut out = Vec::new();
    for job in jobs {
        for span in &job.spans {
            if span.opaque || job.human.contains_key(&(span.key.clone(), span.index)) {
                continue;
            }
            super::ftbquests::push_display_candidates(&span.text, &mut out);
        }
    }
    out
}

/// 依英文檔結構寫出 zh_tw；回傳寫出的檔數。`map` 必須已過 output guard。
pub(crate) fn write_outputs(
    jobs: &[LangJob],
    map: &HashMap<String, String>,
    dest_root: &Path,
    mc: &Path,
    work: &Path,
) -> Result<usize, String> {
    let mut written = 0usize;
    for job in jobs {
        let label = job.zh_rel.to_string_lossy().replace('\\', "/");
        let text = render(&job.en_text, &job.spans, |span| {
            if span.opaque {
                return None;
            }
            if let Some(human) = job.human.get(&(span.key.clone(), span.index)) {
                // 審查 F7：人工值也要檢查（只免長度），不合格退回英文
                return Some(super::output_guard::check_human(&label, &span.key, &span.text, human));
            }
            super::ftbquests::translate_display_value(&span.text, map)
        });
        if text == job.en_text {
            continue;
        }
        let out_path = dest_root.join(&job.zh_rel);
        let name = out_path.to_string_lossy().to_string();
        let bytes = super::output_guard::finish_file(&name, job.en_text.as_bytes(), text.into_bytes());
        if bytes == job.en_text.as_bytes() {
            // 檔案層檢查沒過：整檔維持英文（遊戲會退回英文），不寫半成品
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&out_path, bytes).map_err(|e| format!("{}: {e}", out_path.display()))?;
        super::text_sources::record(work, &out_path, mc, &job.en_path, &job.read_path, "ftbquests");
        written += 1;
    }
    Ok(written)
}

/// 把每個字串值換成 `choose` 給的新值（`None`＝保留原文）；其餘字元原樣保留。
pub(crate) fn render(text: &str, spans: &[Span], mut choose: impl FnMut(&Span) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 2);
    let mut last = 0usize;
    for span in spans {
        out.push_str(&text[last..span.start]);
        // 審查 F11：看不懂的跳脫整段原樣保留
        let chosen = if span.opaque { None } else { choose(span) };
        match chosen {
            Some(value) if value != span.text => {
                out.push('"');
                out.push_str(&escape(&value));
                out.push('"');
            }
            _ => out.push_str(&text[span.start..span.end]),
        }
        last = span.end;
    }
    out.push_str(&text[last..]);
    out
}

/// 解析頂層物件的「key: 字串」與「key: [字串…]」。結構看不懂就回 `None`（整檔不處理）。
pub(crate) fn parse_spans(text: &str) -> Option<Vec<Span>> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut depth = 0i32;
    let mut key: Option<String> = None;
    let mut expect_key = true;
    let mut in_list = false;
    let mut index = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' | b',' | b':' => i += 1,
            b'{' => {
                depth += 1;
                if depth > 1 {
                    return None;
                }
                expect_key = true;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
            }
            b'[' => {
                if key.is_none() || in_list || depth != 1 {
                    return None;
                }
                in_list = true;
                index = 0;
                i += 1;
            }
            b']' => {
                if !in_list {
                    return None;
                }
                in_list = false;
                key = None;
                expect_key = true;
                i += 1;
            }
            b'"' | b'\'' => {
                let (value, end, opaque) = read_quoted(text, i)?;
                if depth != 1 {
                    return None;
                }
                if in_list {
                    spans.push(Span { start: i, end, key: key.clone()?, index, text: value, opaque });
                    index += 1;
                } else if expect_key {
                    key = Some(value);
                    expect_key = false;
                } else {
                    spans.push(Span { start: i, end, key: key.take()?, index: 0, text: value, opaque });
                    expect_key = true;
                }
                i = end;
            }
            _ => {
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b':' | b',' | b'\n' | b'\r' | b' ' | b'\t' | b'{' | b'}' | b'[' | b']') {
                    i += 1;
                }
                if in_list {
                    index += 1;
                } else if expect_key {
                    key = Some(text[start..i].to_string());
                    expect_key = false;
                } else {
                    key = None;
                    expect_key = true;
                }
            }
        }
    }
    (depth == 0).then_some(spans)
}

fn read_quoted(text: &str, start: usize) -> Option<(String, usize, bool)> {
    let quote = text[start..].chars().next()?;
    let mut out = String::new();
    let mut opaque = false;
    let mut chars = text[start + 1..].char_indices();
    while let Some((offset, c)) = chars.next() {
        if c == '\\' {
            let (_, next) = chars.next()?;
            match next {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' | '\'' | '\\' => out.push(next),
                other => {
                    opaque = true;
                    out.push('\\');
                    out.push(other);
                }
            }
        } else if c == quote {
            return Some((out, start + 1 + offset + c.len_utf8(), opaque));
        } else {
            out.push(c);
        }
    }
    None
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[path = "ftbquests_lang_tests.rs"]
mod tests;
