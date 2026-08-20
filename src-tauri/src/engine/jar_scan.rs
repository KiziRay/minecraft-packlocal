//! 本地蒐集語言檔：**全程不呼叫 AI**。
//! 來源：mods jar、資源包、KubeJS、config 內 lang、常見設定亂碼處理前置資料。

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use walkdir::WalkDir;
use zip::ZipArchive;

use super::cancel;
use super::convert::{apply_phrase_dict, convert_s2tw, strip_of_suffix_zhi};
use super::lang_provenance::{set_source, LangSource, ProvenanceMap};
use super::translation_quality::is_usable_zh;
use super::security::{check_jar_size, is_safe_zip_entry_name, MAX_ZIP_ENTRY_BYTES};
use super::scan_cache::ScanCache;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub minecraft_dir: String,
    pub jars_scanned: usize,
    pub resourcepacks_scanned: usize,
    pub loose_lang_files: usize,
    pub namespaces: usize,
    pub keys_zh: usize,
    pub keys_need_ai: usize,
    pub keys_from_zh_tw: usize,
    pub keys_from_zh_cn: usize,
    /// 僅 zh_hk 來源、已 s2twp 補缺的 key（不算台灣已完成）
    pub keys_from_zh_hk_hint: usize,
    /// 台灣可玩覆蓋（Tw + CnConverted，不含純港繁 hint）
    pub keys_tw_playable: usize,
    pub scan_cache_hits: usize,
    pub errors: Vec<String>,
}

/// namespace -> (lang_key -> value)
pub type LangMap = HashMap<String, HashMap<String, String>>;

/// ns -> locale -> map
type RawLang = HashMap<String, HashMap<String, HashMap<String, String>>>;

pub fn resolve_minecraft_dir(instance_or_mc: &Path) -> Result<PathBuf, String> {
    let p = instance_or_mc;
    if p.join("mods").is_dir() {
        return Ok(p.to_path_buf());
    }
    if p.join("minecraft").join("mods").is_dir() {
        return Ok(p.join("minecraft"));
    }
    if p.join(".minecraft").join("mods").is_dir() {
        return Ok(p.join(".minecraft"));
    }
    Err("找不到 mods 資料夾。請選遊戲實例資料夾（裡面有 minecraft 或 mods）。".into())
}

