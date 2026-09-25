//! 這次翻譯**實際會用的設定**，以及每個被改掉的欄位的原因（P0-01）。
//!
//! 舊行為：`run_one_click` 收下 UI 傳來的翻譯模式／品質／完整度，然後整組丟掉，
//! 直接寫死 `Append` + `Thorough` + `Max` + 進階解包。被忽略的選擇只寫進一行日誌，
//! 使用者在設定畫面看到的選項與實際行為不同——那不只是文案問題，
//! 它影響費用、耗時、掃描範圍，也讓「我明明選了快速」這種回報無從查起。
//!
//! 現在改成：後端仍然可以有固定範圍的產品決策，但必須把「你要的是什麼、
//! 實際用了什麼、為什麼」變成**回得去 UI 的結構化資料**，而不是一行 log。

use serde::Serialize;

use super::coverage_tier::CoverageTier;
use super::translation_mode::{TranslationMode, TranslationQuality};

/// 這次執行屬於哪一條路徑。不同路徑對使用者選擇的尊重程度不同，
/// 但每一條都必須把差異說清楚。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunIntent {
    /// 主要的一鍵翻譯：固定完整範圍，讓第一次使用的人不會因為選錯而翻不完整。
    OneClick,
    /// 補翻與修復：使用者已經知道自己在做什麼，尊重他選的模式。
    Supplement,
}

/// UI 送來的請求。全部是 Option＝「沒指定」，不是「指定了空的」。
#[derive(Debug, Clone, Default)]
pub struct RunPlanRequest {
    pub mode: Option<String>,
    pub quality: Option<String>,
    pub tier: Option<String>,
    pub advanced_unpack: Option<bool>,
}

/// 單一欄位的決定過程。`requested` 與 `effective` 不同時，`reason` 必須有值。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FieldDecision {
    pub field: String,
    /// 使用者要的（沒指定時為 None）
    pub requested: Option<String>,
    /// 實際採用的
    pub effective: String,
    pub overridden: bool,
    /// 為什麼跟你要的不一樣。給玩家看的白話，不是 enum 名稱。
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPlan {
    pub mode: String,
    pub quality: String,
    pub tier: String,
    pub advanced_unpack: bool,
    /// 逐欄位的 requested／effective／reason。UI 照這個顯示，不自己推導。
    pub decisions: Vec<FieldDecision>,
}

impl RunPlan {
    #[allow(dead_code)]
    pub fn translation_mode(&self) -> TranslationMode {
        TranslationMode::parse(Some(&self.mode))
    }
    #[allow(dead_code)]
    pub fn translation_quality(&self) -> TranslationQuality {
        TranslationQuality::parse(Some(&self.quality))
    }
    pub fn coverage_tier(&self) -> CoverageTier {
        CoverageTier::parse(Some(&self.tier))
    }
    /// 有沒有任何欄位被後端改掉。UI 用它決定要不要顯示說明。
    pub fn has_overrides(&self) -> bool {
        self.decisions.iter().any(|d| d.overridden)
    }
    /// 被改掉的欄位摘要，可直接寫進日誌或結果頁。
    pub fn override_summary(&self) -> Option<String> {
        let lines: Vec<String> = self
            .decisions
            .iter()
            .filter(|d| d.overridden)
            .map(|d| {
                let requested = d.requested.clone().unwrap_or_else(|| "未指定".into());
                format!("{}：你選的是「{}」，這次實際用「{}」——{}", d.field, requested, d.effective, d.reason.clone().unwrap_or_default())
            })
            .collect();
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }
}

fn decide(
    field: &str,
    requested_raw: Option<&str>,
    requested_label: Option<String>,
    effective_label: String,
    reason: Option<&str>,
) -> FieldDecision {
    // 使用者沒指定就不算「被改掉」——沒選過的東西談不上被忽略。
    let specified = requested_raw.map(|s| !s.trim().is_empty()).unwrap_or(false);
    let overridden = specified && requested_label.as_deref() != Some(effective_label.as_str());
    FieldDecision {
        field: field.to_string(),
        requested: if specified { requested_label } else { None },
        effective: effective_label,
        overridden,
        reason: if overridden { reason.map(|s| s.to_string()) } else { None },
    }
}

