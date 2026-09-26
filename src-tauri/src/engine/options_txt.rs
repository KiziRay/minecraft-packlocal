//! `options.txt` 的逐行修改與還原：啟用翻譯資源包（放清單最後＝最高優先）、
//! 把遊戲語言設成繁中，並記下原本的值，讓「移除翻譯」可以只拿掉工具加的東西。
//!
//! 原則：只改 `resourcePacks:` 與 `lang:` 這兩行，其他行（按鍵、畫質、音量…）一個字都不動，
//! 行尾（CRLF／LF）也照原檔。還原時不整檔覆蓋——玩家套用後自己改的設定要留著。

use std::fs;
use std::path::Path;

use super::resource_pack_guard::{diff_pack_lists, parse_pack_list};

/// 工具設定的遊戲語言。
pub const TOOL_LANG: &str = "zh_tw";
const PACKS_KEY: &str = "resourcePacks:";
const INCOMPATIBLE_KEY: &str = "incompatibleResourcePacks:";
const LANG_KEY: &str = "lang:";

/// 這次對 options.txt 做了什麼（寫進套用紀錄，移除翻譯時照著反轉）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OptionsEdit {
    pub text: String,
    /// 改之前的語言；原本沒有 `lang:` 這行時是 `None`
    pub original_lang: Option<String>,
    /// 這次有沒有把語言從別的值改成繁中
    pub lang_changed: bool,
    /// 這次才加進清單的項目（原本就有的不算）
    pub pack_added: bool,
}

pub fn pack_entry(pack_file_name: &str) -> String {
    format!("file/{pack_file_name}")
}

/// 原檔的換行風格；CRLF 與 LF 混用時以多數為準（一樣多時用 CRLF，Windows 版遊戲寫的是 CRLF）。
fn line_ending(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if crlf > 0 && crlf >= lf {
        "\r\n"
    } else {
        "\n"
    }
}

fn split_lines(text: &str) -> Vec<String> {
    text.lines().map(|line| line.to_string()).collect()
}

/// 原檔結尾有換行才補換行（沒有就不加），其餘照原樣。
fn join_lines(lines: &[String], ending: &str, had_trailing: bool) -> String {
    let mut out = lines.join(ending);
    if had_trailing && !lines.is_empty() {
        out.push_str(ending);
    }
    out
}

/// 讀 `lang:` 的值。
pub fn read_lang(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix(LANG_KEY))
        .map(|value| value.trim().to_string())
}

/// 解析一行清單的值（JSON 陣列）；半損毀時退回寬鬆解析。
fn parse_list_value(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value.trim())
        .unwrap_or_else(|_| parse_pack_list(&format!("{PACKS_KEY}{value}")))
}

fn render_list(key: &str, list: &[String]) -> String {
    format!("{key}{}", serde_json::to_string(list).unwrap_or_else(|_| "[]".into()))
}

/// 啟用資源包並放到清單最後（遊戲裡最高優先），需要時把語言設成繁中。
/// 已在清單中的會被移到最後，不會重複。
pub fn enable_pack_last(text: &str, entry: Option<&str>, set_lang: bool) -> OptionsEdit {
    let ending = line_ending(text);
    let had_trailing = text.ends_with('\n');
    let mut lines = split_lines(text);
    let original_lang = read_lang(text);
    let mut pack_added = entry.is_some();
    let mut found_packs = false;
    for line in lines.iter_mut() {
        let Some(entry) = entry else {
            break;
        };
        if let Some(value) = line.strip_prefix(PACKS_KEY) {
            found_packs = true;
            let mut list = parse_list_value(value);
            if list.iter().any(|item| item == entry) {
                pack_added = false;
            }
            list.retain(|item| item != entry);
            list.push(entry.to_string());
            *line = render_list(PACKS_KEY, &list);
        } else if let Some(value) = line.strip_prefix(INCOMPATIBLE_KEY) {
            // 版本號對不上時遊戲會把包放進「不相容」清單；我們的包已宣告寬範圍，不該留在那裡
            let mut list = parse_list_value(value);
            let before = list.len();
            list.retain(|item| item != entry);
            if list.len() != before {
                *line = render_list(INCOMPATIBLE_KEY, &list);
            }
        }
    }
    if let (false, Some(entry)) = (found_packs, entry) {
        lines.push(render_list(PACKS_KEY, &[entry.to_string()]));
    }
    let mut lang_changed = false;
    if set_lang && original_lang.as_deref() != Some(TOOL_LANG) {
        lang_changed = true;
        let mut replaced = false;
        for line in lines.iter_mut() {
            if line.starts_with(LANG_KEY) {
                *line = format!("{LANG_KEY}{TOOL_LANG}");
                replaced = true;
            }
        }
        if !replaced {
            lines.push(format!("{LANG_KEY}{TOOL_LANG}"));
        }
    }
    OptionsEdit {
        text: join_lines(&lines, ending, had_trailing),
        original_lang,
        lang_changed,
        pack_added,
    }
}

