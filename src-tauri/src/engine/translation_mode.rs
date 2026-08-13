//! 翻譯工作模式：把重跑、續翻與完整度門檻說清楚，避免玩家誤以為每次都會重送 AI。

use super::jar_scan::LangMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationMode {
    Append,
    SkipIfComplete,
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationQuality {
    Fast,
    Balanced,
    Thorough,
}

impl TranslationQuality {
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("balanced").trim().to_ascii_lowercase().as_str() {
            "fast" | "quick" => Self::Fast,
            "thorough" | "quality" | "good" => Self::Thorough,
            _ => Self::Balanced,
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Thorough => "thorough",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "介面優先（較快）",
            Self::Balanced => "一般翻譯（建議）",
            Self::Thorough => "劇情與書本優先（較仔細）",
        }
    }

    pub fn batch_size(self) -> usize {
        match self {
            Self::Fast => 180,
            Self::Balanced => 140,
            Self::Thorough => 70,
        }
    }
}

impl TranslationMode {
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("append").trim().to_ascii_lowercase().as_str() {
            "skip" | "skip-if-complete" | "skip90" => Self::SkipIfComplete,
            "force" | "重新翻譯" => Self::Force,
            _ => Self::Append,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Append => "接續翻譯",
            Self::SkipIfComplete => "已達九成就略過",
            Self::Force => "重新翻譯缺漏（忽略記憶）",
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::SkipIfComplete => "skip-if-complete",
            Self::Force => "force",
        }
    }
}

/// Skip90 只略過「目前已中文／待補」達門檻的命名空間；未達門檻的照常處理。
/// 回傳被略過的 key 數，呼叫端必須寫進工作階段／報告，不能靜默丟掉。
pub fn skip_complete_namespaces(
    zh: &LangMap,
    en_only: &mut LangMap,
    threshold_percent: usize,
) -> usize {
    let mut skipped = 0usize;
    let namespaces = en_only.keys().cloned().collect::<Vec<_>>();
    for namespace in namespaces {
        let pending = en_only.get(&namespace).map(|m| m.len()).unwrap_or(0);
        let translated = zh.get(&namespace).map(|m| m.len()).unwrap_or(0);
        let total = translated.saturating_add(pending);
        if total == 0 || translated.saturating_mul(100) < total.saturating_mul(threshold_percent) {
            continue;
        }
        skipped += pending;
        en_only.remove(&namespace);
    }
    skipped
}

/// 以工作階段可序列化的摘要格式記錄模式，避免把 enum 直接暴露給舊版 UI。
pub fn mode_note(mode: TranslationMode, skipped: usize) -> String {
    match mode {
        TranslationMode::Append => "模式：接續翻譯；已有譯名與記憶優先重用。".into(),
        TranslationMode::SkipIfComplete => format!(
            "模式：{}；本次略過 {} 條已達完成門檻的缺漏。",
            mode.label(), skipped
        ),
        TranslationMode::Force => "模式：重新翻譯缺漏；本次不使用既有翻譯記憶，但仍會套用術語表與格式護盾。".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn map(pairs: &[(&str, &[(&str, &str)])]) -> LangMap {
        pairs
            .iter()
            .map(|(ns, entries)| {
                (
                    (*ns).into(),
                    entries
                        .iter()
                        .map(|(key, value)| ((*key).into(), (*value).into()))
                        .collect::<HashMap<_, _>>(),
                )
            })
            .collect()
    }

    #[test]
    fn skip_only_removes_namespaces_at_threshold() {
        let zh = map(&[("done", &[("a", "甲"), ("b", "乙"), ("c", "丙")])]);
        let mut pending = map(&[("done", &[("d", "D")]), ("small", &[("a", "A")])]);
        assert_eq!(skip_complete_namespaces(&zh, &mut pending, 75), 1);
        assert!(!pending.contains_key("done"));
        assert!(pending.contains_key("small"));
    }
}
