//! FTB Quests 任務／劇情文字（config/ftbquests/**/*.snbt）
//! 這些不是 lang json，舊流程完全掃不到 → 任務會一直英文。

use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::convert::convert_s2tw_batch;
use super::deepseek::translate_plain_strings;

/// 逐項統計。目前彙整成 `note` 給玩家看，其餘欄位保留供排查回報用。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct QuestTranslateResult {
    pub files_seen: usize,
    pub strings_found: usize,
    pub strings_unique: usize,
    pub strings_translated: usize,
    pub files_written: usize,
    pub output_dir: String,
    pub note: String,
}

/// 將遊戲裡的 ftbquests 翻譯後輸出到 out/config/ftbquests（不直接改遊戲，避免弄壞）
pub fn translate_ftbquests<F>(
    minecraft_dir: &Path,
    output_dir: &Path,
    use_ai: bool,
    mut on_progress: F,
) -> Result<QuestTranslateResult, String>
where
    F: FnMut(u8, &str),
{
    let src = minecraft_dir.join("config").join("ftbquests");
    if !src.is_dir() {
        return Ok(QuestTranslateResult {
            files_seen: 0,
            strings_found: 0,
            strings_unique: 0,
            strings_translated: 0,
            files_written: 0,
            output_dir: String::new(),
            note: "此整合包沒有 config/ftbquests，略過任務翻譯。".into(),
        });
    }

    on_progress(5, "任務：掃描 FTB Quests（.snbt）…");
    let mut files: Vec<PathBuf> = Vec::new();
    for e in WalkDir::new(&src).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_file()
            && p.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("snbt"))
                .unwrap_or(false)
        {
            files.push(p.to_path_buf());
        }
    }

    // 擷取可翻譯字串：title / description 內的雙引號字串
    let re_quoted = Regex::new(r#""((?:\\.|[^"\\])*)""#).map_err(|e| e.to_string())?;

    let mut file_texts: Vec<(PathBuf, String)> = Vec::new();
    let mut all_strings: Vec<String> = Vec::new();
    let mut string_set: HashMap<String, ()> = HashMap::new();

    for path in &files {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for cap in re_quoted.captures_iter(&text) {
            let raw = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            let s = unescape_snbt(raw);
            if should_translate_quest_string(&s) {
                if string_set.insert(s.clone(), ()).is_none() {
                    all_strings.push(s);
                }
            }
        }
        file_texts.push((path.clone(), text));
    }

    let strings_found: usize = file_texts
        .iter()
        .map(|(_, t)| {
            re_quoted
                .captures_iter(t)
                .filter(|c| {
                    let s = unescape_snbt(c.get(1).map(|m| m.as_str()).unwrap_or(""));
                    should_translate_quest_string(&s)
                })
                .count()
        })
        .sum();

    on_progress(
        20,
        &format!(
            "任務：找到 {} 個 snbt、約 {} 條字串（唯一 {}）",
            files.len(),
            strings_found,
            all_strings.len()
        ),
    );

    if all_strings.is_empty() {
        return Ok(QuestTranslateResult {
            files_seen: files.len(),
            strings_found: 0,
            strings_unique: 0,
            strings_translated: 0,
            files_written: 0,
            output_dir: String::new(),
            note: "任務檔裡沒有可翻譯的英文說明（可能已是中文或格式特殊）。".into(),
        });
    }

    // 翻譯表
    let mut map: HashMap<String, String> = HashMap::new();
    if use_ai {
        on_progress(30, "任務：呼叫 AI 翻譯任務／劇情文字…");
        let app_prog = &mut on_progress;
        let translated = translate_plain_strings(&all_strings, |pct, msg| {
            let mapped = 30 + (pct as u16 * 50 / 100) as u8;
            app_prog(mapped.min(80), msg);
        })?;
        for (i, en) in all_strings.iter().enumerate() {
            if let Some(zh) = translated.get(i) {
                let t = zh.trim();
                if !t.is_empty() {
                    map.insert(en.clone(), t.to_string());
                }
            }
        }
    } else {
        // 無 AI：僅對已是中文的字串做 OpenCC
        on_progress(40, "任務：未勾 AI，僅對既有中文做台灣繁轉換…");
        let converted = convert_s2tw_batch(&all_strings);
        for (i, s) in all_strings.iter().enumerate() {
            if looks_chinese(s) {
                if let Some(c) = converted.get(i) {
                    if c != s {
                        map.insert(s.clone(), c.clone());
                    }
                }
            }
        }
    }

    // AI 結果再強制 s2twp
    if !map.is_empty() {
        on_progress(82, "任務：簡體→台灣繁體…");
        let keys: Vec<String> = map.keys().cloned().collect();
        let vals: Vec<String> = keys.iter().map(|k| map.get(k).unwrap().clone()).collect();
        let conv = convert_s2tw_batch(&vals);
        for (i, k) in keys.iter().enumerate() {
            if let Some(v) = conv.get(i) {
                map.insert(k.clone(), v.clone());
            }
        }
    }

    // 寫出
    let dest_root = output_dir.join("config").join("ftbquests");
    on_progress(88, "任務：寫出翻譯後的 quest 檔…");
    let mut written = 0usize;
    let mut applied = 0usize;

    for (src_path, text) in &file_texts {
        let rel = src_path.strip_prefix(&src).unwrap_or(src_path.as_path());
        let mut new_text = text.clone();
        // 由長到短替換，減少部分重疊
        let mut pairs: Vec<_> = map.iter().collect();
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        for (en, zh) in pairs {
            let from = format!("\"{}\"", escape_snbt(en));
            let to = format!("\"{}\"", escape_snbt(zh));
            if new_text.contains(&from) {
                let n = new_text.matches(&from).count();
                new_text = new_text.replace(&from, &to);
                applied += n;
            }
        }
        if new_text != *text {
            let out_path = dest_root.join(rel);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&out_path, new_text.as_bytes()).map_err(|e| e.to_string())?;
            written += 1;
        }
    }

    // 使用說明
    let readme = output_dir.join("【任務翻譯】請複製到遊戲.txt");
    let _ = fs::write(
        &readme,
        format!(
            "【FTB Quests 任務／劇情翻譯】\n\
1. 關閉遊戲。\n\
2. 備份遊戲裡的 config\\ftbquests。\n\
3. 把本工具輸出目錄下的 config\\ftbquests 整份複製覆蓋到遊戲：\n\
   <實例>\\minecraft\\config\\ftbquests\n\
4. 開遊戲檢查任務書。\n\
\n\
統計：掃描 {} 檔、唯一字串 {}、套用約 {} 處、寫出 {} 檔。\n\
輸出：{}\n",
            files.len(),
            all_strings.len(),
            applied,
            written,
            dest_root.display()
        ),
    );

    on_progress(100, "任務翻譯完成");
    Ok(QuestTranslateResult {
        files_seen: files.len(),
        strings_found,
        strings_unique: all_strings.len(),
        strings_translated: map.len(),
        files_written: written,
        output_dir: dest_root.display().to_string(),
        note: format!(
            "任務／劇情已處理：唯一 {} 條 → 寫出 {} 個 snbt。請把輸出的 config\\ftbquests 覆蓋進遊戲。",
            map.len(),
            written
        ),
    })
}

fn should_translate_quest_string(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 2 {
        return false;
    }
    // 跳過 id、路徑、材質、純數字、物品 id
    if t.chars().all(|c| c.is_ascii_hexdigit()) && t.len() >= 8 {
        return false;
    }
    if t.contains(':') && !t.contains(' ') && t.is_ascii() {
        // minecraft:book 等
        return false;
    }
    if t.starts_with("http://") || t.starts_with("https://") {
        return false;
    }
    if looks_mostly_code(t) {
        return false;
    }
    // 有英文字母或中文才處理
    let has_alpha = t.chars().any(|c| c.is_ascii_alphabetic());
    let has_cjk = looks_chinese(t);
    has_alpha || has_cjk
}

fn looks_chinese(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn looks_mostly_code(s: &str) -> bool {
    if s.starts_with('{') && s.contains('}') {
        return true;
    }
    if s.contains("\"id\":") || s.contains("Count:") {
        return true;
    }
    false
}

fn unescape_snbt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\"') => out.push('\"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape_snbt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}
