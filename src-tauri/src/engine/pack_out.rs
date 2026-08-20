use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::jar_scan::LangMap;
use super::security::{sanitize_folder_name, sanitize_namespace};

/// 找不到任何線索時的保底值（1.20.1，模組整合包長年主力版本）。
const FALLBACK_PACK_FORMAT: u32 = 15;

/// Minecraft 版本 → 資源包 `pack_format`。
///
/// 版本對不上時遊戲會把資源包標成「不相容」，玩家得手動點「仍要載入」，
/// 很多人就以為翻譯失敗了。所以這裡值得認真判斷，而不是寫死一個數字。
///
/// 對照表僅列各版本線的**起始版**；查表時取「不大於目標版本」的最後一筆。
/// 只列到 1.21.8（單一 `pack_format` 整數制的最後一段）；1.21.9＋與年份版（26.x）
/// 改用 `min_format`/`max_format` 範圍制，見 [`is_modern_pack_version`] 與 [`pack_mcmeta_value`]。
const VERSION_TO_FORMAT: &[(&str, u32)] = &[
    ("1.13", 4),
    ("1.14", 4),
    ("1.15", 5),
    ("1.16", 6),
    ("1.16.2", 6),
    ("1.17", 7),
    ("1.18", 8),
    ("1.19", 9),
    ("1.19.3", 12),
    ("1.19.4", 13),
    ("1.20", 15),
    ("1.20.2", 18),
    ("1.20.3", 22),
    ("1.20.5", 32),
    ("1.21", 34),
    ("1.21.2", 42),
    ("1.21.4", 46),
    ("1.21.5", 55),
    ("1.21.6", 63),
    ("1.21.9", 68),
];

/// 偵測 `pack_format`：先讀遊戲實例的版本，再退回既有資源包的 mcmeta。
pub fn detect_pack_format(minecraft_dir: &Path) -> u32 {
    if let Some(v) = detect_minecraft_version(minecraft_dir) {
        if let Some(f) = pack_format_for_version(&v) {
            return f;
        }
    }
    if let Some(f) = pack_format_from_existing_packs(minecraft_dir) {
        return f;
    }
    FALLBACK_PACK_FORMAT
}

/// 1.21.9＋與年份版（26.x…）改用範圍制 `min_format`/`max_format`。
/// 我們的翻譯包只有語言檔，實際上跨版本都能用，所以宣告一段寬範圍讓新版一律接受，
/// 同時保留 legacy `pack_format` 給 1.21.8 以下。這樣不必硬編每個 26.x 的確切格式號
/// （Mojang 2026 起改年份制、格式號還在往上跑），未來版本也自動涵蓋。
const MODERN_MIN_FORMAT: u32 = 6; // 涵蓋 1.16 之後的整個現代區間
const MODERN_MAX_FORMAT: u32 = 999; // 遠高於目前（26.1≈84），未來多年不必動

/// 目標版本是否屬於「範圍制」（1.21.9＋ 或年份制 26.x…）。
///
/// 判準：不是 `1.x` 開頭（→ 年份制），或是 `1.21.9` 以上。
pub fn is_modern_pack_version(version: &str) -> bool {
    let Some(v) = parse_version(version) else {
        return false;
    };
    if v[0] != 1 {
        // 26.x、27.x… 年份制
        return true;
    }
    // 1.x：1.21.9 起
    let minor = v.get(1).copied().unwrap_or(0);
    let patch = v.get(2).copied().unwrap_or(0);
    minor > 21 || (minor == 21 && patch >= 9)
}

/// 產生 `pack.mcmeta` 的 `pack` 物件。
/// - 舊版（≤1.21.8）：單一 `pack_format`（與過去完全一致，零回歸風險）。
/// - 新版（1.21.9＋／年份制）：`min_format`/`max_format` 範圍 ＋ 保留 legacy `pack_format`。
pub fn pack_mcmeta_value(
    target_version: Option<&str>,
    legacy_format: u32,
    description: &str,
) -> serde_json::Value {
    let fmt = if legacy_format == 0 {
        FALLBACK_PACK_FORMAT
    } else {
        legacy_format
    };
    let modern = target_version.map(is_modern_pack_version).unwrap_or(false);
    if modern {
        serde_json::json!({
            "pack": {
                "pack_format": fmt,
                "min_format": [MODERN_MIN_FORMAT, 0],
                "max_format": [MODERN_MAX_FORMAT, 0],
                "description": description
            }
        })
    } else {
        serde_json::json!({
            "pack": {
                "pack_format": fmt,
                "description": description
            }
        })
    }
}

