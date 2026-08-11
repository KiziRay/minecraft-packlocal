//! 開不了遊戲／進不了世界時的診斷。
//!
//! 玩家最容易誤會的情境：套用我們的翻譯後遊戲開不起來，就怪翻譯——但十之八九是
//! **整合包本身缺模組／缺前置**（結構模組、相依模組），跟語言檔無關。這個模組讀遊戲的
//! 當機報告與 `latest.log`，判斷是哪一類，並**點名缺了哪個模組**，讓玩家知道該去補什麼，
//! 而不是空等或誤刪翻譯。
//!
//! 分類（優先序）：
//! 1. **缺模組／前置**（Forge/NeoForge/Fabric 的相依錯誤）→ 與翻譯無關，點名缺什麼。
//! 2. **可能是我們的檔**（datapack／json 載入錯誤，或訊息提到我們的輸出）→ 建議先「還原」排除。
//! 3. **缺內容／註冊項**（missing registry）→ 缺內容模組，非翻譯。
//! 4. **不確定** → 建議先還原排除翻譯，再看當機報告。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use regex::Regex;
use std::sync::OnceLock;

/// 單一檔最多讀這麼多（當機報告／log 可能很大）。
const MAX_READ_BYTES: usize = 3 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchDiagnosis {
    /// missing_mod | maybe_our_files | content_missing | unknown | no_logs
    pub verdict: String,
    /// 給玩家的白話結論
    pub summary: String,
    /// 點名缺的模組／前置（可空）
    pub missing: Vec<String>,
    /// 是否可能與我們的翻譯有關（true＝建議先還原排除）
    pub translation_related: bool,
    /// 依據來源（哪個檔）
    pub source: String,
    /// A stable local classification code, not a Minecraft exit code.
    pub error_code: String,
    pub primary_error: String,
    pub evidence: Vec<String>,
}

