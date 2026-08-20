//! 搜尋系統：掃描 → 分析 → 理解 → 整合 → 上下文工作圖。
//!
//! 不呼叫 AI；只列舉來源桶與四態，並把同詞異義／跨檔對齊做成可給翻譯／知識庫消費的單元。
//! 已同意進階時，進階來源桶一併納入（裁定 A）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::deepseek::context_hint;
use super::jar_scan::resolve_minecraft_dir;
use super::shared_tm::{normalize_source, semantic_hash};

/// 來源桶四態（玩家報告用白話對應）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BucketState {
    /// 已有繁中可沿用
    Covered,
    /// 可覆寫翻譯
    Overwritable,
    /// 僅線索（如 class 字串），不可當可覆寫
    ClueOnly,
    /// 明確無法（圖上字／硬編碼等）
    Impossible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketInventory {
    pub id: String,
    pub label: String,
    pub state: BucketState,
    pub files_found: usize,
    pub string_estimate: usize,
    pub note: String,
    pub advanced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkUnit {
    /// 穩定單元 id（語意雜湊）
    pub unit_id: String,
    pub source_text: String,
    pub context: Option<String>,
    pub bucket: String,
    pub namespace: Option<String>,
    pub key: Option<String>,
    pub paths: Vec<String>,
    pub conflict: bool,
    pub conflict_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkGraph {
    pub include_advanced: bool,
    pub buckets: Vec<BucketInventory>,
    pub units: Vec<WorkUnit>,
    pub aligned_count: usize,
    pub split_polysemy_count: usize,
    pub conflict_count: usize,
    pub player_summary: String,
}

#[derive(Debug, Clone)]
struct RawHit {
    bucket: String,
    source: String,
    context: Option<String>,
    namespace: Option<String>,
    key: Option<String>,
    path: String,
}

/// 執行搜尋系統全管線。
pub fn run_search_pipeline(
    instance: &Path,
    include_advanced: bool,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<WorkGraph, String> {
    on_progress(4, "正在搜尋並整理全文案（含跨檔對齊）…");
    let mc = resolve_minecraft_dir(instance).unwrap_or_else(|_| instance.to_path_buf());

    // 1) 掃描：來源桶列舉（不 AI）
    let buckets = inventory_buckets(&mc, include_advanced, on_progress)?;

    // 2–4) 分析／理解／整合：由可覆寫桶抽樣鍵路徑組成工作圖
    on_progress(12, "分析與對齊用語…");
    let mut raw = collect_raw_hits(&mc, include_advanced)?;
    analyze_hits(&mut raw);
    let graph = integrate_work_graph(raw, buckets, include_advanced);

    on_progress(
        18,
        &format!(
            "搜尋完成：{} 個來源桶、{} 個翻譯單元",
            graph.buckets.len(),
            graph.units.len()
        ),
    );
    Ok(graph)
}

/// 搜尋管線記憶體結果保留；1.0.2+ 不再寫入工作根除錯盤點檔。
pub fn write_search_artifacts(_work_root: &Path, _graph: &WorkGraph) -> Result<(), String> {
    Ok(())
}

fn inventory_buckets(
    mc: &Path,
    include_advanced: bool,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<Vec<BucketInventory>, String> {
    on_progress(6, "列舉文案來源…");
    let mut out = Vec::new();

    out.push(count_glob_bucket(
        "jar_lang",
        "模組語言檔",
        &mc.join("mods"),
        &["assets/", "/lang/"],
        &[".json", ".lang"],
        BucketState::Overwritable,
        false,
        "從模組檔讀取，不改原檔",
    ));
    out.push(count_dir_lang(
        "loose_lang",
        "資料夾語言檔",
        &[
            mc.join("resourcepacks"),
            mc.join("kubejs"),
            mc.join("config").join("openloader"),
        ],
        BucketState::Overwritable,
        false,
    ));
    out.push(count_path_bucket(
        "ftb_snbt",
        "FTB 任務",
        &mc.join("config").join("ftbquests"),
        &[".snbt"],
        BucketState::Overwritable,
        false,
        "",
    ));
    out.push(count_path_bucket(
        "text_overlay",
        "設定／覆寫文字",
        &mc.join("config"),
        &[".json", ".txt", ".snbt"],
        BucketState::Overwritable,
        false,
        "只翻顯示欄位",
    ));
    out.push(count_path_bucket(
        "patchouli_books",
        "實例書本",
        &mc.join("patchouli_books"),
        &[".json"],
        BucketState::Overwritable,
        false,
        "",
    ));
    out.push(count_glob_bucket(
        "jar_patchouli",
        "模組內 Patchouli 書",
        &mc.join("mods"),
        &["data/", "/patchouli_books/"],
        &[".json", ".txt"],
        BucketState::Overwritable,
        false,
        "寫入 jar-translated 副本，不改原 jar",
    ));
    out.push(count_glob_bucket(
        "jar_citadel_books",
        "模組內 Citadel 書",
        &mc.join("mods"),
        &["assets/", "/book/"],
        &[".json", ".txt"],
        BucketState::Overwritable,
        false,
        "寫入 jar-translated 副本，不改原 jar",
    ));
    out.push(count_path_bucket(
        "openloader_zip",
        "OpenLoader 資源包",
        &mc.join("config").join("openloader").join("resources"),
        &[".zip"],
        BucketState::Overwritable,
        false,
        "ZIP 內語言檔",
    ));
    out.push(count_path_bucket(
        "openloader_data",
        "OpenLoader 資料包文字",
        &mc.join("config").join("openloader").join("data"),
        &[".json", ".snbt"],
        BucketState::Overwritable,
        false,
        "",
    ));
    out.push(count_path_bucket(
        "fancymenu",
        "FancyMenu 選單",
        &mc.join("config").join("fancymenu"),
        &[".txt", ".json", ".local", ".properties"],
        BucketState::Overwritable,
        false,
        "含無引號 description／.local",
    ));
    out.push(count_path_bucket(
        "minemenu",
        "快捷選單 MineMenu",
        &mc.join("minemenu"),
        &["menu.json", ".json"],
        BucketState::Overwritable,
        false,
        "翻譯 title 並以 unicode 寫回",
    ));
    out.push(count_path_bucket(
        "armorsets_loot",
        "套裝／起始／掉落訊息",
        &mc.join("config"),
        &[
            "armorsets",
            "starterkit",
            "EntityLootDrops",
            "firstjoin",
            "deathbackup",
        ],
        BucketState::Overwritable,
        false,
        "路徑關鍵字命中；檔數>0 不代表字串已抽完",
    ));
    out.push(count_path_bucket(
        "datapacks",
        "鬆散資料包",
        &mc.join("datapacks"),
        &[".zip", ".json"],
        BucketState::Overwritable,
        false,
        "",
    ));
    out.push(BucketInventory {
        id: "jar_docs".into(),
        label: "模組檔內文字線索".into(),
        state: BucketState::ClueOnly,
        files_found: 0,
        string_estimate: 0,
        note: "僅供排查，不會標成可翻譯".into(),
        advanced: false,
    });
    out.push(BucketInventory {
        id: "images_hardcode".into(),
        label: "圖片字／硬編碼".into(),
        state: BucketState::Impossible,
        files_found: 0,
        string_estimate: 0,
        note: "不處理、不承諾".into(),
        advanced: false,
    });

    if include_advanced {
        out.push(count_glob_bucket(
            "advanced_jar_strings",
            "進階：模組內可抽取字串",
            &mc.join("mods"),
            &["assets/", "data/"],
            &[".json", ".lang", ".snbt"],
            BucketState::Overwritable,
            true,
            "僅在同意進階後納入；寫入副本",
        ));
    }

    Ok(out)
}

fn collect_raw_hits(mc: &Path, include_advanced: bool) -> Result<Vec<RawHit>, String> {
    let mut hits = Vec::new();
    // 輕量：從既有 zh_tw／en_us 語言檔抽鍵做語境分群樣本（完整字串仍由後續 scan／overlay 翻譯）
    let roots = [
        mc.join("resourcepacks"),
        mc.join("config").join("openloader"),
        mc.join("kubejs"),
        mc.join("patchouli_books"),
    ];
    for root in &roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(12)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !(name.ends_with("en_us.json")
                || name.ends_with("zh_tw.json")
                || name.ends_with("zh_cn.json"))
            {
                continue;
            }
            let Ok(text) = fs::read_to_string(path) else {
                continue;
            };
            let Ok(map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&text) else {
                continue;
            };
            let ns = path
                .components()
                .find_map(|c| {
                    let s = c.as_os_str().to_string_lossy();
                    if s.contains(':') {
                        None
                    } else if path.to_string_lossy().contains("assets") {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
                .or_else(|| Some("unknown".into()));
            for (k, v) in map {
                let Some(s) = v.as_str() else { continue };
                if s.trim().is_empty() {
                    continue;
                }
                let ctx = context_hint(&k).map(str::to_owned);
                hits.push(RawHit {
                    bucket: "loose_lang".into(),
                    source: s.to_string(),
                    context: ctx,
                    namespace: ns.clone(),
                    key: Some(k),
                    path: path.display().to_string(),
                });
                if hits.len() >= 8000 {
                    return Ok(hits);
                }
            }
        }
    }
    if include_advanced {
        // 標記進階來源存在即可；字串本體仍由既有進階產物路徑消費
        hits.push(RawHit {
            bucket: "advanced_jar_strings".into(),
            source: "__advanced_enabled__".into(),
            context: Some("advanced".into()),
            namespace: None,
            key: None,
            path: mc.join("mods").display().to_string(),
        });
    }
    Ok(hits)
}

fn analyze_hits(hits: &mut [RawHit]) {
    for hit in hits.iter_mut() {
        hit.source = normalize_source(&hit.source);
        if hit.context.is_none() {
            if let Some(key) = &hit.key {
                hit.context = context_hint(key).map(str::to_owned);
            }
        }
    }
}

fn integrate_work_graph(
    raw: Vec<RawHit>,
    buckets: Vec<BucketInventory>,
    include_advanced: bool,
) -> WorkGraph {
    // key = semantic(source, context)；同原文異語境分開
    let mut by_sem: HashMap<String, WorkUnit> = HashMap::new();
    let mut source_contexts: HashMap<String, Vec<String>> = HashMap::new();
    let mut split_polysemy = 0usize;
    let mut aligned = 0usize;

    for hit in raw {
        if hit.source == "__advanced_enabled__" {
            continue;
        }
        let ctx = hit.context.clone().unwrap_or_default();
        source_contexts
            .entry(hit.source.clone())
            .or_default()
            .push(ctx.clone());
        let unit_id = semantic_hash(
            hit.key.as_deref().unwrap_or(""),
            &hit.source,
            hit.context.as_deref(),
        );
        let entry = by_sem.entry(unit_id.clone()).or_insert_with(|| WorkUnit {
            unit_id: unit_id.clone(),
            source_text: hit.source.clone(),
            context: hit.context.clone(),
            bucket: hit.bucket.clone(),
            namespace: hit.namespace.clone(),
            key: hit.key.clone(),
            paths: Vec::new(),
            conflict: false,
            conflict_note: None,
        });
        if !entry.paths.contains(&hit.path) {
            entry.paths.push(hit.path);
            if entry.paths.len() > 1 {
                aligned += 1;
            }
        }
    }

    for (_src, ctxs) in &source_contexts {
        let mut uniq = ctxs.clone();
        uniq.sort();
        uniq.dedup();
        if uniq.len() > 1 {
            split_polysemy += 1;
        }
    }

    let mut conflict_count = 0usize;
    let mut units: Vec<WorkUnit> = by_sem.into_values().collect();
    for u in &mut units {
        if u.paths.len() > 3 && u.context.is_none() {
            u.conflict = true;
            u.conflict_note = Some("多處出現但語境不明，翻譯時請分開確認".into());
            conflict_count += 1;
        }
    }
    units.sort_by(|a, b| a.unit_id.cmp(&b.unit_id));

    let overwritable: Vec<&BucketInventory> = buckets
        .iter()
        .filter(|b| b.state == BucketState::Overwritable && b.files_found > 0)
        .collect();
    let listed_files: usize = overwritable.iter().map(|b| b.files_found).sum();
    let player_summary = if units.is_empty() && listed_files > 0 {
        format!(
            "已列舉來源桶 {} 個（可覆寫檔約 {}）；翻譯單元 0——此次僅完成盤點／抽樣，尚未抽出可譯單元，不代表翻譯已完成。{}",
            buckets.len(),
            listed_files,
            if include_advanced {
                "（已含進階來源）"
            } else {
                "（安全模式：未含進階解包來源）"
            }
        )
    } else {
        format!(
            "已搜尋並整理全文案。來源桶 {} 個；翻譯單元 {} 個。{}",
            buckets.len(),
            units.len(),
            if include_advanced {
                "（已含進階來源）"
            } else {
                "（安全模式：未含進階解包來源）"
            }
        )
    };

    WorkGraph {
        include_advanced,
        buckets,
        units,
        aligned_count: aligned,
        split_polysemy_count: split_polysemy,
        conflict_count,
        player_summary,
    }
}

fn count_path_bucket(
    id: &str,
    label: &str,
    root: &Path,
    exts_or_keywords: &[&str],
    state: BucketState,
    advanced: bool,
    note: &str,
) -> BucketInventory {
    if !root.exists() {
        return BucketInventory {
            id: id.into(),
            label: label.into(),
            state,
            files_found: 0,
            string_estimate: 0,
            note: if note.is_empty() {
                "路徑不存在".into()
            } else {
                note.into()
            },
            advanced,
        };
    }
    let mut files = 0usize;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(10)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let p = entry.path().to_string_lossy().to_ascii_lowercase();
        let hit = exts_or_keywords.iter().any(|k| p.contains(&k.to_ascii_lowercase()));
        if hit {
            files += 1;
        }
    }
    BucketInventory {
        id: id.into(),
        label: label.into(),
        state: if files == 0 { state } else { state },
        files_found: files,
        string_estimate: files.saturating_mul(8),
        note: note.into(),
        advanced,
    }
}

fn count_dir_lang(
    id: &str,
    label: &str,
    roots: &[PathBuf],
    state: BucketState,
    advanced: bool,
) -> BucketInventory {
    let mut files = 0usize;
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(14)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let name = entry
                .path()
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if name.ends_with(".json") || name.ends_with(".lang") {
                let p = entry.path().to_string_lossy().to_ascii_lowercase();
                if p.contains("/lang/") || p.contains("\\lang\\") {
                    files += 1;
                }
            }
        }
    }
    BucketInventory {
        id: id.into(),
        label: label.into(),
        state,
        files_found: files,
        string_estimate: files.saturating_mul(40),
        note: String::new(),
        advanced,
    }
}

fn count_glob_bucket(
    id: &str,
    label: &str,
    root: &Path,
    path_parts: &[&str],
    exts: &[&str],
    state: BucketState,
    advanced: bool,
    note: &str,
) -> BucketInventory {
    if !root.exists() {
        return BucketInventory {
            id: id.into(),
            label: label.into(),
            state,
            files_found: 0,
            string_estimate: 0,
            note: "路徑不存在".into(),
            advanced,
        };
    }
    // mods 下只計 jar 數量作估（不解壓，避免搜尋階段爆 I/O）
    let mut jars = 0usize;
    if let Ok(rd) = fs::read_dir(root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if name.ends_with(".jar") {
                jars += 1;
            }
        }
    }
    let _ = (path_parts, exts);
    BucketInventory {
        id: id.into(),
        label: label.into(),
        state,
        files_found: jars,
        string_estimate: jars.saturating_mul(120),
        note: note.into(),
        advanced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polysemy_splits_by_context() {
        let raw = vec![
            RawHit {
                bucket: "loose_lang".into(),
                source: "Open".into(),
                context: Some("介面文字".into()),
                namespace: Some("demo".into()),
                key: Some("gui.open".into()),
                path: "a.json".into(),
            },
            RawHit {
                bucket: "loose_lang".into(),
                source: "Open".into(),
                context: Some("狀態".into()),
                namespace: Some("demo".into()),
                key: Some("status.open".into()),
                path: "b.json".into(),
            },
        ];
        let graph = integrate_work_graph(raw, vec![], false);
        assert_eq!(graph.units.len(), 2);
        assert!(graph.split_polysemy_count >= 1);
    }

    #[test]
    fn same_context_aligns_paths() {
        let raw = vec![
            RawHit {
                bucket: "loose_lang".into(),
                source: "Backpack".into(),
                context: Some("物品名".into()),
                namespace: Some("demo".into()),
                key: Some("item.backpack".into()),
                path: "a.json".into(),
            },
            RawHit {
                bucket: "loose_lang".into(),
                source: "Backpack".into(),
                context: Some("物品名".into()),
                namespace: Some("demo".into()),
                key: Some("item.backpack".into()),
                path: "b.json".into(),
            },
        ];
        let graph = integrate_work_graph(raw, vec![], false);
        assert_eq!(graph.units.len(), 1);
        assert_eq!(graph.units[0].paths.len(), 2);
    }

    #[test]
    fn player_summary_mentions_mode() {
        let g = integrate_work_graph(vec![], vec![], true);
        assert!(g.player_summary.contains("進階"));
        let g2 = integrate_work_graph(vec![], vec![], false);
        assert!(g2.player_summary.contains("安全模式"));
    }

    #[test]
    fn inventory_includes_jar_book_buckets() {
        let root = std::env::temp_dir().join(format!("inv_books_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/demo.jar"), b"pk").unwrap();
        let buckets = inventory_buckets(&root, false, &mut |_, _| {}).unwrap();
        let patchouli = buckets
            .iter()
            .find(|b| b.id == "jar_patchouli")
            .expect("jar_patchouli bucket");
        let citadel = buckets
            .iter()
            .find(|b| b.id == "jar_citadel_books")
            .expect("jar_citadel_books bucket");
        assert!(patchouli.note.contains("jar-translated"));
        assert!(citadel.note.contains("不改原 jar"));
        assert_eq!(patchouli.files_found, 1);
        let _ = fs::remove_dir_all(root);
    }
}
