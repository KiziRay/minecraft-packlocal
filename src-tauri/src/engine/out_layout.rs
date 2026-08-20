//! 結果目錄配置：使用者只選「根目錄」，工具自行建立「翻譯結果」與子資料夾。
//! 不再要求結果必須是遊戲 resourcepacks。

use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// 固定結果資料夾名稱
pub const RESULT_DIR_NAME: &str = "翻譯結果";
/// 字體包專用結果資料夾（與翻譯完全分開）
pub const FONT_RESULT_DIR_NAME: &str = "字體結果";

/// 清掉翻譯中斷後留下的暫存，不碰玩家的翻譯結果與備份。
pub fn cleanup_transient_work(work_root: &Path) -> Result<(), String> {
    for name in [
        ".archive-overlay-stage",
        ".jar-display-stage",
        ".jar-patchouli-stage",
        ".jar-patchouli-translated",
    ] {
        let path = work_root.join(name);
        if path.exists() {
            fs::remove_dir_all(&path).map_err(|e| format!("無法清理暫存 {}：{e}", path.display()))?;
        }
    }
    for root_name in ["jar-translated", "resourcepacks-extra"] {
        let root = work_root.join(root_name);
        if !root.is_dir() {
            continue;
        }
        let temporary_files = WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
            })
            .map(|entry| entry.into_path())
            .collect::<Vec<_>>();
        for path in temporary_files {
            fs::remove_file(&path).map_err(|e| format!("無法清理暫存 {}：{e}", path.display()))?;
        }
    }
    Ok(())
}

/// 完整路徑組合都先算好，呼叫端取用哪幾個由流程決定（未取用的仍保留供診斷）。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ResultLayout {
    /// 使用者在 UI 選的根目錄
    pub user_base: PathBuf,
    /// 實際工作根：user_base/翻譯結果（或 user 已選到翻譯結果本身）
    pub work_root: PathBuf,
    /// work_root/resourcepacks — 放 zip 與工作用資料夾
    pub resourcepacks: PathBuf,
    /// work_root/config — 任務等設定輸出
    pub config: PathBuf,
    /// work_root/minemenu
    pub minemenu: PathBuf,
}

impl ResultLayout {
    pub fn readme_path(&self) -> PathBuf {
        self.work_root.join("【請閱讀】輸出說明.txt")
    }
}

/// 由使用者選擇的路徑解析並建立完整結果目錄樹。
pub fn ensure_result_layout(user_output: &Path) -> Result<ResultLayout, String> {
    if user_output.as_os_str().is_empty() {
        return Err("結果路徑是空的。".into());
    }
    fs::create_dir_all(user_output).map_err(|e| format!("無法建立結果根目錄：{e}"))?;

    let work_root = resolve_work_root(user_output);
    fs::create_dir_all(&work_root).map_err(|e| format!("無法建立「{RESULT_DIR_NAME}」：{e}"))?;
    cleanup_transient_work(&work_root)?;

    let resourcepacks = work_root.join("resourcepacks");
    let config = work_root.join("config");
    let minemenu = work_root.join("minemenu");
    fs::create_dir_all(&resourcepacks).map_err(|e| e.to_string())?;
    fs::create_dir_all(&config).map_err(|e| e.to_string())?;
    fs::create_dir_all(&minemenu).map_err(|e| e.to_string())?;

    let layout = ResultLayout {
        user_base: user_output.to_path_buf(),
        work_root: work_root.clone(),
        resourcepacks,
        config,
        minemenu,
    };
    write_readme(&layout)?;
    Ok(layout)
}

fn resolve_work_root(user_output: &Path) -> PathBuf {
    let name = user_output
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    // 使用者已選到「翻譯結果」本身
    if name == RESULT_DIR_NAME {
        return user_output.to_path_buf();
    }
    // 舊版：直接選 resourcepacks 當輸出 — 升到上一層再建翻譯結果，避免塞進遊戲資源包根
    if name.eq_ignore_ascii_case("resourcepacks") {
        if let Some(parent) = user_output.parent() {
            return parent.join(RESULT_DIR_NAME);
        }
    }
    user_output.join(RESULT_DIR_NAME)
}