fn re(src: &'static str) -> &'static Regex {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<&'static str, &'static Regex>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut g = cache.lock().expect("diagnose regex cache");
    g.entry(src).or_insert_with(|| {
        Box::leak(Box::new(Regex::new(src).expect("diagnose regex compiles")))
    })
}

/// 從遊戲實例找最新的當機報告／latest.log 並診斷。
pub fn diagnose(instance_or_mc: &Path) -> LaunchDiagnosis {
    let mc = super::jar_scan::resolve_minecraft_dir(instance_or_mc)
        .unwrap_or_else(|_| instance_or_mc.to_path_buf());

    let Some((text, source)) = read_combined_logs(&mc) else {
        return LaunchDiagnosis {
            verdict: "no_logs".into(),
            summary: "找不到當機報告或執行紀錄（crash-reports／logs/latest.log）。\n\
請先讓遊戲跑到當掉一次，產生紀錄後再診斷；或確認選的是正確的整合包資料夾。"
                .into(),
            missing: vec![],
            translation_related: false,
            source: String::new(),
            error_code: "NO_LOGS".into(),
            primary_error: String::new(),
            evidence: Vec::new(),
        };
    };

    classify(&text, &source)
}

/// 核心分類（純函式，方便測試）。
pub fn classify(log: &str, source: &str) -> LaunchDiagnosis {
    let missing = extract_missing_mods(log);

    // 1) 缺模組／前置：最常見、最該優先判定
    if !missing.is_empty() || looks_like_dependency_failure(log) {
        let names = if missing.is_empty() {
            "（報告未明確點名，請看下方來源檔）".to_string()
        } else {
            missing.join("、")
        };
        return LaunchDiagnosis {
            verdict: "missing_mod".into(),
            summary: format!(
                "這是**整合包缺模組／缺前置**造成的，**跟翻譯無關**（我們只加語言檔、不改模組）。\n\
偵測到需要但缺少的：{names}\n\n\
怎麼辦：到 CurseForge／Modrinth 補上這些模組（版本要對應你的 Minecraft 與載入器），\n\
或用整合包原本的安裝方式重裝。補齊後就能開了。"
            ),
            missing,
            translation_related: false,
            source: source.to_string(),
            error_code: "MISSING_MOD".into(),
            primary_error: extract_primary_error(log),
            evidence: collect_evidence(log),
        };
    }

    // 2) 可能是我們的檔：datapack／json 載入錯誤，或訊息提到我們的輸出
    if mentions_our_output(log) || looks_like_datapack_or_json_error(log) {
        return LaunchDiagnosis {
            verdict: "maybe_our_files".into(),
            summary: "偵測到資料包／JSON 載入相關錯誤，**有可能**與套用的翻譯檔有關。\n\
請先按「還原上次套用」把我們的檔移除，再開一次遊戲：\n\
• 開得起來 → 就是某個翻譯檔的問題，請把當機報告給我們修\n\
• 還是開不起來 → 不是翻譯，是整合包本身（多半仍是缺模組）"
                .into(),
            missing: vec![],
            translation_related: true,
            source: source.to_string(),
            error_code: "TRANSLATED_FILE_LOAD".into(),
            primary_error: extract_primary_error(log),
            evidence: collect_evidence(log),
        };
    }

    // 3) 缺內容／註冊項（missing registry）→ 缺內容模組
    if looks_like_missing_registry(log) {
        return LaunchDiagnosis {
            verdict: "content_missing".into(),
            summary: "偵測到缺少內容／註冊項（missing registry entries）——通常是**缺內容模組**\n\
或版本不合，跟翻譯無關。請確認整合包完整、版本與載入器正確。"
                .into(),
            missing: vec![],
            translation_related: false,
            source: source.to_string(),
            error_code: "CONTENT_REGISTRY".into(),
            primary_error: extract_primary_error(log),
            evidence: collect_evidence(log),
        };
    }

    // 4) 不確定
    LaunchDiagnosis {
        verdict: "unknown".into(),
        summary: "無法從紀錄明確判斷原因。建議：\n\
1. 先按「還原上次套用」移除我們的檔，再開一次——藉此排除是不是翻譯造成的。\n\
2. 若還原後仍開不起來，多半是整合包本身（缺模組／版本不合），把當機報告提供給整合包作者。\n\
（本工具不直接寫入原始 mods/*.jar；翻譯副本可能影響載入，還原即可排除。本工具不處理閃退本身。）"
            .into(),
        missing: vec![],
        translation_related: true,
        source: source.to_string(),
        error_code: "UNKNOWN".into(),
        primary_error: extract_primary_error(log),
        evidence: collect_evidence(log),
    }
}

// ─── 讀取最新紀錄 ───────────────────────────────────────────

#[allow(dead_code)]
fn read_newest_log(mc: &Path) -> Option<(String, String)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    // crash-reports/*.txt（最新的通常最相關）
    let cr = mc.join("crash-reports");
    if cr.is_dir() {
        if let Ok(rd) = fs::read_dir(&cr) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("txt") {
                    candidates.push(p);
                }
            }
        }
    }
    // logs/latest.log
    let latest = mc.join("logs").join("latest.log");
    if latest.is_file() {
        candidates.push(latest);
    }
    if candidates.is_empty() {
        return None;
    }
    // 取修改時間最新的
    candidates.sort_by_key(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    let newest = candidates.last()?.clone();
    let text = read_bounded(&newest)?;
    Some((text, newest.display().to_string()))
}

fn read_bounded(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let slice = if bytes.len() > MAX_READ_BYTES {
        &bytes[bytes.len() - MAX_READ_BYTES..] // 取末段（錯誤通常在後面）
    } else {
        &bytes[..]
    };
    Some(String::from_utf8_lossy(slice).into_owned())
}

// ─── 分類判斷 ───────────────────────────────────────────────

