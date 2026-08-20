//! 開不了遊戲／進不了世界時的本機診斷子系統。
//!
//! 管線：Normalize → Facts → Match（互斥層）→ Decide（單一勝者）→ Present。
//! 輸入可為貼上 log，或整合包／實例目錄（記錄＋ mods 交叉驗證）。

use std::path::Path;

mod decide;
mod io;
mod pack_scan;
mod support;

use decide::{clip_evidence, clip_steps, DIAGNOSIS_SCHEMA_VERSION};
use io::read_combined_logs;
use pack_scan::{looks_like_existing_pack_path, scan_pack_filesystem, FilesystemEvidence};
use support::{
    classify_runtime_failure, empty_findings, extract_class_missing, extract_findings,
    extract_missing_mods, looks_like_advancement_corruption, looks_like_data_file_error,
    looks_like_ftb_typeid_corruption, looks_like_missing_registry, looks_like_mod_loading_failure,
    looks_like_mojibake_text, looks_like_patchouli_type_corruption, looks_like_resource_path_corruption,
    looks_like_world_tick_failure,
    normalize_log, strong_translation_evidence, Findings,
};

pub const ANALYSIS_MODE_PASTED_LOG: &str = "pasted_log";
pub const ANALYSIS_MODE_PACK_DIR: &str = "pack_dir";

