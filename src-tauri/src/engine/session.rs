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

/// 從 pending 去掉已有中文的 key
pub fn remaining_pending(pending: &LangMap, zh: &LangMap) -> LangMap {
    let mut out: LangMap = HashMap::new();
    for (ns, map) in pending {
        for (k, en) in map {
            let has = zh.get(ns).and_then(|m| m.get(k)).is_some();
            if !has {
                // 略過空字串（無法翻譯）
                if en.trim().is_empty() {
                    continue;
                }
                out.entry(ns.clone())
                    .or_default()
                    .insert(k.clone(), en.clone());
            }
        }
    }
    out
}

pub fn count_map(m: &LangMap) -> usize {
    m.values().map(|x| x.len()).sum()
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
