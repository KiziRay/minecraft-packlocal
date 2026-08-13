//! 錯誤分析的文字特徵擷取。

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Default)]
pub(super) struct Findings {
    pub(super) primary_error: String,
    pub(super) evidence: Vec<String>,
    pub(super) suspected_mods: Vec<String>,
    pub(super) confidence: String,
    pub(super) game_exit_code: Option<String>,
    pub(super) log_kind: String,
}

pub(super) fn empty_findings(log_kind: &str) -> Findings {
    Findings {
        confidence: "low".into(),
        log_kind: log_kind.into(),
        ..Findings::default()
    }
}

fn re(src: &'static str) -> &'static Regex {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<&'static str, &'static Regex>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = cache.lock().expect("diagnose regex cache");
    guard
        .entry(src)
        .or_insert_with(|| Box::leak(Box::new(Regex::new(src).expect("diagnose regex compiles"))))
}

pub(super) fn normalize_log(log: &str) -> String {
    re(r"\x1b\[[0-9;]*[ -/]*[@-~]")
        .replace_all(log, "")
        .replace('\r', "")
}

pub(super) fn extract_findings(log: &str, source: &str) -> Findings {
    let primary_error = extract_primary_error(log);
    let evidence = collect_evidence(log);
    let suspected_mods = extract_suspected_mods(log);
    let confidence = if has_strong_signature(log) {
        "high"
    } else if !primary_error.is_empty() || evidence.len() >= 2 {
        "medium"
    } else {
        "low"
    };
    Findings {
        primary_error,
        evidence,
        suspected_mods,
        confidence: confidence.into(),
        game_exit_code: extract_game_exit_code(log),
        log_kind: detect_log_kind(log, source),
    }
}

fn detect_log_kind(log: &str, source: &str) -> String {
    let lower_source = source.to_ascii_lowercase();
    if lower_source.contains("crash") || log.contains("---- Minecraft Crash Report") {
        "crash_report".into()
    } else if lower_source.contains("debug.log") {
        "debug_log".into()
    } else if lower_source.contains("latest.log") || log.contains("[main/") {
        "latest_log".into()
    } else if source.contains("+") {
        "mixed".into()
    } else {
        "pasted_text".into()
    }
}

fn has_strong_signature(log: &str) -> bool {
    looks_like_dependency_failure(log)
        || classify_runtime_failure(log).is_some()
        || looks_like_mod_loading_failure(log)
        || looks_like_world_tick_failure(log)
        || looks_like_missing_registry(log)
        || strong_translation_evidence(log)
}

pub(super) fn classify_runtime_failure(log: &str) -> Option<(&'static str, String, Vec<&'static str>)> {
    let lower = log.to_ascii_lowercase();
    if lower.contains("unsupportedclassversionerror")
        || lower.contains("class file version")
        || lower.contains("has been compiled by a more recent version")
    {
        return Some((
            "JAVA_VERSION",
            "Java 版本不符合遊戲或模組需求。這是執行環境問題，不是翻譯檔造成的。\n\
記錄中出現 Java class version 不相容。"
                .into(),
            vec![
                "在啟動器設定中選擇整合包要求的 Java 版本，Minecraft 1.20.5 以上通常需要 Java 21。",
                "不要只更新系統 Java，確認 PrismLauncher／其他啟動器實際使用的 Java 路徑。",
            ],
        ));
    }
    if lower.contains("outofmemoryerror")
        || lower.contains("java heap space")
        || lower.contains("gc overhead limit exceeded")
        || lower.contains("could not reserve enough space for object heap")
    {
        return Some((
            "OUT_OF_MEMORY",
            "Java 記憶體不足，記錄沒有指向翻譯檔。這通常是整合包太大、分配記憶體不足，或其他模組造成記憶體暴增。"
                .into(),
            vec![
                "關閉其他吃記憶體的程式，再確認啟動器分配的 RAM 不低於整合包作者建議值。",
                "不要把 RAM 一次分到接近電腦總記憶體，Windows 也需要保留空間。",
            ],
        ));
    }
    if lower.contains("stackoverflowerror") || lower.contains("stack overflow") {
        return Some((
            "STACK_OVERFLOW",
            "Java 呼叫堆疊溢位。這通常是模組事件循環、Mixin 或世界內容互相觸發，不是一般語言檔錯誤。"
                .into(),
            vec![
                "查看證據中的第一個可疑模組，先更新或暫時移除它。",
                "如果只在單一世界發生，先用世界備份測試，不要直接覆蓋存檔。",
            ],
        ));
    }
    if lower.contains("exception_access_violation")
        || lower.contains("hs_err_pid")
        || lower.contains("glfw error")
        || lower.contains("failed to create window")
        || (lower.contains("opengl") && lower.contains("error"))
        || (lower.contains("lwjgl") && lower.contains("error"))
    {
        return Some((
            "GRAPHICS_RUNTIME",
            "記錄比較像 Java 虛擬機、OpenGL、LWJGL 或顯示卡驅動錯誤，沒有翻譯檔證據。"
                .into(),
            vec![
                "更新顯示卡驅動，並確認啟動器沒有使用錯誤的 Java 或顯示卡。",
                "如果使用 shader、OptiFine、Embeddium 或 ModernUI，先暫時停用後重試。",
            ],
        ));
    }
    None
}

pub(super) fn looks_like_dependency_failure(log: &str) -> bool {
    let lower = log.to_ascii_lowercase();
    lower.contains("missing or unsupported mandatory dependencies")
        || lower.contains("mod resolution failed")
        || lower.contains("incompatible mods found")
        || lower.contains("could not find required mod")
        || lower.contains("failed to find required mod")
        || lower.contains("requires a mod")
        || (lower.contains("requires") && lower.contains("which is missing"))
        || (lower.contains("depends on") && (lower.contains("missing") || lower.contains("not found")))
        || (lower.contains("dependency") && (lower.contains("missing") || lower.contains("not found")))
}

pub(super) fn extract_missing_mods(log: &str) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    let whole_lower = log.to_ascii_lowercase();
    let in_dependency_section = whole_lower.contains("missing or unsupported mandatory dependencies")
        || whole_lower.contains("missing mandatory dependencies")
        || whole_lower.contains("mod resolution failed");
    for line in log.lines() {
        let lower = line.to_ascii_lowercase();
        let relevant = (in_dependency_section && lower.contains("mod id"))
            || lower.contains("missing")
            || lower.contains("not found")
            || lower.contains("mandatory")
            || lower.contains("requires")
            || lower.contains("dependency");
        if !relevant {
            continue;
        }
        for pattern in [
            r"(?i)requires\s+(?:version\s+\S+\s+of\s+)?\{?([a-z0-9][a-z0-9_-]{1,})\}?(?:\s|,|\.|$)",
            r#"(?i)mod\s+id\s*[:=]\s*['"]([a-z0-9][a-z0-9_-]{1,})['"]"#,
            r#"(?i)dependency\s*[:=]?\s*['"]([a-z0-9][a-z0-9_-]{1,})['"]"#,
            r#"(?i)(?:required\s+mod|missing\s+mod|find\s+required\s+mod)\s*[:=]?\s*['"]?([a-z0-9][a-z0-9_-]{1,})"#,
            r#"(?i)of\s+mod\s+['"][^'"]+['"]\s*\(([a-z0-9][a-z0-9_-]{1,})\)"#,
        ] {
            for capture in re(pattern).captures_iter(line) {
                if let Some(value) = capture.get(1) {
                    let id = value.as_str().to_ascii_lowercase();
                    if is_plausible_mod_id(&id) {
                        out.insert(id);
                    }
                }
            }
        }
    }
    out.into_iter().take(20).collect()
}