/// 搜尋／整合階段：**不呼叫 AI**。
/// 回傳 (已有中文 map, 僅英文待譯 map, 來源標記, 報告)
pub fn scan_instance<F>(
    instance_or_mc: &Path,
    phrase_dict: &HashMap<String, String>,
    do_opencc: bool,
    strip_of_zhi: bool,
    mut on_progress: F,
) -> Result<(LangMap, LangMap, ProvenanceMap, ScanReport), String>
where
    F: FnMut(u8, &str),
{
    let mc = resolve_minecraft_dir(instance_or_mc)?;
    let mut raw: RawLang = HashMap::new();
    let mut jars = 0usize;
    let mut rps = 0usize;
    let mut loose = 0usize;
    let mut errors = Vec::new();
    let mut non_priority_lang_skips = 0usize;
    let mut scan_cache = ScanCache::load();

    // ─── 1) mods jar / zip（平行掃描；含 Essential 等雙 jar，只讀 lang 不拆包）───
    on_progress(6, "本地整理：列出模組檔…");
    let jar_paths = list_archive_files(&mc.join("mods"), 2);
    let total_j = jar_paths.len().max(1);
    on_progress(
        8,
        &format!("本地整理：讀模組語言（共 {} 個）…", jar_paths.len()),
    );
    if !jar_paths.is_empty() {
        let available = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        let configured = std::env::var("MODPACK_I18N_JAR_WORKERS")
            .ok()
            .and_then(|value| value.trim().parse::<usize>().ok());
        let n_workers = worker_count(jar_paths.len(), available, configured);
        let chunk_size = (jar_paths.len() + n_workers - 1) / n_workers;
        let mut handles = Vec::with_capacity(n_workers);
        for chunk in jar_paths.chunks(chunk_size) {
            let chunk: Vec<PathBuf> = chunk.to_vec();
            handles.push(thread::spawn(move || {
                let mut local_raw: RawLang = HashMap::new();
                let mut local_errors: Vec<String> = Vec::new();
                let mut local_skips = 0usize;
                let mut local_jars = 0usize;
                for path in &chunk {
                    if cancel::is_cancelled() {
                        break;
                    }
                    local_jars += 1;
                    let name = file_name_str(path);
                    if let Err(e) = check_jar_size(path) {
                        local_errors.push(format!("{}: {}", name, e));
                        continue;
                    }
                    if let Err(e) =
                        harvest_archive(path, &mut local_raw, &mut local_errors, &mut local_skips)
                    {
                        local_errors.push(format!("{}: {}", name, e));
                    }
                }
                (local_raw, local_errors, local_jars, local_skips)
            }));
        }
        let mut done = 0usize;
        for (gi, handle) in handles.into_iter().enumerate() {
            match handle.join() {
                Ok((partial, errs, count, skips)) => {
                    merge_raw_lang(&mut raw, partial);
                    errors.extend(errs);
                    non_priority_lang_skips += skips;
                    jars += count;
                    done += count;
                    let pct = 8 + (((done as u32) * 14) / total_j as u32).min(14) as u8;
                    on_progress(
                        pct,
                        &format!(
                            "本地整理：模組批次 {}/{}（已 {}/{}）",
                            gi + 1,
                            n_workers,
                            done,
                            jar_paths.len()
                        ),
                    );
                }
                Err(_) => {
                    errors.push("模組掃描執行緒異常結束".into());
                }
            }
        }
    }

    cancel::check()?;

    // ─── 2) resourcepacks（資料夾 + zip）───
    on_progress(23, "本地整理：讀資源包語言…");
    let rp_root = mc.join("resourcepacks");
    if rp_root.is_dir() {
        let entries: Vec<PathBuf> = fs::read_dir(&rp_root)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_dir()
                            || matches!(
                                p.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
                                Some("zip") | Some("jar")
                            )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let total_r = entries.len().max(1);
        for (idx, path) in entries.iter().enumerate() {
            rps += 1;
            let pct = 23 + (((idx + 1) as u32 * 6) / total_r as u32).min(6) as u8;
            on_progress(
                pct,
                &format!(
                    "本地整理：資源包（{}/{}）{}",
                    idx + 1,
                    entries.len(),
                    truncate_name(&file_name_str(path), 36)
                ),
            );
            if path.is_dir() {
                match harvest_folder_pack(path, &mut raw, &mut errors, &mut non_priority_lang_skips)
                {
                    Ok(n) => loose += n,
                    Err(e) => errors.push(format!("{}: {}", file_name_str(path), e)),
                }
            } else {
                if let Err(e) = check_jar_size(path) {
                    errors.push(format!("{}: {}", file_name_str(path), e));
                    continue;
                }
                if let Err(e) =
                    harvest_archive(path, &mut raw, &mut errors, &mut non_priority_lang_skips)
                {
                    errors.push(format!("{}: {}", file_name_str(path), e));
                }
            }
        }
    }

    // ─── 3) 鬆散 lang：手翻同路徑、多根（不同模組常塞不同資料夾）───
    // 含 openloader 巢狀 pack、defaultconfigs、global_packs、datapacks 等。
    // 只收 `…/lang/<locale>.json|.lang`，不碰 gameplay id。
    on_progress(30, "本地整理：讀各資料夾語言檔…");
    // (相對 minecraft 根的路徑片段, max_depth) — openloader 巢狀較深
    let loose_roots: &[(&str, usize)] = &[
        ("kubejs", 12),
        ("config", 16), // config/openloader/data/<pack>/data|assets/…/lang
        ("defaultconfigs", 12),
        ("datapacks", 14),
        ("global_packs", 14),
        ("paxi", 12),
        ("data", 10),
        // FTB Quest Localizer 等遊戲內輔助模組的匯出語言檔。
        ("FTBLang", 12),
        // 部分任務匯出工具使用這個資料夾名稱。
        ("exported", 12),
    ];
    for (sub, depth) in loose_roots {
        let root = mc.join(sub);
        if !root.is_dir() {
            continue;
        }
        match harvest_loose_lang_tree(
            &root,
            &mut raw,
            *depth,
            &mut errors,
            &mut non_priority_lang_skips,
            &mut scan_cache,
        ) {
            Ok(n) => {
                loose += n;
                if n > 0 {
                    on_progress(
                        31,
                        &format!("本地整理：{} 找到 {} 個語言檔", sub, n),
                    );
                }
            }
            Err(e) => errors.push(format!("{sub}: {e}")),
        }
    }

    // FTB Quest Localizer 的 FTBLang 會放在「實例根目錄」，不一定在
    // instance/minecraft/。把實例本身與上一層也列入，但只掃任務匯出資料夾，
    // 避免把啟動器的其他資料夾整棵掃進來。
    let mut helper_roots = Vec::new();
    for base in [instance_or_mc, instance_or_mc.parent().unwrap_or(instance_or_mc)] {
        for sub in ["FTBLang", "ftblang", "exported"] {
            let root = base.join(sub);
            if root.is_dir() {
                helper_roots.push(root);
            }
        }
    }
    let mut seen_helper_roots = HashSet::new();
    for root in helper_roots {
        let key = root.to_string_lossy().to_ascii_lowercase();
        if !seen_helper_roots.insert(key) {
            continue;
        }
        match harvest_loose_lang_tree(
            &root,
            &mut raw,
            12,
            &mut errors,
            &mut non_priority_lang_skips,
            &mut scan_cache,
        ) {
            Ok(n) => loose += n,
            Err(e) => errors.push(format!("{}: {e}", root.display())),
        }
    }

    let cache_hits = scan_cache.hits();
    if let Err(error) = scan_cache.save() {
        errors.push(format!("掃描快取未能保存（不影響本次翻譯）：{error}"));
    }
    if non_priority_lang_skips > 0 {
        errors.push(format!(
            "另略過 {} 個非必要語系損壞語言檔（不影響繁中合併）",
            non_priority_lang_skips
        ));
    }
    if cache_hits > 0 {
        on_progress(34, &format!("本地整理：重用 {} 個未變語言檔快取", cache_hits));
    }

    cancel::check()?;

    // ─── 4) 三階合併：zh_tw → zh_cn(s2tw) → zh_hk hint(s2tw 僅補缺) ───
    on_progress(33, "本地整理：合併各來源中文與待譯文…");
    let mut zh: LangMap = HashMap::new();
    let mut en_only: LangMap = HashMap::new();
    let mut provenance: ProvenanceMap = HashMap::new();
    let mut from_tw = 0usize;
    let mut from_cn = 0usize;
    let mut from_hk = 0usize;

    for (ns, locales) in raw {
        let (out, eo, tw, cn, hk) = merge_namespace_locales(&ns, &locales, do_opencc, &mut provenance);
        from_tw += tw;
        from_cn += cn;
        from_hk += hk;
        if !eo.is_empty() {
            en_only.insert(ns.clone(), eo);
        }
        if !out.is_empty() {
            zh.insert(ns, out);
        }
    }

    // ─── 5) 詞典／「之」— 仍純本地（不再整包 OpenCC；cn/hk 已在合併時轉過）───
    let _ = do_opencc;

    on_progress(38, "本地整理：套用詞典與詞綴規則…");
    for map in zh.values_mut() {
        for v in map.values_mut() {
            let mut s = apply_phrase_dict(v, phrase_dict);
            if strip_of_zhi {
                s = strip_of_suffix_zhi(&s);
            }
            *v = s;
        }
    }

    // 去掉已能用中文覆蓋的 en（雙保險）
    en_only.retain(|ns, map| {
        map.retain(|k, _| zh.get(ns).and_then(|m| m.get(k)).is_none());
        !map.is_empty()
    });

    let keys_zh: usize = zh.values().map(|m| m.len()).sum();
    let keys_need_ai: usize = en_only.values().map(|m| m.len()).sum();
    let keys_tw_playable = super::lang_provenance::count_playable(&provenance);

    on_progress(
        40,
        &format!(
            "本地整理完成：模組 {}、資源包 {}、鬆散語言檔 {}；中文 {} 條（台灣可玩 {}）、港繁提示 {}、待 AI {} 條",
            jars, rps, loose, keys_zh, keys_tw_playable, from_hk, keys_need_ai
        ),
    );

    let report = ScanReport {
        minecraft_dir: mc.display().to_string(),
        jars_scanned: jars,
        resourcepacks_scanned: rps,
        loose_lang_files: loose,
        namespaces: zh.len().max(en_only.len()),
        keys_zh,
        keys_need_ai,
        keys_from_zh_tw: from_tw,
        keys_from_zh_cn: from_cn,
        keys_from_zh_hk_hint: from_hk,
        keys_tw_playable,
        scan_cache_hits: cache_hits,
        errors,
    };
    Ok((zh, en_only, provenance, report))
}

