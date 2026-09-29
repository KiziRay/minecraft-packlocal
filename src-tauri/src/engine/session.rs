//! 翻譯工作階段：讓「補翻」不必重掃 mods。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::jar_scan::{resolve_minecraft_dir, LangMap};
use super::out_layout::{layout_search_bases, RESULT_DIR_NAME};
use super::pack_out::resourcepacks_root;

pub const SESSION_FILE: &str = "翻譯工作階段.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateSession {
    pub version: u32,
    /// Number of complete review passes after the first translation.
    #[serde(default)]
    pub review_pass: u32,
    pub instance_path: String,
    pub output_dir: String,
    pub pack_name: String,
    pub pack_path: String,
    /// 仍缺中文、可用 AI 補的英文原文
    pub pending_en: LangMap,
    pub pending_count: usize,
    /// AI 品質檢查失敗、一般補充時暫不重送的項目。
    /// 舊版工作階段沒有此欄位時，視為尚未建立暫緩清單。
    #[serde(default)]
    pub quality_deferred: LangMap,
    pub keys_zh: usize,
    /// 港繁提示轉台條數（舊檔缺欄視為 0）
    #[serde(default)]
    pub keys_hk_hint: usize,
    pub note: String,
    /// 使用者指定的目標 MC 版本（補翻／修復重建 pack.mcmeta 時沿用）。舊檔沒有 → None。
    #[serde(default)]
    pub target_version: Option<String>,
    #[serde(default = "default_translation_mode")]
    pub translation_mode: String,
    #[serde(default = "default_translation_quality")]
    pub translation_quality: String,
    /// 完整度授權（quick／standard／max）；舊工作階段缺欄位時視為 standard。
    #[serde(default = "default_coverage_tier")]
    pub coverage_tier: String,
    /// 建立工作階段當下，`mods/` 資料夾內容的指紋（檔名＋大小）。
    ///
    /// 舊版「本機已有翻譯」只比對 `instance_path` 字串是否相同——如果使用者把同一個
    /// 資料夾路徑改裝成完全不同的整合包（啟動器常見操作：換掉 mods/ 但沿用同一個
    /// instance 目錄），路徑比對照樣通過，工具會拿舊整合包的翻譯階段去對新整合包，
    /// 顯示「本機已有翻譯，可接續補翻」，接續下去等於把舊翻譯套進錯的整合包。
    /// 0＝舊工作階段沒有這個欄位（遷移前），比對時視為「未知，跳過檢查」，不誤擋。
    #[serde(default)]
    pub mods_fingerprint: u64,
    /// 這一包的執行偏好，中斷續翻時要沿用。
    ///
    /// 使用者實測回報：選了「不保留翻譯結果、不備份、直接覆蓋」之後，工具中途
    /// 被關掉；重開續翻時又產出了檔案。原因是那個選擇只活在當次執行的前端變數裡，
    /// 工具一關就沒了，續翻走的是設定裡的預設值。存進工作階段才能跨重啟沿用。
    #[serde(default)]
    pub run_preferences: RunPreferences,
    /// 上一次執行是**跑完**、被取消、還是中途出錯。
    ///
    /// 為什麼需要：`pending_count` 是「本地整理完成當下」寫進來的快照，
    /// 之後被共享庫／術語表補掉的條目不會回寫。實測站長那一包在 63% 崩潰，
    /// 留下 `pendingCount=33233`，但日誌顯示其中 32151 條早就被共享庫補上了。
    /// 重開工具讀到這個過期數字，就會謊報「還有三萬多條缺漏」。
    ///
    /// 所以：**只有 `completed` 的計數才可以拿來對使用者講缺漏**。
    /// 其餘狀態一律交給「上次的翻譯沒有做完」接續卡處理。
    #[serde(default)]
    pub last_run_outcome: RunOutcome,
    /// B6a-1：判斷整合包有沒有更新的依據（英文雜湊 sourceHashes、MC 版本 mcVersion、模組清單 modFiles）。
    /// 舊工作階段沒有這些欄位＝無法確認，不判「已更新」「版本變了」；補翻時重掃一次補上。
    #[serde(flatten)]
    pub update_basis: super::pack_update::UpdateBasis,
}

/// 上一次執行的收尾狀態。舊工作階段沒有這個欄位 → `Unknown`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// 舊檔沒有這個欄位；當成「不確定」，不拿計數對使用者講缺漏
    #[default]
    Unknown,
    /// 正常跑完，計數是新鮮的
    Completed,
    /// 使用者自己按停止
    Aborted,
    /// 出錯或崩潰中斷
    Crashed,
}