fn extract_suspected_mods(log: &str) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for capture in re(r"(?im)^\s*suspected\s+mods?\s*:\s*(.+)$").captures_iter(log) {
        if let Some(value) = capture.get(1) {
            for id in re(r"[a-z0-9][a-z0-9_-]{1,}").find_iter(value.as_str()) {
                let name = id.as_str().to_ascii_lowercase();
                if is_plausible_mod_id(&name) {
                    out.insert(name);
                }
            }
        }
    }
    for pattern in [
        r#"(?i)(?:provided by|requested by|mod\s+id)\s*[:=]?\s*['"]?([a-z0-9][a-z0-9_-]{1,})"#,
        r"(?im)^\s*at\s+([a-z0-9][a-z0-9_-]{1,})@[0-9][^/\s]*/",
        r#"(?i)mod\s+['"][^'"]+['"]\s*\(([a-z0-9][a-z0-9_-]{1,})\)"#,
    ] {
        for capture in re(pattern).captures_iter(log) {
            if let Some(value) = capture.get(1) {
                let name = value.as_str().to_ascii_lowercase();
                if is_plausible_mod_id(&name) {
                    out.insert(name);
                }
            }
        }
    }
    for capture in re(r"(?im)^\s*mod\s+file:\s*(.+?\.jar)\s*$").captures_iter(log) {
        if let Some(value) = capture.get(1) {
            let path = value.as_str().trim();
            if let Some(file) = Path::new(path).file_name().and_then(|name| name.to_str()) {
                out.insert(format!("[jar] {file}"));
            }
        }
    }
    out.into_iter().take(12).collect()
}