/// 寫完之後的檢查：原本清單裡的每一項都還在、我們的包排在最後、語言符合預期。
pub fn verify_enabled(before: &str, after: &str, entry: Option<&str>, expect_lang: bool) -> Result<(), String> {
    let packs_before = parse_pack_list(before);
    let packs_after = parse_pack_list(after);
    let diff = diff_pack_lists(&packs_before, &packs_after);
    if !diff.is_safe() {
        return Err(format!(
            "原本的資源包清單少了 {} 項（{}）",
            diff.missing.len(),
            diff.missing.join("、")
        ));
    }
    if entry.is_some() && packs_after.last().map(String::as_str) != entry {
        return Err("翻譯資源包沒有排在最高優先".into());
    }
    if expect_lang && read_lang(after).as_deref() != Some(TOOL_LANG) {
        return Err("遊戲語言沒有成功設成繁體中文".into());
    }
    Ok(())
}

/// 移除翻譯時的逐行還原：只拿掉工具加的資源包項目；語言若仍是工具設的繁中，改回原值。
/// 玩家之後自己改的語言、自己加的資源包都不動。回傳 (新內容, 有沒有改動)。
pub fn restore_lines(
    text: &str,
    packs_added: &[String],
    lang_changed: bool,
    original_lang: Option<&str>,
) -> (String, bool) {
    let ending = line_ending(text);
    let had_trailing = text.ends_with('\n');
    let mut changed = false;
    let mut out = Vec::new();
    for line in split_lines(text) {
        let key = if line.starts_with(PACKS_KEY) {
            Some(PACKS_KEY)
        } else if line.starts_with(INCOMPATIBLE_KEY) {
            Some(INCOMPATIBLE_KEY)
        } else {
            None
        };
        if let Some(key) = key {
            let mut list = parse_list_value(&line[key.len()..]);
            let before = list.len();
            list.retain(|item| !packs_added.iter().any(|added| added == item));
            if list.len() != before {
                changed = true;
                out.push(render_list(key, &list));
                continue;
            }
            out.push(line);
            continue;
        }
        if lang_changed {
            if let Some(value) = line.strip_prefix(LANG_KEY) {
                if value.trim() == TOOL_LANG {
                    changed = true;
                    if let Some(original) = original_lang {
                        out.push(format!("{LANG_KEY}{original}"));
                    }
                    continue;
                }
            }
        }
        out.push(line);
    }
    (join_lines(&out, ending, had_trailing), changed)
}

