//! `options.txt` 資源包清單的保護與修復。
//!
//! 背景（使用者實測的閃退，完整因果鏈）：
//! 1. 遊戲啟動時整合包自己的 ~150 個資源包全部不在 `resourcePacks:` 清單裡，
//!    只剩工具自己那一個
//! 2. 少了 `ProminenceFancyServerListing.zip` → 字體 builder 找不到
//!    `prominent:textures/gui/realms.png` → `Default font failed to load`
//! 3. 字體失敗讓整個資源重載壞掉 → 模型沒烘焙完
//! 4. 標題畫面渲染 Create 的按鈕 → `BakedModel` 是 null → 閃退
//!
//! 工具動了 `options.txt` 卻沒有驗證動完之後清單還完整。這個模組補上那道驗證，
//! 並提供「清單掉了就修回來」的能力。

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// 解析 `resourcePacks:[...]` 這一行，取出所有項目（去掉引號與空白）。
///
/// Minecraft 寫的是 JSON 陣列語法，但我們不需要完整 JSON 解析——項目一律是
/// 字串，用引號切開就夠，而且對半損毀的檔案更耐受。
pub fn parse_pack_list(options_text: &str) -> Vec<String> {
    let Some(line) = options_text
        .lines()
        .find(|l| l.starts_with("resourcePacks:"))
    else {
        return Vec::new();
    };
    let value = line.strip_prefix("resourcePacks:").unwrap_or("").trim();
    let mut out = Vec::new();
    let mut in_quote = false;
    let mut current = String::new();
    for c in value.chars() {
        match c {
            '"' => {
                if in_quote {
                    let entry = current.trim().to_string();
                    if !entry.is_empty() {
                        out.push(entry);
                    }
                    current.clear();
                }
                in_quote = !in_quote;
            }
            _ if in_quote => current.push(c),
            _ => {}
        }
    }
    out
}

/// 寫入後的驗證結果。`missing` 非空代表原本有的項目不見了。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackListDiff {
    /// 原本有、現在沒了的項目
    pub missing: Vec<String>,
    /// 這次新增的項目
    pub added: Vec<String>,
}

impl PackListDiff {
    pub fn is_safe(&self) -> bool {
        self.missing.is_empty()
    }
}

/// 比對「動之前」與「動之後」的清單，找出被弄丟的項目。
pub fn diff_pack_lists(before: &[String], after: &[String]) -> PackListDiff {
    let before_set: BTreeSet<&String> = before.iter().collect();
    let after_set: BTreeSet<&String> = after.iter().collect();
    PackListDiff {
        missing: before_set
            .difference(&after_set)
            .map(|s| (*s).clone())
            .collect(),
        added: after_set
            .difference(&before_set)
            .map(|s| (*s).clone())
            .collect(),
    }
}

/// 健檢報告：實際存在的資源包檔案 vs `options.txt` 啟用的清單。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackHealthReport {
    /// `resourcePacks:` 是不是完全空的（`[]`）——遊戲多半開不起來
    pub list_empty: bool,
    /// **工具動過、而且現在不見了**的項目——修復只處理這些。
    ///
    /// 不含「整合包自己附上但預設關閉」的第三方資源包：整合包作者常刻意
    /// 附一堆可選的 UI 風格／材質變體，全部啟用會改變整合包原本的樣子，
    /// 甚至互相衝突。那是使用者（或作者）的決定，工具沒有立場去動它。
    pub present_but_disabled: Vec<String>,
    /// 這台電腦上找得到的最近一次套用備份（沒有就是空字串）——說明用
    pub backup_used: String,
    /// 目前啟用的項目數
    pub enabled_count: usize,
    /// 資料夾裡的 zip 數
    pub available_count: usize,
    /// 給使用者看的一句話
    pub summary: String,
}

