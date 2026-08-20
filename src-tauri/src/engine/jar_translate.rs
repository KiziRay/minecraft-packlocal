//! Translate language files inside copied JARs.
//!
//! The source JAR is never opened for writing. A complete archive copy is
//! written under the managed work directory, and only the copy is installed
//! into the instance after the normal backup step.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use super::cancel;
use super::jar_scan::{resolve_minecraft_dir, LangMap};
use super::security::{check_jar_size, is_safe_zip_entry_name, MAX_ZIP_ENTRY_BYTES};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JarTranslationReport {
    pub jars_scanned: usize,
    pub jars_rewritten: usize,
    pub lang_files_written: usize,
    pub keys_written: usize,
    pub fallback_keys_kept: usize,
    pub output_root: String,
    pub errors: Vec<String>,
}

/// Rebuild translated copies of every mod JAR with a usable zh_tw language
/// file. The original files in `mods/` are read-only inputs.
pub fn rewrite_translated_jars(
    instance_or_mc: &Path,
    translated: &LangMap,
    fallback_english: &LangMap,
    work_root: &Path,
    mut on_progress: impl FnMut(u64, u64, &str),
) -> Result<JarTranslationReport, String> {
    let mc = resolve_minecraft_dir(instance_or_mc)?;
    let mods = mc.join("mods");
    let output_root = work_root.join("jar-translated");
    if output_root.exists() {
        fs::remove_dir_all(&output_root)
            .map_err(|e| format!("無法清理舊的 JAR 翻譯副本：{e}"))?;
    }
    fs::create_dir_all(&output_root).map_err(|e| e.to_string())?;

    let mut report = JarTranslationReport {
        output_root: output_root.display().to_string(),
        ..Default::default()
    };
    let values = merge_values(translated, fallback_english);
    let jars = list_jars(&mods);
    report.jars_scanned = jars.len();

    let workers = std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 8))
        .unwrap_or(4);
    let total_done = 0usize;
    if report.jars_scanned > 0 {
        on_progress(0, report.jars_scanned as u64, "JAR 翻譯副本：準備重建…");
    }
    let mut done = total_done;
    for chunk in jars.chunks(workers) {
        cancel::check()?;
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for jar in chunk {
                let jar = jar.clone();
                let output_root = output_root.clone();
                let mods = mods.clone();
                let values = &values;
                let translated = translated;
                handles.push(scope.spawn(move || {
                    let relative = match jar.strip_prefix(&mods) {
                        Ok(r) => r.to_path_buf(),
                        Err(e) => {
                            return Err(format!("JAR 路徑不在 mods 目錄內：{e}"));
                        }
                    };
                    let output = output_root.join(&relative);
                    let stem = jar
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown.jar")
                        .to_string();
                    match rewrite_one_jar(&jar, &output, values, translated) {
                        Ok(stats) if stats.changed => Ok(Some((stem, stats))),
                        Ok(_) => {
                            let _ = remove_empty_parent(&output, &output_root);
                            Ok(None)
                        }
                        Err(error) => Err(format!("{stem}：{error}")),
                    }
                }));
            }
            for handle in handles {
                match handle.join() {
                    Ok(Ok(Some((_stem, stats)))) => {
                        report.jars_rewritten += 1;
                        report.lang_files_written += stats.files_written;
                        report.keys_written += stats.keys_written;
                        report.fallback_keys_kept += stats.fallback_keys_kept;
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => report.errors.push(error),
                    Err(_) => report.errors.push("JAR 翻譯副本背景工作 panic".into()),
                }
                done += 1;
                on_progress(
                    done as u64,
                    report.jars_scanned as u64,
                    &format!(
                        "JAR 翻譯副本：已處理 {}/{} 個 JAR",
                        done,
                        report.jars_scanned
                    ),
                );
            }
        });
    }

    Ok(report)
}

#[derive(Debug, Default)]
struct RewriteStats {
    changed: bool,
    files_written: usize,
    keys_written: usize,
    fallback_keys_kept: usize,
}

fn merge_values(translated: &LangMap, fallback_english: &LangMap) -> HashMap<String, BTreeMap<String, String>> {
    let mut out: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    for (namespace, entries) in fallback_english {
        let target = out.entry(namespace.clone()).or_default();
        for (key, value) in entries {
            target.insert(key.clone(), value.clone());
        }
    }
    for (namespace, entries) in translated {
        let target = out.entry(namespace.clone()).or_default();
        for (key, value) in entries {
            target.insert(key.clone(), value.clone());
        }
    }
    out
}