fn worker_count(jar_count: usize, available: usize, configured: Option<usize>) -> usize {
    let requested = configured.unwrap_or_else(|| available.clamp(8, 16));
    requested.clamp(1, 16).min(jar_count.max(1))
}

fn list_archive_files(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    if !root.is_dir() {
        return vec![];
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "jar" || ext == "zip" {
            out.push(path.to_path_buf());
        }
    }
    out
}

fn merge_raw_lang(into: &mut RawLang, from: RawLang) {
    for (ns, locales) in from {
        let ns_entry = into.entry(ns).or_default();
        for (loc, map) in locales {
            ns_entry.entry(loc).or_default().extend(map);
        }
    }
}

/// Minecraft locale：`xx_yy`（小寫字母／數字），例如 en_us、ja_jp、pt_br。
fn is_locale_code(s: &str) -> bool {
    let Some((a, b)) = s.split_once('_') else {
        return false;
    };
    if a.is_empty()
        || b.is_empty()
        || a.len() > 8
        || b.len() > 12
        || a.contains('_')
        || b.contains('_')
    {
        return false;
    }
    a.chars().all(|c| c.is_ascii_lowercase())
        && b.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn is_image_ext(lower_name: &str) -> bool {
    lower_name.ends_with(".png")
        || lower_name.ends_with(".jpg")
        || lower_name.ends_with(".jpeg")
        || lower_name.ends_with(".gif")
        || lower_name.ends_with(".webp")
        || lower_name.ends_with(".bmp")
        || lower_name.ends_with(".tga")
}

fn is_priority_lang_locale(locale: &str) -> bool {
    matches!(
        locale,
        "en_us" | "en_gb" | "zh_tw" | "zh_cn" | "zh_hk"
    )
}

fn push_lang_parse_error(
    locale: &str,
    detail: String,
    errors: &mut Vec<String>,
    non_priority_skips: &mut usize,
) {
    if is_priority_lang_locale(locale) {
        errors.push(detail);
    } else {
        *non_priority_skips += 1;
    }
}

fn harvest_folder_pack(
    pack_root: &Path,
    raw: &mut RawLang,
    errors: &mut Vec<String>,
    non_priority_skips: &mut usize,
) -> Result<usize, String> {
    // 標準：pack/assets/<ns>/lang/*
    let assets = pack_root.join("assets");
    if assets.is_dir() {
        return harvest_assets_tree(&assets, raw, 6, errors, non_priority_skips);
    }
    // 少數包根目錄直接是 assets
    let mut cache = ScanCache::default();
    harvest_loose_lang_tree(pack_root, raw, 6, errors, non_priority_skips, &mut cache)
}

fn harvest_assets_tree(
    assets: &Path,
    raw: &mut RawLang,
    max_depth: usize,
    errors: &mut Vec<String>,
    non_priority_skips: &mut usize,
) -> Result<usize, String> {
    let mut count = 0usize;
    for entry in WalkDir::new(assets)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if try_ingest_lang_file(path, raw, errors, non_priority_skips) {
            count += 1;
        }
    }
    Ok(count)
}

