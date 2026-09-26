//! 失敗／待補項目表：一張可以整批處理的清單。
//!
//! 使用者反映：想把沒翻到的東西拿去線上 AI 翻，但目前只能一個一個開檔案複製，
//! 「有點慘」。這裡把所有待補項目輸出成單一 CSV，並支援把翻好的結果貼回來。
//!
//! 流程設計成三步，不用碰檔案總管：
//!   1. 完成畫面按「複製失敗項目」→ 整張表進剪貼簿
//!   2. 貼進線上 AI 翻譯
//!   3. 按「匯入翻譯」貼回來 → 逐條驗證佔位符後併入
//!
//! 匯入一定要驗證：線上 AI 很容易把 `%s`／`§a` 弄丟或翻掉，直接寫進遊戲會讓
//! 文字格式錯亂甚至崩潰。驗證不過的項目原樣退回並列出來，不會靜默吞掉。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::jar_scan::LangMap;

pub const FAILED_ITEMS_FILE: &str = "失敗項目.csv";

/// CSV 欄位需要跳脫的字元。
fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// 把待補項目輸出成 CSV 文字（含表頭）。
///
/// 開頭寫 UTF-8 BOM 由呼叫端負責——貼到剪貼簿時不要 BOM，寫成檔案給 Excel 開才要。
pub fn build_failed_items_csv(pending: &LangMap, reason: &str) -> String {
    let mut out = String::from("命名空間,鍵,原文,譯文,失敗原因\n");
    let mut namespaces: Vec<&String> = pending.keys().collect();
    namespaces.sort();
    for ns in namespaces {
        let Some(map) = pending.get(ns) else { continue };
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for key in keys {
            let Some(source) = map.get(key) else { continue };
            out.push_str(&csv_escape(ns));
            out.push(',');
            out.push_str(&csv_escape(key));
            out.push(',');
            out.push_str(&csv_escape(source));
            // 「譯文」留空給使用者填；線上 AI 翻完貼回這一欄
            out.push_str(",,");
            out.push_str(&csv_escape(reason));
            out.push('\n');
        }
    }
    out
}

/// 寫出 CSV 檔（含 BOM，Excel 直接開不會亂碼）。
pub fn write_failed_items_csv(
    work_root: &Path,
    pending: &LangMap,
    reason: &str,
) -> Result<PathBuf, String> {
    fs::create_dir_all(work_root).map_err(|e| e.to_string())?;
    let path = work_root.join(FAILED_ITEMS_FILE);
    let body = build_failed_items_csv(pending, reason);
    let mut bytes = Vec::with_capacity(body.len() + 3);
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]); // UTF-8 BOM
    bytes.extend_from_slice(body.as_bytes());
    fs::write(&path, bytes).map_err(|e| format!("寫入失敗項目表失敗：{e}"))?;
    Ok(path)
}

/// 匯入結果：成功併入幾條、哪些被退回。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub accepted: usize,
    /// 佔位符對不上、或譯文是空的——原樣退回，不寫進遊戲
    pub rejected: Vec<String>,
    /// 找不到對應鍵（使用者可能改了鍵名）
    pub unknown_keys: Vec<String>,
    pub summary: String,
}

/// 解析使用者貼回來的內容。
///
/// 同時接受兩種格式，因為線上 AI 的輸出五花八門：
/// - CSV：`命名空間,鍵,原文,譯文,...`（就是我們匯出的那張表）
/// - TSV：`鍵<TAB>譯文`（最常見的「貼兩欄」形式）
///
/// 回傳 `(命名空間可選, 鍵, 譯文)`。命名空間拿不到時由呼叫端用鍵去比對。
pub fn parse_import_text(text: &str) -> Vec<(Option<String>, String, String)> {
    let mut out = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        // 跳過表頭
        if index == 0 && (line.starts_with("命名空間") || line.starts_with("\u{feff}命名空間")) {
            continue;
        }
        if line.contains('\t') {
            let mut parts = line.splitn(2, '\t');
            let key = parts.next().unwrap_or("").trim().to_string();
            let value = parts.next().unwrap_or("").trim().to_string();
            if !key.is_empty() && !value.is_empty() {
                out.push((None, key, value));
            }
            continue;
        }
        let fields = split_csv_line(line);
        // 我們匯出的表：命名空間,鍵,原文,譯文,原因
        if fields.len() >= 4 {
            let ns = fields[0].trim();
            let key = fields[1].trim();
            let value = fields[3].trim();
            if !key.is_empty() && !value.is_empty() {
                out.push((
                    (!ns.is_empty()).then(|| ns.to_string()),
                    key.to_string(),
                    value.to_string(),
                ));
            }
        } else if fields.len() == 2 {
            let key = fields[0].trim();
            let value = fields[1].trim();
            if !key.is_empty() && !value.is_empty() {
                out.push((None, key.to_string(), value.to_string()));
            }
        }
    }
    out
}

/// 逗號分隔，支援雙引號包住的欄位與 `""` 跳脫。
fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quote && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => in_quote = !in_quote,
            ',' if !in_quote => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    fields.push(current);
    fields
}