impl RunOutcome {
    /// 這次的計數可以拿來對使用者講「還缺幾條」嗎？
    pub fn counts_are_trustworthy(self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// 跨重啟需要沿用的執行偏好與進度。全部欄位都有預設值，舊工作階段可直接反序列化。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPreferences {
    /// 不保留「翻譯結果」資料夾（完成後清掉）。
    #[serde(default)]
    pub skip_result_folder: bool,
    /// 套用前是否備份。`false` 代表使用者明確選了「直接覆蓋、不備份」。
    #[serde(default)]
    pub backup_before_apply: bool,
    /// 這一輪用的 AI 來源（local／custom／gpt／空＝未使用 AI）。
    #[serde(default)]
    pub ai_mode: String,
    /// 每個步驟已累積的毫秒數，續翻時接續顯示而不是從 0 重算。
    #[serde(default)]
    pub step_elapsed_ms: HashMap<String, u64>,
}

/// 對 `instance/mods/` 底下的 `.jar` 檔名＋大小做一次輕量指紋，偵測「這還是同一包嗎」。
///
/// 只挑檔名＋大小、不讀檔案內容：整合包常有兩三百個 mod，逐一算雜湊太慢；
/// 檔名＋大小已經足夠分辨「換了一批完全不同的 mod」這種情況，不需要密碼學等級的比對。
/// 讀不到 `mods/` 資料夾（尚未解壓、路徑錯誤等）回 0，等同「未知」，比對時不擋人。
///
/// `instance` 可以是啟動器的頂層實例資料夾（例如 PrismLauncher 的
/// `instances\某整合包\`，`mods/` 實際在其巢狀的 `minecraft/mods/` 底下），
/// 也可以是已經解析好的 `.minecraft` 對等資料夾——一律先用 `resolve_minecraft_dir`
/// 正規化再找 `mods/`。過去這裡直接 `instance.join("mods")`，對巢狀結構的實例
/// （PrismLauncher 最常見的佈局）永遠讀不到，指紋永遠是 0，讓「換了不同整合包」
/// 的防呆從第九輪上線後對這類實例一直是靜默失效的狀態。
pub fn mods_fingerprint(instance: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mc = resolve_minecraft_dir(instance).unwrap_or_else(|_| instance.to_path_buf());
    let mods_dir = mc.join("mods");
    let Ok(entries) = fs::read_dir(&mods_dir) else {
        return 0;
    };
    let mut names: Vec<(String, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str())?.eq_ignore_ascii_case("jar") {
                let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
                let size = e.metadata().ok()?.len();
                Some((name, size))
            } else {
                None
            }
        })
        .collect();
    if names.is_empty() {
        return 0;
    }
    names.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    names.hash(&mut hasher);
    hasher.finish()
}

/// 在 `instance` 的同層資料夾（例如 PrismLauncher 的 `instances\` 這一層）裡，
/// 找出模組內容（`mods_fingerprint`）跟它完全一樣的其他資料夾。
///
/// 用途：使用者可能把實例資料夾改名（例如複製一份改叫 `ct`），導致工具記住、
/// 也正確套用的仍是原本那個資料夾，使用者卻在改名後的那份裡找翻譯——套用
/// 機制本身沒有錯，只是套用對象跟使用者以為的不是同一個。找到指紋相同的
/// 其他資料夾時只回傳清單讓呼叫端提醒，不會阻擋或影響套用流程本身。
///
/// 指紋是 0（讀不到 `mods/`）、沒有上層資料夾、或掃描上層資料夾失敗，一律
/// 回傳空清單——這是提醒用的資訊，寧可漏掉也不要在正常情況下誤報。
pub fn find_sibling_instances_with_same_mods(instance: &Path) -> Vec<PathBuf> {
    let fp = mods_fingerprint(instance);
    if fp == 0 {
        return Vec::new();
    }
    let Some(parent) = instance.parent() else {
        return Vec::new();
    };
    let canon_self = fs::canonicalize(instance).unwrap_or_else(|_| instance.to_path_buf());
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| fs::canonicalize(p).unwrap_or_else(|_| p.clone()) != canon_self)
        .filter(|p| mods_fingerprint(p) == fp)
        .collect()
}

fn default_translation_mode() -> String {
    "append".into()
}

fn default_translation_quality() -> String {
    "balanced".into()
}

fn default_coverage_tier() -> String {
    "max".into()
}

/// 可能存放工作階段的目錄（翻譯結果／舊版相容）
pub fn session_search_dirs(output_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let push = |v: &mut Vec<PathBuf>, p: PathBuf| {
        if !v.iter().any(|x| x == &p) {
            v.push(p);
        }
    };
    for b in layout_search_bases(output_dir) {
        push(&mut dirs, b.clone());
        push(&mut dirs, b.join(RESULT_DIR_NAME));
        push(&mut dirs, resourcepacks_root(&b));
        push(&mut dirs, b.join(RESULT_DIR_NAME).join("resourcepacks"));
    }
    dirs
}

pub fn find_session_file(output_dir: &Path) -> Option<PathBuf> {
    for d in session_search_dirs(output_dir) {
        let p = d.join(SESSION_FILE);
        if p.is_file() {
            return Some(p);
        }
        // 正式檔不見、但暫存檔還在：代表上次剛好在「刪舊檔 → 改名」之間被中斷。
        // 那份暫存檔是完整寫完的，直接拿來用比讓使用者重翻一整包好得多。
        let tmp = p.with_extension("json.tmp");
        if tmp.is_file() {
            if fs::rename(&tmp, &p).is_ok() {
                return Some(p);
            }
            return Some(tmp);
        }
    }
    None
}

