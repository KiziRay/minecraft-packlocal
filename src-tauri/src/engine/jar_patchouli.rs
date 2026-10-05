//! JAR 內 Patchouli 書本的非破壞式翻譯。
//!
//! Patchouli 書頁可能位於 `data/<namespace>/patchouli_books` 或
//! `assets/<namespace>/patchouli_books`（兩種根目錄都是 Patchouli 支援的合法
//! 位置，後者是 Botania、Ad Astra、Croptopia 等主流模組實際採用的位置），不在
//! lang 檔。抽出後交給共用文字掃描器，再嵌回 `jar-translated` 副本；原始 JAR
//! 不會被改寫。`work/data` 僅作除錯產出，不會套用到遊戲的 `minecraft/data/`。

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::ZipArchive;

use super::cancel;
use super::jar_display::{is_signature_path, jar_key, rebuild_jar};
use super::jar_scan::resolve_minecraft_dir;
use super::security::{check_jar_size, is_safe_zip_entry_name, MAX_ZIP_ENTRY_BYTES};
use super::text_overlay::translate_text_overlays;
use super::translation_scope::TranslationScope;

#[derive(Debug, Clone, Default)]
pub struct JarPatchouliReport {
    pub jars_scanned: usize,
    pub books_found: usize,
    pub files_written: usize,
    pub strings_translated: usize,
    pub skipped: Vec<String>,
    pub note: String,
}

struct ExtractedJar {
    jar_key: String,
    source_jar: PathBuf,
    /// 實際讀的檔（原檔或原檔備份）
    read_path: PathBuf,
    entries_scanned: usize,
}

pub fn translate_jar_patchouli<F>(
    instance_or_mc: &Path,
    work_root: &Path,
    use_ai: bool,
    scope: Option<&TranslationScope>,
    mut on_progress: F,
) -> Result<JarPatchouliReport, String>
where
    F: FnMut(u8, &str),
{
    let mc = resolve_minecraft_dir(instance_or_mc)?;
    let mods = mc.join("mods");
    // B6a-2：本輪產出清單（assets 書本放進主資源包，記來源 JAR 的指紋）。沒有模組或沒有書也算「完整跑完」
    super::text_sources::begin(work_root, "jar_patchouli");
    let jars = list_jars(&mods);
    let mut report = JarPatchouliReport {
        jars_scanned: jars.len(),
        ..Default::default()
    };
    if jars.is_empty() {
        report.note = "JAR 內 Patchouli：沒有找到模組 JAR".into();
        super::text_sources::commit(work_root, "jar_patchouli", &mc);
        return Ok(report);
    }

    let tool_index = super::tool_products::ToolIndex::for_game(&mc);
    let stage_root = work_root.join(".jar-patchouli-stage");
    let translated_root = work_root.join(".jar-patchouli-translated");
    let _ = fs::remove_dir_all(&stage_root);
    let _ = fs::remove_dir_all(&translated_root);
    fs::create_dir_all(&stage_root).map_err(|e| e.to_string())?;
    let _stage_cleanup = TempDirGuard(vec![stage_root.clone(), translated_root.clone()]);

    let mut extracted: Vec<ExtractedJar> = Vec::new();
    for (index, jar) in jars.iter().enumerate() {
        cancel::check()?;
        on_progress(
            1 + ((index * 40) / jars.len().max(1)) as u8,
            &format!("JAR 書本：模組 {}/{}", index + 1, jars.len()),
        );
        // B3#7：工具翻過的 JAR 改讀原檔備份；沒有原檔就不收它的 zh_tw 書頁（可能是機翻）
        let read = tool_index.jar_read(jar);
        match extract_patchouli(&read.path, &stage_root, read.keep_zh_tw_only.is_some()) {
            Ok(Some(mut item)) => {
                item.source_jar = jar.clone();
                item.read_path = read.path.clone();
                report.books_found += item.entries_scanned;
                extracted.push(item);
            }
            Ok(None) => {}
            Err(error) => report.skipped.push(format!("{}：{error}", jar.display())),
        }
    }
    if extracted.is_empty() {
        report.note = "JAR 內 Patchouli：沒有找到 data/*/patchouli_books 文字頁面".into();
        super::text_sources::commit(work_root, "jar_patchouli", &mc);
        return Ok(report);
    }

    let overlay = translate_text_overlays(&stage_root, &translated_root, use_ai, scope, |pct, msg| {
        on_progress(42 + pct.saturating_mul(40) / 100, msg);
    })?;
    report.strings_translated = overlay.strings_translated;
    // B3#5：assets/ 底下的書本 zh_tw 放進主資源包（不改寫模組 JAR）；data/ 書本仍需重建 JAR
    report.files_written += super::pack_assets::move_into(&translated_root, work_root)?;
    {
        let providers = |rel: &Path| -> Vec<(PathBuf, PathBuf)> {
            // pack-assets/assets/<ns>/…：提供這個命名空間書本的 JAR 全部算來源（任一個換了都算變了）
            let ns = rel.components().nth(1).map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
            extracted
                .iter()
                .filter(|item| stage_root.join("resourcepacks").join(&item.jar_key).join("assets").join(&ns).is_dir())
                .map(|item| (item.source_jar.clone(), item.read_path.clone()))
                .collect()
        };
        super::pack_books::record_pack_assets(work_root, &translated_root, &mc, "jar_patchouli", &providers);
        super::text_sources::commit(work_root, "jar_patchouli", &mc);
    }

    for (index, item) in extracted.iter().enumerate() {
        cancel::check()?;
        let jar_translated = translated_root
            .join("resourcepacks")
            .join(&item.jar_key);
        if !jar_translated.is_dir() || dir_is_empty(&jar_translated) {
            continue;
        }
        let relative = item
            .source_jar
            .strip_prefix(&mods)
            .map_err(|e| format!("JAR 不在 mods 根目錄：{e}"))?;
        let output = work_root.join("jar-translated").join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let base = if output.is_file() {
            output.as_path()
        } else {
            item.source_jar.as_path()
        };
        let stage = stage_root.join("resourcepacks").join(&item.jar_key);
        match rebuild_jar(base, &output, &jar_translated, &stage) {
            Ok(()) => {
                report.files_written += count_files(&jar_translated);
                if let Err(e) = super::jar_sources::record_source(work_root, relative, &item.source_jar) {
                    report.skipped.push(format!("{}：{e}", item.source_jar.display()));
                }
            }
            Err(error) => report
                .skipped
                .push(format!("{}：{error}", item.source_jar.display())),
        }
        on_progress(
            84 + ((index * 12) / extracted.len().max(1)) as u8,
            &format!(
                "JAR 書本：重建模組 {}/{}",
                index + 1,
                extracted.len()
            ),
        );
    }

    let debug_data = work_root.join("data");
    let _ = copy_translated_data_for_debug(&translated_root, &debug_data);

    report.note = format!(
        "JAR 內 Patchouli：掃描 {} 個 JAR、找到 {} 個書本檔，翻譯 {} 條，寫入 jar-translated 副本（不改原 jar）{}",
        report.jars_scanned,
        report.books_found,
        report.strings_translated,
        if report.skipped.is_empty() {
            String::new()
        } else {
            format!("；{} 個 JAR 略過", report.skipped.len())
        }
    );
    on_progress(100, "JAR 內 Patchouli 完成");
    Ok(report)
}