/// 單一 namespace 的三階合併（可單元測試）。
/// 回傳：(zh_out, en_only, from_tw, from_cn, from_hk)
pub(crate) fn merge_namespace_locales(
    ns: &str,
    locales: &HashMap<String, HashMap<String, String>>,
    do_opencc: bool,
    provenance: &mut ProvenanceMap,
) -> (
    HashMap<String, String>,
    HashMap<String, String>,
    usize,
    usize,
    usize,
) {
    let en_map = locales.get("en_us").or_else(|| locales.get("en_gb"));
    let mut out = HashMap::new();
    let mut from_tw = 0usize;
    let mut from_cn = 0usize;
    let mut from_hk = 0usize;

    if let Some(m) = locales.get("zh_tw") {
        for (k, v) in m {
            if out.contains_key(k) {
                continue;
            }
            let en = en_map.and_then(|e| e.get(k)).map(|s| s.as_str()).unwrap_or("");
            if is_usable_zh(en, v) {
                out.insert(k.clone(), v.clone());
                set_source(provenance, ns, k, LangSource::Tw);
                from_tw += 1;
            }
        }
    }

    if let Some(m) = locales.get("zh_cn") {
        for (k, v) in m {
            if out.contains_key(k) {
                continue;
            }
            let en = en_map.and_then(|e| e.get(k)).map(|s| s.as_str()).unwrap_or("");
            if is_usable_zh(en, v) {
                let text = if do_opencc {
                    convert_s2tw(v)
                } else {
                    v.clone()
                };
                out.insert(k.clone(), text);
                set_source(provenance, ns, k, LangSource::CnConverted);
                from_cn += 1;
            }
        }
    }

    if let Some(m) = locales.get("zh_hk") {
        for (k, v) in m {
            if out.contains_key(k) {
                continue;
            }
            let en = en_map.and_then(|e| e.get(k)).map(|s| s.as_str()).unwrap_or("");
            if is_usable_zh(en, v) {
                let text = if do_opencc {
                    convert_s2tw(v)
                } else {
                    v.clone()
                };
                out.insert(k.clone(), text);
                set_source(provenance, ns, k, LangSource::HkHint);
                from_hk += 1;
            }
        }
    }

    let mut eo = HashMap::new();
    let mut prefer: Vec<&str> = Vec::new();
    if locales.contains_key("en_us") {
        prefer.push("en_us");
    }
    if locales.contains_key("en_gb") {
        prefer.push("en_gb");
    }
    let mut others: Vec<&str> = locales
        .keys()
        .map(|s| s.as_str())
        .filter(|loc| {
            *loc != "zh_tw" && *loc != "zh_hk" && *loc != "zh_cn" && *loc != "en_us" && *loc != "en_gb"
        })
        .collect();
    others.sort_unstable();
    prefer.extend(others);
    for loc in ["zh_tw", "zh_cn", "zh_hk"] {
        if locales.contains_key(loc) {
            prefer.push(loc);
        }
    }

    for loc in &prefer {
        let Some(m) = locales.get(*loc) else {
            continue;
        };
        for (k, v) in m {
            if out.contains_key(k) || eo.contains_key(k) {
                continue;
            }
            if v.trim().is_empty() {
                continue;
            }
            eo.insert(k.clone(), v.clone());
        }
    }

    (out, eo, from_tw, from_cn, from_hk)
}