fn read_combined_logs(mc: &Path) -> Option<(String, String)> {
    let latest = mc.join("logs").join("latest.log");
    let newest_crash = fs::read_dir(mc.join("crash-reports"))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("txt"))
        .max_by_key(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH)
        });

    let mut parts = Vec::new();
    let mut sources = Vec::new();
    if let Some(path) = newest_crash {
        if let Some(text) = read_bounded(&path) {
            parts.push(format!("--- crash report: {} ---\n{}", path.display(), text));
            sources.push(path.display().to_string());
        }
    }
    if let Some(text) = read_bounded(&latest) {
        parts.push(format!("--- latest.log: {} ---\n{}", latest.display(), text));
        sources.push(latest.display().to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some((parts.join("\n"), sources.join(" + ")))
    }
}

fn looks_like_dependency_failure(log: &str) -> bool {
    let l = log.to_ascii_lowercase();
    l.contains("missing or unsupported mandatory dependencies")
        || l.contains("mod resolution failed")
        || l.contains("incompatible mods found")
        || l.contains("requires a mod")
        || (l.contains("requires") && l.contains("which is missing"))
        || (l.contains("depends on") && l.contains("missing"))
}

/// 抓出被點名的缺失模組／前置名稱。涵蓋 Forge/NeoForge 與 Fabric 的常見訊息。
fn extract_missing_mods(log: &str) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();

    // Forge/NeoForge：Mod ID: 'jei', Requested by: 'x', ... 或 "requires 'sophisticatedcore'"
    for pat in [
        r"(?i)requires\s+['\x22]([a-z0-9_\-]{2,})['\x22]",
        r"(?i)mod\s+id\s*[:=]\s*['\x22]([a-z0-9_\-]{2,})['\x22]",
        r"(?i)dependency\s+['\x22]([a-z0-9_\-]{2,})['\x22]",
        // Fabric：requires version x.y of {fabric-api}, which is missing
        r"(?i)requires\s+(?:version\s+\S+\s+of\s+)?\{?([a-z0-9_\-]{2,})\}?[^\n]*?which is missing",
        // Fabric：Mod '...' (modid) ... requires ... of mod '...'(depid)
        r"(?i)of mod\s+['\x22][^'\x22]+['\x22]\s*\(([a-z0-9_\-]{2,})\)",
    ] {
        for c in re(pat).captures_iter(log) {
            if let Some(m) = c.get(1) {
                let id = m.as_str().to_ascii_lowercase();
                if is_plausible_mod_id(&id) {
                    out.insert(id);
                }
            }
        }
    }
    out.into_iter().take(20).collect()
}

fn is_plausible_mod_id(id: &str) -> bool {
    // 排除明顯的非模組字（載入器自身、泛用字）
    const NOISE: &[&str] = &[
        "minecraft", "forge", "neoforge", "fabricloader", "fabric", "java", "mod", "the",
    ];
    id.len() >= 2 && !NOISE.contains(&id) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn looks_like_datapack_or_json_error(log: &str) -> bool {
    let l = log.to_ascii_lowercase();
    l.contains("failed to load datapacks")
        || l.contains("error while loading pack")
        || l.contains("couldn't parse")
        || l.contains("failed to parse")
        || (l.contains("datapack") && l.contains("error"))
        || l.contains("jsonsyntaxexception")
        || l.contains("jsonparseexception")
}

fn mentions_our_output(log: &str) -> bool {
    let l = log.to_ascii_lowercase();
    // 我們輸出的特徵：zh_tw 語言檔、預設資源包名、覆蓋範圍說明
    l.contains("zh_tw.json") || l.contains("繁體中文翻譯") || l.contains("台灣繁")
}

fn looks_like_missing_registry(log: &str) -> bool {
    let l = log.to_ascii_lowercase();
    l.contains("missing registry entries")
        || l.contains("unknown registry")
        || l.contains("non-registered")
        || l.contains("registry remapping")
        || l.contains("unbound value")
        || l.contains("unbound values in registry")
}

fn extract_primary_error(log: &str) -> String {
    for line in log.lines().rev() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if (lower.contains("exception")
            || lower.contains("error")
            || lower.contains("unbound value")
            || lower.contains("failed to load"))
            && !trimmed.starts_with("at ")
        {
            return trimmed.chars().take(400).collect();
        }
    }
    String::new()
}

