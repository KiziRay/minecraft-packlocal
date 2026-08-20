//! 翻譯工作階段：讓「補翻」不必重掃 mods。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::jar_scan::LangMap;
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
    pub keys_zh: usize,
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
    }
    None
}

pub fn save_session(output_dir: &Path, session: &TranslateSession) -> Result<(), String> {
    // output_dir 應為「翻譯結果」工作根
    fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    let primary = output_dir.join(SESSION_FILE);
    let s = serde_json::to_string_pretty(session).map_err(|e| e.to_string())?;
    fs::write(&primary, s + "\n").map_err(|e| e.to_string())?;
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
    let session: TranslateSession =
        serde_json::from_str(&t).map_err(|e| format!("工作階段檔損壞（{}）：{e}", p.display()))?;
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
pub fn filter_local_untranslatable(pending: &mut LangMap) -> usize {
    use super::deepseek::looks_untranslatable;
    let mut skipped = 0usize;
    for map in pending.values_mut() {
        let before = map.len();
        map.retain(|_, v| !looks_untranslatable(v));
        skipped += before - map.len();
    }
    pending.retain(|_, m| !m.is_empty());
    skipped
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
}