fn list_jars(mods: &Path) -> Vec<PathBuf> {
    if !mods.is_dir() {
        return Vec::new();
    }
    WalkDir::new(mods)
        .min_depth(1)
        .max_depth(2)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("jar"))
                    .unwrap_or(false)
        })
        .collect()
}

fn rewrite_one_jar(
    source: &Path,
    output: &Path,
    values: &HashMap<String, BTreeMap<String, String>>,
    translated: &LangMap,
) -> Result<RewriteStats, String> {
    check_jar_size(source)?;
    let source_file = File::open(source).map_err(|e| e.to_string())?;
    let mut source_zip = ZipArchive::new(source_file).map_err(|e| format!("JAR 不是有效 ZIP：{e}"))?;
    let mut existing_targets: BTreeMap<String, String> = BTreeMap::new();
    let mut jar_namespaces = BTreeSet::new();
    let mut language_templates: HashMap<String, (String, String)> = HashMap::new();

    for index in 0..source_zip.len() {
        let entry = source_zip
            .by_index(index)
            .map_err(|e| format!("讀取 JAR 目錄失敗：{e}"))?;
        let name = entry.name().replace('\\', "/");
        if !is_safe_zip_entry_name(&name) {
            return Err(format!("包含不安全的 ZIP 路徑：{name}"));
        }
        if is_signature_path(&name) {
            return Err("JAR 含有簽章檔（META-INF/*.SF、*.RSA、*.DSA 或 *.EC），拒絕重打包以免留下失效簽章。".into());
        }
        if let Some((namespace, _, extension)) = parse_language_path(&name) {
            jar_namespaces.insert(namespace.clone());
            if let Some((directory, _)) = name.rsplit_once('/') {
                language_templates
                    .entry(namespace.clone())
                    .or_insert_with(|| (directory.to_string(), extension.clone()));
            }
        }
        if let Some((namespace, extension)) = parse_zh_tw_path(&name) {
            if translated.contains_key(&namespace) {
                existing_targets.insert(name, extension);
            }
        }
    }
    drop(source_zip);

    let mut targets_by_namespace: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (name, extension) in &existing_targets {
        if let Some((namespace, _, _)) = parse_language_path(name) {
            targets_by_namespace
                .entry(namespace)
                .or_default()
                .push((name.clone(), extension.clone()));
        }
    }
    for namespace in translated.keys().filter(|namespace| jar_namespaces.contains(*namespace)) {
        if !targets_by_namespace.contains_key(namespace) {
            let (directory, extension) = language_templates
                .get(namespace)
                .cloned()
                .unwrap_or_else(|| (format!("assets/{namespace}/lang"), "json".to_string()));
            targets_by_namespace.insert(
                namespace.clone(),
                vec![(
                    format!("{directory}/zh_tw.{extension}"),
                    extension,
                )],
            );
        }
    }
    if targets_by_namespace.is_empty() {
        return Ok(RewriteStats::default());
    }

    let parent = output
        .parent()
        .ok_or_else(|| "JAR 輸出路徑沒有父資料夾".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = output.with_extension("jar.tmp");
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|e| e.to_string())?;
    }

    let source_file = File::open(source).map_err(|e| e.to_string())?;
    let mut source_zip = ZipArchive::new(source_file).map_err(|e| e.to_string())?;
    let target_names: BTreeSet<String> = existing_targets.keys().cloned().collect();
    let output_file = File::create(&temporary).map_err(|e| e.to_string())?;
    let mut output_zip = ZipWriter::new(output_file);
    let mut written_names = BTreeSet::new();
    let mut counted_namespaces = BTreeSet::new();
    let mut stats = RewriteStats {
        changed: true,
        ..Default::default()
    };

    for index in 0..source_zip.len() {
        cancel::check()?;
        let mut entry = source_zip
            .by_index(index)
            .map_err(|e| format!("讀取 JAR 內容失敗：{e}"))?;
        let name = entry.name().replace('\\', "/");
        if target_names.contains(&name) {
            let (namespace, _, extension) = parse_language_path(&name)
                .ok_or_else(|| format!("無法判斷語言檔命名空間：{name}"))?;
            let map = values
                .get(&namespace)
                .ok_or_else(|| format!("找不到命名空間翻譯：{namespace}"))?;
            if entry.size() > MAX_ZIP_ENTRY_BYTES {
                return Err(format!("語言檔超過大小上限：{name}"));
            }
            let mut original = Vec::new();
            entry.read_to_end(&mut original).map_err(|e| e.to_string())?;
            let content = render_language_file(map, &extension, Some(&original))?;
            let options = entry.options();
            output_zip
                .start_file(&name, options)
                .map_err(|e| format!("寫入 JAR 項目失敗：{e}"))?;
            output_zip.write_all(&content).map_err(|e| e.to_string())?;
            written_names.insert(name);
            stats.files_written += 1;
            if counted_namespaces.insert(namespace.clone()) {
                stats.keys_written += map.len();
                if let Some(translated_map) = translated.get(&namespace) {
                    stats.fallback_keys_kept += map
                        .keys()
                        .filter(|key| !translated_map.contains_key(*key))
                        .count();
                }
            }
        } else {
            output_zip
                .raw_copy_file(entry)
                .map_err(|e| format!("複製 JAR 項目失敗：{e}"))?;
        }
    }

    for (namespace, targets) in &targets_by_namespace {
        let map = values
            .get(namespace)
            .ok_or_else(|| format!("找不到命名空間翻譯：{namespace}"))?;
        for (name, extension) in targets {
            if written_names.contains(name) {
                continue;
            }
            let options = SimpleFileOptions::default();
            output_zip
                .start_file(name, options)
                .map_err(|e| format!("新增語言檔失敗：{e}"))?;
            let content = render_language_file(map, extension, None)?;
            output_zip.write_all(&content).map_err(|e| e.to_string())?;
            stats.files_written += 1;
            if counted_namespaces.insert(namespace.clone()) {
                stats.keys_written += map.len();
                if let Some(translated_map) = translated.get(namespace) {
                    stats.fallback_keys_kept += map
                        .keys()
                        .filter(|key| !translated_map.contains_key(*key))
                        .count();
                }
            }
        }
    }

    output_zip
        .finish()
        .map_err(|e| format!("完成 JAR 重打包失敗：{e}"))?;
    if output.exists() {
        fs::remove_file(output).map_err(|e| e.to_string())?;
    }
    fs::rename(&temporary, output).map_err(|e| format!("儲存 JAR 翻譯副本失敗：{e}"))?;
    Ok(stats)
}