/// 動 options.txt 前在旁邊另存一份（幾 KB，但壞掉的代價是遊戲開不起來）。
/// 只在第一次建立：已存在就不覆寫，回 `Ok(None)`；這次建立的回 `Ok(Some(位置))`。
pub fn backup_beside_once(options: &Path, original: &str) -> std::io::Result<Option<std::path::PathBuf>> {
    let backup = options.with_extension("txt.mcpl-bak");
    let long = super::paths::long_path(&backup);
    if long.exists() {
        return Ok(None);
    }
    fs::write(&long, original)?;
    Ok(Some(backup))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "version:3465\r\nlang:en_us\r\nresourcePacks:[\"vanilla\",\"file/OtherZh.zip\",\"file/tool.zip\",\"file/Gui.zip\"]\r\nguiScale:3\r\n";

    #[test]
    fn enabling_moves_our_pack_last_and_sets_lang() {
        let edit = enable_pack_last(SAMPLE, Some("file/tool.zip"), true);
        let packs = parse_pack_list(&edit.text);
        assert_eq!(packs.last().unwrap(), "file/tool.zip", "我們的包要最高優先");
        assert_eq!(packs.iter().filter(|p| *p == "file/tool.zip").count(), 1);
        assert!(packs.contains(&"file/OtherZh.zip".to_string()), "其他中文資源包不能被拿掉");
        assert_eq!(read_lang(&edit.text).as_deref(), Some("zh_tw"));
        assert_eq!(edit.original_lang.as_deref(), Some("en_us"));
        assert!(edit.lang_changed);
        assert!(!edit.pack_added, "原本就在清單中，不算工具新增");
        assert!(edit.text.contains("\r\nguiScale:3\r\n"), "其他行與 CRLF 行尾要原樣保留");
        verify_enabled(SAMPLE, &edit.text, Some("file/tool.zip"), true).unwrap();
    }

    #[test]
    fn enabling_twice_is_stable() {
        let first = enable_pack_last(SAMPLE, Some("file/new.zip"), true);
        let second = enable_pack_last(&first.text, Some("file/new.zip"), true);
        assert_eq!(first.text, second.text);
        assert!(!second.lang_changed, "第二次語言已是繁中，不能把原語言記成繁中");
        assert!(first.pack_added);
    }

    #[test]
    fn restore_removes_only_our_entries_and_puts_language_back() {
        let edit = enable_pack_last(SAMPLE, Some("file/new.zip"), true);
        // 玩家套用後自己又加了一個包、改了畫面大小
        let played = edit
            .text
            .replace("guiScale:3", "guiScale:2")
            .replace("\"file/new.zip\"]", "\"file/new.zip\",\"file/Mine.zip\"]");
        let (restored, changed) = restore_lines(&played, &["file/new.zip".into()], true, Some("en_us"));
        assert!(changed);
        let packs = parse_pack_list(&restored);
        assert!(!packs.contains(&"file/new.zip".to_string()));
        assert!(packs.contains(&"file/Mine.zip".to_string()), "玩家自己加的包要留著");
        assert_eq!(read_lang(&restored).as_deref(), Some("en_us"));
        assert!(restored.contains("guiScale:2"), "玩家後來改的設定不能被蓋回去");
    }

    #[test]
    fn restore_leaves_language_alone_if_player_changed_it() {
        let edit = enable_pack_last(SAMPLE, Some("file/new.zip"), true);
        let played = edit.text.replace("lang:zh_tw", "lang:ja_jp");
        let (restored, _) = restore_lines(&played, &["file/new.zip".into()], true, Some("en_us"));
        assert_eq!(read_lang(&restored).as_deref(), Some("ja_jp"));
    }

    #[test]
    fn missing_lang_line_is_added_and_removed_again() {
        let text = "resourcePacks:[\"vanilla\"]\n";
        let edit = enable_pack_last(text, Some("file/a.zip"), true);
        assert_eq!(edit.original_lang, None);
        let (restored, _) = restore_lines(&edit.text, &["file/a.zip".into()], true, None);
        assert_eq!(restored, "resourcePacks:[\"vanilla\"]\n");
    }

    #[test]
    fn incompatible_list_no_longer_holds_our_pack() {
        let text = "resourcePacks:[\"vanilla\"]\nincompatibleResourcePacks:[\"file/a.zip\"]\n";
        let edit = enable_pack_last(text, Some("file/a.zip"), false);
        assert!(edit.text.contains("incompatibleResourcePacks:[]"));
        assert!(!edit.lang_changed);
    }

    #[test]
    fn language_only_when_there_is_no_pack() {
        let edit = enable_pack_last(SAMPLE, None, true);
        assert!(edit.lang_changed);
        assert!(!edit.pack_added);
        assert_eq!(parse_pack_list(&edit.text), parse_pack_list(SAMPLE));
        verify_enabled(SAMPLE, &edit.text, None, true).unwrap();
    }

    #[test]
    fn keeps_missing_trailing_newline_and_majority_line_ending() {
        let no_trailing = "lang:en_us\nresourcePacks:[\"vanilla\"]";
        let edit = enable_pack_last(no_trailing, Some("file/a.zip"), true);
        assert!(!edit.text.ends_with('\n'), "原檔尾沒有換行就不要加：{:?}", edit.text);
        let (restored, _) = restore_lines(&edit.text, &["file/a.zip".into()], true, Some("en_us"));
        assert_eq!(restored, no_trailing);
        // 混用時以多數為準：3 個 LF、1 個 CRLF → 用 LF
        let mixed = "a:1\nb:2\nlang:en_us\r\nresourcePacks:[\"vanilla\"]\n";
        let edit = enable_pack_last(mixed, Some("file/a.zip"), false);
        assert!(!edit.text.contains("\r\n"), "{:?}", edit.text);
    }

    #[test]
    fn verify_catches_lost_packs() {
        let err = verify_enabled(SAMPLE, "resourcePacks:[\"file/tool.zip\"]\n", Some("file/tool.zip"), false).unwrap_err();
        assert!(err.contains("少了"));
    }
}