/// 寫出工作階段。**必須是原子的**。
///
/// 這個檔案在大型整合包上實測有 2.6 MB，而且翻譯過程中會反覆重寫。
/// 舊版直接 `fs::write` 覆蓋原檔：寫到一半工具被關掉、當機或斷電，就會留下
/// 一份被截斷的 JSON——`load_session()` 解析失敗，於是**整輪翻譯的待補清單、
/// 品質暫緩清單與執行偏好全部救不回來**，使用者只能從頭重翻。使用者實測遇過
/// 「翻到一半關掉工具」，正是會踩到這條的情境。
///
/// 改成「先寫暫存檔、再 rename 蓋上去」：rename 在同一個磁碟區上是原子操作，
/// 中途失敗時原本那份完整的舊檔仍然在，最多損失這一次的更新。
pub fn save_session(output_dir: &Path, session: &TranslateSession) -> Result<(), String> {
    // output_dir 應為「翻譯結果」工作根
    fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    let primary = output_dir.join(SESSION_FILE);
    let s = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
    let tmp = primary.with_extension("json.tmp");
    fs::write(&tmp, s + "\n").map_err(|e| format!("寫入工作階段暫存檔失敗：{e}"))?;
    // Windows 的 rename 不能覆蓋既有檔案，得先移除舊的。
    // 這一瞬間如果斷電，暫存檔還在，下次 load_session 會從它復原（見 find_session_file）。
    if primary.exists() {
        fs::remove_file(&primary).map_err(|e| format!("置換工作階段檔失敗：{e}"))?;
    }
    fs::rename(&tmp, &primary).map_err(|e| format!("儲存工作階段失敗：{e}"))?;
    crate::dev_log!(
        "session",
        "寫入工作階段 {} 待譯命名空間={} 待譯條數={}",
        primary.display(),
        session.pending_en.len(),
        session.pending_en.values().map(|m| m.len()).sum::<usize>()
    );
    Ok(())
}