#[derive(Debug, Clone, Default)]
struct ClassifyContext {
    analysis_mode: String,
    pack_root: Option<String>,
    fs: Option<FilesystemEvidence>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchDiagnosis {
    /// 契約版本（前端／測試可依此相容）。
    pub schema_version: u32,
    /// pasted_log | pack_dir
    pub analysis_mode: String,
    /// 目錄模式解析後的遊戲資料根。
    pub pack_root: Option<String>,
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
    ctx: &ClassifyContext,
) -> LaunchDiagnosis {
    LaunchDiagnosis {
        schema_version: DIAGNOSIS_SCHEMA_VERSION,
        analysis_mode: if ctx.analysis_mode.is_empty() {
            ANALYSIS_MODE_PASTED_LOG.into()
        } else {
            ctx.analysis_mode.clone()
        },
        pack_root: ctx.pack_root.clone(),
        verdict: verdict.into(),
        summary,
        missing,
        translation_related,
        source: source.into(),
        error_code: error_code.into(),
        primary_error: findings.primary_error,
        evidence: clip_evidence(findings.evidence),
        suspected_mods: findings.suspected_mods,
        confidence: findings.confidence,
        next_steps: clip_steps(
            next_steps
                .iter()
                .map(|step| (*step).to_string())
                .collect(),
        ),
        game_exit_code: findings.game_exit_code,
        log_kind: findings.log_kind,
    }
}

fn pasted_ctx() -> ClassifyContext {
    ClassifyContext {
        analysis_mode: ANALYSIS_MODE_PASTED_LOG.into(),
        pack_root: None,
        fs: None,
    }
}

fn pack_ctx(fs: FilesystemEvidence) -> ClassifyContext {
    ClassifyContext {
        analysis_mode: ANALYSIS_MODE_PACK_DIR.into(),
        pack_root: Some(fs.pack_root.display().to_string()),
        fs: Some(fs),
    }
}


/// 從遊戲實例找最新的當機報告／latest.log／debug.log 並診斷。
pub fn diagnose(instance_or_mc: &Path) -> LaunchDiagnosis {
    diagnose_pack_dir(instance_or_mc)
}

/// 明確以整合包／實例目錄分析（讀記錄＋ mods 交叉驗證）。
pub fn diagnose_pack_dir(instance_or_mc: &Path) -> LaunchDiagnosis {
    let fs = match scan_pack_filesystem(instance_or_mc) {
        Ok(fs) => fs,
        Err(reason) => {
            let ctx = ClassifyContext {
                analysis_mode: ANALYSIS_MODE_PACK_DIR.into(),
                pack_root: Some(instance_or_mc.display().to_string()),
                fs: None,
            };
            return make_diagnosis(
                "unknown",
                format!("{reason}\n\n沒有足夠的目錄／記錄證據可定罪。"),
                vec![],
                false,
                "pack_dir",
                "INSUFFICIENT_EVIDENCE",
                empty_findings("no_logs"),
                &[
                    "請選含 mods 的實例資料夾，或改貼完整 crash report。",
                    "確認路徑存在且可讀取。",
                ],
                &ctx,
            );
        }
    };
    let ctx = pack_ctx(fs.clone());
    let Some((text, source)) = read_combined_logs(&fs.pack_root) else {
        let mut findings = empty_findings("no_logs");
        findings.evidence.push(fs.jar_summary.clone());
        // 無 crash 時不得只憑缺 jar 高信心定罪。
        return make_diagnosis(
            "no_logs",
            format!(
                "已解析整合包目錄，但找不到當機報告或執行紀錄（crash-reports／logs/latest.log）。\n\
{}\n\
僅有 mods 清單不足以判定根因；請先讓遊戲跑到當掉一次，或改貼完整 crash report。",
                fs.jar_summary
            ),
            vec![],
            false,
            "pack_dir",
            "INSUFFICIENT_EVIDENCE",
            findings,
            &[
                "先啟動遊戲直到產生 crash-reports 或 logs/latest.log，再分析。",
                "或直接貼上完整 crash report 文字。",
                "確認選的是正確實例資料夾。",
            ],
            &ctx,
        );
    };
    classify_with_context(&text, &source, ctx)
}

/// 貼文／路徑自動分流：單行既有目錄 → pack_dir，否則 pasted_log。
pub fn classify_input(text: &str) -> LaunchDiagnosis {
    let trimmed = text.trim();
    if looks_like_existing_pack_path(trimmed) {
        return diagnose_pack_dir(Path::new(trimmed));
    }
    classify(trimmed, "使用者貼上的錯誤文字")
}

/// 核心分類（純函式，方便測試）。
pub fn classify(log: &str, source: &str) -> LaunchDiagnosis {
    classify_with_context(log, source, pasted_ctx())
}

fn classify_with_context(log: &str, source: &str, ctx: ClassifyContext) -> LaunchDiagnosis {
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
            &ctx,
        );
    }

    let mut findings = extract_findings(&normalized, source);
    if let Some(fs) = ctx.fs.as_ref() {
        if !fs.jar_summary.is_empty() && !findings.evidence.iter().any(|e| e.contains("mods 掃描")) {
            findings.evidence.insert(0, fs.jar_summary.clone());
        }
    }
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
            &ctx,
        );
    }

    // CLASS_MISSING 優先於廣域 mod_loading／GRAPHICS 雜訊（互斥層 3）。
    if let Some(fqcn) = extract_class_missing(&normalized) {
        let srp = support::looks_like_srparasites_class_missing(&fqcn);
        let cross = ctx.fs.as_ref().map(|f| {
            (
                f.has_rlmixins || f.jar_contains("rlmixins"),
                f.has_srparasites
                    || f.jar_contains("srparasites")
                    || f.jar_contains("scapeandrun"),
            )
        });
        if srp {
            if let Some((has_rl, has_srp)) = cross {
                if has_rl && !has_srp {
                    findings.evidence.push(format!(
                        "mods 交叉驗證：找到 RLMixins，未找到 SRParasites（缺 class：{fqcn}）"
                    ));
                    findings.confidence = "high".into();
                } else if !has_srp {
                    findings.evidence.push(format!(
                        "mods 交叉驗證：未找到 SRParasites（缺 class：{fqcn}）"
                    ));
                }
            } else {
                findings.evidence.push(format!("缺少類別：{fqcn}（疑似 SRParasites）"));
            }
            return make_diagnosis(
                "mod_loading",
                format!(
                    "記錄出現缺類別例外，指向 Scape and Run Parasites（SRP）相關類別：{fqcn}。\n\
這通常是 SRParasites 模組缺失、版本不符，或與 RLMixins 等呼叫端不相容，不是翻譯檔造成。"
                ),
                vec!["srparasites".into()],
                false,
                source,
                "CLASS_MISSING_SRP",
                findings,
                &[
                    "對照整合包 mods：確認已安裝正確版本的 SRParasites。",
                    "若有 RLMixins／RLCraft 相關模組，請用同一整合包版本組合，勿混裝。",
                    "補齊後刪除舊 crash 再重開，確認新報告不再缺同一 class。",
                ],
                &ctx,
            );
        }
        findings.evidence.push(format!("缺少類別：{fqcn}"));
        let caller = findings
            .suspected_mods
            .first()
            .cloned()
            .unwrap_or_else(|| "（未從堆疊明確抓到）".into());
        return make_diagnosis(
            "mod_loading",
            format!(
                "記錄出現 NoClassDefFoundError／ClassNotFoundException，缺少類別：{fqcn}。\n\
常見原因：缺模組、模組版本不符，或呼叫端模組（堆疊可疑：{caller}）依賴了未安裝的內容。\n\
這不是翻譯語言檔問題。"
            ),
            vec![],
            false,
            source,
            "CLASS_MISSING",
            findings,
            &[
                "用缺的類別套件名對照 mods，補上對應模組或改回整合包指定版本。",
                "若剛更新過單一模組，先還原該模組版本再測。",
                "不要只看 System Details 的 OpenGL 區塊；真正原因在例外鏈。",
            ],
            &ctx,
        );
    }

    if let Some((code, summary, steps)) = classify_runtime_failure(&normalized) {
        let step_refs: Vec<&str> = steps.iter().copied().collect();
        return make_diagnosis(
            "runtime",
            summary,
            vec![],
            false,
            source,
            code,
            findings,
            &step_refs,
            &ctx,
        );
    }

    // 書頁／資源路徑檔名被翻成中文：ResourceLocationException + 非 ASCII path（Citadel 等）。
    if looks_like_resource_path_corruption(&normalized) {
        return make_diagnosis(
            "maybe_our_files",
            "記錄顯示遊戲把某個檔名或資源路徑當成 ResourceLocation，但路徑含有非 [a-z0-9/._-] 字元\
（常見是書頁 JSON 的 text／parent／linked_page 被翻成中文，例如 `zh_tw/根.txt`）。\n\
這不是缺模組。請還原上次套用後，用含此修復的版本重跑 JAR 顯示文字；勿再套用舊的 jar-translated。"
                .into(),
            vec![],
            true,
            source,
            "RESOURCE_PATH_CORRUPTED",
            findings,
            &[
                "在錯誤分析頁按「還原上次套用」，把 mods 裡被改過的 JAR 還原後再開遊戲。",
                "還原後用含此修復的版本重跑翻譯（或只重建 JAR 顯示文字再套用）。",
                "勿把舊工作目錄裡的 jar-translated 再套回實例。",
            ],
            &ctx,
        );
    }

    // FTB type／shape 等結構 id 被翻壞：ResourceLocationException + ftbquests／非 ASCII path。
    if looks_like_ftb_typeid_corruption(&normalized) {
        return make_diagnosis(
            "maybe_our_files",
            "記錄顯示 FTB Quests 的任務結構欄（type／shape／auto 等）可能被翻成中文或非法字元，\
遊戲無法把它們當成 ResourceLocation，因而崩潰。\n\
這不是缺模組，而是任務檔結構被改壞。請還原備份或改用新版工具重跑（新版只翻標題／說明，不會動 type）。"
                .into(),
            vec![],
            true,
            source,
            "FTB_TYPE_CORRUPTED",
            findings,
            &[
                "在錯誤分析頁按「還原上次套用」，把 config/ftbquests 還原後再開遊戲。",
                "若沒有備份：用 scripts/rescue_ftb_types.py 對照原整合包搶救 type／shape，或重裝實例後用 0.3.1+ 工具重翻。",
                "勿再手動把 type／shape／auto／id 等欄位翻成中文。",
            ],
            &ctx,
        );
    }

    // 進度 JSON 的 requirements／frame／translate 被翻壞：Unknown required criterion（中文）或 FrameType NPE。
    if looks_like_advancement_corruption(&normalized) {
        return make_diagnosis(
            "maybe_our_files",
            "記錄顯示 Minecraft 進度（advancement）的條件 id 或 frame 可能被翻成中文，\
導致 `Unknown required criterion` 或 `FrameType` 空值 NPE，建立／載入世界失敗。\n\
進度標題應走語言檔；不該改 datapack／JAR 內的 requirements、frame、translate 鍵。請還原上次套用後，用新版工具重跑。"
                .into(),
            vec![],
            true,
            source,
            "ADVANCEMENT_CORRUPTED",
            findings,
            &[
                "在錯誤分析頁按「還原上次套用」，把 mods 翻譯副本還原後再開遊戲。",
                "還原後用含此修復的版本重跑翻譯（或只重建 JAR 顯示文字再套用）。",
                "若仍失敗且日誌沒有 Unknown required criterion，再查其他模組／世界內容。",
            ],
            &ctx,
        );
    }

    // Patchouli 書頁 type 被翻壞：Unknown page type／ResourceLocation 含非 ASCII。
    if looks_like_patchouli_type_corruption(&normalized) {
        return make_diagnosis(
            "maybe_our_files",
            "記錄顯示 Patchouli 書本頁面的 type（或其他結構 id）可能被翻成中文或非法字元，\
導致未知頁型或 ResourceLocation 例外。\n\
這不是缺模組，而是書本 JSON 結構被改壞。請還原備份或改用新版工具重跑（新版只翻標題／內文，不會動 type）。"
                .into(),
            vec![],
            true,
            source,
            "PATCHOULI_TYPE_CORRUPTED",
            findings,
            &[
                "在錯誤分析頁按「還原上次套用」，把 patchouli／openloader 相關覆寫還原後再開遊戲。",
                "還原後用含此修復的版本重跑翻譯（勿把舊壞檔直接套回）。",
                "勿再手動把 page type／recipe／anchor／flag 等欄位翻成中文。",
            ],
            &ctx,
        );
    }

    // 顯示怪碼 Ã/å/æ：編碼／第三方包問題，不是缺字形（方框另見字體導引）。
    if looks_like_mojibake_text(&normalized) {
        return make_diagnosis(
            "maybe_our_files",
            "貼文或記錄出現 Ã、å、æ 這類「UTF-8 被錯解」的怪碼，且提到語言檔／資源包。\n\
這通常不是缺字體（缺字體會變□方框），而是舊壞檔、第三方語言包，或錯誤編碼覆寫。\n\
請還原上次套用後，用本工具重新產出；並檢查 resourcepacks 是否有其他語言包蓋過。"
                .into(),
            vec![],
            true,
            source,
            "TEXT_MOJIBAKE",
            findings,
            &[
                "在錯誤分析頁按「還原上次套用」。",
                "用新版工具重跑翻譯並再套用（勿直接套舊的壞 zh_tw／jar-translated）。",
                "遊戲資源包選單：停用可疑第三方語言包；字體包置頂；關閉 Force Unicode。",
                "若只是□方框而不是怪碼，請改走「字體資源包」服務換支援繁中的字體。",
            ],
            &ctx,
        );
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
            &ctx,
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
            &ctx,
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
            &ctx,
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
            &ctx,
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
            &ctx,
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
        &ctx,
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

    #[test]
    fn detects_resource_path_corruption_citadel_book() {
        let log = "net.minecraft.ResourceLocationException: Non [a-z0-9/._-] character in path of location: alexsmobs:book/animal_dictionary/zh_tw/根.txt\n\
at net.minecraft.resources.ResourceLocation.assertValidPath\n\
at com.github.alexthe666.citadel.client.gui.GuiBasicBook.refreshSpacing\n\
at com.github.alexthe666.alexsmobs.client.gui.GUIAnimalDictionary.init";
        let diagnosis = classify(log, "crash-2026-08-17_18.31.24-client.txt");
        assert_eq!(diagnosis.verdict, "maybe_our_files");
        assert!(diagnosis.translation_related);
        assert_eq!(diagnosis.error_code, "RESOURCE_PATH_CORRUPTED");
        assert_ne!(diagnosis.error_code, "PATCHOULI_TYPE_CORRUPTED");
        assert_ne!(diagnosis.error_code, "FTB_TYPE_CORRUPTED");
        assert!(diagnosis.next_steps.iter().any(|s| s.contains("還原")));
        assert!(super::support::looks_like_resource_path_corruption(log));
        assert!(!super::support::looks_like_patchouli_type_corruption(log));
    }

    #[test]
    fn resource_path_corruption_matches_zh_tw_txt_without_citadel() {
        let log = "ResourceLocationException: Non [a-z0-9/._-] character in path of location: examplemod:zh_tw/根.txt";
        assert!(super::support::looks_like_resource_path_corruption(log));
        let diagnosis = classify(log, "latest.log");
        assert_eq!(diagnosis.error_code, "RESOURCE_PATH_CORRUPTED");
        assert_ne!(diagnosis.error_code, "PATCHOULI_TYPE_CORRUPTED");
    }

    #[test]
    fn detects_ftb_typeid_resourcelocation_crash() {
        let log = "net.minecraft.ResourceLocationException: Non [a-z0-9/._-] character in path of location: ftbquests:勾選標記\n\
Caused by: java.lang.RuntimeException: Failed to load quests";
        let diagnosis = classify(log, "latest.log");
        assert_eq!(diagnosis.verdict, "maybe_our_files");
        assert!(diagnosis.translation_related);
        assert_eq!(diagnosis.error_code, "FTB_TYPE_CORRUPTED");
        assert!(diagnosis.summary.contains("結構欄") || diagnosis.summary.contains("type"));
        assert!(diagnosis.next_steps.iter().any(|s| s.contains("還原")));
    }

    #[test]
    fn ftb_typeid_corruption_helper_matches_non_ascii_path() {
        assert!(super::support::looks_like_ftb_typeid_corruption(
            "ResourceLocationException: Non [a-z0-9/._-] character in path of location: ftbquests:圓形"
        ));
        assert!(!super::support::looks_like_ftb_typeid_corruption(
            "ResourceLocationException: Non [a-z0-9/._-] character in path of location: minecraft:stone"
        ));
    }

    #[test]
    fn detects_advancement_criterion_corruption() {
        let log = "Parsing error loading custom advancement bakery:main/place_strawberry_crate: Unknown required criterion '草莓箱'\n\
Parsing error loading custom advancement farmersdelight:main/get_rich_soil: Unknown required criterion '獲得肥沃土壤'\n\
Caused by: java.lang.NullPointerException: Cannot invoke \"net.minecraft.advancements.FrameType.m_15552_()\" because the return value of \"net.minecraft.advancements.DisplayInfo.m_14992_()\" is null";
        let diagnosis = classify(log, "latest.log");
        assert_eq!(diagnosis.verdict, "maybe_our_files");
        assert!(diagnosis.translation_related);
        assert_eq!(diagnosis.error_code, "ADVANCEMENT_CORRUPTED");
        assert!(diagnosis.summary.contains("進度") || diagnosis.summary.contains("advancement"));
        assert!(diagnosis.next_steps.iter().any(|s| s.contains("還原")));
    }

    #[test]
    fn advancement_corruption_helper_requires_non_ascii_or_frame_npe() {
        assert!(super::support::looks_like_advancement_corruption(
            "Unknown required criterion '草莓箱'"
        ));
        assert!(super::support::looks_like_advancement_corruption(
            "Parsing error loading custom advancement foo:bar\n\
NullPointerException: FrameType because DisplayInfo is null"
        ));
        assert!(!super::support::looks_like_advancement_corruption(
            "Unknown required criterion 'has_iron'"
        ));
    }

    #[test]
    fn detects_patchouli_type_corruption() {
        let log = "vazkii.patchouli.client.book.BookEntry: Unknown page type: 合成\n\
Failed to load patchouli_books/example/en_us/entries/foo.json";
        let diagnosis = classify(log, "latest.log");
        assert_eq!(diagnosis.verdict, "maybe_our_files");
        assert!(diagnosis.translation_related);
        assert_eq!(diagnosis.error_code, "PATCHOULI_TYPE_CORRUPTED");
        assert!(diagnosis.next_steps.iter().any(|s| s.contains("還原")));
    }

    #[test]
    fn patchouli_corruption_helper_positive_and_negative() {
        assert!(super::support::looks_like_patchouli_type_corruption(
            "patchouli: Unknown page type: crafting_table"
        ));
        assert!(super::support::looks_like_patchouli_type_corruption(
            "ResourceLocationException: Non [a-z0-9/._-] in patchouli_books path: 合成"
        ));
        assert!(!super::support::looks_like_patchouli_type_corruption(
            "Unknown page type without book context"
        ));
        assert!(!super::support::looks_like_patchouli_type_corruption(
            "patchouli book loaded successfully"
        ));
    }

    #[test]
    fn detects_mojibake_vs_font_guidance() {
        let log = "resourcepacks/zh_tw.json shows Ã¤Ã¶å¸¸";
        let diagnosis = classify(log, "pasted text");
        assert_eq!(diagnosis.error_code, "TEXT_MOJIBAKE");
        assert!(diagnosis.next_steps.iter().any(|s| s.contains("還原")));
        assert!(super::support::looks_like_mojibake_text(log));
        assert!(!super::support::looks_like_mojibake_text(
            "just some boxes □□□ without encoding markers"
        ));
    }

    #[test]
    fn schema_marks_pasted_log_mode() {
        let diagnosis = classify("Process exited with code: -1", "pasted");
        assert_eq!(diagnosis.schema_version, 1);
        assert_eq!(diagnosis.analysis_mode, ANALYSIS_MODE_PASTED_LOG);
        assert!(diagnosis.pack_root.is_none());
    }

    #[test]
    fn gl_info_plus_ncdfe_is_class_missing_not_graphics() {
        let log = "---- Minecraft Crash Report ----\n\
Description: Unexpected error\n\
java.lang.NoClassDefFoundError: com/dhanantry/scapeandrunparasites/potion/SRPPotions\n\
Caused by: java.lang.ClassNotFoundException: com.dhanantry.scapeandrunparasites.potion.SRPPotions\n\
-- System Details --\n\
GL info: NVIDIA OpenGL error string dummy\n\
OpenGL: 4.6";
        let diagnosis = classify(log, "crash-report.txt");
        assert_eq!(diagnosis.error_code, "CLASS_MISSING_SRP");
        assert_ne!(diagnosis.error_code, "GRAPHICS_RUNTIME");
        assert!(!diagnosis.evidence.iter().any(|e| e.to_ascii_lowercase().contains("gl info")));
        assert!(diagnosis.next_steps.iter().all(|s| !s.contains("顯示卡")));
        assert_eq!(diagnosis.evidence.len(), diagnosis.evidence.len().min(5));
        assert!(diagnosis.next_steps.len() <= 3);
    }

    #[test]
    fn pure_access_violation_is_graphics() {
        let diagnosis = classify(
            "EXCEPTION_ACCESS_VIOLATION (0xc0000005)\n# Problematic frame:\n# C  [nvoglv64.dll+0x123]",
            "hs_err_pid.log",
        );
        assert_eq!(diagnosis.error_code, "GRAPHICS_RUNTIME");
    }

    #[test]
    fn classify_input_routes_crash_text_as_pasted_log() {
        let diagnosis = classify_input(
            "java.lang.NoClassDefFoundError: foo.Bar\nCaused by: java.lang.ClassNotFoundException: foo.Bar",
        );
        assert_eq!(diagnosis.analysis_mode, ANALYSIS_MODE_PASTED_LOG);
        assert_eq!(diagnosis.error_code, "CLASS_MISSING");
    }

    #[test]
    fn pack_dir_without_crash_is_insufficient_not_high_confidence() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("packlocal-diag-insuff-{nanos}"));
        let mods = root.join("mods");
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("RLMixins-1.0.jar"), b"x").unwrap();
        let diagnosis = diagnose_pack_dir(&root);
        assert_eq!(diagnosis.analysis_mode, ANALYSIS_MODE_PACK_DIR);
        assert_eq!(diagnosis.error_code, "INSUFFICIENT_EVIDENCE");
        assert_ne!(diagnosis.confidence, "high");
        assert!(diagnosis.pack_root.is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pack_dir_cross_checks_rlmixins_without_srp() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("packlocal-diag-srp-{nanos}"));
        let mods = root.join("mods");
        let crash = root.join("crash-reports");
        fs::create_dir_all(&mods).unwrap();
        fs::create_dir_all(&crash).unwrap();
        fs::write(mods.join("RLMixins-1.0.jar"), b"x").unwrap();
        fs::write(
            crash.join("crash-2026-01-01.txt"),
            "---- Minecraft Crash Report ----\n\
Description: Unexpected error\n\
java.lang.NoClassDefFoundError: com/dhanantry/scapeandrunparasites/potion/SRPPotions\n\
Caused by: java.lang.ClassNotFoundException: com.dhanantry.scapeandrunparasites.potion.SRPPotions\n\
-- System Details --\n\
GL info: OpenGL error noise\n",
        )
        .unwrap();
        let diagnosis = diagnose_pack_dir(&root);
        assert_eq!(diagnosis.analysis_mode, ANALYSIS_MODE_PACK_DIR);
        assert_eq!(diagnosis.error_code, "CLASS_MISSING_SRP");
        assert!(diagnosis
            .evidence
            .iter()
            .any(|e| e.contains("RLMixins") && e.contains("SRParasites")));
        assert_eq!(diagnosis.confidence, "high");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn classify_input_routes_existing_dir_as_pack_dir() {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("packlocal-diag-route-{nanos}"));
        fs::create_dir_all(root.join("mods")).unwrap();
        let diagnosis = classify_input(root.to_str().unwrap());
        assert_eq!(diagnosis.analysis_mode, ANALYSIS_MODE_PACK_DIR);
        assert_eq!(diagnosis.error_code, "INSUFFICIENT_EVIDENCE");
        let _ = fs::remove_dir_all(&root);
    }
}
