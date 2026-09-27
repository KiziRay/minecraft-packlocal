//! B3#2：模組 JAR 內嵌的子 JAR（`META-INF/jars/*.jar`，Fabric／NeoForge Jar-in-Jar）。
//!
//! Create 內嵌 Ponder／Flywheel、許多模組內嵌函式庫，它們的語言檔在子 JAR 裡；
//! 以前只讀外層 JAR，這些字（例如思索教學的介面字）整片英文。
//!
//! 只遞迴一層、只收語言檔（`…/lang/<locale>.json|.lang`）。子 JAR 屬於模組本身，
//! 所以它的 zh_tw 也算「模組自帶」（在 jar_scan 合併資源包之前收進 raw，G2.23 時點不變）。

use std::collections::HashMap;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use super::jar_scan::{harvest_zip, RawLang};

/// 子 JAR 讀進記憶體的上限（多數函式庫 < 5 MB；超過就略過並記錄）。
const MAX_NESTED_JAR_BYTES: u64 = 64 * 1024 * 1024;

/// `lower`＝小寫、正斜線的 zip 項目名。
pub(crate) fn is_nested_jar(lower: &str) -> bool {
    lower.starts_with("meta-inf/jars/") && lower.ends_with(".jar")
}

/// 讀一個子 JAR 的語言檔進 `raw`（不再往下遞迴）。失敗只記錄，不影響外層 JAR。
pub(crate) fn harvest(
    entry: &mut zip::read::ZipFile<'_>,
    outer_label: &str,
    name: &str,
    raw: &mut RawLang,
    errors: &mut Vec<String>,
    non_priority_skips: &mut usize,
    mod_identity: &mut HashMap<String, String>,
) {
    let label = format!("{outer_label}!{name}");
    if entry.size() > MAX_NESTED_JAR_BYTES {
        errors.push(format!("{label}：內嵌模組太大，已略過"));
        return;
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    if entry.take(MAX_NESTED_JAR_BYTES + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_NESTED_JAR_BYTES {
        errors.push(format!("{label}：讀不到內嵌模組，已略過"));
        return;
    }
    let mut zip = match ZipArchive::new(Cursor::new(bytes)) {
        Ok(zip) => zip,
        Err(e) => {
            errors.push(format!("{label}：{e}"));
            return;
        }
    };
    let identity = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .trim_end_matches(".jar")
        .trim_end_matches(".JAR")
        .trim()
        .to_ascii_lowercase();
    if let Err(e) = harvest_zip(&mut zip, &label, &identity, raw, errors, non_priority_skips, mod_identity, false) {
        errors.push(format!("{label}：{e}"));
    }
}

#[cfg(test)]
#[path = "jar_nested_tests.rs"]
mod tests;
