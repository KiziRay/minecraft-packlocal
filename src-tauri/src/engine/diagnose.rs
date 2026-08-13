//! 開不了遊戲／進不了世界時的本機診斷。
//!
//! 這裡只做證據整理與規則判讀，不把任何一行 `ERROR` 直接當成根因。
//! 讀取與文字特徵擷取放在 `diagnose_support`，本檔只負責分類與對玩家說明。

use std::path::Path;

#[path = "diagnose_support.rs"]
mod support;
#[path = "diagnose_io.rs"]
mod io;

use support::{
    classify_runtime_failure, empty_findings, extract_findings, extract_missing_mods,
    looks_like_data_file_error, looks_like_missing_registry, looks_like_mod_loading_failure,
    looks_like_world_tick_failure, normalize_log, strong_translation_evidence, Findings,
};
use io::read_combined_logs;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchDiagnosis {
    /// missing_mod | runtime | mod_loading | world_content | maybe_our_files |
    /// content_missing | content_data | unknown | no_logs
    pub verdict: String,
    pub summary: String,
    pub missing: Vec<String>,
    /// 只有在記錄明確指向翻譯輸出時才會是 true。
    pub translation_related: bool,
    pub source: String,
    /// 工具的穩定分類代碼，不是 Minecraft 退出碼。
    pub error_code: String,
    /// 最接近根因的錯誤行，不再直接取 log 最後一行。
    pub primary_error: String,
    pub evidence: Vec<String>,
    pub suspected_mods: Vec<String>,
    /// high | medium | low，代表規則命中的證據強度。
    pub confidence: String,
    pub next_steps: Vec<String>,
    /// 從記錄擷取到的遊戲退出碼，不等同於 error_code。
    pub game_exit_code: Option<String>,
    pub log_kind: String,
}

fn make_diagnosis(
    verdict: &str,
    summary: String,
    missing: Vec<String>,
    translation_related: bool,
    source: &str,
    error_code: &str,
    findings: Findings,
    next_steps: &[&str],
) -> LaunchDiagnosis {
    LaunchDiagnosis {
        verdict: verdict.into(),
        summary,
        missing,
        translation_related,
        source: source.into(),
        error_code: error_code.into(),
        primary_error: findings.primary_error,
        evidence: findings.evidence,
        suspected_mods: findings.suspected_mods,
        confidence: findings.confidence,
        next_steps: next_steps.iter().map(|step| (*step).into()).collect(),
        game_exit_code: findings.game_exit_code,
        log_kind: findings.log_kind,
    }
}

/// 從遊戲實例找最新的當機報告／latest.log／debug.log 並診斷。
pub fn diagnose(instance_or_mc: &Path) -> LaunchDiagnosis {
    let mc = super::jar_scan::resolve_minecraft_dir(instance_or_mc)
        .unwrap_or_else(|_| instance_or_mc.to_path_buf());

    let Some((text, source)) = read_combined_logs(&mc) else {
        return LaunchDiagnosis {
            verdict: "no_logs".into(),
            summary: "找不到當機報告或執行紀錄（crash-reports／logs/latest.log）。\n\
請先讓遊戲跑到當掉一次，產生紀錄後再診斷；也要確認選的是正確的整合包資料夾。"
                .into(),
            missing: vec![],
            translation_related: false,
            source: String::new(),
            error_code: "NO_LOGS".into(),
            primary_error: String::new(),
            evidence: Vec::new(),
            suspected_mods: Vec::new(),
            confidence: "low".into(),
            next_steps: vec![
                "先讓遊戲再啟動一次，等它產生 crash-reports 或 logs/latest.log。".into(),
                "確認工具選的是包含 minecraft 資料夾的正確實例。".into(),
            ],
            game_exit_code: None,
            log_kind: "no_logs".into(),
        };
    };

    classify(&text, &source)
}