/// 檢查這個實例的資源包狀態。
pub fn check_pack_health(mc: &Path) -> PackHealthReport {
    let options = mc.join("options.txt");
    let text = fs::read_to_string(&options).unwrap_or_default();
    let enabled = parse_pack_list(&text);
    let has_line = text.lines().any(|l| l.starts_with("resourcePacks:"));

    let mut available: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(mc.join("resourcepacks")) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // 資料夾型資源包與 zip 都算
            if path.is_dir() || name.to_ascii_lowercase().ends_with(".zip") {
                available.push(name.to_string());
            }
        }
    }
    available.sort();

    let enabled_files: BTreeSet<String> = enabled
        .iter()
        .filter_map(|e| e.strip_prefix("file/").map(|s| s.to_string()))
        .collect();

    // ── 修復範圍：只還原「工具動過的」，不碰整合包原本的設定 ──
    //
    // 舊版把 resourcepacks/ 裡**所有**沒列進 options.txt 的檔案都算成待修復。
    // 但整合包作者常刻意附上預設關閉的可選資源包（UI 風格、材質變體），
    // 全部啟用會改變整合包原本的樣子，甚至互相衝突。
    //
    // 現在只收兩種：
    //   (1) 工具自己產出的翻譯資源包，檔案還在但清單裡沒有
    //   (2) 套用前備份的 options.txt 裡有、現在不見了、而且檔案還在的項目
    // 「本來就存在但沒啟用」的第三方資源包一律不碰、也不列出來。
    let (backup_enabled, backup_used) = read_backup_pack_list(mc);
    let backup_files: BTreeSet<String> = backup_enabled
        .iter()
        .filter_map(|e| e.strip_prefix("file/").map(|s| s.to_string()))
        .collect();
    let present_but_disabled: Vec<String> = available
        .iter()
        .filter(|name| !enabled_files.contains(*name))
        .filter(|name| {
            super::session::is_tool_resource_pack(name.trim_end_matches(".zip"))
                || backup_files.contains(*name)
        })
        .cloned()
        .collect();

    let list_empty = has_line && enabled.is_empty();
    let summary = if list_empty {
        "資源包清單是空的，遊戲很可能開不起來（字體與模型會載入失敗）。建議按下修復。".to_string()
    } else if !present_but_disabled.is_empty() {
        format!(
            "有 {} 個項目原本啟用著、現在從清單裡不見了。修復會把它們加回去；整合包原本就關著的資源包不會被動到。",
            present_but_disabled.len()
        )
    } else {
        "資源包清單看起來正常。".to_string()
    };

    PackHealthReport {
        list_empty,
        present_but_disabled,
        backup_used,
        enabled_count: enabled.len(),
        available_count: available.len(),
        summary,
    }
}

/// 讀最近一次「翻譯套用備份_*」裡的 `options.txt` 資源包清單。
///
/// 回傳 `(清單, 用到的備份資料夾名)`；找不到就回空的。
/// 這是「工具動之前長什麼樣」的唯一可靠依據——沒有它就無從分辨
/// 「原本啟用著被弄丟」與「整合包本來就沒啟用」。
fn read_backup_pack_list(mc: &Path) -> (Vec<String>, String) {
    let Ok(entries) = fs::read_dir(mc) else {
        return (Vec::new(), String::new());
    };
    let mut candidates: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .filter(|name| name.starts_with("翻譯套用備份_"))
        .collect();
    // 名稱含時間戳，字典序＝時間序，取最後一個就是最近一次
    candidates.sort();
    for name in candidates.iter().rev() {
        let path = mc.join(name).join("options.txt");
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let list = parse_pack_list(&text);
        if !list.is_empty() {
            return (list, name.clone());
        }
    }
    (Vec::new(), String::new())
}