#[cfg(test)]
mod tests {
    use super::{is_priority_lang_locale, merge_namespace_locales, worker_count};
    use crate::engine::lang_provenance::{get_source, LangSource, ProvenanceMap};
    use crate::engine::translation_mode::skip_complete_namespaces_with_provenance;
    use std::collections::HashMap;

    #[test]
    fn worker_count_honors_safe_override_and_caps_to_input() {
        assert_eq!(worker_count(3, 16, Some(1)), 1);
        assert_eq!(worker_count(3, 16, Some(99)), 3);
        assert_eq!(worker_count(20, 4, None), 8);
        assert_eq!(worker_count(0, 4, Some(0)), 1);
    }

    #[test]
    fn priority_locales_cover_en_and_zh_variants() {
        assert!(is_priority_lang_locale("en_us"));
        assert!(is_priority_lang_locale("zh_tw"));
        assert!(is_priority_lang_locale("zh_cn"));
        assert!(!is_priority_lang_locale("ru_ru"));
        assert!(!is_priority_lang_locale("ja_jp"));
    }

    fn locale_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// E1：僅 zh_hk → HkHint，skip 不觸發
    #[test]
    fn e1_zh_hk_only_is_hint_not_skip() {
        let mut locales = HashMap::new();
        locales.insert(
            "zh_hk".into(),
            locale_map(&[("a", "打印機"), ("b", "資料夾")]),
        );
        locales.insert("en_us".into(), locale_map(&[("a", "Printer"), ("b", "Folder")]));
        let mut prov = ProvenanceMap::new();
        let (zh, eo, _tw, _cn, hk) = merge_namespace_locales("demo", &locales, true, &mut prov);
        assert_eq!(hk, 2);
        assert!(eo.is_empty());
        assert!(!get_source(&prov, "demo", "a")
            .unwrap_or(LangSource::Tw)
            .is_tw_playable());
        let mut pending = HashMap::new();
        pending.insert("demo".into(), locale_map(&[("c", "More")]));
        let mut zh_map = HashMap::new();
        zh_map.insert("demo".into(), zh);
        assert_eq!(
            skip_complete_namespaces_with_provenance(&zh_map, Some(&prov), &mut pending, 90),
            0
        );
    }

