//! B2#5：列舉所有「寫出譯文」的函式，確認每一個都接上 output guard 的**內容檢查**。漏接就失敗。
//!
//! 兩層：
//! 1. 登記表 [`WRITERS`]：每個寫譯文的函式必須：
//!    - 本體直接呼叫內容檢查函式（[`CONTENT_CHECKS`]；只有 `zip_entry_ok` 不算）；或
//!    - 呼叫登記表裡另一個已做內容檢查的函式（`Via`）；或
//!    - 內容由上游把關（`Upstream`）：寫入的內容全部來自指定上游，測試驗證上游有做內容檢查，
//!      而且所有呼叫這個函式的地方都先呼叫了上游。
//! 2. 自動偵測：engine/ 底下任何檔案只要「會產生譯文」（呼叫翻譯／重建函式或寫 zh_tw）
//!    又「會寫檔」，就必須出現在登記表或排除表裡——新增寫檔點忘了登記也會被抓到。

use std::path::PathBuf;

/// 算得上「內容檢查」的 output_guard 呼叫。
const CONTENT_CHECKS: &[&str] = &[
    "output_guard::check(",
    "output_guard::check_entry(",
    "output_guard::guard_map(",
    "output_guard::guard_langmap(",
    "output_guard::lang_entry(",
    "output_guard::finish_file(",
];

#[derive(Clone, Copy)]
enum Guard {
    /// 本體直接做內容檢查
    Direct,
    /// 呼叫登記表中已做內容檢查的函式
    Via(&'static str),
    /// 上游把關：寫入的內容全部由這些上游產生（上游必須是 Direct）
    Upstream(&'static [&'static str]),
}

use Guard::*;

const WRITERS: &[(&str, &str, Guard)] = &[
    ("pack_out.rs", "build_resource_pack_skipping_bundled", Direct),
    ("ftbquests.rs", "translate_ftbquests", Direct),
    ("text_overlay.rs", "translate_text_overlays", Direct),
    ("script_literals.rs", "translate_kubejs_literals", Direct),
    ("jar_translate.rs", "rewrite_translated_jars", Direct),
    ("jar_translate.rs", "rewrite_one_jar", Direct),
    // 上游把關：translate_text_overlays（本體內呼叫，產生 zip 內要換的檔）
    ("archive_overlay.rs", "process_archive", Upstream(&["translate_text_overlays"])),
    // 上游把關：translate_text_overlays／translate_origins（各呼叫端先翻完再重建）
    ("jar_display.rs", "rebuild_jar", Upstream(&["translate_text_overlays", "translate_origins"])),
    ("jar_patchouli.rs", "translate_jar_patchouli", Via("rebuild_jar")),
    ("jar_display.rs", "translate_jar_display_texts", Via("rebuild_jar")),
    ("jar_origins.rs", "translate_jar_origins", Via("rebuild_jar")),
    ("origins.rs", "translate_origins", Direct),
    ("minemenu.rs", "translate_minemenu", Direct),
    ("minemenu.rs", "write_minemenu_outputs", Direct),
    ("quests_books.rs", "translate_quests_books", Direct),
    ("merge_ref.rs", "merge_fill_missing", Direct),
    // lib.rs::import_translations_cmd（貼回翻譯）的寫入路徑：merge_imported → build_resource_pack
    ("failed_items.rs", "merge_imported", Direct),
];

/// 會寫 zh_tw 字樣但不是寫譯文的檔（附原因）。
const NOT_TRANSLATION_WRITERS: &[(&str, &str)] = &[
    ("glossary.rs", "只寫使用者術語表範本"),
    ("options_txt.rs", "只改 options.txt 的語言設定"),
    ("session.rs", "只寫工作階段紀錄，譯文由資源包寫出"),
    ("jar_docs.rs", "只把模組 JAR 裡的英文文件原樣抽出供複查，不含譯文；zip 路徑由 zip_entry_ok 把關"),
];

fn engine_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("engine")
}

fn read(file: &str) -> String {
    std::fs::read_to_string(engine_dir().join(file)).unwrap_or_else(|e| panic!("讀不到 {file}：{e}"))
}

/// 去掉 `#[cfg(test)]` 之後的內容（測試模組不算寫檔點）。
fn non_test(src: &str) -> &str {
    src.find("#[cfg(test)]").map_or(src, |i| &src[..i])
}

/// 函式本體：從 `fn 名稱` 到下一個頂層函式（行首的 fn／pub fn／pub(crate) fn）。
fn body_of<'a>(src: &'a str, name: &str) -> &'a str {
    let starts = [
        format!("\nfn {name}"),
        format!("\npub fn {name}"),
        format!("\npub(crate) fn {name}"),
        format!("\nasync fn {name}"),
    ];
    let start = starts
        .iter()
        .filter_map(|s| src.find(s.as_str()).filter(|i| {
            let after = &src[i + s.len()..];
            after.starts_with('(') || after.starts_with('<')
        }))
        .min()
        .unwrap_or_else(|| panic!("找不到函式 {name}"));
    let rest = &src[start + 1..];
    let end = ["\nfn ", "\npub fn ", "\npub(crate) fn ", "\n#[cfg(test)]"]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}