pub fn load_session(output_dir: &Path) -> Result<(TranslateSession, PathBuf), String> {
    let p = find_session_file(output_dir).ok_or_else(|| {
        format!(
            "找不到「{}」。\n\
已檢查：\n{}\n\
請確認「結果存哪」與上次相同，或先再跑一次「開始一鍵翻譯」。",
            SESSION_FILE,
            session_search_dirs(output_dir)
                .iter()
                .map(|d| format!("• {}", d.display()))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })?;
    let t = fs::read_to_string(&p).map_err(|e| e.to_string())?;
    // 舊版工作階段缺少的欄位由 serde 預設值補齊；沒有原文雜湊的標記為需重掃（B6 使用）。
    let (session, needs_rescan) = super::migrate::read_session_text(&t)
        .map_err(|e| format!("工作階段檔損壞（{}）：{e}", p.display()))?;
    if needs_rescan {
        crate::dev_log!("session", "舊版工作階段沒有原文雜湊，標記需重掃：{}", p.display());
    }
    Ok((session, p))
}

pub fn has_session_file(output_dir: &Path) -> bool {
    find_session_file(output_dir).is_some()
}

/// 讀取已產出資源包裡的 zh_tw.json（資料夾或 .zip）
pub fn load_pack_zh(pack_path: &Path) -> Result<LangMap, String> {
    super::pack_out::load_pack_zh_any(pack_path)
}

/// 從 pending 去掉「有效中文」的 key；假中文／混雜仍算待補。
pub fn remaining_pending(pending: &LangMap, zh: &LangMap) -> LangMap {
    let mut out: LangMap = HashMap::new();
    for (ns, map) in pending {
        for (k, en) in map {
            if en.trim().is_empty() {
                continue;
            }
            let zh_val = zh.get(ns).and_then(|m| m.get(k));
            let done = zh_val
                .map(|z| super::translation_quality::is_usable_zh(en, z))
                .unwrap_or(false);
            if !done {
                out.entry(ns.clone())
                    .or_default()
                    .insert(k.clone(), en.clone());
            }
        }
    }
    out
}

/// 從本輪可送出的 pending 中排除已經因品質失敗而暫緩的項目。
/// 只比對 namespace/key，不以原文作為唯一鍵，避免同一句英文在不同欄位互相影響。
pub fn filter_quality_deferred(pending: &mut LangMap, deferred: &LangMap) -> usize {
    let mut removed = 0usize;
    for (ns, map) in pending.iter_mut() {
        let Some(blocked) = deferred.get(ns) else {
            continue;
        };
        let before = map.len();
        map.retain(|key, _| !blocked.contains_key(key));
        removed += before.saturating_sub(map.len());
    }
    pending.retain(|_, map| !map.is_empty());
    removed
}

/// 翻譯成功後移除已不再需要暫緩的項目；品質不合格或仍缺漏的項目會保留。
pub fn prune_quality_deferred(deferred: &mut LangMap, zh: &LangMap) {
    for (ns, map) in deferred.iter_mut() {
        if let Some(zh_map) = zh.get(ns) {
            map.retain(|key, source| {
                zh_map
                    .get(key)
                    .map(|translated| !super::translation_quality::is_usable_zh(source, translated))
                    .unwrap_or(true)
            });
        }
    }
    deferred.retain(|_, map| !map.is_empty());
}

/// 將資源包內不合格譯文（仍英／混雜）重新列入待補；來源字串用現有值（常即英文或碎片）。
pub fn rework_unusable_zh(zh: &LangMap) -> LangMap {
    use super::translation_quality::{is_mixed_fragment, is_still_english, is_usable_zh};
    let mut out: LangMap = HashMap::new();
    for (ns, map) in zh {
        for (k, v) in map {
            if v.trim().is_empty() {
                continue;
            }
            if is_usable_zh("", v) {
                continue;
            }
            if is_still_english(v) || is_mixed_fragment(v) {
                out.entry(ns.clone())
                    .or_default()
                    .insert(k.clone(), v.clone());
            }
        }
    }
    out
}

/// 合併兩份 pending（後者覆蓋同 key）。
pub fn merge_pending(into: &mut LangMap, extra: &LangMap) {
    for (ns, map) in extra {
        let slot = into.entry(ns.clone()).or_default();
        for (k, v) in map {
            slot.insert(k.clone(), v.clone());
        }
    }
}

pub fn count_map(m: &LangMap) -> usize {
    m.values().map(|x| x.len()).sum()
}

/// 本機略過免譯字串（資源 id、純數字、URL 等），避免進 pending／送 AI。
///
/// 同時看**鍵名**與**值**：有些字光看值分不出來（`"1.0.0"` 可能是版本號也可能
/// 是遊戲內文字），但鍵名 `htp_metadata_version` 講得很清楚。實測資料顯示
/// 1361 條待補裡有數百條屬於這類「鍵名就已經說明不該翻」的項目。
pub fn filter_local_untranslatable(pending: &mut LangMap) -> usize {
    use super::eligibility::{classify, Candidate};
    use super::translation_quality::source_stays_unchanged;
    let mut skipped = 0usize;
    for map in pending.values_mut() {
        let before = map.len();
        map.retain(|k, v| {
            // 走集中判定（P0-03）：語言 key、結構欄位、單位符號等不算待補缺口。
            // 舊版在這裡只把 value 丟給 looks_untranslatable，於是 `botania.entry.x`
            // 這種東西會留在 pending 裡，讓覆蓋報告出現「還缺 908 條」的假缺口。
            let verdict = classify(Candidate { source_kind: "lang", logical_key: k, text: v });
            verdict.counts_as_pending() && !key_is_untranslatable(k) && !source_stays_unchanged(v)
        });
        skipped += before - map.len();
    }
    pending.retain(|_, m| !m.is_empty());
    skipped
}

/// 鍵名本身就表明「這不是玩家會看到的顯示文字」。
///
/// 全部比對小寫化後的鍵名，規則刻意保守——只收「幾乎不可能是顯示文字」的形態，
/// 寧可漏放行也不要把真的該翻的句子擋掉。
pub fn key_is_untranslatable(key: &str) -> bool {
    let k = key.trim().to_ascii_lowercase();
    if k.is_empty() {
        return false;
    }
    // 註解：__comment__、_comment.foo、supplementaries.comment4、#comment_npcs
    if k.starts_with('#')
        || k.starts_with("__")
        || k.starts_with("_comment")
        || k.contains("comment")
    {
        return true;
    }
    // 中繼資料：htp_metadata_version、htp_metadata_credits、*_version、*_credits
    if k.contains("metadata") || k.ends_with("_version") || k.ends_with("_credits") {
        return true;
    }
    // 指令用法字串：commands.ftbquests.import.usage
    if k.starts_with("commands.") && k.ends_with(".usage") {
        return true;
    }
    // 作者署名（畫作作者等），翻了反而讓人找不到原作者
    if k.ends_with(".author") || k.ends_with("_author") {
        return true;
    }
    // 網址欄位
    if k.ends_with("_url") || k.ends_with(".url") || k.ends_with("_link") || k.ends_with(".link") {
        return true;
    }
    // 附魔等級／藥水效力：值是羅馬數字，翻了會讓等級顯示錯亂
    if k.contains("enchantment.level.") || k.contains("potion.potency.") {
        return true;
    }
    // 字型圖示鍵（icon.star = "§f" 這種私用區字元）
    if k.starts_with("icon.") || k.contains(".icon") || k.ends_with(".icon") {
        return true;
    }
    false
}

const TOOL_PACK_PREFIX: &str = "模組包翻譯工具";

pub fn is_tool_resource_pack(stem: &str) -> bool {
    stem.starts_with(TOOL_PACK_PREFIX)
}

/// 刪除 `resourcepacks/` 內其他「模組包翻譯工具+*」zip／同名資料夾，保留 `keep_stem`。
pub fn prune_stale_tool_packs(rp_root: &Path, keep_stem: &str) -> Result<Vec<String>, String> {
    let mut removed = Vec::new();
    if !rp_root.is_dir() {
        return Ok(removed);
    }
    let keep = keep_stem.trim().trim_end_matches(".zip").trim_end_matches(".ZIP");
    for entry in fs::read_dir(rp_root).map_err(|e| format!("讀取 resourcepacks 失敗：{e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let stem = name
            .trim_end_matches(".zip")
            .trim_end_matches(".ZIP")
            .to_string();
        if !is_tool_resource_pack(&stem) {
            continue;
        }
        if stem.eq_ignore_ascii_case(keep) {
            continue;
        }
        if path.is_file() {
            fs::remove_file(&path).map_err(|e| format!("無法刪除舊版工具資源包 {}：{e}", path.display()))?;
            removed.push(name);
        } else if path.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|e| format!("無法刪除舊版工具資源包資料夾 {}：{e}", path.display()))?;
            removed.push(name);
        }
    }
    Ok(removed)
}

