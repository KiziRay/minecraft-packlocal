//! 本地蒐集語言檔：**全程不呼叫 AI**。
//! 來源：mods jar、資源包、KubeJS、config 內 lang、常見設定亂碼處理前置資料。

use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use walkdir::WalkDir;
use zip::ZipArchive;

use super::cancel;
use super::convert::{apply_phrase_dict, convert_s2tw_batch, strip_of_suffix_zhi};
use super::security::{check_jar_size, is_safe_zip_entry_name, MAX_ZIP_ENTRY_BYTES};

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
/// 回傳 (已有中文 map, 僅英文待譯 map, 報告)
pub fn scan_instance<F>(
    instance_or_mc: &Path,
    phrase_dict: &HashMap<String, String>,
    do_opencc: bool,
    strip_of_zhi: bool,
    mut on_progress: F,
) -> Result<(LangMap, LangMap, ScanReport), String>
where
    F: FnMut(u8, &str),
{
    let mc = resolve_minecraft_dir(instance_or_mc)?;
    let mut raw: RawLang = HashMap::new();
    let mut jars = 0usize;
    let mut rps = 0usize;
    let mut loose = 0usize;
    let mut errors = Vec::new();

    // ─── 1) mods jar / zip（平行掃描；含 Essential 等雙 jar，只讀 lang 不拆包）───
    on_progress(6, "本地整理：列出模組檔…");
    let jar_paths = list_archive_files(&mc.join("mods"), 2);
    let total_j = jar_paths.len().max(1);
    on_progress(
        8,
        &format!("本地整理：讀模組語言（共 {} 個）…", jar_paths.len()),
    );
    if !jar_paths.is_empty() {
        let n_workers = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
            .clamp(8, 16)
            .min(jar_paths.len());
        let chunk_size = (jar_paths.len() + n_workers - 1) / n_workers;
        let mut handles = Vec::with_capacity(n_workers);
        for chunk in jar_paths.chunks(chunk_size) {
            let chunk: Vec<PathBuf> = chunk.to_vec();
            handles.push(thread::spawn(move || {
                let mut local_raw: RawLang = HashMap::new();
                let mut local_errors: Vec<String> = Vec::new();
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
                    if let Err(e) = harvest_archive(path, &mut local_raw, &mut local_errors) {
                        local_errors.push(format!("{}: {}", name, e));
                    }
                }
                (local_raw, local_errors, local_jars)
            }));
        }
        let mut done = 0usize;
        for (gi, handle) in handles.into_iter().enumerate() {
            match handle.join() {
                Ok((partial, errs, count)) => {
                    merge_raw_lang(&mut raw, partial);
                    errors.extend(errs);
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
                match harvest_folder_pack(path, &mut raw, &mut errors) {
                    Ok(n) => loose += n,
                    Err(e) => errors.push(format!("{}: {}", file_name_str(path), e)),
                }
            } else {
                if let Err(e) = check_jar_size(path) {
                    errors.push(format!("{}: {}", file_name_str(path), e));
                    continue;
                }
                if let Err(e) = harvest_archive(path, &mut raw, &mut errors) {
                    errors.push(format!("{}: {}", file_name_str(path), e));
                }
            }
        }
    }

    // ─── 3) kubejs / config 鬆散 lang（不含 versions 快取）───
    on_progress(30, "本地整理：讀 KubeJS／設定內語言檔…");
    for sub in ["kubejs", "config"] {
        let root = mc.join(sub);
        if !root.is_dir() {
            continue;
        }
        match harvest_loose_lang_tree(&root, &mut raw, 8, &mut errors) {
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

    cancel::check()?;

    // ─── 4) 合併：zh_tw/zh_hk → 繁中；zh_cn 補缺；其餘非繁中 locale 當待譯來源 ───
    on_progress(33, "本地整理：合併各來源中文與待譯文…");
    let mut zh: LangMap = HashMap::new();
    let mut en_only: LangMap = HashMap::new();
    let mut from_tw = 0usize;
    let mut from_cn = 0usize;

    for (ns, locales) in raw {
        let mut out = HashMap::new();
        for loc in ["zh_tw", "zh_hk"] {
            if let Some(m) = locales.get(loc) {
                for (k, v) in m {
                    if !out.contains_key(k) {
                        out.insert(k.clone(), v.clone());
                        from_tw += 1;
                    }
                }
            }
        }
        if let Some(m) = locales.get("zh_cn") {
            for (k, v) in m {
                if !out.contains_key(k) {
                    out.insert(k.clone(), v.clone());
                    from_cn += 1;
                }
            }
        }

        // 缺 key：en_us → en_gb → 其他非 zh_tw/zh_hk locale（ja_jp、ko_kr、zh_cn…）
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
            .filter(|loc| *loc != "zh_tw" && *loc != "zh_hk" && *loc != "en_us" && *loc != "en_gb")
            .collect();
        others.sort_unstable();
        prefer.extend(others);

        for loc in &prefer {
            let Some(m) = locales.get(*loc) else {
                continue;
            };
            for (k, v) in m {
                if !out.contains_key(k) && !eo.contains_key(k) {
                    eo.insert(k.clone(), v.clone());
                }
            }
        }
        if !eo.is_empty() {
            en_only.insert(ns.clone(), eo);
        }
        if !out.is_empty() {
            zh.insert(ns, out);
        }
    }

    // ─── 5) OpenCC／字典／「之」— 仍純本地 ───
    if do_opencc {
        on_progress(35, "本地整理：簡體→台灣繁體（OpenCC，非 AI）…");
        let mut flat: Vec<(String, String)> = Vec::new();
        for (ns, map) in &zh {
            for (k, v) in map {
                flat.push((format!("{ns}\0{k}"), v.clone()));
            }
        }
        let texts: Vec<String> = flat.iter().map(|(_, v)| v.clone()).collect();
        let converted = convert_s2tw_batch(&texts);
        for (i, (id, _)) in flat.iter().enumerate() {
            let Some((ns, k)) = id.split_once('\0') else {
                continue;
            };
            if let Some(map) = zh.get_mut(ns) {
                if let Some(slot) = map.get_mut(k) {
                    if let Some(nv) = converted.get(i) {
                        *slot = nv.clone();
                    }
                }
            }
        }
    }

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

    on_progress(
        40,
        &format!(
            "本地整理完成：模組 {}、資源包 {}、鬆散語言檔 {}；中文 {} 條、待 AI {} 條",
            jars, rps, loose, keys_zh, keys_need_ai
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
        errors,
    };
    Ok((zh, en_only, report))
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

fn harvest_folder_pack(
    pack_root: &Path,
    raw: &mut RawLang,
    errors: &mut Vec<String>,
) -> Result<usize, String> {
    // 標準：pack/assets/<ns>/lang/*
    let assets = pack_root.join("assets");
    if assets.is_dir() {
        return harvest_assets_tree(&assets, raw, 6, errors);
    }
    // 少數包根目錄直接是 assets
    harvest_loose_lang_tree(pack_root, raw, 6, errors)
}

fn harvest_assets_tree(
    assets: &Path,
    raw: &mut RawLang,
    max_depth: usize,
    errors: &mut Vec<String>,
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
        if try_ingest_lang_file(path, raw, errors) {
            count += 1;
        }
    }
    Ok(count)
}

fn harvest_loose_lang_tree(
    root: &Path,
    raw: &mut RawLang,
    max_depth: usize,
    errors: &mut Vec<String>,
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
        if try_ingest_lang_file(path, raw, errors) {
            count += 1;
        }
    }
    Ok(count)
}

/// 若是 assets/.../lang/<locale>.json|.lang 則吸入（接受所有 xx_yy locale）
fn try_ingest_lang_file(path: &Path, raw: &mut RawLang, errors: &mut Vec<String>) -> bool {
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
                errors.push(format!("語言檔解析失敗（已略過）：{} — {e}", path.display()));
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

fn harvest_archive(path: &Path, raw: &mut RawLang, errors: &mut Vec<String>) -> Result<(), String> {
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
                    errors.push(format!(
                        "語言檔解析失敗（已略過）：{}!{} — {e}",
                        file_name_str(path),
                        name
                    ));
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
