//! 玩家可選的完整度授權：決定這次要掃／翻多深（輕量偏好，不是法律同意）。

use super::translation_mode::TranslationQuality;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageTier {
    Quick,
    Standard,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverageSourceFlags {
    pub jar_lang: bool,
    pub ftbquests: bool,
    pub quests_books: bool,
    pub text_overlay: bool,
    pub archive_overlay: bool,
    pub jar_patchouli: bool,
    pub jar_display: bool,
    pub origins: bool,
    pub script_literals: bool,
    /// Max 档：額外寫出「待補缺口摘要.txt」（樣本鍵清單，非完整五層盤點）。
    pub write_gap_summary: bool,
}

impl CoverageTier {
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or("standard").trim().to_ascii_lowercase().as_str() {
            "quick" | "fast" | "light" | "先翻能玩的" => Self::Quick,
            "max" | "full" | "thorough" | "盡量完整" => Self::Max,
            _ => Self::Standard,
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Max => "max",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Quick => "先翻能玩的",
            Self::Standard => "標準（建議）",
            Self::Max => "盡量完整",
        }
    }

    pub fn default_quality(self) -> TranslationQuality {
        match self {
            Self::Quick => TranslationQuality::Fast,
            Self::Standard => TranslationQuality::Balanced,
            Self::Max => TranslationQuality::Thorough,
        }
    }

    pub fn sources(self) -> CoverageSourceFlags {
        match self {
            Self::Quick => CoverageSourceFlags {
                jar_lang: true,
                ftbquests: true,
                quests_books: true,
                text_overlay: true,
                archive_overlay: false,
                jar_patchouli: false,
                jar_display: false,
                origins: false,
                script_literals: false,
                write_gap_summary: false,
            },
            Self::Standard => CoverageSourceFlags {
                jar_lang: true,
                ftbquests: true,
                quests_books: true,
                text_overlay: true,
                archive_overlay: true,
                jar_patchouli: true,
                jar_display: true,
                origins: true,
                script_literals: true,
                write_gap_summary: false,
            },
            Self::Max => CoverageSourceFlags {
                jar_lang: true,
                ftbquests: true,
                quests_books: true,
                text_overlay: true,
                archive_overlay: true,
                jar_patchouli: true,
                jar_display: true,
                origins: true,
                script_literals: true,
                write_gap_summary: true,
            },
        }
    }

    pub fn note(self) -> String {
        match self {
            Self::Quick => "完整度：先翻能玩的（物品／介面／任務主線；略過 ZIP／JAR 顯示／腳本來源）。".into(),
            Self::Standard => "完整度：標準（多數可讀文字來源；預設一般 AI 品質）。".into(),
            Self::Max => "完整度：盡量完整（來源同標準、預設較仔細品質；另寫待補缺口摘要；圖內字與程式硬編碼仍可能留下）。".into(),
        }
    }
}

/// 將子進度 pct(0–100) 映射到固定帶寬，避免多來源擠在同一百分比。
pub fn map_stage_progress(base: u8, span: u8, pct: u8) -> u8 {
    if span == 0 {
        return base.min(100);
    }
    let mapped = base as u16 + (pct as u16 * span as u16) / 100;
    mapped.min(100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tiers() {
        assert_eq!(CoverageTier::parse(Some("quick")), CoverageTier::Quick);
        assert_eq!(CoverageTier::parse(Some("max")), CoverageTier::Max);
        assert_eq!(CoverageTier::parse(None), CoverageTier::Standard);
    }

    #[test]
    fn quick_skips_deep_sources() {
        let s = CoverageTier::Quick.sources();
        assert!(s.ftbquests);
        assert!(!s.archive_overlay);
        assert!(!s.jar_display);
        assert!(!s.script_literals);
        assert!(!s.write_gap_summary);
    }

    #[test]
    fn max_writes_gap_summary_only() {
        let std = CoverageTier::Standard.sources();
        let max = CoverageTier::Max.sources();
        assert_eq!(std.archive_overlay, max.archive_overlay);
        assert_eq!(std.jar_display, max.jar_display);
        assert!(!std.write_gap_summary);
        assert!(max.write_gap_summary);
    }

    #[test]
    fn map_progress_clamps() {
        assert_eq!(map_stage_progress(90, 5, 100), 95);
        assert_eq!(map_stage_progress(97, 3, 100), 100);
        assert_eq!(map_stage_progress(50, 0, 80), 50);
    }
}