    /// E2：90% zh_hk + 10% en → 不 skip
    #[test]
    fn e2_mostly_hk_does_not_skip() {
        let mut locales = HashMap::new();
        let mut hk = locale_map(&[]);
        let mut en = locale_map(&[]);
        for i in 0..9 {
            let k = format!("k{i}");
            hk.insert(k.clone(), format!("港繁{i}"));
            en.insert(k, format!("En{i}"));
        }
        en.insert("k9".into(), "EnglishOnly".into());
        locales.insert("zh_hk".into(), hk);
        locales.insert("en_us".into(), en);
        let mut prov = ProvenanceMap::new();
        let (zh, eo, ..) = merge_namespace_locales("pack", &locales, true, &mut prov);
        assert_eq!(zh.len(), 9);
        assert_eq!(eo.len(), 1);
        let mut pending = HashMap::new();
        pending.insert("pack".into(), eo);
        let mut zh_map = HashMap::new();
        zh_map.insert("pack".into(), zh);
        assert_eq!(
            skip_complete_namespaces_with_provenance(&zh_map, Some(&prov), &mut pending, 90),
            0
        );
    }

    /// E3：全 zh_tw → 可 skip；文字不經 s2tw 改寫
    #[test]
    fn e3_full_zh_tw_can_skip_without_s2tw() {
        let mut locales = HashMap::new();
        locales.insert(
            "zh_tw".into(),
            locale_map(&[("a", "印表機"), ("b", "資料夾")]),
        );
        locales.insert("en_us".into(), locale_map(&[("a", "Printer"), ("b", "Folder")]));
        let mut prov = ProvenanceMap::new();
        let (zh, eo, tw, cn, hk) = merge_namespace_locales("fulltw", &locales, true, &mut prov);
        assert_eq!((tw, cn, hk), (2, 0, 0));
        assert!(eo.is_empty());
        assert_eq!(zh.get("a").map(String::as_str), Some("印表機"));
        assert_eq!(get_source(&prov, "fulltw", "a"), Some(LangSource::Tw));
        let mut pending = HashMap::new();
        pending.insert("fulltw".into(), locale_map(&[("x", "unused")]));
        let mut zh_map = HashMap::new();
        zh_map.insert("fulltw".into(), zh);
        // 待譯 1、可玩 2 → 覆蓋約 66%，用 50% 門檻應 skip
        assert_eq!(
            skip_complete_namespaces_with_provenance(&zh_map, Some(&prov), &mut pending, 50),
            1
        );
    }

    /// E4：同 key zh_tw 勝 zh_hk
    #[test]
    fn e4_zh_tw_wins_over_zh_hk() {
        let mut locales = HashMap::new();
        locales.insert("zh_tw".into(), locale_map(&[("a", "台灣譯")]));
        locales.insert("zh_hk".into(), locale_map(&[("a", "香港譯")]));
        locales.insert("en_us".into(), locale_map(&[("a", "A")]));
        let mut prov = ProvenanceMap::new();
        let (zh, ..) = merge_namespace_locales("ns", &locales, true, &mut prov);
        assert_eq!(zh.get("a").map(String::as_str), Some("台灣譯"));
        assert_eq!(get_source(&prov, "ns", "a"), Some(LangSource::Tw));
    }

