//! B6a-1：mods/ 相關的更新判斷（從 pack_update.rs 拆出，控制檔案大小）。全部只讀。
//!
//! - 模組清單以「原檔」為準（審查 F3）：工具覆蓋進 mods/ 的翻譯版（`.mcpl` 標記認得是工具內容）
//!   改用原檔備份的大小（沒有備份時用上一輪記下的大小），套用翻譯不會被當成「整合包已更新」，
//!   也不會把工具版當成原檔記下來。
//! - 「模組被拿掉」必須先確認（審查 F1）：掃描沒有錯誤，而且上一輪提供該命名空間的模組檔
//!   確實已不在 mods/（`.jar.disabled` 算暫時停用、還在）。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::jar_scan::{resolve_minecraft_dir, LangMap};
use super::pack_update::{text_hash, UpdateBasis};

fn game(instance: &Path) -> PathBuf {
    resolve_minecraft_dir(instance).unwrap_or_else(|_| instance.to_path_buf())
}

/// mods/ 第一層的 .jar：小寫檔名 → (實際路徑, 大小)。讀不到回空。
fn jars(mc: &Path) -> BTreeMap<String, (PathBuf, u64)> {
    let Ok(entries) = fs::read_dir(mc.join("mods")) else { return BTreeMap::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.extension().and_then(|s| s.to_str())?.eq_ignore_ascii_case("jar") {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
            let size = e.metadata().ok()?.len();
            Some((name, (path, size)))
        })
        .collect()
}

/// mods/ 第一層所有檔名（小寫，含 `.jar.disabled`），判斷「模組檔還在不在」用。讀不到回空（＝無法確認）。
pub fn mods_now(instance: &Path) -> BTreeSet<String> {
    let Ok(entries) = fs::read_dir(game(instance).join("mods")) else { return BTreeSet::new() };
    entries.flatten().map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).collect()
}

/// 以原檔為準的模組清單（審查 F3）。`recorded`＝上一輪記下的清單（沒有給空）；
/// `work`＝翻譯結果（jar-translated 裡同大小的就是工具放的那份，免算指紋）。
pub fn effective_mod_files(instance: &Path, work: Option<&Path>, recorded: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    let mc = game(instance);
    let mut out = BTreeMap::new();
    let mut index: Option<super::tool_products::ToolIndex> = None;
    for (name, (path, size)) in jars(&mc) {
        if recorded.get(&name) == Some(&size) {
            out.insert(name, size);
            continue;
        }
        let rel = super::apply_record::rel_key(&mc, &path);
        let Some(marker) = super::mcpl_marker::read_file_marker(&mc, &rel) else {
            out.insert(name, size);
            continue;
        };
        let same_as_output = work.and_then(|w| {
            let file = path.file_name()?;
            fs::metadata(w.join("jar-translated").join(file)).ok().map(|m| m.len() == size)
        }) == Some(true);
        let tool = same_as_output || marker.is_tool_content(super::apply_record::file_sha256(&path).as_deref());
        if !tool {
            out.insert(name, size);
            continue;
        }
        // 第二輪 2：原檔備份一律經 ToolIndex 確認（備份標記屬於本包、就是這個檔、原檔指紋相符）才採用它的大小
        let index = index.get_or_insert_with(|| super::tool_products::ToolIndex::for_game(&mc));
        let original = index
            .original_backup(&path)
            .and_then(|b| fs::metadata(&b).ok())
            .map(|m| m.len())
            .or_else(|| recorded.get(&name).copied());
        out.insert(name, original.unwrap_or(size));
    }
    out
}

/// 模組有沒有變：有記模組清單就逐檔比（以原檔為準）；舊工作階段只有指紋時照舊比指紋。讀不到＝不判。
pub fn mods_changed(recorded_fingerprint: u64, basis: &UpdateBasis, instance: &Path, work: Option<&Path>) -> bool {
    if !basis.mod_files.is_empty() {
        let now = effective_mod_files(instance, work, &basis.mod_files);
        return !now.is_empty() && now != basis.mod_files;
    }
    let live = super::session::mods_fingerprint(instance);
    recorded_fingerprint != 0 && live != 0 && recorded_fingerprint != live
}