fn resolve_font_work_root(user_output: &Path) -> PathBuf {
    let name = user_output
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if name == FONT_RESULT_DIR_NAME {
        return user_output.to_path_buf();
    }
    if name.eq_ignore_ascii_case("resourcepacks") {
        if let Some(parent) = user_output.parent() {
            return parent.join(FONT_RESULT_DIR_NAME);
        }
    }
    user_output.join(FONT_RESULT_DIR_NAME)
}

/// 字體包專用結果根：user_output/字體結果（不建翻譯 config 等）。
pub fn ensure_font_result_layout(user_output: &Path) -> Result<PathBuf, String> {
    if user_output.as_os_str().is_empty() {
        return Err("字體輸出路徑是空的。".into());
    }
    fs::create_dir_all(user_output).map_err(|e| format!("無法建立字體輸出根目錄：{e}"))?;
    let work_root = resolve_font_work_root(user_output);
    fs::create_dir_all(&work_root).map_err(|e| format!("無法建立「{FONT_RESULT_DIR_NAME}」：{e}"))?;
    let resourcepacks = work_root.join("resourcepacks");
    fs::create_dir_all(&resourcepacks).map_err(|e| e.to_string())?;
    let readme = work_root.join("【請閱讀】字體輸出說明.txt");
    if !readme.is_file() {
        let body = format!(
            "【字體資源包 — 輸出目錄說明】\n\
\n\
本資料夾只放字體資源包，與「翻譯結果」完全分開。\n\
\n\
  {FONT_RESULT_DIR_NAME}/\n\
    resourcepacks/  ← 建立的字體包資料夾\n\
    字體執行日誌.txt ← 字體分頁的操作紀錄（若有）\n\
\n\
工作根目錄：\n{}\n",
            work_root.display()
        );
        let _ = fs::write(readme, body);
    }
    Ok(work_root)
}

fn write_readme(layout: &ResultLayout) -> Result<(), String> {
    let body = format!(
        "【模組包繁中翻譯 — 輸出目錄說明】\n\
\n\
本資料夾由工具自動建立，請不要只把整個根目錄當 resourcepacks。\n\
每個整合包預設各有獨立結果位置（勿多包共用同一資料夾）。\n\
\n\
目錄結構：\n\
  {RESULT_DIR_NAME}/\n\
    resourcepacks/     ← 翻譯完成會直接套用；需要手動時才複製到遊戲 resourcepacks\n\
    config/ftbquests/  ← 任務／劇情：完成時依備份選項覆蓋到遊戲 config\\ftbquests\n\
    config/openloader/ ← 文字覆寫（若有）\n\
    config/starterkit/、armorsets/、minecolonies/ 等 ← 顯示型設定文字（若有）\n\
    patchouli_books/   ← 書本（若有）\n\
    kubejs/            ← 語言覆寫與安全白名單腳本字串（若有）\n\
    jar-translated/    ← 翻譯後 JAR 副本（套用時依備份選項放入 mods）\n\
    minemenu/          ← 若有快捷選單修正檔\n\
    翻譯工作階段.json  ← 補翻／修復用，勿亂刪\n\
    覆蓋範圍說明.txt   ← 會翻什麼／不會翻什麼（社群誠實原則）\n\
    翻譯錯誤日誌.txt、執行日誌.txt ← 回報用\n\
    字體包請用「字體」分頁，輸出到「字體結果/」，勿混在本資料夾。\n\
\n\
建議流程：\n\
1. 關閉遊戲\n\
2. 工具完成時會依選項備份並直接套用；只有手動處理時才複製\n\
3. 開遊戲 → 語言繁中（台灣）→ 啟用資源包\n\
\n\
工作根目錄：\n{}\n",
        layout.work_root.display()
    );
    fs::write(layout.readme_path(), body).map_err(|e| e.to_string())?;
    Ok(())
}

/// 覆蓋範圍報告：社群要求「不吹 100%」、說清楚會／不會翻什麼
#[derive(Debug, Clone)]
pub struct CoverageStats {
    pub keys_zh: usize,
    pub keys_pending: usize,
    pub keys_tw_playable: usize,
    pub keys_hk_hint: usize,
    pub ai_filled: usize,
    pub ai_enabled: bool,
    pub jars_scanned: usize,
    pub jars_rewritten: usize,
    pub jar_lang_files: usize,
    pub jar_errors: usize,
    pub quests_note: String,
    pub ref_note: String,
    pub pack_path: String,
    pub pack_format: u32,
    pub source_notes: Vec<String>,
    pub unsupported: Vec<String>,
    pub glossary_hits: usize,
    pub tm_hits: usize,
    pub shared_hits: usize,
    pub coverage_tier: String,
}

