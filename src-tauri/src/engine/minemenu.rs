//! MineMenu 快捷選單：翻譯 `title` 並以 `\uXXXX` 寫回，避免遊戲亂碼。

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::convert::{apply_phrase_dict, convert_s2tw};
use super::deepseek::translate_plain_strings_with_scope;
use super::glossary::load_phrase_dict;
use super::translation_quality::{is_still_english, is_usable_zh};
use super::translation_scope::TranslationScope;

/// 翻譯 MineMenu 標題並寫入工作目錄；可選回寫實例（unicode escape）。
pub fn translate_minemenu<F>(
    minecraft_dir: &Path,
    output_dir: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<String, String>
where
    F: FnMut(u8, &str),
{
    let menu = minecraft_dir.join("minemenu").join("menu.json");
    if !menu.is_file() {
        return Ok("此整合包沒有快捷選單設定，已跳過（正常）。".into());
    }
    on_progress(5, "快捷選單：讀取 menu.json…");
    let text = fs::read_to_string(&menu).map_err(|e| e.to_string())?;
    let mut data: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;

    let mut titles = Vec::new();
    collect_titles(&data, &mut titles);
    titles.sort();
    titles.dedup();
    let need: Vec<String> = titles
        .into_iter()
        .filter(|t| {
            let t = t.trim();
            !t.is_empty() && (is_still_english(t) || !is_usable_zh("", t))
        })
        .collect();

    if need.is_empty() {
        // 仍確保 unicode 寫出，避免既有中文亂碼
        let out = write_minemenu_outputs(minecraft_dir, output_dir, &data)?;
        return Ok(format!(
            "快捷選單：標題皆已可用中文；已寫出 unicode 副本（{out}）。"
        ));
    }

    on_progress(
        20,
        &format!("快捷選單：待譯標題 {} 條…", need.len()),
    );
    let dict = load_phrase_dict(None);
    let mut map: HashMap<String, String> = HashMap::new();
    for t in &need {
        let mut zh = convert_s2tw(t);
        zh = apply_phrase_dict(&zh, &dict);
        if is_usable_zh(t, &zh) && zh != *t {
            map.insert(t.clone(), zh);
        }
    }
    let still: Vec<String> = need
        .iter()
        .filter(|t| !map.contains_key(*t))
        .cloned()
        .collect();
    if use_ai && !still.is_empty() {
        on_progress(40, "快捷選單：AI 翻譯標題…");
        match translate_plain_strings_with_scope(&still, scope, |pct, msg| {
            let mapped = 40 + (pct as u16 * 40 / 100) as u8;
            on_progress(mapped.min(85), msg);
        }) {
            Ok(trs) => {
                for (src, tr) in still.iter().zip(trs.iter()) {
                    let mut zh = convert_s2tw(tr);
                    zh = apply_phrase_dict(&zh, &dict);
                    if is_usable_zh(src, &zh) {
                        map.insert(src.clone(), zh);
                    }
                }
            }
            Err(e) => {
                on_progress(50, &format!("快捷選單：AI 略過（{e}），僅用本機轉換"));
            }
        }
    }
    if !map.is_empty() {
        let _ = super::shared_tm::contribute_plain_pairs(&map, &HashMap::new(), "overlay", scope);
    }

    apply_titles(&mut data, &map);
    let out_note = write_minemenu_outputs(minecraft_dir, output_dir, &data)?;
    // 回寫實例（與舊行為一致：直接修正遊戲內 menu，並留 .bak）
    let final_s = to_ascii_json(&data);
    let bak = menu.with_extension("json.bak");
    if !bak.exists() {
        let _ = fs::copy(&menu, &bak);
    }
    fs::write(&menu, final_s.as_bytes()).map_err(|e| e.to_string())?;

    Ok(format!(
        "快捷選單：翻譯 {}／待譯 {} 條標題，已寫 unicode 並套用（{out_note}）。",
        map.len(),
        need.len()
    ))
}

fn write_minemenu_outputs(
    _minecraft_dir: &Path,
    output_dir: &Path,
    data: &Value,
) -> Result<String, String> {
    let out_menu = output_dir.join("minemenu");
    fs::create_dir_all(&out_menu).map_err(|e| e.to_string())?;
    let dest = out_menu.join("menu.json");
    let final_s = to_ascii_json(data);
    fs::write(&dest, final_s.as_bytes()).map_err(|e| e.to_string())?;
    Ok(dest.display().to_string())
}

fn collect_titles(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                if k == "title" {
                    if let Value::String(s) = val {
                        out.push(s.clone());
                    }
                }
                collect_titles(val, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_titles(x, out);
            }
        }
        _ => {}
    }
}

fn apply_titles(v: &mut Value, map: &HashMap<String, String>) {
    match v {
        Value::Object(m) => {
            for (k, val) in m.iter_mut() {
                if k == "title" {
                    if let Value::String(s) = val {
                        if let Some(t) = map.get(s) {
                            *s = t.clone();
                        }
                    }
                } else {
                    apply_titles(val, map);
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                apply_titles(x, map);
            }
        }
        _ => {}
    }
}

fn to_ascii_json(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{}\"", escape_str(s)),
        Value::Array(a) => {
            let inner: Vec<String> = a.iter().map(to_ascii_json).collect();
            format!("[\n  {}\n]", inner.join(",\n  "))
        }
        Value::Object(m) => {
            let mut parts = Vec::new();
            for (k, val) in m {
                parts.push(format!("\"{}\": {}", escape_str(k), to_ascii_json(val)));
            }
            format!("{{\n  {}\n}}", parts.join(",\n  "))
        }
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c if (c as u32) > 0x7f => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collect_and_apply_titles() {
        let mut v = json!({
            "main": [
                {"title": "Exile GUI Settings", "action": "x"},
                {"title": "Jade Settings", "action": "y"}
            ]
        });
        let mut titles = Vec::new();
        collect_titles(&v, &mut titles);
        assert_eq!(titles.len(), 2);
        let mut map = HashMap::new();
        map.insert("Exile GUI Settings".into(), "介面設定".into());
        apply_titles(&mut v, &map);
        assert_eq!(v["main"][0]["title"], "介面設定");
        assert_eq!(v["main"][1]["title"], "Jade Settings");
        let ascii = to_ascii_json(&json!("介面"));
        assert!(ascii.contains("\\u"));
    }

    #[test]
    fn missing_menu_is_ok() {
        let tmp = std::env::temp_dir().join(format!("minemenu-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let msg = translate_minemenu(&tmp, &tmp, false, None, |_, _| {}).unwrap();
        assert!(msg.contains("沒有快捷選單"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
