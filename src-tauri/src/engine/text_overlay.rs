//! 遊戲內可讀文字覆寫（非 jar lang）：patchouli / openloader / kubejs / datapacks / fancymenu。
//! 只輸出到 work 目錄，不直接改實例。

use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::convert::convert_s2tw_batch;
use super::deepseek::translate_plain_strings;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_AI_UNIQUE: usize = 8000;
const MAX_WALK_DEPTH: usize = 12;

/// `strings_translated` 目前只進 `note`，保留欄位供排查回報用。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct OverlayTranslateResult {
    pub files_written: usize,
    pub strings_translated: usize,
    pub note: String,
}

/// 掃描並翻譯 minecraft_dir 內文字覆寫，寫入 output_dir（保留相對路徑）。
pub fn translate_text_overlays<F>(
    minecraft_dir: &Path,
    output_dir: &Path,
    use_ai: bool,
    mut on_progress: F,
) -> Result<OverlayTranslateResult, String>
where
    F: FnMut(u8, &str),
{
    if !minecraft_dir.is_dir() {
        return Err(format!(
            "找不到 minecraft 目錄：{}",
            minecraft_dir.display()
        ));
    }

    on_progress(
        3,
        "覆寫文字：掃描 patchouli／openloader／kubejs／datapacks／fancymenu…",
    );
    let files = collect_overlay_files(minecraft_dir);
    if files.is_empty() {
        return Ok(OverlayTranslateResult {
            files_written: 0,
            strings_translated: 0,
            note:
                "未找到可處理的文字覆寫檔（patchouli／openloader／kubejs／datapacks／fancymenu）。"
                    .into(),
        });
    }

    on_progress(
        12,
        &format!("覆寫文字：候補 {} 個檔，擷取字串…", files.len()),
    );

    // path → 原始內容 + 類型
    let mut file_payloads: Vec<FilePayload> = Vec::new();
    let mut unique: Vec<String> = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut parse_failures: Vec<String> = Vec::new();

    for path in &files {
        match load_payload(path) {
            Ok(payload) => {
                for s in payload.collect_strings() {
                    if should_translate_overlay_string(&s) && seen.insert(s.clone(), ()).is_none() {
                        unique.push(s);
                    }
                }
                file_payloads.push(payload);
            }
            // 解析失敗不再靜默：記下來，讓使用者分得清是檔案壞了還是工具漏翻。
            Err(e) => parse_failures.push(e),
        }
    }
    if !parse_failures.is_empty() {
        on_progress(
            21,
            &format!(
                "覆寫文字：{} 個檔解析失敗已略過（詳見錯誤日誌）",
                parse_failures.len()
            ),
        );
        write_overlay_parse_errors(output_dir, &parse_failures);
    }

    on_progress(
        22,
        &format!(
            "覆寫文字：{} 檔可讀、唯一可譯字串 {}",
            file_payloads.len(),
            unique.len()
        ),
    );

    if unique.is_empty() {
        return Ok(OverlayTranslateResult {
            files_written: 0,
            strings_translated: 0,
            note: format!(
                "掃描 {} 檔，但沒有符合條件的可譯字串（可能已是繁中或僅 id／路徑）。",
                files.len()
            ),
        });
    }

    // original → translated
    let mut map: HashMap<String, String> = HashMap::new();
    let mut capped_note = String::new();

    // 1) OpenCC：看起來有中文的一律 s2twp
    on_progress(28, "覆寫文字：簡體→台灣繁體（OpenCC）…");
    let chinese_idx: Vec<usize> = unique
        .iter()
        .enumerate()
        .filter(|(_, s)| looks_chinese(s))
        .map(|(i, _)| i)
        .collect();
    if !chinese_idx.is_empty() {
        let batch: Vec<String> = chinese_idx.iter().map(|&i| unique[i].clone()).collect();
        let conv = convert_s2tw_batch(&batch);
        for (j, &i) in chinese_idx.iter().enumerate() {
            let orig = &unique[i];
            if let Some(c) = conv.get(j) {
                if c != orig && !c.trim().is_empty() {
                    map.insert(orig.clone(), c.clone());
                }
            }
        }
    }

    // 2) AI：剩餘非中文（或未進 map 的拉丁文）
    if use_ai {
        let mut need_ai: Vec<String> = Vec::new();
        let mut skipped_over_cap = 0usize;
        for s in &unique {
            if map.contains_key(s) {
                continue;
            }
            // 已是純中文且轉換無差 → 略過 AI
            if looks_chinese(s) && !has_latin_letter(s) {
                continue;
            }
            if need_ai.len() >= MAX_AI_UNIQUE {
                skipped_over_cap += 1;
                continue;
            }
            need_ai.push(s.clone());
        }
        // 上限是有意義的成本保護，但不能默默砍掉還說「完成」
        if skipped_over_cap > 0 {
            capped_note = format!(
                "；覆寫文字超過單次上限 {}，本輪未處理 {} 條（再按一次「再補一些」可續翻）",
                MAX_AI_UNIQUE, skipped_over_cap
            );
            on_progress(
                38,
                &format!(
                    "覆寫文字：超過上限，本輪先處理 {} 條，剩 {} 條下次再補",
                    need_ai.len(),
                    skipped_over_cap
                ),
            );
        }

        if !need_ai.is_empty() {
            on_progress(
                40,
                &format!(
                    "覆寫文字：AI 翻譯 {} 條（上限 {}）…",
                    need_ai.len(),
                    MAX_AI_UNIQUE
                ),
            );
            let app = &mut on_progress;
            let translated = translate_plain_strings(&need_ai, |pct, msg| {
                let mapped = 40 + (pct as u16 * 40 / 100) as u8;
                app(mapped.min(80), msg);
            })?;
            for (i, en) in need_ai.iter().enumerate() {
                if let Some(zh) = translated.get(i) {
                    let t = zh.trim();
                    if !t.is_empty() && t != en {
                        map.insert(en.clone(), t.to_string());
                    }
                }
            }
            // AI 結果再跑一次 s2twp
            if !map.is_empty() {
                on_progress(82, "覆寫文字：AI 結果再轉台灣繁…");
                let keys: Vec<String> = map.keys().cloned().collect();
                let vals: Vec<String> = keys.iter().map(|k| map[k].clone()).collect();
                let conv = convert_s2tw_batch(&vals);
                for (i, k) in keys.iter().enumerate() {
                    if let Some(v) = conv.get(i) {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
        } else {
            on_progress(70, "覆寫文字：無需 AI（皆已中文／OpenCC 處理）");
        }
    } else {
        on_progress(50, "覆寫文字：未勾 AI，僅 OpenCC 處理既有中文");
    }

    if map.is_empty() {
        return Ok(OverlayTranslateResult {
            files_written: 0,
            strings_translated: 0,
            note: format!(
                "掃描 {} 檔、唯一字串 {}；OpenCC／AI 皆無變更（可能已是繁中）。",
                file_payloads.len(),
                unique.len()
            ),
        });
    }

    // 3) 寫出
    on_progress(88, "覆寫文字：寫出檔案…");
    let mut written = 0usize;
    for payload in &file_payloads {
        if let Some(new_bytes) = payload.apply(&map)? {
            let rel = payload
                .path
                .strip_prefix(minecraft_dir)
                .unwrap_or(payload.path.as_path());
            let out_path = output_dir.join(rel);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&out_path, new_bytes).map_err(|e| format!("{}: {e}", out_path.display()))?;
            written += 1;
        }
    }

    let note = format!(
        "覆寫文字：掃描 {} 檔、唯一字串 {}、翻譯表 {} 條、寫出 {} 檔（patchouli／openloader／kubejs／datapacks／fancymenu）。請把輸出目錄對應路徑覆蓋進遊戲{}",
        file_payloads.len(),
        unique.len(),
        map.len(),
        written,
        if capped_note.is_empty() {
            "。".to_string()
        } else {
            capped_note.clone()
        }
    );
    on_progress(100, "覆寫文字完成");
    Ok(OverlayTranslateResult {
        files_written: written,
        strings_translated: map.len(),
        note,
    })
}

/// 把覆寫檔的解析失敗追加到結果目錄的錯誤日誌，讓使用者查得到（不再靜默）。
fn write_overlay_parse_errors(output_dir: &Path, failures: &[String]) {
    if failures.is_empty() {
        return;
    }
    let p = output_dir.join("翻譯錯誤日誌.txt");
    let mut body = String::new();
    if let Ok(old) = fs::read_to_string(&p) {
        body.push_str(&old);
    } else {
        body.push_str("【模組包繁中翻譯 — 錯誤／警告日誌】\n有問題時請把本檔內容一併提供。\n");
    }
    body.push_str("\n======== 覆寫文字解析失敗 ========\n");
    for f in failures {
        body.push_str("• ");
        body.push_str(f);
        body.push('\n');
    }
    let _ = fs::write(p, body);
}

// ─── 檔案收集 ───────────────────────────────────────────────

fn collect_overlay_files(mc: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.iter().any(|x| x == &p) {
            out.push(p);
        }
    };

    // patchouli_books/**/*.json
    let pb = mc.join("patchouli_books");
    if pb.is_dir() {
        for p in walk_files(&pb, &["json"]) {
            push(p);
        }
    }

    // config/openloader
    let ol = mc.join("config").join("openloader");
    if ol.is_dir() {
        for p in walk_files(&ol, &["json"]) {
            if is_openloader_text_json(&p, &ol) {
                push(p);
            }
        }
    }

    // kubejs/**/lang/*.json
    let kjs = mc.join("kubejs");
    if kjs.is_dir() {
        for p in walk_files(&kjs, &["json"]) {
            if path_has_segment(&p, "lang") {
                push(p);
            }
        }
    }

    // datapacks/（只走目錄內檔，略過 .zip）
    let dp = mc.join("datapacks");
    if dp.is_dir() {
        for p in walk_files(&dp, &["json"]) {
            if path_has_segment(&p, "lang") || path_has_segment(&p, "advancements") {
                push(p);
            }
        }
    }

    // config/fancymenu：.txt / .json（略過二進位副檔名）
    let fm = mc.join("config").join("fancymenu");
    if fm.is_dir() {
        for p in walk_files(&fm, &["txt", "json"]) {
            if looks_text_file(&p) {
                push(p);
            }
        }
    }

    out
}

fn walk_files(root: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in WalkDir::new(root)
        .max_depth(MAX_WALK_DEPTH)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        if is_skipped_binary_ext(p) {
            continue;
        }
        let ext_ok = p
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| {
                let sl = s.to_ascii_lowercase();
                exts.iter().any(|e| *e == sl)
            })
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        if let Ok(meta) = p.metadata() {
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
        }
        v.push(p.to_path_buf());
    }
    v
}