fn parse_language_path(name: &str) -> Option<(String, String, String)> {
    let parts: Vec<&str> = name.split('/').collect();
    let li = parts.iter().position(|part| part.eq_ignore_ascii_case("lang"))?;
    if li == 0 || li + 2 != parts.len() {
        return None;
    }
    let (locale, extension) = parts[li + 1].rsplit_once('.')?;
    if !extension.eq_ignore_ascii_case("json") && !extension.eq_ignore_ascii_case("lang") {
        return None;
    }
    Some((
        parts[li - 1].to_string(),
        locale.to_ascii_lowercase(),
        extension.to_ascii_lowercase(),
    ))
}

fn parse_zh_tw_path(name: &str) -> Option<(String, String)> {
    let (namespace, locale, extension) = parse_language_path(name)?;
    if locale != "zh_tw" {
        return None;
    }
    Some((namespace, extension))
}

fn render_language_file(
    map: &BTreeMap<String, String>,
    extension: &str,
    original: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if extension.eq_ignore_ascii_case("lang") {
        if let Some(original) = original {
            return Ok(render_lang_preserving_lines(map, original));
        }
        let mut out = String::new();
        for (key, value) in map {
            out.push_str(key);
            out.push('=');
            out.push_str(&escape_lang_value(value));
            out.push('\n');
        }
        return Ok(out.into_bytes());
    }
    let mut object = original
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    for (key, value) in map {
        object.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    let mut output = serde_json::to_vec_pretty(&serde_json::Value::Object(object))
        .map_err(|e| format!("建立 JSON 語言檔失敗：{e}"))?;
    output.push(b'\n');
    Ok(output)
}

fn render_lang_preserving_lines(map: &BTreeMap<String, String>, original: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(original);
    let mut seen = BTreeSet::new();
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let separator = line.find(|c| c == '=' || c == ':');
        let Some(separator) = separator else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        let key = line[..separator].trim();
        if let Some(value) = map.get(key) {
            out.push_str(key);
            out.push('=');
            out.push_str(&escape_lang_value(value));
            out.push('\n');
            seen.insert(key.to_string());
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    for (key, value) in map {
        if !seen.contains(key) {
            out.push_str(key);
            out.push('=');
            out.push_str(&escape_lang_value(value));
            out.push('\n');
        }
    }
    out.into_bytes()
}

fn is_signature_path(name: &str) -> bool {
    let normalized = name.replace('\\', "/");
    let upper = normalized.to_ascii_uppercase();
    if !upper.starts_with("META-INF/") {
        return false;
    }
    matches!(
        Path::new(&upper)
            .extension()
            .and_then(|value| value.to_str()),
        Some("SF") | Some("RSA") | Some("DSA") | Some("EC")
    )
}

fn escape_lang_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\r', "")
        .replace('\n', "\\n")
}

fn remove_empty_parent(path: &Path, root: &Path) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent == root {
        return Ok(());
    }
    if parent.exists()
        && fs::read_dir(parent)
            .map_err(|e| e.to_string())?
            .next()
            .is_none()
    {
        fs::remove_dir(parent).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use zip::write::SimpleFileOptions;

    fn make_jar(path: &Path) {
        let file = File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("assets/example/lang/en_us.json", options).unwrap();
        zip.write_all(br#"{"item.example":"Hello"}"#).unwrap();
        zip.start_file("META-INF/mods.toml", options).unwrap();
        zip.write_all(b"modLoader=javafml").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn rewrites_copy_and_preserves_non_language_entries() {
        let root = std::env::temp_dir().join(format!("jar_translate_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("minecraft/mods")).unwrap();
        make_jar(&root.join("minecraft/mods/example.jar"));
        let original = fs::read(root.join("minecraft/mods/example.jar")).unwrap();
        let mut translated = LangMap::new();
        translated
            .entry("example".into())
            .or_default()
            .insert("item.example".into(), "範例".into());
        let report = rewrite_translated_jars(
            &root.join("minecraft"),
            &translated,
            &LangMap::new(),
            &root.join("work"),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(report.jars_rewritten, 1);
        assert_eq!(fs::read(root.join("minecraft/mods/example.jar")).unwrap(), original);
        let output = root.join("work/jar-translated/example.jar");
        let mut archive = ZipArchive::new(File::open(output).unwrap()).unwrap();
        let mut lang = String::new();
        archive
            .by_name("assets/example/lang/zh_tw.json")
            .unwrap()
            .read_to_string(&mut lang)
            .unwrap();
        assert!(lang.contains("範例"));
        assert!(archive.by_name("META-INF/mods.toml").is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn creates_legacy_lang_copy_using_the_jar_template_path() {
        let root = std::env::temp_dir().join(format!("jar_translate_legacy_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("minecraft/mods")).unwrap();
        let file = File::create(root.join("minecraft/mods/example.jar")).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("data/example/lang/en_us.lang", options).unwrap();
        zip.write_all(b"item.example=Hello\n").unwrap();
        zip.finish().unwrap();

        let mut translated = LangMap::new();
        translated
            .entry("example".into())
            .or_default()
            .insert("item.example".into(), "範例".into());
        rewrite_translated_jars(
            &root.join("minecraft"),
            &translated,
            &LangMap::new(),
            &root.join("work"),
            |_, _, _| {},
        )
        .unwrap();

        let output = root.join("work/jar-translated/example.jar");
        let mut archive = ZipArchive::new(File::open(output).unwrap()).unwrap();
        let mut lang = String::new();
        archive
            .by_name("data/example/lang/zh_tw.lang")
            .unwrap()
            .read_to_string(&mut lang)
            .unwrap();
        assert!(lang.contains("item.example=範例"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn writes_legacy_lang_files() {
        let mut map = BTreeMap::new();
        map.insert("key".to_string(), "line\nnext".to_string());
        let bytes = render_language_file(&map, "lang", None).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "key=line\\nnext\n");
    }

    #[test]
    fn recognizes_only_zh_tw_language_paths() {
        assert_eq!(parse_zh_tw_path("assets/a/lang/zh_tw.json"), Some(("a".into(), "json".into())));
        assert!(parse_zh_tw_path("assets/a/lang/en_us.json").is_none());
        assert!(parse_zh_tw_path("assets/a/lang/zh_tw.json/extra").is_none());
        assert_eq!(
            parse_language_path("data/example/lang/en_us.lang"),
            Some(("example".into(), "en_us".into(), "lang".into()))
        );
    }
}