fn collect_evidence(log: &str) -> Vec<String> {
    let mut evidence = Vec::new();
    for line in log.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("unbound value")
            || lower.contains("unbound values in registry")
            || lower.contains("missing or unsupported")
            || lower.contains("requires a mod")
            || lower.contains("failed to load datapacks")
            || lower.contains("encountered an unexpected exception")
        {
            let line = trimmed.chars().take(400).collect::<String>();
            if !evidence.contains(&line) {
                evidence.push(line);
            }
            if evidence.len() >= 8 {
                break;
            }
        }
    }
    evidence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_forge_missing_dependency_and_names_it() {
        let log = "Missing or unsupported mandatory dependencies:\n\
\tMod ID: 'sophisticatedcore', Requested by: 'sophisticatedbackpacks', Expected range: '[1.0,)'";
        let d = classify(log, "crash.txt");
        assert_eq!(d.verdict, "missing_mod");
        assert!(!d.translation_related);
        assert!(d.missing.contains(&"sophisticatedcore".to_string()));
    }

    #[test]
    fn detects_fabric_missing_dependency() {
        let log = "Mod resolution failed\nMod 'X' (x) requires version 0.90 of {fabric-api}, which is missing!";
        let d = classify(log, "log");
        assert_eq!(d.verdict, "missing_mod");
        assert!(d.missing.iter().any(|m| m.contains("fabric-api")));
    }

    #[test]
    fn missing_mod_is_not_blamed_on_translation() {
        let log = "requires 'jei' which is missing";
        let d = classify(log, "log");
        assert_eq!(d.verdict, "missing_mod");
        assert!(!d.translation_related);
    }

    #[test]
    fn datapack_error_suggests_restore_first() {
        let log = "Failed to load datapacks, can't proceed with server load. JsonSyntaxException";
        let d = classify(log, "log");
        assert_eq!(d.verdict, "maybe_our_files");
        assert!(d.translation_related);
    }

    #[test]
    fn error_mentioning_our_output_flags_our_files() {
        let log = "Error while loading pack assets/foo/lang/zh_tw.json";
        let d = classify(log, "log");
        assert_eq!(d.verdict, "maybe_our_files");
    }

    #[test]
    fn missing_registry_points_at_content_mod() {
        let log = "Missing registry entries for minecraft:item";
        let d = classify(log, "log");
        assert_eq!(d.verdict, "content_missing");
        assert!(!d.translation_related);
        assert_eq!(d.error_code, "CONTENT_REGISTRY");
    }

    #[test]
    fn unbound_registry_value_is_not_blamed_on_translation() {
        let log = "Unbound values in registry: structory_towers:end/end_tower\n\
            IllegalStateException: Trying to access unbound value";
        let d = classify(log, "crash-report.txt");
        assert_eq!(d.verdict, "content_missing");
        assert_eq!(d.error_code, "CONTENT_REGISTRY");
        assert!(!d.translation_related);
        assert!(!d.evidence.is_empty());
    }

    #[test]
    fn unknown_log_advises_restore_to_rule_out() {
        let d = classify("some unrelated stack trace", "log");
        assert_eq!(d.verdict, "unknown");
        assert!(d.translation_related);
    }

    #[test]
    fn loader_names_are_not_reported_as_missing_mods() {
        // 不要把 minecraft／forge 自己當成缺的模組
        let log = "requires 'minecraft' and 'forge'";
        let d = classify(log, "log");
        assert!(d.missing.is_empty() || !d.missing.iter().any(|m| m == "minecraft" || m == "forge"));
    }
}