fn is_openloader_text_json(path: &Path, _ol_root: &Path) -> bool {
    // lang json
    if path_has_segment(path, "lang") {
        return true;
    }
    // advancements
    if path_has_segment(path, "advancements") {
        return true;
    }
    // patchouli 書本頁面
    if path_has_segment(path, "patchouli_books") || path_has_segment(path, "patchouli") {
        return true;
    }
    false
}

fn path_has_segment(path: &Path, seg: &str) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s.eq_ignore_ascii_case(seg))
            .unwrap_or(false)
    })
}

fn is_skipped_binary_ext(path: &Path) -> bool {
    const SKIP: &[&str] = &[
        "png", "jpg", "jpeg", "webp", "gif", "ogg", "wav", "mp3", "ttf", "otf", "zip", "jar",
        "class", "nbt", "bin",
    ];
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| SKIP.contains(&s.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn looks_text_file(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    if bytes.is_empty() {
        return false;
    }
    // 含 NUL 當二進位
    if bytes.iter().take(4096).any(|&b| b == 0) {
        return false;
    }
    true
}

// ─── 載入／套用 ─────────────────────────────────────────────

enum FileKind {
    Json,
    /// FancyMenu 等：引號字串替換
    QuotedText,
}

struct FilePayload {
    path: PathBuf,
    kind: FileKind,
    raw: String,
}

impl FilePayload {
    fn collect_strings(&self) -> Vec<String> {
        match self.kind {
            FileKind::Json => {
                let Ok(v) = super::lenient_json::parse(&self.raw) else {
                    return vec![];
                };
                let mut out = Vec::new();
                collect_json_strings(&v, &mut out);
                out
            }
            FileKind::QuotedText => {
                let Ok(re) = Regex::new(r#""((?:\\.|[^"\\])*)""#) else {
                    return vec![];
                };
                re.captures_iter(&self.raw)
                    .filter_map(|c| c.get(1).map(|m| unescape_json_str(m.as_str())))
                    .collect()
            }
        }
    }

    fn apply(&self, map: &HashMap<String, String>) -> Result<Option<Vec<u8>>, String> {
        match self.kind {
            FileKind::Json => {
                let mut v: Value = super::lenient_json::parse(&self.raw)
                    .map_err(|e| format!("{}: {e}", self.path.display()))?;
                let changed = apply_json_strings(&mut v, map);
                if !changed {
                    return Ok(None);
                }
                let s = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
                Ok(Some(s.into_bytes()))
            }
            FileKind::QuotedText => {
                let mut new_text = self.raw.clone();
                let mut pairs: Vec<_> = map.iter().collect();
                pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
                let mut any = false;
                for (en, zh) in pairs {
                    let from = format!("\"{}\"", escape_json_str(en));
                    let to = format!("\"{}\"", escape_json_str(zh));
                    if new_text.contains(&from) {
                        new_text = new_text.replace(&from, &to);
                        any = true;
                    }
                }
                if any && new_text != self.raw {
                    Ok(Some(new_text.into_bytes()))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

fn load_payload(path: &Path) -> Result<FilePayload, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "json" {
        // 寬鬆驗證（容忍註解／尾逗號）。真的壞掉回描述性錯誤，由呼叫端記錄，不靜默。
        super::lenient_json::parse(&raw)
            .map_err(|e| format!("{} 解析失敗：{e}", path.display()))?;
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Json,
            raw,
        });
    }

    // .txt：若是 JSON 當 JSON，否則引號字串
    if super::lenient_json::parse(&raw).is_ok() {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            kind: FileKind::Json,
            raw,
        });
    }
    Ok(FilePayload {
        path: path.to_path_buf(),
        kind: FileKind::QuotedText,
        raw,
    })
}

fn collect_json_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => {
            for x in a {
                collect_json_strings(x, out);
            }
        }
        Value::Object(m) => {
            for (_k, x) in m {
                collect_json_strings(x, out);
            }
        }
        _ => {}
    }
}