fn is_plausible_mod_id(id: &str) -> bool {
    const NOISE: &[&str] = &[
        "minecraft", "forge", "neoforge", "fabricloader", "fabric", "java", "mod", "mods",
        "the", "version", "which", "missing", "required", "dependency", "dependencies",
        "mandatory", "unsupported", "found", "not", "of", "by",
    ];
    id.len() >= 2
        && !NOISE.contains(&id)
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}

pub(super) fn looks_like_mod_loading_failure(log: &str) -> bool {
    let lower = log.to_ascii_lowercase();
    [
        "failed to create mod instance",
        "error during mod loading",
        "mod loading error",
        "failed to load class",
        "could not execute entrypoint",
        "mixin apply failed",
        "mixintransformererror",
        "invalidinjectionexception",
        "injection failure",
        "nosuchmethoderror",
        "nosuchfielderror",
        "abstractmethoderror",
        "noclassdeffounderror",
        "classnotfoundexception",
        "exception in mod",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn looks_like_world_tick_failure(log: &str) -> bool {
    let lower = log.to_ascii_lowercase();
    lower.contains("ticking entity")
        || lower.contains("ticking block entity")
        || lower.contains("exception in server tick loop")
        || lower.contains("error ticking world")
        || lower.contains("failed to load level")
        || lower.contains("error executing task on server")
}

pub(super) fn mentions_our_output(log: &str) -> bool {
    let lower = log.to_ascii_lowercase();
    lower.contains("zh_tw.json")
        || lower.contains("zh_tw.lang")
        || lower.contains("zh_hant")
        || lower.contains("jar-translated")
        || lower.contains("翻譯結果")
        || lower.contains("繁體中文翻譯")
        || lower.contains("ftbquests")
        || lower.contains("kubejs")
        || lower.contains("openloader")
        || (looks_like_format_arg_failure(log) && mentions_lang_path(log))
}

pub(super) fn looks_like_format_arg_failure(log: &str) -> bool {
    log.to_ascii_lowercase()
        .contains("missingformatargumentexception")
}

fn mentions_lang_path(log: &str) -> bool {
    let lower = log.to_ascii_lowercase();
    lower.contains("/lang/")
        || lower.contains("\\lang\\")
        || lower.contains("lang/")
        || lower.contains("zh_tw")
        || lower.contains(".lang")
}

/// 路徑／輸出證據＋資料載入錯誤，或 MissingFormatArgumentException＋翻譯輸出。
pub(super) fn strong_translation_evidence(log: &str) -> bool {
    if !mentions_our_output(log) {
        return false;
    }
    looks_like_data_file_error(log) || looks_like_format_arg_failure(log)
}

pub(super) fn looks_like_data_file_error(log: &str) -> bool {
    let lower = log.to_ascii_lowercase();
    lower.contains("failed to load datapacks")
        || lower.contains("error while loading pack")
        || lower.contains("couldn't parse")
        || lower.contains("failed to parse")
        || lower.contains("jsonsyntaxexception")
        || lower.contains("jsonparseexception")
        || lower.contains("malformedjsonexception")
        || lower.contains("failed to load resource")
        || (lower.contains("datapack") && lower.contains("error"))
        || (lower.contains("pack.mcmeta") && lower.contains("error"))
}

pub(super) fn looks_like_missing_registry(log: &str) -> bool {
    let lower = log.to_ascii_lowercase();
    lower.contains("missing registry entries")
        || lower.contains("unknown registry")
        || lower.contains("non-registered")
        || lower.contains("registry remapping")
        || lower.contains("unbound value")
        || lower.contains("unbound values in registry")
}

fn extract_primary_error(log: &str) -> String {
    let mut caused_by = Vec::new();
    let mut descriptions = Vec::new();
    let mut errors = Vec::new();
    for line in log.lines() {
        let text = clean_line(line);
        let lower = text.to_ascii_lowercase();
        if text.is_empty() || is_cleanup_noise(&lower) {
            continue;
        }
        if lower.contains("caused by:") {
            caused_by.push(text);
        } else if lower.starts_with("description:") || lower.contains("description:") {
            descriptions.push(text);
        } else if is_error_line(&lower) && !lower.starts_with("at ") {
            errors.push(text);
        }
    }
    caused_by
        .last()
        .or_else(|| descriptions.first())
        .or_else(|| errors.last())
        .cloned()
        .unwrap_or_default()
        .chars()
        .take(500)
        .collect()
}

fn collect_evidence(log: &str) -> Vec<String> {
    let mut scored: Vec<(i32, String)> = Vec::new();
    for line in log.lines() {
        let text = clean_line(line);
        let lower = text.to_ascii_lowercase();
        if text.is_empty() || is_cleanup_noise(&lower) {
            continue;
        }
        let score = if lower.contains("caused by:") {
            100
        } else if lower.starts_with("description:") || lower.contains("description:") {
            95
        } else if lower.contains("suspected mods") || lower.contains("mod file:") {
            90
        } else if lower.contains("mixin")
            || lower.contains("nosuchmethod")
            || lower.contains("classnotfound")
            || lower.contains("outofmemory")
            || lower.contains("unsupportedclassversion")
        {
            85
        } else if lower.contains("missing")
            || lower.contains("not found")
            || lower.contains("unbound value")
            || lower.contains("registry")
        {
            80
        } else if is_error_line(&lower) {
            60
        } else if lower.starts_with("at ") && lower.contains('@') {
            45
        } else {
            continue;
        };
        scored.push((score, text.chars().take(500).collect()));
    }
    scored.sort_by_key(|(score, _)| Reverse(*score));
    let mut evidence = Vec::new();
    for (_, line) in scored {
        if !evidence.contains(&line) {
            evidence.push(line);
        }
        if evidence.len() >= 12 {
            break;
        }
    }
    evidence
}

fn extract_game_exit_code(log: &str) -> Option<String> {
    // 不含裸 `code`：避免誤抓 log 內一般「code: …」欄位。
    re(r"(?i)(?:process\s+exited\s+with\s+code|exit\s*code|exitcode|returned\s+code|代碼|退出碼)\s*[:=：]?\s*(-?\d{1,12})")
        .captures(log)
        .and_then(|capture| capture.get(1).map(|value| value.as_str().into()))
}

fn clean_line(line: &str) -> String {
    line.trim().chars().take(700).collect()
}

fn is_cleanup_noise(lower: &str) -> bool {
    lower.contains("alltheleaks")
        || lower.contains("explicit gc")
        || lower.contains("closing selected")
        || lower.contains("craftpresence")
        || lower.contains("finished unloading")
        || lower.contains("process exited")
        || lower.contains("assertion failed: !(handle->flags")
}

fn is_error_line(lower: &str) -> bool {
    lower.contains("exception")
        || lower.contains("error")
        || lower.contains("throwable")
        || lower.contains("failed to")
        || lower.contains("could not")
        || lower.contains("access violation")
}
