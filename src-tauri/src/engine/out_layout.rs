//! 結果目錄配置：使用者只選「根目錄」，工具自行建立「翻譯結果」與子資料夾。
//! 不再要求結果必須是遊戲 resourcepacks。

use std::fs;
use std::path::{Path, PathBuf};

/// 固定結果資料夾名稱
pub const RESULT_DIR_NAME: &str = "翻譯結果";

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

fn write_readme(layout: &ResultLayout) -> Result<(), String> {
    let body = format!(
        "【模組包繁中翻譯 — 輸出目錄說明】\n\
\n\
本資料夾由工具自動建立，請不要只把整個根目錄當 resourcepacks。\n\
\n\
目錄結構：\n\
  {RESULT_DIR_NAME}/\n\
    resourcepacks/     ← 把這裡的 .zip 複製到遊戲 resourcepacks 並啟用\n\
    config/ftbquests/  ← 任務／劇情：備份後覆蓋到遊戲 config\\ftbquests\n\
    config/openloader/ ← 文字覆寫（若有）\n\
    patchouli_books/   ← 書本（若有）\n\
    kubejs/            ← 語言覆寫（若有）\n\
    minemenu/          ← 若有快捷選單修正檔\n\
    翻譯工作階段.json  ← 補翻／修復用，勿亂刪\n\
    覆蓋範圍說明.txt   ← 會翻什麼／不會翻什麼（社群誠實原則）\n\
\n\
建議流程：\n\
1. 關閉遊戲\n\
2. 用工具「一鍵套用到遊戲」（會先備份）或手動複製\n\
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
    pub ai_filled: usize,
    pub jars_scanned: usize,
    pub quests_note: String,
    pub ref_note: String,
    pub pack_path: String,
    pub pack_format: u32,
}

pub fn write_coverage_report(layout: &ResultLayout, stats: &CoverageStats) -> Result<PathBuf, String> {
    let path = layout.work_root.join("覆蓋範圍說明.txt");
    let covered_pct = if stats.keys_zh + stats.keys_pending > 0 {
        (stats.keys_zh as f64 * 100.0) / (stats.keys_zh + stats.keys_pending) as f64
    } else {
        0.0
    };
    let body = format!(
        "【覆蓋範圍說明 — 請先讀】\n\
（依全球 Minecraft／整合包玩家社群常見期望撰寫；本工具不宣稱 100% 漢化）\n\
\n\
═══ 這次大概蓋到什麼 ═══\n\
• 中文鍵約 {} 條（含本機合併＋AI 新補）\n\
• AI 本次新寫入約 {} 條\n\
• 仍待補英文約 {} 條（語言檔層級粗估完成度約 {:.1}%）\n\
• 掃過模組 jar 約 {} 個（只讀語言檔，不改 jar）\n\
• 資源包：{}\n\
• pack_format：{}（不相容時遊戲會提示，可回報版本）\n\
• 參考包：{}\n\
• 任務／劇情：{}\n\
\n\
═══ 會處理（期望對齊）═══\n\
1. mods／資源包／KubeJS 等語言檔 → 產出 zh_tw 資源包 zip\n\
2. 簡中 → 台灣繁體（OpenCC s2twp，本機）\n\
3. 可合併社群／先前全翻參考包，AI 只補「仍是英文」的字\n\
4. FTB Quests 任務文字（輸出到 config/ftbquests，需套用才進遊戲）\n\
5. 文字覆寫：patchouli_books／openloader／kubejs 等（需套用才進遊戲）\n\
6. 任務／書本系統：Better Questing／HQM／Heracles／Modonomicon（顯示欄位，best-effort）\n\
7. Origins／Apoli 能力名稱與說明（路徑感知，不動識別字）\n\
8. 一鍵套用前會備份會被覆蓋的檔案（不改 mods/*.jar）\n\
\n\
═══ 通常蓋不到／仍可能英文（誠實列出）═══\n\
1. 圖片上的字（紅線，本工具不處理圖片）\n\
2. 寫死在 Java 程式碼／KubeJS 腳本裡的字串（不解析程式碼）\n\
3. GuideME 的 Markdown 書本、被壓成 .zip 的資料包（本版尚未支援，未來可加）\n\
4. 基岩版（Bedrock）整合包（格式不同，不支援）\n\
5. 未掃到的特殊格式、動態生成文字\n\
6. 世界閃退、缺模組、結構包問題（與翻譯無關，本工具不修）\n\
7. 機翻腔、專有名詞不一致（社群包也會寫「不保證完美」）\n\
\n\
═══ 不該做的事（本工具紅線）═══\n\
1. 不直接改 mods/*.jar 內容\n\
2. 不把機翻吹成官方全漢化\n\
3. 不刪你的原任務／原資源包而不備份（套用會先備份）\n\
4. 不用 AI 做分類找檔；AI 只翻字串\n\
5. 不把簡中當繁中交差\n\
\n\
═══ 建議你怎麼用 ═══\n\
1. 關遊戲 → 工具「一鍵套用到遊戲」或手動複製 zip／任務\n\
2. 語言選繁體中文（台灣）並啟用資源包\n\
3. 不滿意用備份還原；可「只補缺漏」續跑\n\
\n\
產生位置：\n{}\n",
        stats.keys_zh,
        stats.ai_filled,
        stats.keys_pending,
        covered_pct,
        stats.jars_scanned,
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