fn apply_json_strings(v: &mut Value, map: &HashMap<String, String>) -> bool {
    match v {
        Value::String(s) => {
            if let Some(t) = map.get(s) {
                if t != s {
                    *s = t.clone();
                    return true;
                }
            }
            false
        }
        Value::Array(a) => {
            let mut any = false;
            for x in a {
                if apply_json_strings(x, map) {
                    any = true;
                }
            }
            any
        }
        Value::Object(m) => {
            let mut any = false;
            for (_k, x) in m.iter_mut() {
                if apply_json_strings(x, map) {
                    any = true;
                }
            }
            any
        }
        _ => false,
    }
}

// ─── 字串過濾 ───────────────────────────────────────────────

fn should_translate_overlay_string(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 2 {
        return false;
    }
    // 資源路徑／圖片／音效
    let lower = t.to_ascii_lowercase();
    for suf in [
        ".png", ".jpg", ".jpeg", ".webp", ".gif", ".ogg", ".wav", ".mp3", ".json", ".mcmeta",
        ".ttf", ".otf", ".nbt", ".zip",
    ] {
        if lower.ends_with(suf) {
            return false;
        }
    }
    if t.starts_with("http://") || t.starts_with("https://") {
        return false;
    }
    // 純色碼 / 格式碼
    if is_color_or_format_only(t) {
        return false;
    }
    // 純 item id namespace:path
    if t.contains(':') && !t.contains(' ') && t.is_ascii() && !t.contains('\n') {
        // 允許 "Hello: world" 類；純 id 略過
        if t.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '/' || c == '.' || c == '-'
        }) {
            return false;
        }
    }
    // 長 hex id
    if t.chars().all(|c| c.is_ascii_hexdigit()) && t.len() >= 8 {
        return false;
    }
    // 看起來像程式碼／JSON 片段
    if looks_mostly_code(t) {
        return false;
    }
    // 要有字母或中文
    let has_alpha = t.chars().any(|c| c.is_ascii_alphabetic());
    let has_cjk = looks_chinese(t);
    has_alpha || has_cjk
}