/// session 的 `pack_name` 優先；否則取 mtime 最新的工具 zip／資料夾。
pub fn resolve_canonical_tool_zip(work_root: &Path) -> Option<PathBuf> {
    let rp = resourcepacks_root(work_root);
    if !rp.is_dir() {
        return None;
    }
    if let Ok((session, _)) = load_session(work_root) {
        let name = session.pack_name.trim();
        if !name.is_empty() {
            let zip = rp.join(format!("{name}.zip"));
            if zip.is_file() {
                return Some(zip);
            }
            let dir = rp.join(name);
            if dir.is_dir() {
                return Some(dir);
            }
            if let Some(found) = find_pack_near(work_root, name, session.pack_path.trim()) {
                return Some(found);
            }
        }
    }
    let mut best: Option<(u64, PathBuf)> = None;
    let Ok(rd) = fs::read_dir(&rp) else {
        return None;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !(path.is_file() || path.is_dir()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let stem = name
            .trim_end_matches(".zip")
            .trim_end_matches(".ZIP")
            .to_string();
        if !is_tool_resource_pack(&stem) {
            continue;
        }
        let mtime = fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if best.as_ref().map(|(m, _)| mtime > *m).unwrap_or(true) {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, p)| p)
}

/// 發現可接續的先前 zh 資源包（同機）。回傳去重後路徑，建議由新到舊合併。
/// `pack_version` 為 [`PackVersionInfo.version`]（嵌入自動檔名），不是 session schema。
pub fn discover_prior_zh_sources(
    work: &Path,
    pack_name: &str,
    pack_version: &str,
    instance: &Path,
) -> Vec<PathBuf> {
    let mut ranked: Vec<(u64, PathBuf)> = Vec::new();
    let mut push = |path: PathBuf| {
        if !(path.is_file() || path.is_dir()) {
            return;
        }
        if ranked.iter().any(|(_, p)| p == &path) {
            return;
        }
        let mtime = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        ranked.push((mtime, path));
    };

    let rp_work = resourcepacks_root(work);
    let exact_zip = rp_work.join(format!("{pack_name}.zip"));
    let exact_dir = rp_work.join(pack_name);
    // 精確名稍後仍會依 mtime 排序；先收集
    push(exact_zip);
    push(exact_dir);

    collect_tool_packs_in_dir(&rp_work, pack_version, &mut push);

    if let Ok((session, _)) = load_session(work) {
        if let Some(found) = find_pack_near(work, &session.pack_name, &session.pack_path) {
            push(found);
        }
        if !session.pack_path.trim().is_empty() {
            push(PathBuf::from(session.pack_path.trim()));
        }
    }

    if let Ok(mc) = super::jar_scan::resolve_minecraft_dir(instance) {
        let rp_game = mc.join("resourcepacks");
        push(rp_game.join("繁體中文翻譯.zip"));
        push(rp_game.join("繁體中文翻譯"));
        push(rp_game.join(format!("{pack_name}.zip")));
        push(rp_game.join(pack_name));
        collect_tool_packs_in_dir(&rp_game, pack_version, &mut push);
    }

    // 精確當前名優先（mtime 加成），其餘依 mtime 新→舊
    let exact_name = pack_name.to_string();
    ranked.sort_by(|a, b| {
        let a_exact = path_matches_pack_name(&a.1, &exact_name);
        let b_exact = path_matches_pack_name(&b.1, &exact_name);
        match (a_exact, b_exact) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.0.cmp(&a.0),
        }
    });
    ranked.into_iter().map(|(_, p)| p).collect()
}

fn path_matches_pack_name(path: &Path, pack_name: &str) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let stem = name.trim_end_matches(".zip").trim_end_matches(".ZIP");
    stem.eq_ignore_ascii_case(pack_name)
}

fn collect_tool_packs_in_dir(dir: &Path, pack_version: &str, push: &mut dyn FnMut(PathBuf)) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let stem = name
            .trim_end_matches(".zip")
            .trim_end_matches(".ZIP")
            .to_string();
        if !is_tool_resource_pack(&stem) {
            continue;
        }
        // 跨版本仍可用：接續上次不強制 +version 一致（同 stem 前綴即可）。
        let _ = pack_version;
        if path.is_file() || path.is_dir() {
            push(path);
        }
    }
}