/// Max 档待補樣本：列出部分仍缺譯的 `namespace:key`（上限 sample_limit），不是完整盤點。
pub fn write_gap_summary_file(
    work_root: &Path,
    pending: &crate::engine::jar_scan::LangMap,
    sample_limit: usize,
) -> Result<PathBuf, String> {
    let path = work_root.join("待補缺口摘要.txt");
    let mut total = 0usize;
    let mut lines: Vec<String> = Vec::new();
    let mut namespaces: Vec<_> = pending.keys().cloned().collect();
    namespaces.sort();
    for ns in namespaces {
        let Some(map) = pending.get(&ns) else { continue };
        let mut keys: Vec<_> = map.keys().cloned().collect();
        keys.sort();
        for key in keys {
            total += 1;
            if lines.len() < sample_limit {
                lines.push(format!("{ns}:{key}"));
            }
        }
    }
    let shown = lines.len();
    let more = total.saturating_sub(shown);
    let mut body = String::from(
        "【待補缺口摘要】\n\
（Max 完整度選項產出；只列語言表層級仍缺譯的樣本鍵，不是完整五層盤點，也不代表遊戲內一定看得到這些字。）\n\n",
    );
    body.push_str(&format!("仍待補（粗估）：{total} 條\n"));
    body.push_str(&format!("本檔列出：{shown} 條"));
    if more > 0 {
        body.push_str(&format!("（另有約 {more} 條未列出）"));
    }
    body.push_str("\n\n");
    if lines.is_empty() {
        body.push_str("目前沒有待補英文鍵，或待補已清空。\n");
    } else {
        body.push_str("樣本：\n");
        for line in lines {
            body.push_str(&line);
            body.push('\n');
        }
    }
    fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn write_coverage_report(layout: &ResultLayout, stats: &CoverageStats) -> Result<PathBuf, String> {
    let path = layout.work_root.join("覆蓋範圍說明.txt");
    let covered_pct = if stats.keys_zh + stats.keys_pending > 0 {
        (stats.keys_tw_playable as f64 * 100.0) / (stats.keys_zh + stats.keys_pending) as f64
    } else {
        0.0
    };
    let _ = covered_pct;
    let source_summary = if stats.ai_enabled {
        format!("（含本機合併與 AI 新補；AI 本次新寫入約 {} 條）", stats.ai_filled)
    } else {
        "（只含本機合併與內建轉換）".to_string()
    };
    let reference_summary = if stats.ai_enabled {
        "3. 可合併社群／先前全翻參考包；AI 只補「仍是英文」的字\n"
    } else {
        "3. 可合併社群／先前全翻參考包；不使用線上翻譯服務\n"
    };
    let ai_rule = if stats.ai_enabled {
        "4. 不用 AI 做分類找檔；AI 只翻字串\n"
    } else {
        "4. 不用線上服務做分類找檔；掃描與分類都在本機完成\n"
    };
    let redline4 = if stats.ai_enabled {
        "4. 線上翻譯可能有錯；歡迎用診斷頁回報。不滿意請先還原上次套用。\n"
    } else {
        "4. 不把金鑰寫進分享檔或公開貼文\n"
    };
    let handling_34 = format!("{reference_summary}{ai_rule}");
    let source_lines = if stats.source_notes.is_empty() {
        "• 本次沒有額外來源統計。\n".to_string()
    } else {
        stats
            .source_notes
            .iter()
            .map(|line| format!("• {line}\n"))
            .collect::<String>()
    };
    let unsupported_lines = if stats.unsupported.is_empty() {
        "• 本次沒有記錄到額外略過原因。\n".to_string()
    } else {
        stats
            .unsupported
            .iter()
            .map(|line| format!("• {line}\n"))
            .collect::<String>()
    };
    let body = format!(
        "【覆蓋範圍說明 — 請先讀】\n\
（依全球 Minecraft／整合包玩家社群常見期望撰寫；本工具不宣稱 100% 漢化）\n\
\n\
═══ 這次大概蓋到什麼 ═══\n\
• 【台灣繁中已覆蓋】約 {} 條（不含純港繁提示）\n\
• 【港繁提示已轉台】約 {} 條（仍可能需補缺）\n\
• 【仍待譯】約 {} 條\n\
• 中文鍵合計約 {} 條{}\n\
• 掃過模組 jar 約 {} 個；翻譯副本重建 {} 個、寫入 {} 個語言檔、{} 個失敗\n\
• 完整度授權：{}\n\
• 補譯命中：術語表 {}／翻譯記憶 {}／共享庫 {}\n\
• 資源包：{}\n\
• pack_format：{}（不相容時遊戲會提示，可回報版本）\n\
• 參考包：{}\n\
• 任務／劇情：{}\n\
\n\
═══ 會處理（期望對齊）═══\n\
1. mods／資源包／KubeJS 等語言檔 → 產出 zh_tw 資源包 zip\n\
2. 簡中 → 台灣繁體（OpenCC s2twp，本機）\n\
{}\
5. FTB Quests 任務文字（輸出到 config/ftbquests，完成時直接套用）\n\
6. 文字覆寫：patchouli_books／openloader／kubejs／顯示型 config 等（完成時直接套用）\n\
7. 任務／書本系統：Better Questing／HQM／Heracles／Modonomicon（顯示欄位，best-effort）\n\
8. Origins／Apoli 能力名稱與說明（路徑感知，不動識別字）\n\
9. JAR 原檔只讀；翻譯副本是否在套用前備份同名 mods 檔，由玩家選項決定\n\
\n\
═══ 本次來源明細 ═══\n\
{}\
\n\
═══ 通常蓋不到／仍可能英文（誠實列出）═══\n\
1. 圖片上的字（紅線，本工具不處理圖片）\n\
2. 寫死在 Java 程式碼／KubeJS 任意腳本邏輯裡的字串（不解析程式碼；僅處理明確白名單的顯示 API）\n\
3. 特殊或動態生成的 Markdown 結構仍可能需要手動檢查；一般文字 ZIP 已會安全重建\n\
4. 基岩版（Bedrock）整合包（格式不同；市集加密包無法合規自動翻）\n\
5. 未掃到的特殊格式、動態生成文字\n\
6. 世界閃退、缺模組、結構包問題（與翻譯無關，本工具不修）\n\
7. 機翻腔、專有名詞不一致（社群包也會寫「不保證完美」）\n\
8. Essential 等客戶端模組：部分 UI 寫死在 class／快取，資源包無法覆蓋（見手翻 UNTRANSLATABLE 說明）\n\
9. MIDI／Drop Rate 等若僅在介面或 class 硬編碼、無 lang 鍵，本工具無法翻譯\n\
10. 僅含港繁（zh_hk）的模組不算台灣繁中已完成\n\
\n\
本次略過／需要人工檢查：\n\
{}\
\n\
═══ 不該做的事（本工具紅線）═══\n\
1. 不直接改 mods/*.jar 原檔；只建立翻譯副本並在套用時替換\n\
2. 不把機翻吹成官方全漢化\n\
3. 不會自行刪除原任務／原資源包；是否建立套用備份由玩家選項決定\n\
{}\
5. 不把簡中當繁中交差\n\
\n\
═══ 建議你怎麼用 ═══\n\
1. 關遊戲 → 等工具完成直接套用；若要手動安裝再複製 zip／任務\n\
2. 語言選繁體中文（台灣）並啟用資源包\n\
3. 若遊戲仍英文，請查看上方「港繁提示／仍待譯」欄\n\
4. 不滿意用備份還原；可「只補缺漏」續跑；閃退請先還原再診斷回報\n\
\n\
產生位置：\n{}\n",
        stats.keys_tw_playable,
        stats.keys_hk_hint,
        stats.keys_pending,
        stats.keys_zh,
        source_summary,
        stats.jars_scanned,
        stats.jars_rewritten,
        stats.jar_lang_files,
        stats.jar_errors,
        if stats.coverage_tier.is_empty() {
            "standard"
        } else {
            stats.coverage_tier.as_str()
        },
        stats.glossary_hits,
        stats.tm_hits,
        stats.shared_hits,
        stats.pack_path,
        stats.pack_format,
        if stats.ref_note.is_empty() {
            "無"
        } else {
            stats.ref_note.as_str()
        },
        if stats.quests_note.is_empty() {
            "無"
        } else {
            stats.quests_note.as_str()
        },
        handling_34,
        source_lines,
        unsupported_lines,
        redline4,
        layout.work_root.display()
    );
    fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path)
}

