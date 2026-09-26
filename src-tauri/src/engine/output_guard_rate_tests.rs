//! B2：退回率量測。資料：內建的 Minecraft 官方繁中譯名表（assets/minecraft_glossary_zh_tw.json），
//! 全是遊戲裡實際顯示的物品／方塊／效果名稱——正是短欄位長度限制最容易誤殺的一群。
//! 目標：一般譯文的退回率 < 2%。

use super::check_entry;
use std::collections::BTreeMap;

#[test]
fn b2_rejection_rate_on_official_names_is_below_two_percent() {
    let text = include_str!("../../assets/minecraft_glossary_zh_tw.json");
    let pairs: BTreeMap<String, String> = serde_json::from_str(text).expect("譯名表是合法 JSON");
    let mut rejected = Vec::new();
    for (en, zh) in &pairs {
        if let Err(reason) = check_entry(en, zh) {
            rejected.push(format!("{en} → {zh}（{}）", reason.code()));
        }
    }
    let rate = rejected.len() as f64 * 100.0 / pairs.len() as f64;
    println!(
        "B2 退回率：{}/{} = {:.2}%\n{}",
        rejected.len(),
        pairs.len(),
        rate,
        rejected.join("\n")
    );
    assert!(pairs.len() > 1000, "資料量不足：{}", pairs.len());
    assert!(rate < 2.0, "退回率 {rate:.2}% 超過 2%：{rejected:?}");
}