/// 由版本字串（`1.20.1`、`1.21.4`）查出 pack_format。年份制（26.x）回 `None`
/// （交給範圍制的 mcmeta 處理），呼叫端據此不要當成確切整數用。
pub fn pack_format_for_version(version: &str) -> Option<u32> {
    let target = parse_version(version)?;
    if target[0] != 1 {
        return None; // 年份制沒有單一整數格式號
    }
    let mut best: Option<(Vec<u32>, u32)> = None;
    for (v, fmt) in VERSION_TO_FORMAT {
        let Some(parsed) = parse_version(v) else {
            continue;
        };
        if cmp_version(&parsed, &target) != std::cmp::Ordering::Greater {
            let better = match &best {
                Some((cur, _)) => cmp_version(&parsed, cur) == std::cmp::Ordering::Greater,
                None => true,
            };
            if better {
                best = Some((parsed, *fmt));
            }
        }
    }
    best.map(|(_, f)| f)
}

/// 本工具最低支援 Minecraft 1.13；年份版（major ≥ 26，如 26.1）另算支援。
pub fn is_supported_minecraft_version(version: &str) -> bool {
    let Some(parts) = parse_version(version) else {
        return false;
    };
    if parts[0] >= 26 {
        return true;
    }
    if parts[0] != 1 {
        return false;
    }
    cmp_version(&parts, &[1, 13]) != std::cmp::Ordering::Less
}

pub fn ensure_supported_minecraft_version(version: &str) -> Result<(), String> {
    if is_supported_minecraft_version(version) {
        Ok(())
    } else {
        Err(format!(
            "本工具僅支援 Minecraft 1.13 以上（含年份版 26.x），偵測到 {version}，無法翻譯。"
        ))
    }
}

/// 指定或偵測版本後過閘；兩者皆無或無法解析 → 要求手動指定 1.13+。
pub fn ensure_minecraft_version_for_translate(
    target_version: Option<&str>,
    minecraft_dir: &Path,
) -> Result<String, String> {
    let resolved = target_version
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| detect_minecraft_version(minecraft_dir));
    match resolved {
        Some(v) => {
            ensure_supported_minecraft_version(&v)?;
            Ok(v)
        }
        None => Err(
            "無法確認 Minecraft 版本。請從版本選單指定 1.13 以上（或年份版 26.x）後再翻譯。"
                .into(),
        ),
    }
}

fn parse_version(s: &str) -> Option<Vec<u32>> {
    let cleaned: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let parts: Vec<u32> = cleaned
        .split('.')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() < 2 {
        None
    } else {
        Some(parts)
    }
}