struct TempDirGuard(Vec<PathBuf>);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
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

fn extract_patchouli(source_jar: &Path, stage_root: &Path, drop_zh_tw: bool) -> Result<Option<ExtractedJar>, String> {
    check_jar_size(source_jar)?;
    let file = File::open(source_jar).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("JAR 不是有效 ZIP：{e}"))?;
    let mut has_signature = false;
    let mut entries_scanned = 0usize;
    let key = jar_key(source_jar);
    let stage = stage_root.join("resourcepacks").join(&key);
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

    for index in 0..archive.len() {
        cancel::check()?;
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if !is_safe_zip_entry_name(&name) {
            return Err(format!("包含不安全的 ZIP 路徑：{name}"));
        }
        if is_signature_path(&name) {
            has_signature = true;
        }
        if !is_patchouli_book_entry(&name) || entry.size() > MAX_ZIP_ENTRY_BYTES {
            continue;
        }
        if drop_zh_tw && name.to_ascii_lowercase().split('/').any(|seg| seg == "zh_tw") {
            continue;
        }
        let target = stage.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.contains(&0) {
            continue;
        }
        fs::write(target, bytes).map_err(|e| e.to_string())?;
        entries_scanned += 1;
    }
    drop(archive);
    if has_signature {
        let _ = fs::remove_dir_all(&stage);
        return Err("JAR 含有簽章，不能重建後維持有效簽章；已保留原檔".into());
    }
    if entries_scanned == 0 {
        let _ = fs::remove_dir_all(&stage);
        return Ok(None);
    }
    Ok(Some(ExtractedJar {
        jar_key: key,
        source_jar: source_jar.to_path_buf(),
        read_path: source_jar.to_path_buf(),
        entries_scanned,
    }))
}