    /// E5：zh_hk 打印機 → 印表機 + HkHint
    #[test]
    fn e5_zh_hk_s2tw_to_printer() {
        let mut locales = HashMap::new();
        locales.insert("zh_hk".into(), locale_map(&[("a", "打印機")]));
        locales.insert("en_us".into(), locale_map(&[("a", "Printer")]));
        let mut prov = ProvenanceMap::new();
        let (zh, ..) = merge_namespace_locales("hk", &locales, true, &mut prov);
        assert_eq!(zh.get("a").map(String::as_str), Some("印表機"));
        assert_eq!(get_source(&prov, "hk", "a"), Some(LangSource::HkHint));
    }

    /// E6：zh_cn 簡體 → CnConverted
    #[test]
    fn e6_zh_cn_converted() {
        let mut locales = HashMap::new();
        locales.insert("zh_cn".into(), locale_map(&[("a", "打印机")]));
        locales.insert("en_us".into(), locale_map(&[("a", "Printer")]));
        let mut prov = ProvenanceMap::new();
        let (zh, ..) = merge_namespace_locales("cn", &locales, true, &mut prov);
        assert_eq!(zh.get("a").map(String::as_str), Some("印表機"));
        assert_eq!(get_source(&prov, "cn", "a"), Some(LangSource::CnConverted));
        assert!(LangSource::CnConverted.is_tw_playable());
    }
}

fn harvest_loose_lang_tree(
    root: &Path,
    raw: &mut RawLang,
    max_depth: usize,
    errors: &mut Vec<String>,
    non_priority_skips: &mut usize,
    cache: &mut ScanCache,
) -> Result<usize, String> {
    let mut count = 0usize;
    for entry in WalkDir::new(root)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // 跳過明顯快取／巨大無關
        let s = path.to_string_lossy().to_ascii_lowercase();
        if s.contains("\\versions\\")
            || s.contains("/versions/")
            || s.contains("\\libraries\\")
            || s.contains("/libraries/")
            || s.contains("\\cache\\")
            || s.contains("/cache/")
        {
            continue;
        }
        if let Some(cached) = cache.get(path) {
            raw.entry(cached.namespace)
                .or_default()
                .entry(cached.locale)
                .or_default()
                .extend(cached.entries);
            count += 1;
            continue;
        }
        let mut parsed = RawLang::new();
        if try_ingest_lang_file(path, &mut parsed, errors, non_priority_skips) {
            for (namespace, locales) in &parsed {
                for (locale, entries) in locales {
                    cache.put(path, namespace.clone(), locale.clone(), entries.clone());
                }
            }
            merge_raw_lang(raw, parsed);
            count += 1;
        }
    }
    Ok(count)
}