/// 每個命名空間由哪些模組檔提供（小寫檔名；只看 mods/ 第一層的 assets/<ns>/lang/）。
pub fn ns_jars(instance: &Path) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, (path, _)) in jars(&game(instance)) {
        let Ok(file) = fs::File::open(&path) else { continue };
        let Ok(zip) = zip::ZipArchive::new(file) else { continue };
        let mut seen = BTreeSet::new();
        for entry in zip.file_names() {
            let entry = entry.replace('\\', "/");
            let parts: Vec<&str> = entry.split('/').collect();
            if parts.len() == 4 && parts[0].eq_ignore_ascii_case("assets") && parts[2].eq_ignore_ascii_case("lang") {
                seen.insert(parts[1].to_ascii_lowercase());
            }
        }
        for ns in seen {
            out.entry(ns).or_default().push(name.clone());
        }
    }
    out
}

/// 掃描有沒有錯誤（「不影響本次翻譯」的提醒不算）。
pub fn scan_is_clean(errors: &[String]) -> bool {
    errors.iter().all(|e| e.contains("不影響"))
}

/// 「模組被拿掉」的前置條件。
pub struct RemovalCheck<'a> {
    /// 這次重掃沒有任何錯誤
    pub scan_clean: bool,
    /// 上一輪：命名空間 → 提供它的模組檔（小寫檔名）
    pub old_ns_jars: &'a BTreeMap<String, Vec<String>>,
    /// 現在 mods/ 裡的所有檔名（小寫）
    pub mods_now: &'a BTreeSet<String>,
}

impl RemovalCheck<'_> {
    /// 確定被拿掉：掃描乾淨、上一輪記得是哪些模組檔、那些檔（含 `.disabled`）都不在了。
    pub fn surely_removed(&self, ns: &str) -> bool {
        // 第二輪 1：mods/ 讀不到或是空的（網路磁碟斷線、整個被清掉）＝無法確認，一律不清
        if !self.scan_clean || self.mods_now.is_empty() {
            return false;
        }
        let Some(files) = self.old_ns_jars.get(ns).filter(|f| !f.is_empty()) else { return false };
        files.iter().all(|f| !self.mods_now.contains(f) && !self.mods_now.contains(&format!("{f}.disabled")))
    }
}

/// 只讀新增／更新（檔名或原檔大小不同）的模組檔的英文語言檔，數出（新增模組, 更新模組, 句數）。
pub fn changed_sentences(instance: &Path, work: Option<&Path>, basis: &UpdateBasis) -> (usize, usize, usize) {
    let mc = game(instance);
    let now = effective_mod_files(&mc, work, &basis.mod_files);
    let paths = jars(&mc);
    let mut new_ns = BTreeSet::new();
    let mut updated_ns = BTreeSet::new();
    let mut sentences = 0usize;
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (name, size) in now {
        if basis.mod_files.get(&name) == Some(&size) {
            continue;
        }
        let Some((path, _)) = paths.get(&name) else { continue };
        for (ns, entries) in english_in_jar(path) {
            let old = basis.source_hashes.get(&ns);
            for (key, text) in entries {
                if text.trim().is_empty() || !seen.insert((ns.clone(), key.clone())) {
                    continue;
                }
                if old.and_then(|m| m.get(&key)).map(|h| h != &text_hash(&text)).unwrap_or(true) {
                    sentences += 1;
                    if old.is_some() {
                        updated_ns.insert(ns.clone());
                    } else {
                        new_ns.insert(ns.clone());
                    }
                }
            }
        }
    }
    (new_ns.len(), updated_ns.len(), sentences)
}

/// 模組檔裡的 en_us 語言檔（json 或舊式 .lang）。讀不懂回空（估算用，不擋人）。
fn english_in_jar(jar: &Path) -> LangMap {
    let mut out = LangMap::new();
    if super::security::check_jar_size(jar).is_err() {
        return out;
    }
    let Ok(file) = fs::File::open(jar) else { return out };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return out };
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else { continue };
        let name = entry.name().replace('\\', "/");
        let parts: Vec<&str> = name.split('/').collect();
        if parts.len() != 4 || !parts[0].eq_ignore_ascii_case("assets") || !parts[2].eq_ignore_ascii_case("lang") {
            continue;
        }
        let file_name = parts[3].to_ascii_lowercase();
        let json = file_name == "en_us.json";
        if (!json && file_name != "en_us.lang") || entry.size() > super::security::MAX_ZIP_ENTRY_BYTES {
            continue;
        }
        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_err() {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let text = text.trim_start_matches('\u{FEFF}');
        let map: Vec<(String, String)> = if json {
            super::lenient_json::parse_object_strings(text).map(|m| m.into_iter().collect()).unwrap_or_default()
        } else {
            text.lines()
                .filter(|l| !l.trim_start().starts_with('#'))
                .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.to_string())))
                .collect()
        };
        out.entry(parts[1].to_ascii_lowercase()).or_default().extend(map);
    }
    out
}