fn cmp_version(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// 從各家啟動器的實例設定裡找出 Minecraft 版本。
pub fn detect_minecraft_version(minecraft_dir: &Path) -> Option<String> {
    let instance_roots = [
        minecraft_dir.to_path_buf(),
        minecraft_dir.parent().map(|p| p.to_path_buf())?,
    ];

    for root in &instance_roots {
        // CurseForge / Overwolf
        if let Some(v) = read_json_pointer(
            &root.join("minecraftinstance.json"),
            &["baseModLoader", "minecraftVersion"],
        )
        .or_else(|| read_json_pointer(&root.join("minecraftinstance.json"), &["gameVersion"]))
        {
            return Some(v);
        }
        // Modrinth App
        if let Some(v) = read_json_pointer(&root.join("profile.json"), &["game_version"]) {
            return Some(v);
        }
        // Prism / MultiMC / PolyMC：mmc-pack.json 的 net.minecraft 元件
        if let Some(v) = read_mmc_pack(&root.join("mmc-pack.json")) {
            return Some(v);
        }
        // ATLauncher / GDLauncher 之類的 instance.json
        if let Some(v) = read_json_pointer(&root.join("instance.json"), &["id"])
            .or_else(|| read_json_pointer(&root.join("instance.json"), &["loaderVersion", "mcVersion"]))
        {
            if parse_version(&v).is_some() {
                return Some(v);
            }
        }
        // Prism 的 instance.cfg（純文字 key=value）
        if let Some(v) = read_ini_value(&root.join("instance.cfg"), "IntendedVersion") {
            return Some(v);
        }
    }

    // 官方啟動器：versions/<版本>/
    let versions = minecraft_dir.join("versions");
    if versions.is_dir() {
        let mut best: Option<(Vec<u32>, String)> = None;
        if let Ok(rd) = fs::read_dir(&versions) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(parsed) = parse_version(&name) {
                    let better = match &best {
                        Some((cur, _)) => cmp_version(&parsed, cur) == std::cmp::Ordering::Greater,
                        None => true,
                    };
                    if better {
                        best = Some((parsed, name));
                    }
                }
            }
        }
        if let Some((_, name)) = best {
            return Some(name);
        }
    }
    None
}

fn read_json_pointer(path: &Path, keys: &[&str]) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut cur = &v;
    for k in keys {
        cur = cur.get(*k)?;
    }
    let s = cur.as_str()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn read_mmc_pack(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    for comp in v.get("components")?.as_array()? {
        if comp.get("uid").and_then(|u| u.as_str()) == Some("net.minecraft") {
            let ver = comp.get("version").and_then(|x| x.as_str())?;
            return Some(ver.to_string());
        }
    }
    None
}