fn is_guarded(file: &str, name: &str) -> bool {
    let src = read(file);
    let body = body_of(non_test(&src), name);
    CONTENT_CHECKS.iter().any(|c| body.contains(c))
}

fn file_of(name: &str) -> &'static str {
    WRITERS
        .iter()
        .find(|(_, n, _)| *n == name)
        .map(|(f, _, _)| *f)
        .unwrap_or_else(|| panic!("{name} 不在登記表"))
}

/// 引擎裡所有呼叫 `name(` 的函式本體（排除測試區與定義本身）。
fn callers_of(name: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(engine_dir()).unwrap().flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if !file.ends_with(".rs") || file.ends_with("_tests.rs") {
            continue;
        }
        let src = read(&file);
        let code = non_test(&src);
        for chunk in code.split("\nfn ").chain(code.split("\npub fn ")).chain(code.split("\npub(crate) fn ")) {
            let fn_name: String = chunk.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if fn_name.is_empty() || fn_name == name {
                continue;
            }
            let body_end = ["\nfn ", "\npub fn ", "\npub(crate) fn "]
                .iter()
                .filter_map(|m| chunk.find(m))
                .min()
                .unwrap_or(chunk.len());
            // 註解裡提到函式名不算呼叫
            let body: String = chunk[..body_end]
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("
");
            if body.contains(&format!("{name}(")) {
                out.push((format!("{file}::{fn_name}"), body));
            }
        }
    }
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn guard_ok(file: &str, name: &str, guard: Guard) -> Result<(), String> {
    match guard {
        Direct => is_guarded(file, name)
            .then_some(())
            .ok_or_else(|| format!("{file}::{name} 沒有呼叫內容檢查（只有 zip_entry_ok 不算）")),
        Via(via) => {
            let body_calls = body_of(non_test(&read(file)), name).contains(&format!("{via}("));
            let via_guard = WRITERS.iter().find(|(_, n, _)| *n == via).map(|(_, _, g)| *g).unwrap();
            if !body_calls {
                return Err(format!("{file}::{name} 沒有呼叫 {via}"));
            }
            guard_ok(file_of(via), via, via_guard)
        }
        Upstream(ups) => {
            for up in ups {
                if !is_guarded(file_of(up), up) {
                    return Err(format!("上游 {up} 沒有做內容檢查"));
                }
            }
            let own_src = read(file);
            let own = body_of(non_test(&own_src), name);
            if ups.iter().any(|up| own.contains(&format!("{up}("))) {
                return Ok(());
            }
            let callers = callers_of(name);
            if callers.is_empty() {
                return Err(format!("找不到 {name} 的呼叫端"));
            }
            for (caller, body) in callers {
                if !ups.iter().any(|up| body.contains(&format!("{up}("))) {
                    return Err(format!("{caller} 呼叫 {name} 前沒有經過上游把關 {ups:?}"));
                }
            }
            Ok(())
        }
    }
}

#[test]
fn b2_every_translation_writer_calls_output_guard() {
    let missing: Vec<String> = WRITERS
        .iter()
        .filter_map(|(file, name, guard)| guard_ok(file, name, *guard).err())
        .collect();
    assert!(missing.is_empty(), "這些寫譯文的函式沒有接 output guard 的內容檢查：{missing:?}");
}

#[test]
fn b2_import_command_goes_through_guarded_merge() {
    let lib = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs")).unwrap();
    let body = body_of(&lib, "import_translations_cmd");
    assert!(body.contains("merge_imported("), "貼回翻譯必須走 merge_imported（已接 guard）");
    assert!(body.contains("build_resource_pack("), "貼回翻譯重建資源包時也會過 guard");
    assert!(is_guarded("failed_items.rs", "merge_imported"));
}

#[test]
fn b2_no_unregistered_translation_writer() {
    const PRODUCERS: &[&str] = &[
        "translate_plain_strings",
        "fill_missing_with_ai",
        "translate_text_overlays(",
        "translate_origins(",
        "rebuild_jar(",
        "render_language_file(",
        "zh_tw.json",
        "\"zh_tw\"",
    ];
    let mut unregistered = Vec::new();
    for entry in std::fs::read_dir(engine_dir()).unwrap().flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if !file.ends_with(".rs") || file.ends_with("_tests.rs") || file.starts_with("output_guard") {
            continue;
        }
        let src = read(&file);
        let code = non_test(&src);
        let produces = PRODUCERS.iter().any(|p| code.contains(p));
        let writes = code.contains("fs::write(") || code.contains("write_all(");
        if !(produces && writes) {
            continue;
        }
        let known = WRITERS.iter().any(|(f, _, _)| *f == file)
            || NOT_TRANSLATION_WRITERS.iter().any(|(f, _)| *f == file);
        if !known {
            unregistered.push(file);
        }
    }
    assert!(
        unregistered.is_empty(),
        "這些檔會產生並寫出譯文，卻不在 output guard 登記表：{unregistered:?}（請接上 guard 並登記）"
    );
}