#[allow(dead_code)]
fn tool_pack_matches_version(stem: &str, pack_version: &str) -> bool {
    let ver = pack_version.trim();
    if ver.is_empty() {
        return true;
    }
    stem.contains(&format!("+{ver}")) || stem.ends_with(ver)
}

/// 依 session 名稱在輸出目錄附近找 zip／資料夾
pub fn find_pack_near(output_dir: &Path, pack_name: &str, pack_path_hint: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let hint = PathBuf::from(pack_path_hint);
    candidates.push(hint.clone());
    if !pack_path_hint.ends_with(".zip") {
        candidates.push(PathBuf::from(format!("{pack_path_hint}.zip")));
    }
    // 常見位置
    for base in session_search_dirs(output_dir) {
        let rp = resourcepacks_root(&base);
        candidates.push(rp.join(format!("{pack_name}.zip")));
        candidates.push(rp.join(pack_name));
        candidates.push(base.join(format!("{pack_name}.zip")));
        candidates.push(base.join(pack_name));
    }
    // 也掃 resourcepacks 裡任何同名（大小寫不敏感）
    for base in session_search_dirs(output_dir) {
        let rp = resourcepacks_root(&base);
        if let Ok(rd) = fs::read_dir(&rp) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                let stem = n.trim_end_matches(".zip").trim_end_matches(".ZIP");
                if stem.eq_ignore_ascii_case(pack_name)
                    || n.eq_ignore_ascii_case(&format!("{pack_name}.zip"))
                {
                    candidates.push(e.path());
                }
            }
        }
    }

    for c in candidates {
        if c.is_file() || c.is_dir() {
            return Some(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mods_fingerprint_changes_when_mod_set_changes() {
        // 這條釘死「同一個 instance 資料夾被換成不同整合包」的偵測依據：
        // 檔名或大小任一變了，指紋就要跟著變；完全沒動則指紋要穩定重現。
        let dir = std::env::temp_dir().join(format!("mcpl-modsfp-{}", std::process::id()));
        let mods = dir.join("mods");
        let _ = fs::create_dir_all(&mods);
        fs::write(mods.join("jei-1.20.1.jar"), vec![0u8; 1000]).unwrap();
        fs::write(mods.join("createx-0.5.jar"), vec![0u8; 2000]).unwrap();

        let a = mods_fingerprint(&dir);
        assert_ne!(a, 0, "有 mods 就該算出非零指紋");
        assert_eq!(a, mods_fingerprint(&dir), "同樣的內容要能重現同一個指紋");

        // 換掉整批 mod（同資料夾路徑，模擬啟動器重灌整合包）
        let _ = fs::remove_dir_all(&mods);
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("botania-1.20.jar"), vec![0u8; 5000]).unwrap();
        let b = mods_fingerprint(&dir);
        assert_ne!(a, b, "整組 mod 換掉，指紋必須跟著變");

        // 只是檔案大小變了（例如 mod 更新版本但檔名不變）也要能反映
        fs::write(mods.join("botania-1.20.jar"), vec![0u8; 5001]).unwrap();
        let c = mods_fingerprint(&dir);
        assert_ne!(b, c);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mods_fingerprint_works_for_prismlauncher_nested_layout() {
        // 釘死這輪修的迴歸：PrismLauncher 的實例資料夾底下是巢狀的
        // instance/minecraft/mods，不是 instance/mods。修之前這裡永遠回 0，
        // 等於「換了不同整合包」的防呆對這種（最常見的）啟動器佈局全面失效。
        let dir = std::env::temp_dir().join(format!("mcpl-modsfp-nested-{}", std::process::id()));
        let mods = dir.join("minecraft").join("mods");
        let _ = fs::create_dir_all(&mods);
        fs::write(mods.join("jei-1.20.1.jar"), vec![0u8; 1000]).unwrap();

        let a = mods_fingerprint(&dir);
        assert_ne!(a, 0, "巢狀 minecraft/mods 也要能算出非零指紋");
        assert_eq!(
            a,
            mods_fingerprint(&dir.join("minecraft")),
            "直接指到 minecraft 資料夾或指到上層實例資料夾，指紋應該一致"
        );

        let _ = fs::remove_dir_all(&mods);
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("totally-different.jar"), vec![0u8; 9999]).unwrap();
        let b = mods_fingerprint(&dir);
        assert_ne!(a, b, "巢狀佈局換了整批 mod，指紋要跟著變");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_sibling_instances_with_same_mods_finds_renamed_copy() {
        // 模擬你回報的情境：ProminenceTM...(1) 被複製/改名成 ct，兩個資料夾
        // 同時存在於同一個上層（PrismLauncher 的 instances\）底下，內容一樣。
        let root = std::env::temp_dir().join(format!("mcpl-siblings-{}", std::process::id()));
        let a = root.join("ProminenceTM II Hasturian Era-v4.0.2 (1)");
        let b = root.join("ct");
        let unrelated = root.join("完全不同的包");
        for (dir, jar, size) in [
            (&a, "same-mod.jar", 1234u64),
            (&b, "same-mod.jar", 1234u64),
            (&unrelated, "other-mod.jar", 9999u64),
        ] {
            let mods = dir.join("minecraft").join("mods");
            fs::create_dir_all(&mods).unwrap();
            fs::write(mods.join(jar), vec![0u8; size as usize]).unwrap();
        }

        let siblings = find_sibling_instances_with_same_mods(&a);
        assert_eq!(siblings.len(), 1, "應該只找到 ct，不該把自己或不相干的包算進去");
        assert!(siblings[0].ends_with("ct"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn find_sibling_instances_with_same_mods_empty_when_no_match() {
        let root = std::env::temp_dir().join(format!("mcpl-siblings-none-{}", std::process::id()));
        let a = root.join("only-one");
        fs::create_dir_all(a.join("minecraft").join("mods")).unwrap();
        fs::write(
            a.join("minecraft").join("mods").join("solo.jar"),
            vec![0u8; 10],
        )
        .unwrap();

        assert!(find_sibling_instances_with_same_mods(&a).is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn mods_fingerprint_is_zero_when_unknown() {
        // 讀不到 mods/（路徑錯、還沒解壓）要回 0＝未知，讓比對端知道不能拿來否決。
        let missing = std::env::temp_dir().join("mcpl-modsfp-does-not-exist-xyz");
        let _ = fs::remove_dir_all(&missing);
        assert_eq!(mods_fingerprint(&missing), 0);

        let empty = std::env::temp_dir().join(format!("mcpl-modsfp-empty-{}", std::process::id()));
        let _ = fs::create_dir_all(empty.join("mods"));
        assert_eq!(mods_fingerprint(&empty), 0, "mods 資料夾存在但是空的，也算未知");
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn filter_local_untranslatable_skips_resource_id() {
        let mut pending: LangMap = HashMap::new();
        let map = pending.entry("minecraft".into()).or_default();
        map.insert("item.stone".into(), "minecraft:stone".into());
        map.insert("item.diamond_sword".into(), "Diamond Sword".into());
        let skipped = filter_local_untranslatable(&mut pending);
        assert_eq!(skipped, 1);
        assert_eq!(count_map(&pending), 1);
        assert_eq!(
            pending["minecraft"]["item.diamond_sword"],
            "Diamond Sword"
        );
    }

    #[test]
    fn session_write_is_atomic_and_recovers_from_interrupted_write() {
        // 使用者實測「翻到一半把工具關掉」。工作階段檔在大型整合包上有 2.6 MB，
        // 舊版直接覆蓋原檔，寫到一半被中斷就留下截斷的 JSON，整輪進度救不回來。
        let dir = std::env::temp_dir().join(format!("mcpl-session-atomic-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut session = TranslateSession {
            version: 1,
            review_pass: 0,
            instance_path: "C:/instance".into(),
            output_dir: dir.display().to_string(),
            pack_name: "pack".into(),
            pack_path: "C:/pack.zip".into(),
            pending_en: HashMap::new(),
            pending_count: 0,
            quality_deferred: HashMap::new(),
            keys_zh: 0,
            keys_hk_hint: 0,
            note: "first".into(),
            target_version: None,
            translation_mode: default_translation_mode(),
            translation_quality: default_translation_quality(),
            coverage_tier: default_coverage_tier(),
            mods_fingerprint: 0,
            run_preferences: RunPreferences::default(),
            last_run_outcome: RunOutcome::Completed,
            update_basis: Default::default(),
        };
        save_session(&dir, &session).unwrap();
        // 寫完不該留下暫存檔
        assert!(
            !dir.join(SESSION_FILE).with_extension("json.tmp").is_file(),
            "正常寫完後不該留下 .tmp"
        );

        // 第二次寫入：舊檔在 rename 成功前都必須保持可讀
        session.note = "second".into();
        save_session(&dir, &session).unwrap();
        let (loaded, _) = load_session(&dir).unwrap();
        assert_eq!(loaded.note, "second");

        // 模擬「刪了舊檔、還沒改名」就斷電：只剩 .tmp 時要能救回來
        let primary = dir.join(SESSION_FILE);
        let tmp = primary.with_extension("json.tmp");
        fs::rename(&primary, &tmp).unwrap();
        assert!(!primary.is_file());
        let found = find_session_file(&dir).expect("只剩暫存檔時也要找得到工作階段");
        let (recovered, _) = load_session(&dir).unwrap();
        assert_eq!(recovered.note, "second", "從暫存檔復原的內容要完整");
        assert!(found.is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_level_filter_matches_real_pending_samples() {
        // 全部取自使用者真實 pendingEn 的鍵名
        for k in [
            "htp_metadata_version",
            "htp_metadata_credits",
            "__comment__",
            "_comment.minecells.005",
            "supplementaries.comment4",
            "#comment_npcs",
            "commands.ftbquests.import_rewards_from_chest.usage",
            "painting.handcrafted.misty_mountains.author",
            "gui.ad_astra.text.flag_url",
            "bclib.updates.modrinth_link",
            "enchantment.level.109",
            "potion.potency.1",
            "icon.star",
            "attribute.obscure_api.parry.icon",
        ] {
            assert!(key_is_untranslatable(k), "{k} 的鍵名已表明不該翻");
        }
        // 真正該翻的鍵不能被誤殺
        for k in [
            "item.ad_astra.ti_69",
            "block.stoneworks.andesite_bricks",
            "tooltip.prominent.armorset.fire_armor_set.default.living_bomb.desc2",
            "options.mipmapLevels",
            "text.owo.config.button.exit_minecraft",
            "advancements.ad_astra.dj.title",
        ] {
            assert!(!key_is_untranslatable(k), "{k} 是顯示文字，不該被擋");
        }
    }

    #[test]
    fn filter_local_untranslatable_drops_roman_levels_and_icons() {
        let mut pending: LangMap = HashMap::new();
        let map = pending.entry("demo".into()).or_default();
        map.insert("enchantment.level.109".into(), "CIX".into());
        map.insert("icon.star".into(), "§f".into());
        map.insert("htp_metadata_version".into(), "1.0.0".into());
        map.insert("create.generic.unit.stress".into(), "su".into());
        map.insert("item.demo.sword".into(), "Diamond Sword".into());

        let skipped = filter_local_untranslatable(&mut pending);
        assert_eq!(skipped, 4, "四條不該翻的要被濾掉");
        assert_eq!(count_map(&pending), 1);
        assert_eq!(pending["demo"]["item.demo.sword"], "Diamond Sword");
    }

    #[test]
    fn legacy_session_without_quality_deferred_is_compatible() {
        let session: TranslateSession = serde_json::from_str(
            r#"{
                "version": 1,
                "instancePath": "C:/instance",
                "outputDir": "C:/output",
                "packName": "pack",
                "packPath": "C:/output/pack.zip",
                "pendingEn": {},
                "pendingCount": 0,
                "keysZh": 0,
                "note": "legacy"
            }"#,
        )
        .unwrap();
        assert!(session.quality_deferred.is_empty());
        assert_eq!(session.translation_mode, "append");
    }

    #[test]
    fn normal_pending_filter_excludes_only_deferred_keys() {
        let mut pending: LangMap = HashMap::new();
        pending
            .entry("demo".into())
            .or_default()
            .extend([
                ("item.one".into(), "One".into()),
                ("item.two".into(), "Two".into()),
            ]);
        let mut deferred: LangMap = HashMap::new();
        deferred
            .entry("demo".into())
            .or_default()
            .insert("item.one".into(), "One".into());

        assert_eq!(filter_quality_deferred(&mut pending, &deferred), 1);
        assert!(!pending["demo"].contains_key("item.one"));
        assert_eq!(pending["demo"]["item.two"], "Two");
    }

    #[test]
    fn prune_quality_deferred_removes_accepted_translation() {
        let mut deferred: LangMap = HashMap::new();
        deferred
            .entry("demo".into())
            .or_default()
            .extend([
                ("item.good".into(), "Diamond Sword".into()),
                ("item.bad".into(), "Golden Apple".into()),
            ]);
        let mut zh: LangMap = HashMap::new();
        zh.entry("demo".into()).or_default().extend([
            ("item.good".into(), "鑽石劍".into()),
            ("item.bad".into(), "Golden Apple".into()),
        ]);

        prune_quality_deferred(&mut deferred, &zh);
        assert!(!deferred["demo"].contains_key("item.good"));
        assert!(deferred["demo"].contains_key("item.bad"));
    }

    #[test]
    fn discover_prior_finds_tool_zip_with_same_version_different_date() {
        let root = std::env::temp_dir().join(format!(
            "modpack-i18n-prior-{}-{}",
            "v",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let rp = root.join("resourcepacks");
        fs::create_dir_all(&rp).unwrap();
        let old = rp.join("模組包翻譯工具+0101+1.2.3.zip");
        let newer = rp.join("模組包翻譯工具+0802+1.2.3.zip");
        fs::write(&old, b"PK\x03\x04").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&newer, b"PK\x03\x04").unwrap();
        // 不同 MC 版本的工具包也應被發現（跨 version 接續）
        fs::write(rp.join("模組包翻譯工具+0101+9.9.9.zip"), b"PK\x03\x04").unwrap();

        let found = discover_prior_zh_sources(&root, "模組包翻譯工具+0802+1.2.3", "1.2.3", &root);
        assert!(
            found.iter().any(|p| p.ends_with("模組包翻譯工具+0802+1.2.3.zip")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|p| p.ends_with("模組包翻譯工具+0101+1.2.3.zip")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|p| p.to_string_lossy().contains("9.9.9")),
            "{found:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn discover_prior_finds_tool_zip_across_versions() {
        let root = std::env::temp_dir().join(format!(
            "mcpl-prior-cross-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let rp = root.join("resourcepacks");
        fs::create_dir_all(&rp).unwrap();
        let old = rp.join("模組包翻譯工具+0101+1.2.3.zip");
        fs::write(&old, b"pk").unwrap();
        let found = discover_prior_zh_sources(&root, "SomePack", "1.2.4", &root);
        let _ = fs::remove_dir_all(&root);
        assert!(
            found.iter().any(|p| p.file_name() == old.file_name()),
            "expected cross-version tool pack, got {found:?}"
        );
    }
}