fn is_patchouli_book_entry(name: &str) -> bool {
    let lower = name.replace('\\', "/").to_ascii_lowercase();
    // Patchouli 書本兩種根目錄都合法：較新的慣例放 data/<ns>/patchouli_books/，
    // 但很多主流模組（Botania、Ad Astra、Croptopia、Archon…）仍放
    // assets/<ns>/patchouli_books/。過去這裡只認 data/，實測對這批模組
    // （單一整合包內就有 13 個、合計 2400+ 條書頁文字）全數略過不翻，
    // 是「掃描/整合不夠全面」回報的主要根因。rebuild_jar() 只按相對路徑比對，
    // 不假設根目錄，所以這裡放寬不需要動其他地方。
    ((lower.starts_with("data/") || lower.starts_with("assets/"))
        && lower.contains("/patchouli_books/")
        && (lower.ends_with(".json") || lower.ends_with(".txt")))
        // B3#4：GuideME（AE2 指南）／Lavender 手冊頁，翻譯後放進主資源包
        || super::markdown_text::is_guide_md_entry(&lower)
}

fn copy_translated_data_for_debug(translated_root: &Path, destination: &Path) -> Result<usize, String> {
    let packs = translated_root.join("resourcepacks");
    if !packs.is_dir() {
        return Ok(0);
    }
    let mut written = 0usize;
    for jar_dir in fs::read_dir(&packs).map_err(|e| e.to_string())? {
        let jar_dir = jar_dir.map_err(|e| e.to_string())?.path();
        let data = jar_dir.join("data");
        if !data.is_dir() {
            continue;
        }
        written += copy_tree(&data, destination)?;
    }
    Ok(written)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<usize, String> {
    let mut written = 0usize;
    for entry in WalkDir::new(source).into_iter().filter_map(|e| e.ok()) {
        if !entry.path().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(source).map_err(|e| e.to_string())?;
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        written += 1;
    }
    Ok(written)
}

fn dir_is_empty(path: &Path) -> bool {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .all(|e| !e.file_type().is_file())
}

fn count_files(path: &Path) -> usize {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn module_has_a_narrow_patchouli_scope() {
        assert!(is_patchouli_book_entry(
            "data/example/patchouli_books/book/en_us/entries/a.json"
        ));
        assert!(is_patchouli_book_entry(
            "data/example/patchouli_books/book/en_us/root.txt"
        ));
        assert!(!is_patchouli_book_entry("data/example/recipes/a.json"));
        assert!(!is_patchouli_book_entry(
            "assets/example/book/animal_dictionary/en_us/root.txt"
        ));
    }

    #[test]
    fn accepts_assets_rooted_patchouli_books_too() {
        // 釘死這輪修的迴歸：Botania、Ad Astra、Croptopia、Archon 等主流模組把
        // 書頁放在 assets/<ns>/patchouli_books/ 而不是 data/<ns>/patchouli_books/，
        // 過去這裡只認 data/，導致這些模組的書頁全數被跳過、完全沒進翻譯流程。
        assert!(is_patchouli_book_entry(
            "assets/ad_astra/patchouli_books/astrodux/en_us/categories/the_moon.json"
        ));
        assert!(is_patchouli_book_entry(
            "assets/botania/patchouli_books/lexicon/en_us/root.txt"
        ));
        // 但同樣在 assets/ 底下、不是 patchouli_books 資料夾的檔案仍要維持排除。
        assert!(!is_patchouli_book_entry(
            "assets/example/book/animal_dictionary/en_us/root.txt"
        ));
    }

    #[test]
    fn embeds_translated_patchouli_into_jar_translated_not_minecraft_data() {
        let root = std::env::temp_dir().join(format!("jar_patchouli_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("work");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(&work).unwrap();

        let jar = mc.join("mods/guidebook.jar");
        {
            let file = File::create(&jar).unwrap();
            let mut writer = ZipWriter::new(file);
            writer
                .start_file(
                    "data/example/patchouli_books/guide/book.json",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(
                    r#"{
  "name": "测试书",
  "landing_text": "这是首页说明"
}"#
                    .as_bytes(),
                )
                .unwrap();
            writer
                .start_file(
                    "data/example/patchouli_books/guide/en_us/categories/village.json",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(r#"{"name":"村庄模块"}"#.as_bytes())
                .unwrap();
            writer.finish().unwrap();
        }

        let report = translate_jar_patchouli(&mc, &work, false, None, |_, _| {}).unwrap();
        assert!(report.books_found >= 2, "{report:?}");
        assert!(report.strings_translated >= 1, "{}", report.note);

        let out_jar = work.join("jar-translated/guidebook.jar");
        assert!(out_jar.is_file(), "{}", report.note);
        let file = File::open(&out_jar).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().replace('\\', "/"))
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("zh_tw/categories/village.json")),
            "{names:?}"
        );

        let mut book = zip
            .by_name("data/example/patchouli_books/guide/book.json")
            .unwrap();
        let mut bytes = Vec::new();
        book.read_to_end(&mut bytes).unwrap();
        drop(book);
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("這是首頁說明") || text.contains("测试书") || text.contains("測試"),
            "{text}"
        );

        assert!(
            !mc.join("data").exists(),
            "不得把 Patchouli 寫進遊戲 minecraft/data"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn embeds_translated_patchouli_from_assets_root_too() {
        // 對應 Botania／Ad Astra 這批把書頁放在 assets/ 而不是 data/ 底下的模組；
        // 端對端跑一次抽取→翻譯→重建，確認不只是路徑判斷通過，實際會寫回 jar。
        let root = std::env::temp_dir().join(format!("jar_patchouli_assets_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("work");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(&work).unwrap();

        let jar = mc.join("mods/assets_guidebook.jar");
        {
            let file = File::create(&jar).unwrap();
            let mut writer = ZipWriter::new(file);
            writer
                .start_file(
                    "assets/example/patchouli_books/guide/en_us/categories/village.json",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(r#"{"name":"村庄模块"}"#.as_bytes())
                .unwrap();
            writer.finish().unwrap();
        }

        let report = translate_jar_patchouli(&mc, &work, false, None, |_, _| {}).unwrap();
        assert!(report.books_found >= 1, "{report:?}");
        assert!(report.strings_translated >= 1, "{}", report.note);

        // B3#5：assets/ 書本改放進主資源包（pack-assets），不再重建模組 JAR
        let zh = work.join("pack-assets/assets/example/patchouli_books/guide/zh_tw/categories/village.json");
        let text = fs::read_to_string(&zh)
            .unwrap_or_else(|_| panic!("assets/ 根目錄的書頁要翻譯並放進主資源包：{}", report.note));
        assert!(text.contains("村莊"), "{text}");
        assert!(!work.join("jar-translated/assets_guidebook.jar").exists(), "只有 assets 書本時不改寫 JAR");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn b3_jar_book_with_human_zh_tw_is_not_overwritten() {
        let root = std::env::temp_dir().join(format!("jar_patchouli_human_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("work");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(&work).unwrap();
        {
            let file = File::create(mc.join("mods/human.jar")).unwrap();
            let mut writer = ZipWriter::new(file);
            for (name, body) in [
                ("assets/h/patchouli_books/g/zh_cn/entries/e.json", r#"{"name":"简体说明"}"#),
                ("assets/h/patchouli_books/g/zh_tw/entries/e.json", r#"{"name":"人工繁中"}"#),
                ("assets/h/patchouli_books/g/zh_cn/entries/f.json", r#"{"name":"简体另一页"}"#),
            ] {
                writer.start_file(name, SimpleFileOptions::default()).unwrap();
                writer.write_all(body.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        translate_jar_patchouli(&mc, &work, false, None, |_, _| {}).unwrap();
        let base = work.join("pack-assets/assets/h/patchouli_books/g/zh_tw/entries");
        assert!(!base.join("e.json").exists(), "已有人工 zh_tw 的書頁不覆蓋");
        let f = fs::read_to_string(base.join("f.json")).expect("缺的頁用簡中轉繁補");
        assert!(f.contains("簡體另一頁"), "{f}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn jar_books_in_other_locales_still_land_in_zh_tw() {
        // JAR 內 Patchouli 走的是同一支 translate_text_overlays，所以站長那個
        // 「書翻好了卻寫進 uk_ua」的問題在 JAR 這條路上也存在。這個測試確認
        // 兩條路都修好了——不是只有鬆散書本。
        let root = std::env::temp_dir().join(format!("jar_patchouli_locale_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mc = root.join("minecraft");
        let work = root.join("work");
        fs::create_dir_all(mc.join("mods")).unwrap();
        fs::create_dir_all(&work).unwrap();

        let jar = mc.join("mods/croptopia.jar");
        {
            let file = File::create(&jar).unwrap();
            let mut writer = ZipWriter::new(file);
            writer
                .start_file(
                    "data/croptopia/patchouli_books/guide/uk_ua/categories/crops.json",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer
                .write_all(r#"{"name":"村庄模块"}"#.as_bytes())
                .unwrap();
            writer.finish().unwrap();
        }

        let report = translate_jar_patchouli(&mc, &work, false, None, |_, _| {}).unwrap();
        let out_jar = work.join("jar-translated/croptopia.jar");
        assert!(out_jar.is_file(), "{}", report.note);
        let file = File::open(&out_jar).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().replace('\\', "/"))
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("patchouli_books/guide/zh_tw/categories/crops.json")),
            "非 en_us 的語系資料夾也要輸出到 zh_tw，否則遊戲讀不到：{names:?}"
        );
        let _ = fs::remove_dir_all(root);
    }
}