fn is_color_or_format_only(s: &str) -> bool {
    // 僅 §x / &x / #RRGGBB / 空白
    let stripped: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if stripped.is_empty() {
        return true;
    }
    // 全是 § 後接一字 或 & 色碼
    let mut chars = stripped.chars().peekable();
    let mut only_codes = true;
    while let Some(c) = chars.next() {
        if c == '§' || c == '&' {
            let _ = chars.next();
            continue;
        }
        if c == '#' {
            for _ in 0..6 {
                match chars.next() {
                    Some(x) if x.is_ascii_hexdigit() => {}
                    _ => {
                        only_codes = false;
                        break;
                    }
                }
            }
            continue;
        }
        only_codes = false;
        break;
    }
    only_codes
}

fn looks_chinese(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn has_latin_letter(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
}

fn looks_mostly_code(s: &str) -> bool {
    if s.starts_with('{') && s.contains('}') {
        return true;
    }
    if s.starts_with('[') && s.ends_with(']') && s.contains('{') {
        return true;
    }
    if s.contains("\"id\":") || s.contains("Count:") {
        return true;
    }
    // 座標／純數字列表
    if s.chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == ',' || c == '-' || c == ' ')
    {
        return true;
    }
    false
}

fn unescape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('u') => {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        if let Some(h) = chars.next() {
                            hex.push(h);
                        }
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                        }
                    }
                }
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

fn escape_json_str(s: &str) -> String {
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
