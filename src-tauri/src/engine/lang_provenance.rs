//! 語言條目來源標記：決定是否計入「台灣可玩覆蓋」與是否再跑 s2twp。

use std::collections::HashMap;

use super::jar_scan::LangMap;

/// 單條譯文從哪裡來。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LangSource {
    /// 原生 zh_tw
    Tw,
    /// zh_cn 經 s2twp
    CnConverted,
    /// 僅 zh_hk 提示，經 s2twp 補缺（**不算**台灣已完成）
    HkHint,
    /// 參考包合併
    RefPack,
    /// AI 補譯
    Ai,
    /// 術語表 exact
    #[allow(dead_code)]
    Glossary,
    /// 翻譯記憶
    #[allow(dead_code)]
    Tm,
}

impl LangSource {
    /// skip-if-complete／覆蓋報告「台灣已覆蓋」只算這些。
    pub fn is_tw_playable(self) -> bool {
        matches!(
            self,
            Self::Tw
                | Self::CnConverted
                | Self::RefPack
                | Self::Ai
                | Self::Glossary
                | Self::Tm
        )
    }

    /// 合併後／寫入前是否應再跑 s2twp（原生台繁不再轉）。
    pub fn needs_s2tw(self) -> bool {
        matches!(self, Self::CnConverted | Self::HkHint | Self::Ai)
    }
}

/// namespace → key → source
pub type ProvenanceMap = HashMap<String, HashMap<String, LangSource>>;

pub fn set_source(prov: &mut ProvenanceMap, ns: &str, key: &str, source: LangSource) {
    prov.entry(ns.to_string())
        .or_default()
        .insert(key.to_string(), source);
}

pub fn get_source(prov: &ProvenanceMap, ns: &str, key: &str) -> Option<LangSource> {
    prov.get(ns).and_then(|m| m.get(key)).copied()
}

/// 僅統計 playable 來源的有效中文 key 數（給 skip／報告）。
#[allow(dead_code)]
pub fn playable_usable_count(zh: &LangMap, prov: &ProvenanceMap) -> HashMap<String, usize> {
    use super::translation_quality::is_usable_zh;
    let mut out = HashMap::new();
    for (ns, map) in zh {
        let mut n = 0usize;
        for (k, v) in map {
            let src = get_source(prov, ns, k).unwrap_or(LangSource::Tw);
            if src.is_tw_playable() && is_usable_zh("", v) {
                n += 1;
            }
        }
        out.insert(ns.clone(), n);
    }
    out
}

#[allow(dead_code)]
pub fn count_by_source(prov: &ProvenanceMap, want: LangSource) -> usize {
    prov.values()
        .map(|m| m.values().filter(|s| **s == want).count())
        .sum()
}

pub fn count_playable(prov: &ProvenanceMap) -> usize {
    prov.values()
        .map(|m| m.values().filter(|s| s.is_tw_playable()).count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hk_hint_not_playable() {
        assert!(!LangSource::HkHint.is_tw_playable());
        assert!(LangSource::Tw.is_tw_playable());
        assert!(LangSource::CnConverted.is_tw_playable());
        assert!(LangSource::HkHint.needs_s2tw());
        assert!(!LangSource::Tw.needs_s2tw());
    }
}