/// 把匯入的譯文併入 `zh`，逐條驗證佔位符與品質。
pub fn merge_imported(
    zh: &mut LangMap,
    pending: &LangMap,
    entries: &[(Option<String>, String, String)],
) -> ImportReport {
    use super::placeholder;
    use super::translation_quality::is_usable_zh;

    // 鍵 → 命名空間（給沒帶命名空間的匯入用）。同鍵出現在多個命名空間時不猜。
    let mut key_owner: HashMap<&str, Vec<&String>> = HashMap::new();
    for (ns, map) in pending {
        for key in map.keys() {
            key_owner.entry(key.as_str()).or_default().push(ns);
        }
    }

    let mut report = ImportReport::default();
    let mut guard = placeholder::GuardStats::default();
    for (ns_opt, key, translated) in entries {
        let namespace = match ns_opt {
            Some(ns) => ns.clone(),
            None => match key_owner.get(key.as_str()) {
                Some(list) if list.len() == 1 => list[0].clone(),
                Some(_) => {
                    // 同一個鍵在多個命名空間都有，沒有命名空間就無法確定要寫哪一個
                    report
                        .unknown_keys
                        .push(format!("{key}（多個命名空間都有這個鍵，請保留命名空間欄）"));
                    continue;
                }
                None => {
                    report.unknown_keys.push(key.clone());
                    continue;
                }
            },
        };
        let Some(source) = pending.get(&namespace).and_then(|m| m.get(key)) else {
            report.unknown_keys.push(format!("{namespace}:{key}"));
            continue;
        };
        // 佔位符必須對得上：線上 AI 很常把 %s／§a 弄丟，寫進去會讓遊戲文字壞掉
        let Some(safe) = placeholder::guard(source, translated, &mut guard) else {
            report
                .rejected
                .push(format!("{namespace}:{key}（%s／§ 等格式符號對不上）"));
            continue;
        };
        if !is_usable_zh(source, &safe) {
            report
                .rejected
                .push(format!("{namespace}:{key}（看起來還是原文或中英混雜）"));
            continue;
        }
        // B2：貼回的譯文同樣要過 output guard（長度、圖示字、換行…）
        let safe = match super::output_guard::check_entry(source, &safe) {
            Ok(safe) => safe,
            Err(reason) => {
                report.rejected.push(format!("{namespace}:{key}（{}）", reason.player_text()));
                continue;
            }
        };
        zh.entry(namespace).or_default().insert(key.clone(), safe);
        report.accepted += 1;
    }

    report.summary = if report.accepted == 0 {
        "沒有任何一條通過檢查，翻譯結果沒有變動。".to_string()
    } else {
        let mut s = format!("已併入 {} 條翻譯。", report.accepted);
        if !report.rejected.is_empty() {
            s.push_str(&format!("{} 條因格式檢查未過退回。", report.rejected.len()));
        }
        if !report.unknown_keys.is_empty() {
            s.push_str(&format!("{} 條找不到對應項目。", report.unknown_keys.len()));
        }
        s
    };
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_fixture() -> LangMap {
        let mut m: LangMap = HashMap::new();
        m.entry("create".into())
            .or_default()
            .insert("item.wrench".into(), "Wrench".into());
        m.entry("create".into())
            .or_default()
            .insert("tip.damage".into(), "Deals %s damage".into());
        m.entry("botania".into())
            .or_default()
            .insert("item.wand".into(), "Wand of the Forest".into());
        m
    }

    #[test]
    fn csv_has_header_and_escapes_special_characters() {
        let mut pending: LangMap = HashMap::new();
        pending
            .entry("demo".into())
            .or_default()
            .insert("a".into(), "text, with comma".into());
        pending
            .entry("demo".into())
            .or_default()
            .insert("b".into(), "quote \" inside".into());
        let csv = build_failed_items_csv(&pending, "尚未翻譯");
        assert!(csv.starts_with("命名空間,鍵,原文,譯文,失敗原因\n"));
        assert!(csv.contains("\"text, with comma\""), "逗號要被引號包住：{csv}");
        assert!(csv.contains("\"quote \"\" inside\""), "雙引號要跳脫：{csv}");
        // 譯文欄留空給使用者填
        assert!(csv.contains(",,尚未翻譯"));
    }

    #[test]
    fn import_accepts_both_csv_and_tab_separated() {
        let csv = "命名空間,鍵,原文,譯文,失敗原因\ncreate,item.wrench,Wrench,扳手,尚未翻譯\n";
        let parsed = parse_import_text(csv);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0.as_deref(), Some("create"));
        assert_eq!(parsed[0].1, "item.wrench");
        assert_eq!(parsed[0].2, "扳手");

        // 線上 AI 最常見的輸出：鍵 TAB 譯文
        let tsv = "item.wrench\t扳手\nitem.wand\t森林之杖\n";
        let parsed = parse_import_text(tsv);
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].0.is_none());
    }

    #[test]
    fn merge_rejects_translations_that_break_placeholders() {
        // 線上 AI 最常見的破壞：把 %s 弄丟。寫進遊戲會讓文字格式錯亂。
        let pending = pending_fixture();
        let mut zh: LangMap = HashMap::new();
        let entries = vec![
            (Some("create".into()), "item.wrench".into(), "扳手".into()),
            (Some("create".into()), "tip.damage".into(), "造成傷害".into()),
        ];
        let report = merge_imported(&mut zh, &pending, &entries);
        assert_eq!(report.accepted, 1, "只有沒問題的那條該併入");
        assert_eq!(report.rejected.len(), 1);
        assert!(report.rejected[0].contains("tip.damage"));
        assert_eq!(zh["create"]["item.wrench"], "扳手");
        assert!(!zh["create"].contains_key("tip.damage"), "壞掉的不能寫進去");
    }

    #[test]
    fn merge_reports_keys_it_cannot_place() {
        let pending = pending_fixture();
        let mut zh: LangMap = HashMap::new();
        let entries = vec![
            (None, "does.not.exist".into(), "不存在".into()),
            // 沒帶命名空間但鍵唯一 → 應該找得到
            (None, "item.wand".into(), "森林之杖".into()),
        ];
        let report = merge_imported(&mut zh, &pending, &entries);
        assert_eq!(report.accepted, 1);
        assert_eq!(report.unknown_keys.len(), 1);
        assert_eq!(zh["botania"]["item.wand"], "森林之杖");
    }
}