/// 尋找工作階段時要掃的目錄（相容舊輸出）
pub fn layout_search_bases(user_or_work: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let push = |list: &mut Vec<PathBuf>, p: PathBuf| {
        if !list.iter().any(|x| x == &p) {
            list.push(p);
        }
    };
    push(&mut v, user_or_work.to_path_buf());
    let name = user_or_work
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if name == RESULT_DIR_NAME {
        if let Some(parent) = user_or_work.parent() {
            push(&mut v, parent.to_path_buf());
        }
    } else {
        push(&mut v, user_or_work.join(RESULT_DIR_NAME));
    }
    // 舊版：根目錄直接放 session／resourcepacks
    push(&mut v, user_or_work.join("resourcepacks"));
    if let Some(parent) = user_or_work.parent() {
        push(&mut v, parent.join(RESULT_DIR_NAME));
        push(&mut v, parent.to_path_buf());
    }
    v
}

/// 建議「結果根目錄」：遊戲實例旁的專用資料夾（不是 resourcepacks）
pub fn suggest_output_base(instance_path: &Path) -> Result<PathBuf, String> {
    let mc = if instance_path.join("mods").is_dir() {
        instance_path.to_path_buf()
    } else if instance_path.join("minecraft").join("mods").is_dir() {
        instance_path.join("minecraft")
    } else if instance_path.join(".minecraft").join("mods").is_dir() {
        instance_path.join(".minecraft")
    } else {
        // 仍允許：用實例資料夾本身
        instance_path.to_path_buf()
    };
    // 放在實例根（…/Craft_to_Exile_2）底下較清楚
    let instance_root = if mc.file_name().and_then(|s| s.to_str()) == Some("minecraft")
        || mc.file_name().and_then(|s| s.to_str()) == Some(".minecraft")
    {
        mc.parent().unwrap_or(&mc).to_path_buf()
    } else {
        mc
    };
    let base = instance_root.join("繁中翻譯輸出");
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn gap_summary_lists_sample_keys() {
        let root = std::env::temp_dir().join(format!("gap_summary_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut pending = HashMap::new();
        let mut map = HashMap::new();
        map.insert("item.a".into(), "A".into());
        map.insert("item.b".into(), "B".into());
        pending.insert("demo".into(), map);
        let path = write_gap_summary_file(&root, &pending, 10).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("demo:item.a"), "{body}");
        assert!(body.contains("仍待補（粗估）：2"), "{body}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn coverage_report_omits_ai_when_disabled() {
        let root = std::env::temp_dir().join(format!("coverage_report_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let work = root.join(RESULT_DIR_NAME);
        fs::create_dir_all(&work).unwrap();
        let layout = ResultLayout {
            user_base: root.clone(),
            work_root: work,
            resourcepacks: root.join(RESULT_DIR_NAME).join("resourcepacks"),
            config: root.join(RESULT_DIR_NAME).join("config"),
            minemenu: root.join(RESULT_DIR_NAME).join("minemenu"),
        };
        let path = write_coverage_report(
            &layout,
            &CoverageStats {
                keys_zh: 3,
                keys_pending: 1,
                keys_tw_playable: 3,
                keys_hk_hint: 0,
                ai_filled: 0,
                ai_enabled: false,
                jars_scanned: 1,
                jars_rewritten: 1,
                jar_lang_files: 1,
                jar_errors: 0,
                quests_note: String::new(),
                ref_note: "未找到參考包。".into(),
                pack_path: "pack.zip".into(),
                pack_format: 15,
                source_notes: Vec::new(),
                unsupported: Vec::new(),
                glossary_hits: 0,
                tm_hits: 0,
                shared_hits: 0,
                coverage_tier: "standard".into(),
            },
        )
        .unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert!(!text.contains("AI"));
        assert!(text.contains("不使用線上翻譯服務"));
        let _ = fs::remove_dir_all(root);
    }
}