/// 核心分類（純函式，方便測試）。
pub fn classify(log: &str, source: &str) -> LaunchDiagnosis {
    let normalized = normalize_log(log);
    if normalized.trim().is_empty() {
        return make_diagnosis(
            "no_logs",
            "沒有可分析的內容。請貼上完整 crash report，或至少貼上從 `Description:`／`Caused by:` 開始的錯誤段落。".into(),
            vec![],
            false,
            source,
            "INSUFFICIENT_EVIDENCE",
            empty_findings("pasted_text"),
            &["不要只貼 Minecraft 的退出碼，請一併貼 crash-reports/*.txt 或 logs/latest.log。"],
        );
    }

    let findings = extract_findings(&normalized, source);
    let missing = extract_missing_mods(&normalized);

    if !missing.is_empty() || support::looks_like_dependency_failure(&normalized) {
        let names = if missing.is_empty() {
            "報告提到缺少前置，但沒有成功擷取模組 ID".to_string()
        } else {
            missing.join("、")
        };
        return make_diagnosis(
            "missing_mod",
            format!(
                "這是整合包的模組依賴沒有滿足，和翻譯檔沒有直接證據關聯。\n\
需要補上的模組：{names}\n\n\
請依照 Minecraft 版本與 Forge／NeoForge／Fabric 版本補齊相同版本的模組；不要只下載同名但不同版本的檔案。"
            ),
            missing,
            false,
            source,
            "MISSING_MOD",
            findings,
            &[
                "先補齊分析結果列出的模組與前置，再重新啟動遊戲。",
                "如果模組已存在，檢查它的版本是否符合報告中的需求範圍。",
            ],
        );
    }

    if let Some((code, summary, steps)) = classify_runtime_failure(&normalized) {
        return make_diagnosis("runtime", summary, vec![], false, source, code, findings, &steps);
    }

    // 強翻譯證據須早於廣域 mod_loading，避免路徑＋資料錯誤／格式佔位符例外被蓋掉。
    if strong_translation_evidence(&normalized) {
        return make_diagnosis(
            "maybe_our_files",
            "記錄明確指向翻譯輸出的語言檔、任務／腳本覆寫，或語言字串的 MissingFormatArgumentException，這次才有足夠理由先排除翻譯。\n\
請先還原上次套用，再重新開啟遊戲：還原後正常，才代表需要修正翻譯輸出；還原後仍失敗，原因在其他模組或遊戲環境。"
                .into(),
            vec![],
            true,
            source,
            "TRANSLATED_FILE_LOAD",
            findings,
            &[
                "在錯誤分析頁按「還原上次套用」，再重新開啟同一個世界。",
                "還原後仍失敗時，請不要再修改翻譯檔，改檢查模組與版本。",
            ],
        );
    }

    if looks_like_mod_loading_failure(&normalized) {
        let mods = if findings.suspected_mods.is_empty() {
            "目前沒有從堆疊明確抓到模組名稱".to_string()
        } else {
            findings.suspected_mods.join("、")
        };
        return make_diagnosis(
            "mod_loading",
            format!(
                "這是模組載入／初始化或版本相容性錯誤，不是看到 `Exception` 就能判定為翻譯。\n\
記錄中較可疑的模組：{mods}\n\n\
這類錯誤常見於模組版本、Minecraft 版本、載入器版本不一致，或兩個模組的 Mixin 衝突。"
            ),
            vec![],
            false,
            source,
            "MOD_LOADING",
            findings,
            &[
                "先確認可疑模組與 Minecraft／載入器版本相符。",
                "若記錄有 `Mixin apply failed`，暫時移除或更新列出的模組後再測試。",
                "只有錯誤路徑明確指向翻譯輸出時，才使用還原功能排除翻譯。",
            ],
        );
    }

    if looks_like_world_tick_failure(&normalized) {
        let mods = if findings.suspected_mods.is_empty() {
            "目前沒有從堆疊明確抓到模組名稱".to_string()
        } else {
            findings.suspected_mods.join("、")
        };
        return make_diagnosis(
            "world_content",
            format!(
                "遊戲是在世界內容更新或建立世界時出錯，這通常和方塊／生物／世界生成模組或存檔內容有關。\n\
記錄中較可疑的模組：{mods}\n\n\
目前沒有看到翻譯語言檔是主因的證據。"
            ),
            vec![],
            false,
            source,
            "WORLD_TICK",
            findings,
            &[
                "先用相同整合包建立新世界測試，分辨是整合包本身還是單一存檔。",
                "查看證據中的模組名稱，更新或暫時移除該模組。",
                "若只在套用翻譯後發生，先還原一次；還原後仍閃退就不是翻譯檔。",
            ],
        );
    }

    if looks_like_missing_registry(&normalized) {
        return make_diagnosis(
            "content_missing",
            "記錄顯示內容註冊或遊戲資料不完整，通常是模組內容缺失、版本不合，或世界資料與模組版本不一致。\n\
目前沒有看到翻譯檔是主因的證據。"
                .into(),
            vec![],
            false,
            source,
            "CONTENT_REGISTRY",
            findings,
            &[
                "確認整合包的模組沒有被移除，並使用原本指定的 Minecraft／載入器版本。",
                "如果只在某個世界發生，先用備份存檔測試，避免覆蓋原存檔。",
            ],
        );
    }

    if looks_like_data_file_error(&normalized) {
        return make_diagnosis(
            "content_data",
            "記錄顯示資料包、JSON 或資源資料載入失敗，但路徑沒有指向翻譯輸出，因此不能把它判定成翻譯問題。"
                .into(),
            vec![],
            false,
            source,
            "CONTENT_DATA",
            findings,
            &[
                "依照證據中的檔案路徑檢查對應模組或資料包。",
                "確認整合包沒有混用不同 Minecraft 版本的設定檔。",
            ],
        );
    }

    let unknown_code = if findings.primary_error.is_empty() {
        "INSUFFICIENT_EVIDENCE"
    } else {
        "UNCLASSIFIED_ERROR"
    };
    let unknown_summary = if findings.primary_error.is_empty() {
        "目前只有錯誤摘要，沒有足夠證據判斷真正原因。這不代表翻譯有問題，也不代表翻譯沒有問題。\n\
請補上完整 crash report 或 latest.log，尤其是 `Description:`、`Caused by:` 和最前面的模組錯誤段落。"
            .into()
    } else {
        format!(
            "記錄有抓到錯誤行，但目前無法對應到可信的原因類型。這不代表翻譯有問題，也不代表翻譯沒有問題。\n\
最接近的錯誤：{}\n\
請補上完整 crash report，讓工具能比對錯誤前後的模組與版本資訊。",
            findings.primary_error
        )
    };
    make_diagnosis(
        "unknown",
        unknown_summary,
        vec![],
        false,
        source,
        unknown_code,
        findings,
        &[
            "不要只貼 `Process exited with code: -1`，那只是結果，不是原因。",
            "請貼完整 crash report，或同時提供 logs/latest.log 與 logs/debug.log。",
            "如果只有套用翻譯後才發生，先還原一次，再用新的完整記錄重新分析。",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_forge_missing_dependency_and_names_it() {
        let log = "Missing or unsupported mandatory dependencies:\n\
\tMod ID: 'sophisticatedcore', Requested by: 'sophisticatedbackpacks', Expected range: '[1.0,)'";
        let diagnosis = classify(log, "crash.txt");
        assert_eq!(diagnosis.verdict, "missing_mod");
        assert!(!diagnosis.translation_related);
        assert!(diagnosis.missing.contains(&"sophisticatedcore".to_string()));
        assert_eq!(diagnosis.confidence, "high");
    }

    #[test]
    fn detects_fabric_missing_dependency() {
        let log = "Mod resolution failed\nMod 'X' (x) requires version 0.90 of {fabric-api}, which is missing!";
        let diagnosis = classify(log, "log");
        assert_eq!(diagnosis.verdict, "missing_mod");
        assert!(diagnosis.missing.iter().any(|item| item == "fabric-api"));
    }

    #[test]
    fn missing_mod_is_not_blamed_on_translation() {
        let diagnosis = classify("requires 'jei' which is missing", "log");
        assert_eq!(diagnosis.verdict, "missing_mod");
        assert!(!diagnosis.translation_related);
    }

    #[test]
    fn detects_java_runtime_failure() {
        let diagnosis = classify(
            "java.lang.UnsupportedClassVersionError: class file version 65.0, this version only recognizes up to 61.0",
            "latest.log",
        );
        assert_eq!(diagnosis.verdict, "runtime");
        assert_eq!(diagnosis.error_code, "JAVA_VERSION");
        assert!(!diagnosis.translation_related);
    }

    #[test]
    fn detects_mixin_and_names_mod() {
        let log = "org.spongepowered.asm.mixin.transformer.throwables.MixinTransformerError: Mixin apply failed\n\
at create@1.2.3/com.example.SomeClass.method(SomeClass.java:12)";
        let diagnosis = classify(log, "crash-report.txt");
        assert_eq!(diagnosis.verdict, "mod_loading");
        assert!(diagnosis.suspected_mods.contains(&"create".to_string()));
        assert!(!diagnosis.translation_related);
    }

    #[test]
    fn detects_world_tick_failure_during_world_creation() {
        let log = "Description: Ticking block entity\n\
Caused by: java.lang.IllegalStateException: example failed\n\
at create@1.2.3/com.example.SomeBlock.tick(SomeBlock.java:12)";
        let diagnosis = classify(log, "crash-report.txt");
        assert_eq!(diagnosis.verdict, "world_content");
        assert_eq!(diagnosis.error_code, "WORLD_TICK");
        assert!(diagnosis.suspected_mods.contains(&"create".to_string()));
        assert!(!diagnosis.translation_related);
    }

    #[test]
    fn json_error_without_translation_path_is_not_blamed_on_translation() {
        let diagnosis = classify(
            "Failed to load datapacks, JsonSyntaxException: data/example/loot.json",
            "latest.log",
        );
        assert_eq!(diagnosis.verdict, "content_data");
        assert!(!diagnosis.translation_related);
    }

    #[test]
    fn json_error_with_translation_path_can_be_isolated() {
        let diagnosis = classify(
            "Error while loading pack assets/foo/lang/zh_tw.json: JsonSyntaxException",
            "latest.log",
        );
        assert_eq!(diagnosis.verdict, "maybe_our_files");
        assert!(diagnosis.translation_related);
    }

    #[test]
    fn deepest_caused_by_beats_cleanup_error() {
        let log = "Description: Exception in server tick loop\n\
Caused by: java.lang.RuntimeException: world failed\n\
Caused by: java.lang.IllegalStateException: real cause\n\
[Server thread/INFO] [CraftPresence]: closing";
        let diagnosis = classify(log, "crash-report.txt");
        assert_eq!(diagnosis.primary_error, "Caused by: java.lang.IllegalStateException: real cause");
        assert!(!diagnosis.evidence.is_empty());
    }

    #[test]
    fn keeps_exit_code_separate_from_local_error_code() {
        let diagnosis = classify("Process exited with code: -1", "pasted text");
        assert_eq!(diagnosis.error_code, "INSUFFICIENT_EVIDENCE");
        assert_eq!(diagnosis.game_exit_code.as_deref(), Some("-1"));
    }

    #[test]
    fn extracts_chinese_exit_code_without_treating_it_as_root_cause() {
        let diagnosis = classify("處理程序已結束，代碼：-1", "pasted text");
        assert_eq!(diagnosis.game_exit_code.as_deref(), Some("-1"));
        assert_eq!(diagnosis.error_code, "INSUFFICIENT_EVIDENCE");
    }

    #[test]
    fn short_unknown_input_requests_more_evidence_without_blaming_translation() {
        let diagnosis = classify("Crash Report UUID: abc", "pasted text");
        assert_eq!(diagnosis.verdict, "unknown");
        assert!(!diagnosis.translation_related);
        assert!(!diagnosis.next_steps.is_empty());
    }

    #[test]
    fn registry_error_points_at_content_mod() {
        let diagnosis = classify("Missing registry entries for minecraft:item", "crash-report.txt");
        assert_eq!(diagnosis.verdict, "content_missing");
        assert!(!diagnosis.translation_related);
        assert_eq!(diagnosis.error_code, "CONTENT_REGISTRY");
    }

    #[test]
    fn loader_names_are_not_reported_as_missing_mods() {
        let diagnosis = classify("requires 'minecraft' and 'forge'", "log");
        assert!(!diagnosis
            .missing
            .iter()
            .any(|item| item == "minecraft" || item == "forge"));
    }

    #[test]
    fn format_arg_exception_with_lang_path_is_translation_related() {
        let log = "java.util.MissingFormatArgumentException: Format specifier '%s'\n\
Failed while formatting assets/foo/lang/zh_tw.json";
        let diagnosis = classify(log, "latest.log");
        assert_eq!(diagnosis.verdict, "maybe_our_files");
        assert!(diagnosis.translation_related);
        assert_eq!(diagnosis.error_code, "TRANSLATED_FILE_LOAD");
    }

    #[test]
    fn translation_path_beats_broad_mod_loading() {
        let log = "Error during mod loading\n\
Error while loading pack config/ftbquests/chapters/a.snbt: JsonParseException";
        let diagnosis = classify(log, "latest.log");
        assert_eq!(diagnosis.verdict, "maybe_our_files");
        assert!(diagnosis.translation_related);
    }

    #[test]
    fn bare_code_field_is_not_treated_as_exit_code() {
        let diagnosis = classify("status code: 503\nCrash Report UUID: abc", "pasted text");
        assert!(diagnosis.game_exit_code.is_none());
    }

    #[test]
    fn stale_crash_is_detected_when_behind_latest_by_over_48h() {
        use std::time::{Duration, SystemTime};
        let latest = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let fresh = latest - Duration::from_secs(47 * 60 * 60);
        let stale = latest - Duration::from_secs(49 * 60 * 60);
        assert!(!super::io::crash_is_stale_vs_latest(fresh, latest));
        assert!(super::io::crash_is_stale_vs_latest(stale, latest));
        assert!(!super::io::crash_is_stale_vs_latest(latest, fresh));
    }

    #[test]
    fn mentions_kubejs_and_openloader_as_our_output() {
        assert!(super::support::mentions_our_output(
            "Failed to parse kubejs/server_scripts/foo.js"
        ));
        assert!(super::support::mentions_our_output(
            "Error while loading pack config/openloader/resources/pack"
        ));
        assert!(super::support::mentions_our_output(
            "MissingFormatArgumentException in assets/mod/lang/en_us.json"
        ));
    }
}