/// 若是 assets/.../lang/<locale>.json|.lang 則吸入（接受所有 xx_yy locale）
fn try_ingest_lang_file(
    path: &Path,
    raw: &mut RawLang,
    errors: &mut Vec<String>,
    non_priority_skips: &mut usize,
) -> bool {
    let lower = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    if !lower.contains("/lang/") {
        return false;
    }
    if is_image_ext(&lower) {
        return false;
    }
    if !(lower.ends_with(".json") || lower.ends_with(".lang")) {
        return false;
    }
    let parts: Vec<&str> = lower.split('/').collect();
    let Some(li) = parts.iter().position(|p| *p == "lang") else {
        return false;
    };
    if li == 0 || li + 1 >= parts.len() {
        return false;
    }
    // 用實際路徑取 namespace（大小寫保留困難，用 parent 名）
    let ns = path
        .parent() // lang
        .and_then(|p| p.parent()) // ns
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    if !is_safe_ns(&ns) {
        return false;
    }
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let locale = fname
        .rsplit_once('.')
        .map(|(a, _)| a)
        .unwrap_or(&fname)
        .to_string();
    if !is_locale_code(&locale) {
        return false;
    }
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.len() == 0 || meta.len() > MAX_ZIP_ENTRY_BYTES {
        return false;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let map = if fname.ends_with(".json") {
        match parse_json_lang(&text) {
            Ok(m) => m,
            Err(e) => {
                push_lang_parse_error(
                    &locale,
                    format!("語言檔解析失敗（已略過）：{} — {e}", path.display()),
                    errors,
                    non_priority_skips,
                );
                return false;
            }
        }
    } else {
        parse_properties_lang(&text)
    };
    if map.is_empty() {
        return false;
    }
    raw.entry(ns)
        .or_default()
        .entry(locale)
        .or_default()
        .extend(map);
    true
}

fn is_safe_ns(ns: &str) -> bool {
    if ns.is_empty() || ns.len() > 64 || ns == ".." {
        return false;
    }
    ns.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

fn harvest_archive(
    path: &Path,
    raw: &mut RawLang,
    errors: &mut Vec<String>,
    non_priority_skips: &mut usize,
) -> Result<(), String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(f).map_err(|e| e.to_string())?;
    if zip.len() > 80_000 {
        return Err("壓縮包條目過多，已略過。".into());
    }
    for i in 0..zip.len() {
        let mut file = match zip.by_index(i) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let name_raw = file.name().to_string();
        if !is_safe_zip_entry_name(&name_raw) {
            continue;
        }
        let name = name_raw.replace('\\', "/");
        let lower = name.to_ascii_lowercase();
        if !lower.contains("/lang/") {
            continue;
        }
        if is_image_ext(&lower) {
            continue;
        }
        if !(lower.ends_with(".json") || lower.ends_with(".lang")) {
            continue;
        }
        if file.size() > MAX_ZIP_ENTRY_BYTES {
            continue;
        }
        let parts: Vec<&str> = lower.split('/').collect();
        let Some(li) = parts.iter().position(|p| *p == "lang") else {
            continue;
        };
        if li == 0 || li + 1 >= parts.len() {
            continue;
        }
        // namespace 用原始路徑片段（大小寫）；locale 用 lower
        let parts_orig: Vec<&str> = name.split('/').collect();
        let ns = if li < parts_orig.len() {
            parts_orig[li - 1].to_string()
        } else {
            parts[li - 1].to_string()
        };
        if !is_safe_ns(&ns) {
            continue;
        }
        let fname = parts[li + 1];
        let locale = fname
            .rsplit_once('.')
            .map(|(a, _)| a)
            .unwrap_or(fname)
            .to_string();
        if !is_locale_code(&locale) {
            continue;
        }
        let mut buf = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        let mut total = 0u64;
        let mut ok = true;
        loop {
            match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    total += n as u64;
                    if total > MAX_ZIP_ENTRY_BYTES {
                        ok = false;
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let text = String::from_utf8_lossy(&buf);
        let map = if lower.ends_with(".json") {
            match parse_json_lang(&text) {
                Ok(m) => m,
                Err(e) => {
                    push_lang_parse_error(
                        &locale,
                        format!(
                            "語言檔解析失敗（已略過）：{}!{} — {e}",
                            file_name_str(path),
                            name
                        ),
                        errors,
                        non_priority_skips,
                    );
                    continue;
                }
            }
        } else {
            parse_properties_lang(&text)
        };
        if map.is_empty() {
            continue;
        }
        raw.entry(ns)
            .or_default()
            .entry(locale)
            .or_default()
            .extend(map);
    }
    Ok(())
}

fn truncate_name(s: &str, max: usize) -> String {
    let t: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{t}…")
    } else {
        t
    }
}

fn file_name_str(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "檔案".into())
}

/// 解析語言 JSON（寬鬆：容忍註解／尾逗號）。真正壞掉回 `Err`，讓呼叫端記錄而非靜默。
fn parse_json_lang(text: &str) -> Result<HashMap<String, String>, String> {
    super::lenient_json::parse_object_strings(text)
}

fn parse_properties_lang(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_string(), v.to_string());
        }
    }
    out
}
