//! 套用相關、給玩家看的文字（依實際套用結果決定說法，不寫開發者術語）。

use super::apply_instance::ApplyResult;

/// 遊戲語言代碼 → 玩家看得懂的名稱。`None`＝設定檔裡沒有語言設定（遊戲預設英文）。
pub fn language_display_name(code: Option<&str>) -> String {
    let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) else {
        return "遊戲預設的英文".into();
    };
    let name = match code.to_ascii_lowercase().as_str() {
        "en_us" | "en_gb" | "en_ca" | "en_au" | "en_nz" => "英文",
        "zh_tw" => "繁體中文（台灣）",
        "zh_hk" => "繁體中文（香港）",
        "zh_cn" => "簡體中文",
        "lzh" => "文言文",
        "ja_jp" => "日文",
        "ko_kr" => "韓文",
        "fr_fr" | "fr_ca" => "法文",
        "de_de" => "德文",
        "es_es" | "es_mx" => "西班牙文",
        "pt_br" | "pt_pt" => "葡萄牙文",
        "ru_ru" => "俄文",
        "it_it" => "義大利文",
        "vi_vn" => "越南文",
        "th_th" => "泰文",
        _ => return format!("{code}（遊戲設定裡的語言代碼，可在遊戲的語言選單確認）"),
    };
    name.into()
}

/// 翻譯／修復結論最後的「接下來」：依有沒有真的套用到遊戲決定說法。
pub fn after_run_next_steps(applied: &ApplyResult) -> String {
    if applied.is_applied() {
        "1. 已自動套用到遊戲、啟用資源包並把遊戲語言設成繁體中文（台灣）\n2. 直接開遊戲即可，不用自己切語言".into()
    } else {
        "1. 翻好了，還沒套用到遊戲（原因見最上面）\n2. 處理好之後按「套用到遊戲」，不用重新翻譯".into()
    }
}

/// 補翻／修復結束後的日誌。`action`＝「複查」「修復」等。
pub fn reapply_log_line(applied: &ApplyResult, action: &str) -> String {
    if applied.is_applied() {
        format!("{action}後已重新套用到遊戲。")
    } else {
        format!("{action}完成：翻好了，還沒套用到遊戲（處理好之後按「套用到遊戲」）。")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::apply_instance::ApplyStatus;

    fn result(status: ApplyStatus) -> ApplyResult {
        ApplyResult {
            status,
            backup_dir: String::new(),
            backup_created: false,
            backup_reused: false,
            zip_copied: None,
            jars_copied: 0,
            quests_copied: false,
            minemenu_copied: false,
            lang_set: true,
            original_lang: Some("en_us".into()),
            pending_overwrites: Vec::new(),
            unknown_files: Vec::new(),
            skipped_changed: Vec::new(),
            quarantined_files: Vec::new(),
            unconfirmed_files: Vec::new(),
            outdated_mods: Vec::new(),
            outdated_texts: Vec::new(),
            stale_outputs: Vec::new(),
            retired_files: Vec::new(),
            retire_skipped: Vec::new(),
            source_removed_texts: Vec::new(),
            unverifiable_texts: Vec::new(),
            player_summary: String::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn language_codes_become_plain_names() {
        assert_eq!(language_display_name(Some("en_us")), "英文");
        assert_eq!(language_display_name(Some("zh_tw")), "繁體中文（台灣）");
        assert_eq!(language_display_name(Some("zh_cn")), "簡體中文");
        assert_eq!(language_display_name(None), "遊戲預設的英文");
        let unknown = language_display_name(Some("tlh_aa"));
        assert!(unknown.contains("tlh_aa") && unknown.contains("語言代碼"), "{unknown}");
    }

    #[test]
    fn next_steps_follow_real_apply_status() {
        let applied = after_run_next_steps(&result(ApplyStatus::Applied));
        assert!(applied.contains("已自動套用到遊戲"), "{applied}");
        for status in [
            ApplyStatus::GameRunning,
            ApplyStatus::NoOptionsTxt,
            ApplyStatus::NeedsBackupChoice,
            ApplyStatus::NeedsOverwriteConfirm,
        ] {
            let pending = after_run_next_steps(&result(status));
            assert!(pending.contains("翻好了，還沒套用到遊戲"), "{pending}");
            assert!(!pending.contains("已自動套用到遊戲"), "{pending}");
            assert!(pending.contains("套用到遊戲"), "{pending}");
        }
    }

    #[test]
    fn reapply_log_does_not_claim_success_when_pending() {
        assert!(reapply_log_line(&result(ApplyStatus::Applied), "修復").contains("已重新套用"));
        let pending = reapply_log_line(&result(ApplyStatus::GameRunning), "修復");
        assert!(!pending.contains("已重新套用"), "{pending}");
        assert!(pending.contains("翻好了，還沒套用到遊戲"), "{pending}");
    }
}
