//! 回歸語料的守衛測試（僅 `#[cfg(test)]` 編譯，不進出貨 binary）。
//!
//! Phase 0 的職責是**建立語料並證明它可安全進版控**，不是實作分類器——
//! 依 `expected` 逐條驗證 eligibility 是 Phase 2 的工作。
//!
//! 這裡守住兩件事：
//! 1. 語料結構完整（欄位齊全、`expected` 只有三種值、七類情境都在）。
//! 2. 語料**不含機密與私人路徑**（總規劃 Phase 0 驗收條件）。

use std::path::PathBuf;

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(relative)
}

fn candidates() -> Vec<serde_json::Value> {
    let path = fixture_path("prominence/candidates.jsonl");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("讀不到語料 {}：{e}", path.display()));
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, line)| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("第 {} 行不是合法 JSON：{e}", i + 1))
        })
        .collect()
}

#[test]
fn every_candidate_has_the_required_shape() {
    let rows = candidates();
    assert!(rows.len() >= 40, "語料太少，涵蓋不了七類情境");

    for row in &rows {
        for field in ["id", "category", "source_kind", "namespace", "logical_key", "text", "expected", "reason"] {
            assert!(
                row.get(field).and_then(|v| v.as_str()).is_some(),
                "候選 {:?} 缺少欄位 {field}",
                row.get("id")
            );
        }
        let expected = row["expected"].as_str().unwrap();
        assert!(
            matches!(expected, "Keep" | "Translate" | "Review"),
            "候選 {:?} 的 expected=\"{expected}\" 不在允許集合內",
            row["id"]
        );
        // reason 是要能寫進報告給人看的原因碼，不得留空
        assert!(!row["reason"].as_str().unwrap().is_empty());
    }
}

#[test]
fn candidate_ids_are_unique() {
    let rows = candidates();
    let mut ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total, "語料有重複的 id，對帳時會漏算");
}

#[test]
fn all_seven_phase0_categories_are_covered() {
    // 總規劃 Phase 0 第 3 項點名的七類。
    let rows = candidates();
    let present: Vec<&str> = rows.iter().map(|r| r["category"].as_str().unwrap()).collect();
    for required in [
        "dotted_key",
        "placeholder",
        "signed_jar",
        "ftbquests",
        "patchouli",
        "quota_exhausted",
        "parser_error",
        "case_sensitive_key",
    ] {
        assert!(present.contains(&required), "語料缺少「{required}」情境");
    }
}

#[test]
fn case_sensitive_sibling_keys_are_preserved() {
    // `text.modern_industrialization.Eu` 與 `...eu` 只差大小寫。
    // 這對鍵是實際輸出裡讓 PowerShell JSON consumer 解析失敗的來源，必須原樣保留。
    let rows = candidates();
    let keys: Vec<&str> = rows.iter().map(|r| r["logical_key"].as_str().unwrap()).collect();
    assert!(keys.contains(&"text.modern_industrialization.Eu"));
    assert!(keys.contains(&"text.modern_industrialization.eu"));
}

#[test]
fn dotted_keys_are_marked_keep_not_pending() {
    // P0-03 的核心：這些是語言 key 被當成待譯值，不是「AI 剛好沒翻」。
    // 期望值必須是 Keep，否則語料本身就把缺陷寫成了正常。
    for row in candidates().iter().filter(|r| r["category"] == "dotted_key") {
        assert_eq!(
            row["expected"], "Keep",
            "dotted key {:?} 的期望值必須是 Keep",
            row["logical_key"]
        );
    }
}

#[test]
fn quota_deferred_items_are_translatable_not_keep() {
    // 配額耗盡是「暫時沒翻」，不是「不該翻」。兩者混為一談會製造假缺口。
    for row in candidates().iter().filter(|r| r["category"] == "quota_exhausted") {
        assert_eq!(
            row["expected"], "Translate",
            "配額耗盡的候選 {:?} 不得被標成 Keep",
            row["id"]
        );
    }
}

#[test]
fn classifier_matches_every_expected_verdict_in_the_corpus() {
    // Phase 2 的核心驗收：真實 Prominence 語料的每一筆，分類結果都要對得上期望值。
    // 這是 P0-03 唯一有意義的證明——不是「我覺得規則寫對了」，而是拿真資料逐筆對帳。
    use crate::engine::eligibility::{classify, Candidate, Eligibility};

    let mut mismatches = Vec::new();
    for row in candidates() {
        let expected = row["expected"].as_str().unwrap();
        let got = classify(Candidate {
            source_kind: row["source_kind"].as_str().unwrap(),
            logical_key: row["logical_key"].as_str().unwrap(),
            text: row["text"].as_str().unwrap(),
        });
        let got_name = match got {
            Eligibility::Translate => "Translate",
            Eligibility::Keep(_) => "Keep",
            Eligibility::Review(_) => "Review",
        };
        if got_name != expected {
            mismatches.push(format!(
                "  {} [{}] key={:?} text={:?}\n    期望 {expected}，實得 {got_name}（{got:?}）",
                row["id"].as_str().unwrap(),
                row["category"].as_str().unwrap(),
                row["logical_key"].as_str().unwrap(),
                row["text"].as_str().unwrap(),
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "語料有 {} 筆分類不符：\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

#[test]
fn nothing_marked_keep_is_ever_sent_to_ai_or_shared_data() {
    // 驗收條件的另一半：被判 Keep 的東西，一條都不能流向 AI／TM／共享庫。
    use crate::engine::eligibility::{classify, Candidate};

    for row in candidates() {
        let got = classify(Candidate {
            source_kind: row["source_kind"].as_str().unwrap(),
            logical_key: row["logical_key"].as_str().unwrap(),
            text: row["text"].as_str().unwrap(),
        });
        if row["expected"] == "Keep" {
            assert!(!got.may_send_to_ai(), "{:?} 不得送 AI", row["id"]);
            assert!(!got.may_store_in_shared_data(), "{:?} 不得寫進共享資料", row["id"]);
            assert!(!got.counts_as_pending(), "{:?} 不該被算成待補缺口", row["id"]);
        }
        if row["expected"] == "Review" {
            assert!(!got.may_send_to_ai(), "{:?} 不確定就不該送 AI", row["id"]);
            assert!(got.counts_as_pending(), "{:?} 需人工確認，要算進待處理", row["id"]);
        }
    }
}

#[test]
fn fixture_contains_no_secrets_or_private_paths() {
    let path = fixture_path("prominence/candidates.jsonl");
    let readme = fixture_path("prominence/README.md");
    let mut blob = std::fs::read_to_string(&path).expect("candidates");
    blob.push_str(&std::fs::read_to_string(&readme).expect("readme"));
    let lower = blob.to_lowercase();

    // 機密樣態
    for pattern in [
        "sk-",                        // OpenAI／DeepSeek 風格 key
        "xoxb-",                      // Slack
        "bearer ",                    // 任何 Authorization header 殘留
        "-----begin",                 // PEM 私鑰
        "discord.com/api/webhooks",   // webhook URL
        "mfa.",                       // Discord token 尾段樣態
        "api_key",
        "apikey",
        "client_secret",
    ] {
        assert!(!lower.contains(pattern), "語料含疑似機密樣態：{pattern}");
    }

    // 私人路徑與使用者名稱
    for pattern in ["c:\\users\\", "c:/users/", "appdata", "/home/", "jolin"] {
        assert!(!lower.contains(pattern), "語料含私人路徑或使用者名稱：{pattern}");
    }
}