/// 把 UI 請求解析成這次真正要用的計畫。
///
/// 這是**唯一**決定翻譯模式／品質／完整度的地方。任何路徑都不得再自己寫死一組值，
/// 否則就會回到「設定畫面看似可選、實際行為不同」的狀態。
pub fn resolve(intent: RunIntent, request: &RunPlanRequest) -> RunPlan {
    let requested_mode = TranslationMode::parse(request.mode.as_deref());
    let requested_tier = CoverageTier::parse(request.tier.as_deref());
    let requested_quality = if request
        .quality
        .as_deref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        requested_tier.default_quality()
    } else {
        TranslationQuality::parse(request.quality.as_deref())
    };

    match intent {
        RunIntent::OneClick => {
            // 產品決策：一鍵翻譯固定跑完整範圍。保留這個決策，但把它講清楚。
            const WHY_SCOPE: &str =
                "一鍵翻譯固定跑最完整的範圍，避免第一次使用的人因為選到較小範圍而以為工具漏翻。要自己控制範圍請用「補充漏翻」。";
            const WHY_MODE: &str =
                "一鍵翻譯採接續模式，已翻好的不會重翻，才不會重複花 AI 費用。要強制重翻請用「補充漏翻」的強制選項。";

            let mode = TranslationMode::Append;
            let quality = TranslationQuality::Thorough;
            let tier = CoverageTier::Max;
            RunPlan {
                mode: mode.value().to_string(),
                quality: quality.value().to_string(),
                tier: tier.value().to_string(),
                advanced_unpack: true,
                decisions: vec![
                    decide(
                        "翻譯模式",
                        request.mode.as_deref(),
                        Some(requested_mode.label().to_string()),
                        mode.label().to_string(),
                        Some(WHY_MODE),
                    ),
                    decide(
                        "翻譯品質",
                        request.quality.as_deref(),
                        Some(requested_quality.label().to_string()),
                        quality.label().to_string(),
                        Some(WHY_SCOPE),
                    ),
                    decide(
                        "完整度",
                        request.tier.as_deref(),
                        Some(requested_tier.label().to_string()),
                        tier.label().to_string(),
                        Some(WHY_SCOPE),
                    ),
                    decide(
                        "進階解包",
                        request.advanced_unpack.map(|_| "指定").as_deref(),
                        request.advanced_unpack.map(|v| bool_label(v).to_string()),
                        bool_label(true).to_string(),
                        Some(WHY_SCOPE),
                    ),
                ],
            }
        }
        RunIntent::Supplement => {
            // 補翻／修復尊重使用者的選擇：他已經看過一次結果才來按這顆。
            let advanced_unpack = request.advanced_unpack.unwrap_or(true);
            RunPlan {
                mode: requested_mode.value().to_string(),
                quality: requested_quality.value().to_string(),
                tier: requested_tier.value().to_string(),
                advanced_unpack,
                decisions: vec![
                    decide("翻譯模式", request.mode.as_deref(), Some(requested_mode.label().to_string()), requested_mode.label().to_string(), None),
                    decide("翻譯品質", request.quality.as_deref(), Some(requested_quality.label().to_string()), requested_quality.label().to_string(), None),
                    decide("完整度", request.tier.as_deref(), Some(requested_tier.label().to_string()), requested_tier.label().to_string(), None),
                    decide(
                        "進階解包",
                        request.advanced_unpack.map(|_| "指定").as_deref(),
                        request.advanced_unpack.map(|v| bool_label(v).to_string()),
                        bool_label(advanced_unpack).to_string(),
                        None,
                    ),
                ],
            }
        }
    }
}