/// 算出修復後的 `options.txt` 內容（只算、不寫）；不需要修復回傳 `None`。
/// 寫入由 pack_repair.rs 走前置檢查、紀錄、標記流程。
///
/// 刻意**只加不減**：使用者可能是刻意停用某些包的，我們無從分辨。
/// 但「清單全空」這種明顯壞掉的狀態一定要救回來。
/// 加回去的順序照檔名排序，工具自己的翻譯包放最後（優先權最高）。
pub fn repaired_options(mc: &Path, original: &str) -> Option<(String, usize)> {
    let report = check_pack_health(mc);
    if report.present_but_disabled.is_empty() {
        return None;
    }

    let mut enabled = parse_pack_list(original);
    let mut added = 0usize;
    let mut tool_packs = Vec::new();
    for name in &report.present_but_disabled {
        let entry = format!("file/{name}");
        if super::session::is_tool_resource_pack(name.trim_end_matches(".zip")) {
            tool_packs.push(entry);
        } else {
            enabled.push(entry);
        }
        added += 1;
    }
    // 工具的翻譯包排最後＝優先權最高，才蓋得過其他語言的資源包
    enabled.extend(tool_packs);
    // vanilla 一定要在最前面，否則基礎資源會缺
    if !enabled.iter().any(|e| e == "vanilla") {
        enabled.insert(0, "vanilla".to_string());
        added += 1;
    }

    let list = enabled
        .iter()
        .map(|e| format!("\"{e}\""))
        .collect::<Vec<_>>()
        .join(",");
    let new_line = format!("resourcePacks:[{list}]");

    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in original.lines() {
        if line.starts_with("resourcePacks:") {
            lines.push(new_line.clone());
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        lines.push(new_line);
    }
    let mut updated = lines.join("\n");
    updated.push('\n');
    Some((updated, added))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repair_pack_list(mc: &Path) -> Result<usize, String> {
        super::super::pack_repair::repair_pack_list_with_game_state(mc, super::super::game_process::GameRunning::No)
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mcpl-packguard-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("resourcepacks")).unwrap();
        dir
    }

    #[test]
    fn parses_minecraft_pack_list_syntax() {
        let text = "guiScale:3\nresourcePacks:[\"vanilla\",\"file/a.zip\",\"file/b.zip\"]\nfov:70\n";
        assert_eq!(
            parse_pack_list(text),
            vec!["vanilla", "file/a.zip", "file/b.zip"]
        );
        // 空清單與缺行都不能 panic
        assert!(parse_pack_list("resourcePacks:[]\n").is_empty());
        assert!(parse_pack_list("guiScale:3\n").is_empty());
    }

    #[test]
    fn diff_detects_entries_that_went_missing() {
        // 這正是使用者遇到的情況：動完之後只剩工具自己的包
        let before = vec![
            "vanilla".to_string(),
            "file/ProminenceFancyServerListing.zip".to_string(),
            "file/Prominent-UI-1.20.1.zip".to_string(),
        ];
        let after = vec!["file/模組包翻譯工具+0831+R1.zip".to_string()];
        let diff = diff_pack_lists(&before, &after);
        assert!(!diff.is_safe(), "弄丟了原本的資源包，必須判定為不安全");
        assert_eq!(diff.missing.len(), 3);
        assert_eq!(diff.added, vec!["file/模組包翻譯工具+0831+R1.zip"]);
    }

    #[test]
    fn diff_is_safe_when_only_appending() {
        let before = vec!["vanilla".to_string(), "file/a.zip".to_string()];
        let after = vec![
            "vanilla".to_string(),
            "file/a.zip".to_string(),
            "file/翻譯.zip".to_string(),
        ];
        assert!(diff_pack_lists(&before, &after).is_safe());
    }

    /// 造一份「套用前備份」，讓健檢知道工具動之前清單長什麼樣。
    fn write_backup(mc: &Path, stamp: &str, list: &[&str]) {
        let dir = mc.join(format!("翻譯套用備份_{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        let entries = list
            .iter()
            .map(|e| format!("\"{e}\""))
            .collect::<Vec<_>>()
            .join(",");
        fs::write(dir.join("options.txt"), format!("resourcePacks:[{entries}]\n")).unwrap();
    }

    #[test]
    fn health_check_flags_the_empty_list_that_caused_the_crash() {
        let mc = scratch("empty");
        fs::write(mc.join("options.txt"), "resourcePacks:[]\n").unwrap();
        fs::write(mc.join("resourcepacks/ProminenceFancyServerListing.zip"), b"x").unwrap();
        fs::write(mc.join("resourcepacks/Prominent-UI-1.20.1.zip"), b"x").unwrap();
        // 兩個套用前都啟用著，備份可以證明
        write_backup(
            &mc,
            "20260901-1200",
            &[
                "vanilla",
                "file/ProminenceFancyServerListing.zip",
                "file/Prominent-UI-1.20.1.zip",
            ],
        );

        let report = check_pack_health(&mc);
        assert!(report.list_empty, "全空清單要被標記出來");
        assert_eq!(report.present_but_disabled.len(), 2);
        assert!(report.summary.contains("開不起來"));
        assert!(
            report.backup_used.contains("20260901"),
            "要講清楚比對的是哪一份備份"
        );
        let _ = fs::remove_dir_all(&mc);
    }

    #[test]
    fn repair_never_enables_packs_the_modpack_shipped_disabled() {
        // 整合包作者常刻意附上預設關閉的可選資源包（UI 風格、材質變體）。
        // 舊版把資料夾裡所有沒啟用的檔案都算成待修復，等於把整合包原本的
        // 樣子改掉，甚至讓幾個材質包互相衝突。
        let mc = scratch("scope");
        fs::write(
            mc.join("options.txt"),
            "resourcePacks:[\"vanilla\",\"file/Base.zip\"]\n",
        )
        .unwrap();
        fs::write(mc.join("resourcepacks/Base.zip"), b"x").unwrap();
        // 整合包附的可選包，作者本來就沒啟用（備份裡也沒有）
        fs::write(mc.join("resourcepacks/Optional-UI-Style.zip"), b"x").unwrap();
        fs::write(mc.join("resourcepacks/Optional-Textures.zip"), b"x").unwrap();
        // 工具自己的翻譯包，檔案在但清單裡不見了
        fs::write(mc.join("resourcepacks/模組包翻譯工具+0831+R1.zip"), b"x").unwrap();
        write_backup(&mc, "20260901-1200", &["vanilla", "file/Base.zip"]);

        let report = check_pack_health(&mc);
        assert_eq!(
            report.present_but_disabled,
            vec!["模組包翻譯工具+0831+R1.zip"],
            "只該修工具自己的翻譯包，不該碰整合包原本關著的兩個：{:?}",
            report.present_but_disabled
        );

        repair_pack_list(&mc).unwrap();
        let list = parse_pack_list(&fs::read_to_string(mc.join("options.txt")).unwrap());
        assert!(
            !list.iter().any(|e| e.contains("Optional-")),
            "整合包原本關著的資源包不可以被啟用：{list:?}"
        );
        assert!(list.iter().any(|e| e.contains("模組包翻譯工具")));
        let _ = fs::remove_dir_all(&mc);
    }

    #[test]
    fn repair_restores_what_the_backup_says_used_to_be_enabled() {
        // 套用前備份是「工具動之前長什麼樣」的唯一依據。
        // 備份裡有、現在不見了、檔案還在 → 那是被弄丟的，要還原。
        let mc = scratch("frombackup");
        fs::write(mc.join("options.txt"), "resourcePacks:[\"vanilla\"]\n").unwrap();
        fs::write(mc.join("resourcepacks/ProminenceFancyServerListing.zip"), b"x").unwrap();
        fs::write(mc.join("resourcepacks/NeverEnabled.zip"), b"x").unwrap();
        write_backup(
            &mc,
            "20260901-1200",
            &["vanilla", "file/ProminenceFancyServerListing.zip"],
        );

        repair_pack_list(&mc).unwrap();
        let list = parse_pack_list(&fs::read_to_string(mc.join("options.txt")).unwrap());
        assert!(
            list.iter().any(|e| e.contains("ProminenceFancyServerListing")),
            "備份裡啟用著的要還原：{list:?}"
        );
        assert!(
            !list.iter().any(|e| e.contains("NeverEnabled")),
            "備份裡沒有的不可以順手啟用：{list:?}"
        );
        let _ = fs::remove_dir_all(&mc);
    }

    #[test]
    fn repair_restores_disabled_packs_and_keeps_tool_pack_last() {
        let mc = scratch("repair");
        fs::write(mc.join("options.txt"), "guiScale:3\nresourcePacks:[]\n").unwrap();
        fs::write(mc.join("resourcepacks/AAA.zip"), b"x").unwrap();
        fs::write(mc.join("resourcepacks/模組包翻譯工具+0831+R1.zip"), b"x").unwrap();
        // AAA.zip 套用前是啟用著的；沒有備份佐證的話它不在修復範圍內
        write_backup(&mc, "20260901-1200", &["vanilla", "file/AAA.zip"]);

        let added = repair_pack_list(&mc).unwrap();
        assert!(added >= 3, "兩個資源包＋vanilla 都要加回來，實得 {added}");

        let text = fs::read_to_string(mc.join("options.txt")).unwrap();
        let list = parse_pack_list(&text);
        assert_eq!(list.first().map(String::as_str), Some("vanilla"), "vanilla 要在最前面");
        assert!(
            list.last().map(|s| s.contains("模組包翻譯工具")).unwrap_or(false),
            "工具的翻譯包要在最後（優先權最高）：{list:?}"
        );
        // 其他設定不能被弄丟
        assert!(text.contains("guiScale:3"));
        // 修改前的備份要在
        assert!(mc.join("options.txt.mcpl-bak").is_file());
        let _ = fs::remove_dir_all(&mc);
    }

    #[test]
    fn repair_is_a_noop_when_everything_is_already_enabled() {
        let mc = scratch("noop");
        fs::write(
            mc.join("options.txt"),
            "resourcePacks:[\"vanilla\",\"file/AAA.zip\"]\n",
        )
        .unwrap();
        fs::write(mc.join("resourcepacks/AAA.zip"), b"x").unwrap();
        assert_eq!(repair_pack_list(&mc).unwrap(), 0);
        let _ = fs::remove_dir_all(&mc);
    }
}