fn read_ini_value(path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                let v = v.trim().to_string();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn pack_format_from_existing_packs(minecraft_dir: &Path) -> Option<u32> {
    let rp = minecraft_dir.join("resourcepacks");
    if !rp.is_dir() {
        return None;
    }
    let rd = fs::read_dir(&rp).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = read_pack_format_from_mcmeta(&p.join("pack.mcmeta")) {
                return Some(f);
            }
        } else if p.extension().and_then(|s| s.to_str()) == Some("zip") {
            if let Ok(file) = File::open(&p) {
                if let Ok(mut z) = ZipArchive::new(file) {
                    for i in 0..z.len().min(40) {
                        if let Ok(mut ent) = z.by_index(i) {
                            let name = ent.name().replace('\\', "/");
                            if name.eq_ignore_ascii_case("pack.mcmeta")
                                || name.ends_with("/pack.mcmeta")
                            {
                                let mut s = String::new();
                                if ent.read_to_string(&mut s).is_ok() {
                                    if let Some(f) = parse_pack_format_json(&s) {
                                        return Some(f);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn read_pack_format_from_mcmeta(path: &Path) -> Option<u32> {
    let s = fs::read_to_string(path).ok()?;
    parse_pack_format_json(&s)
}

fn parse_pack_format_json(s: &str) -> Option<u32> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    v.get("pack")
        .and_then(|p| p.get("pack_format"))
        .and_then(|n| n.as_u64())
        .map(|n| n as u32)
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildOptions {
    pub pack_folder_name: String,
    pub pack_description: String,
    pub output_dir: String,
    /// legacy `pack_format`（≤1.21.8 用）；0＝保底 15
    pub pack_format: u32,
    /// 目標 Minecraft 版本字串（使用者指定或偵測到）。決定 pack.mcmeta 用整數制或範圍制。
    /// `None`＝沿用舊行為（只寫 legacy pack_format）。
    #[serde(default)]
    pub target_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildResult {
    /// 最終給玩家用的 .zip 路徑
    pub pack_path: String,
    /// 工作用資料夾（補翻可讀）
    pub pack_dir: String,
    pub namespaces: usize,
    pub files_written: usize,
    pub keys_total: usize,
}

/// 資源包輸出目錄：一律為「工作根/resourcepacks」
pub fn resourcepacks_root(work_root: &Path) -> PathBuf {
    if work_root
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("resourcepacks"))
        .unwrap_or(false)
    {
        work_root.to_path_buf()
    } else {
        work_root.join("resourcepacks")
    }
}

pub fn build_resource_pack(lang: &LangMap, opts: &BuildOptions) -> Result<BuildResult, String> {
    // opts.output_dir = 工作根（翻譯結果），不是使用者隨便選的任意層
    let work_root = PathBuf::from(&opts.output_dir);
    let safe_name = sanitize_folder_name(&opts.pack_folder_name)?;
    let rp_root = resourcepacks_root(&work_root);
    fs::create_dir_all(&rp_root).map_err(|e| e.to_string())?;

    // 工作目錄（資料夾）方便補翻讀寫
    let pack_dir = rp_root.join(&safe_name);
    if pack_dir.exists() {
        fs::remove_dir_all(&pack_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&pack_dir).map_err(|e| e.to_string())?;

    // 依目標版本決定整數制或範圍制（新版 26.x 需要範圍制才不會被標不相容）
    let meta = pack_mcmeta_value(
        opts.target_version.as_deref(),
        opts.pack_format,
        &opts.pack_description,
    );
    fs::write(
        pack_dir.join("pack.mcmeta"),
        serde_json::to_string_pretty(&meta).unwrap() + "\n",
    )
    .map_err(|e| e.to_string())?;

    let mut files = 1usize;
    let mut keys = 0usize;
    for (ns, map) in lang {
        if map.is_empty() {
            continue;
        }
        let Ok(safe_ns) = sanitize_namespace(ns) else {
            continue;
        };
        keys += map.len();
        let dir = pack_dir.join("assets").join(&safe_ns).join("lang");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut pairs: Vec<_> = map.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        let obj: serde_json::Map<String, serde_json::Value> = pairs
            .into_iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        let path = dir.join("zh_tw.json");
        let s = serde_json::to_string_pretty(&serde_json::Value::Object(obj))
            .map_err(|e| e.to_string())?;
        fs::write(&path, s + "\n").map_err(|e| e.to_string())?;
        files += 1;
    }

    let readme = pack_dir.join("使用說明.txt");
    let mut f = fs::File::create(&readme).map_err(|e| e.to_string())?;
    writeln!(
        f,
        "【怎麼用】\n\
1. 使用壓縮檔「{}.zip」（在「翻譯結果/resourcepacks」）。\n\
2. 複製到遊戲的 resourcepacks 資料夾。\n\
3. 啟動遊戲 → 設定 → 語言 → 繁體中文（台灣）。\n\
4. 設定 → 資源包 → 啟用「{}」。\n\
5. 若中文顯示很怪，把「強制使用 Unicode 字型」關掉再重開。\n\
6. 任務翻譯請見上一層 config\\ftbquests 與【請閱讀】輸出說明.txt。\n",
        safe_name, safe_name
    )
    .ok();
    files += 1;

    // 必為壓縮檔
    let zip_path = rp_root.join(format!("{safe_name}.zip"));
    if zip_path.exists() {
        let _ = fs::remove_file(&zip_path);
    }
    zip_dir_to_file(&pack_dir, &zip_path)?;

    Ok(BuildResult {
        pack_path: zip_path.display().to_string(),
        pack_dir: pack_dir.display().to_string(),
        namespaces: lang.len(),
        files_written: files,
        keys_total: keys,
    })
}

fn zip_dir_to_file(dir: &Path, zip_path: &Path) -> Result<(), String> {
    let file = File::create(zip_path).map_err(|e| format!("無法建立 zip：{e}"))?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path == dir {
            continue;
        }
        let rel = path
            .strip_prefix(dir)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            let name = if rel.ends_with('/') {
                rel
            } else {
                format!("{rel}/")
            };
            zip.add_directory(name, opts)
                .map_err(|e| format!("zip 目錄失敗：{e}"))?;
        } else if path.is_file() {
            zip.start_file(rel, opts)
                .map_err(|e| format!("zip 寫入失敗：{e}"))?;
            let mut f = File::open(path).map_err(|e| e.to_string())?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            zip.write_all(&buf).map_err(|e| e.to_string())?;
        }
    }
    zip.finish().map_err(|e| format!("zip 完成失敗：{e}"))?;
    Ok(())
}

/// 從資料夾或 .zip 讀取 zh_tw
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_minecraft_version_gate() {
        assert!(!is_supported_minecraft_version("1.12.2"));
        assert!(!is_supported_minecraft_version("1.7.10"));
        assert!(!is_supported_minecraft_version("1.12"));
        assert!(!is_supported_minecraft_version("not-a-version"));
        assert!(is_supported_minecraft_version("1.13"));
        assert!(is_supported_minecraft_version("1.13.2"));
        assert!(is_supported_minecraft_version("1.20.1"));
        assert!(is_supported_minecraft_version("26.1"));
        assert!(ensure_supported_minecraft_version("1.12.2").is_err());
        assert!(ensure_supported_minecraft_version("1.13").is_ok());
    }

    #[test]
    fn maps_known_versions_to_pack_format() {
        assert_eq!(pack_format_for_version("1.20.1"), Some(15));
        assert_eq!(pack_format_for_version("1.19.2"), Some(9));
        assert_eq!(pack_format_for_version("1.21.1"), Some(34));
        assert_eq!(pack_format_for_version("1.21.4"), Some(46));
        // 表已下探到 1.13
        assert_eq!(pack_format_for_version("1.13.2"), Some(4));
        assert_eq!(pack_format_for_version("1.15.2"), Some(5));
    }

    #[test]
    fn year_based_versions_have_no_single_integer_format() {
        // 26.x 是範圍制，沒有單一整數格式號
        assert_eq!(pack_format_for_version("26.2"), None);
        assert_eq!(pack_format_for_version("27.1"), None);
    }

    #[test]
    fn modern_version_detection() {
        // 年份制
        assert!(is_modern_pack_version("26.2"));
        assert!(is_modern_pack_version("26.1"));
        // 1.21.9 起
        assert!(is_modern_pack_version("1.21.9"));
        assert!(is_modern_pack_version("1.22.0"));
        // 舊制
        assert!(!is_modern_pack_version("1.21.8"));
        assert!(!is_modern_pack_version("1.20.1"));
        assert!(!is_modern_pack_version("1.16.5"));
    }

    #[test]
    fn classic_version_writes_single_pack_format() {
        let v = pack_mcmeta_value(Some("1.20.1"), 15, "台灣繁中");
        assert_eq!(v["pack"]["pack_format"], 15);
        // 舊版不寫範圍欄位（維持與過去一致、零回歸）
        assert!(v["pack"].get("min_format").is_none());
    }

    #[test]
    fn modern_version_writes_range_plus_legacy() {
        let v = pack_mcmeta_value(Some("26.2"), 68, "台灣繁中");
        // 新版讀範圍
        assert_eq!(v["pack"]["min_format"][0], MODERN_MIN_FORMAT);
        assert_eq!(v["pack"]["max_format"][0], MODERN_MAX_FORMAT);
        // 同時保留 legacy 給 1.21.8 以下
        assert_eq!(v["pack"]["pack_format"], 68);
    }

    #[test]
    fn no_target_version_keeps_legacy_behaviour() {
        let v = pack_mcmeta_value(None, 0, "x");
        assert_eq!(v["pack"]["pack_format"], FALLBACK_PACK_FORMAT);
        assert!(v["pack"].get("min_format").is_none());
    }

    #[test]
    fn unlisted_patch_versions_fall_back_to_their_line() {
        // 1.20.4 沒有單獨列，應沿用 1.20.3 那一線
        assert_eq!(pack_format_for_version("1.20.4"), Some(22));
        // 比表格最新版還新 → 取最後一筆，而不是回到保底的 15
        assert_eq!(pack_format_for_version("1.99.0"), Some(68));
    }

    #[test]
    fn versions_below_the_table_are_unknown() {
        assert_eq!(pack_format_for_version("1.12.2"), None);
    }

    #[test]
    fn tolerates_launcher_suffixes() {
        assert_eq!(pack_format_for_version("1.20.1-forge-47.2.0"), Some(15));
    }

    #[test]
    fn rejects_non_version_strings() {
        assert_eq!(pack_format_for_version("fabric-loader"), None);
        assert_eq!(pack_format_for_version(""), None);
    }

    #[test]
    fn version_ordering_is_numeric_not_lexical() {
        // 字串比較會說 "1.9" > "1.21"，數字比較不會
        assert_eq!(
            cmp_version(&parse_version("1.21").unwrap(), &parse_version("1.9").unwrap()),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn missing_instance_falls_back_instead_of_panicking() {
        let fmt = detect_pack_format(Path::new("Z:/definitely/not/here"));
        assert_eq!(fmt, FALLBACK_PACK_FORMAT);
    }

    #[test]
    fn lang_json_write_is_valid_utf8_with_cjk() {
        let root = std::env::temp_dir().join(format!("pack_utf8_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut lang: LangMap = HashMap::new();
        lang.entry("minecraft".into()).or_default().insert(
            "block.minecraft.stone".into(),
            "石頭與繁中測試—UTF8".into(),
        );
        let opts = BuildOptions {
            output_dir: root.display().to_string(),
            pack_folder_name: "utf8pack".into(),
            pack_description: "測試".into(),
            pack_format: 15,
            target_version: Some("1.20.1".into()),
        };
        let built = build_resource_pack(&lang, &opts).unwrap();
        let zh_path = PathBuf::from(&built.pack_dir)
            .join("assets/minecraft/lang/zh_tw.json");
        let bytes = fs::read(&zh_path).unwrap();
        assert!(std::str::from_utf8(&bytes).is_ok(), "must be UTF-8");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("石頭與繁中測試—UTF8"));
        assert!(!text.contains('Ã'));
        let _ = fs::remove_dir_all(root);
    }
}

pub fn load_pack_zh_any(pack_path: &Path) -> Result<LangMap, String> {
    if pack_path.is_dir() {
        return load_pack_zh_dir(pack_path);
    }
    if pack_path.is_file()
        && pack_path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("zip"))
            .unwrap_or(false)
    {
        return load_pack_zh_zip(pack_path);
    }
    // 容錯：同名資料夾
    if let Some(stem) = pack_path.file_stem() {
        let dir = pack_path.with_file_name(stem);
        if dir.is_dir() {
            return load_pack_zh_dir(&dir);
        }
    }
    Err("找不到資源包（資料夾或 .zip）。".into())
}

fn load_pack_zh_dir(pack_path: &Path) -> Result<LangMap, String> {
    let assets = pack_path.join("assets");
    if !assets.is_dir() {
        return Err("資源包資料夾不完整（沒有 assets）。".into());
    }
    let mut zh: LangMap = HashMap::new();
    for entry in WalkDir::new(&assets).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name != "zh_tw.json" {
            continue;
        }
        let parts: Vec<_> = path
            .strip_prefix(&assets)
            .unwrap_or(path)
            .components()
            .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
            .collect();
        if parts.len() < 3 {
            continue;
        }
        let ns = parts[0].clone();
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let map: HashMap<String, String> = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(_) => continue,
        };
        zh.entry(ns).or_default().extend(map);
    }
    Ok(zh)
}

fn load_pack_zh_zip(zip_path: &Path) -> Result<LangMap, String> {
    let f = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut zh: LangMap = HashMap::new();
    for i in 0..zip.len() {
        let mut file = match zip.by_index(i) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let name = file.name().replace('\\', "/");
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with("/lang/zh_tw.json") && !lower.ends_with("lang/zh_tw.json") {
            continue;
        }
        // assets/<ns>/lang/zh_tw.json
        let parts: Vec<&str> = name.split('/').collect();
        let Some(li) = parts.iter().position(|p| *p == "lang") else {
            continue;
        };
        if li == 0 {
            continue;
        }
        let ns = parts[li - 1].to_string();
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            continue;
        }
        let map: HashMap<String, String> = match serde_json::from_str(&buf) {
            Ok(m) => m,
            Err(_) => continue,
        };
        zh.entry(ns).or_default().extend(map);
    }
    if zh.is_empty() {
        return Err("zip 內找不到 zh_tw.json。".into());
    }
    Ok(zh)
}