fn bool_label(value: bool) -> &'static str {
    if value { "開啟" } else { "關閉" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(mode: &str, quality: &str, tier: &str) -> RunPlanRequest {
        RunPlanRequest {
            mode: Some(mode.into()),
            quality: Some(quality.into()),
            tier: Some(tier.into()),
            advanced_unpack: None,
        }
    }

    #[test]
    fn one_click_keeps_fixed_scope_but_reports_every_override() {
        // 產品決策仍是固定完整範圍——重點是它現在會誠實回報，不是靜默丟掉。
        let plan = resolve(RunIntent::OneClick, &req("skip", "fast", "quick"));
        assert_eq!(plan.coverage_tier(), CoverageTier::Max);
        assert!(plan.advanced_unpack);
        assert!(plan.has_overrides());

        for field in ["翻譯模式", "翻譯品質", "完整度"] {
            let d = plan.decisions.iter().find(|d| d.field == field).expect(field);
            assert!(d.overridden, "{field} 應標記為被改掉");
            assert!(d.requested.is_some(), "{field} 要記得使用者原本要什麼");
            let reason = d.reason.as_deref().unwrap_or("");
            assert!(!reason.is_empty(), "{field} 被改掉就必須有原因");
            // 原因要是給人看的白話，不是 enum 名稱
            assert!(!reason.contains("CoverageTier"), "{field} 的原因不該是內部代號");
        }

        let summary = plan.override_summary().expect("有覆寫就要有摘要");
        assert!(summary.contains("你選的是"));
    }

    #[test]
    fn matching_choice_is_not_reported_as_overridden() {
        // 選的剛好就是實際要用的，不該讓使用者以為被改掉。
        let plan = resolve(RunIntent::OneClick, &req("append", "thorough", "max"));
        assert!(!plan.has_overrides(), "決定值相同時不該標成覆寫：{:?}", plan.decisions);
        assert!(plan.override_summary().is_none());
    }

    #[test]
    fn unspecified_fields_are_not_overrides() {
        // 沒選過的東西談不上「被忽略」。
        let plan = resolve(RunIntent::OneClick, &RunPlanRequest::default());
        for d in &plan.decisions {
            assert!(!d.overridden, "{} 未指定卻被標成覆寫", d.field);
            assert!(d.requested.is_none());
        }
        assert!(!plan.has_overrides());
    }

    #[test]
    fn supplement_honours_every_user_choice() {
        // 補翻是使用者看過結果後的第二次動作，這裡不該再替他決定。
        for (mode, tier) in [("force", "quick"), ("skip", "standard"), ("append", "max")] {
            let plan = resolve(RunIntent::Supplement, &req(mode, "fast", tier));
            assert!(!plan.has_overrides(), "補翻不得覆寫使用者選擇：{mode}/{tier}");
            assert_eq!(plan.coverage_tier(), CoverageTier::parse(Some(tier)));
            assert_eq!(plan.translation_mode(), TranslationMode::parse(Some(mode)));
            assert_eq!(plan.translation_quality(), TranslationQuality::Fast);
        }
    }

    #[test]
    fn advanced_unpack_choice_is_recorded_on_both_paths() {
        let off = RunPlanRequest { advanced_unpack: Some(false), ..Default::default() };
        // 一鍵：固定開啟，但要說出使用者原本要關
        let one_click = resolve(RunIntent::OneClick, &off);
        assert!(one_click.advanced_unpack);
        let d = one_click.decisions.iter().find(|d| d.field == "進階解包").unwrap();
        assert!(d.overridden);
        assert_eq!(d.requested.as_deref(), Some("關閉"));
        assert_eq!(d.effective, "開啟");

        // 補翻：尊重使用者
        let supplement = resolve(RunIntent::Supplement, &off);
        assert!(!supplement.advanced_unpack);
        assert!(!supplement.has_overrides());
    }

    #[test]
    fn quality_defaults_follow_the_requested_tier() {
        // 沒指定品質時跟著完整度走，不是硬塞一個值。
        let plan = resolve(
            RunIntent::Supplement,
            &RunPlanRequest { tier: Some("quick".into()), ..Default::default() },
        );
        assert_eq!(
            plan.translation_quality(),
            CoverageTier::parse(Some("quick")).default_quality()
        );
    }

    #[test]
    fn plan_serialises_for_the_ui() {
        let plan = resolve(RunIntent::OneClick, &req("skip", "fast", "quick"));
        let value = serde_json::to_value(&plan).expect("serialise");
        assert!(value.get("decisions").and_then(|d| d.as_array()).is_some());
        assert!(value.get("advancedUnpack").is_some(), "欄位要用 camelCase 給前端");
        let first = &value["decisions"][0];
        for key in ["field", "requested", "effective", "overridden", "reason"] {
            assert!(first.get(key).is_some(), "decisions 缺欄位 {key}");
        }
    }
}
