//! 社群共享術語表。
//!
//! 共享術語與共享 TM 分開保存。只有同一術語被同一整合包或多個整合包重複
//! 確認，且沒有未解決衝突時才會回傳給桌面工具；單一來源不會直接變成全域
//! 強制譯名，避免一個錯誤翻譯污染所有使用者。

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};

use super::hashutil::sha256_hex;
use super::placeholder;
use super::secrets::MANAGED_BASE_URL;
use super::translation_quality::is_usable_zh;
use super::translation_scope::TranslationScope;

const MAX_ITEMS: usize = 3000;
const MAX_SOURCE_LEN: usize = 160;
const MAX_ZH_LEN: usize = 400;

#[derive(Clone, Debug)]
pub struct SharedGlossaryJob {
    pub source: String,
    pub context: Option<String>,
    pub scope: Option<TranslationScope>,
}

#[derive(Clone, Debug)]
pub struct SharedGlossaryEntry {
    pub source: String,
    pub translated: String,
    pub context: Option<String>,
    pub scope: TranslationScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LookupStatus {
    #[default]
    Skipped,
    Failed,
    Empty,
    Hits,
}

#[derive(Debug, Clone, Default)]
pub struct LookupResult {
    pub hits: HashMap<usize, String>,
    pub status: LookupStatus,
    pub queried: usize,
}

impl LookupResult {
    pub fn player_note(&self) -> String {
        match self.status {
            LookupStatus::Skipped => "社群共享術語：本次未查詢".into(),
            LookupStatus::Failed => "社群共享術語：查詢失敗（已略過）".into(),
            LookupStatus::Empty => format!("社群共享術語：已連線但 0 命中（查 {} 條）", self.queried),
            LookupStatus::Hits => format!(
                "社群共享術語命中 {} 條（查 {} 條）",
                self.hits.len(),
                self.queried
            ),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ContributeResult {
    pub attempted: usize,
    pub accepted: usize,
    pub conflicts: usize,
    pub failed: bool,
}

impl ContributeResult {
    pub fn player_note(&self) -> Option<String> {
        if self.attempted == 0 && !self.failed {
            return None;
        }
        if self.failed {
            return Some("社群共享術語：貢獻失敗已略過".into());
        }
        Some(format!(
            "已匿名貢獻共享術語 accepted＝{}（衝突 {}，送出 {}）",
            self.accepted, self.conflicts, self.attempted
        ))
    }
}

pub fn glossary_hash(source: &str, context: Option<&str>) -> String {
    let source = normalize(source);
    let context = context.unwrap_or("").trim();
    let value = format!("{}\0{}", source, context);
    sha256_hex(value.as_bytes())[..24].to_string()
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .ok()
}

fn base() -> String {
    MANAGED_BASE_URL.trim_end_matches('/').to_string()
}

#[allow(dead_code)]
pub fn lookup(jobs: &[SharedGlossaryJob]) -> HashMap<usize, String> {
    lookup_detailed(jobs).hits
}

pub fn lookup_detailed(jobs: &[SharedGlossaryJob]) -> LookupResult {
    if jobs.is_empty() || super::shared_tm::skip_shared_lookup() {
        return LookupResult {
            hits: HashMap::new(),
            status: LookupStatus::Skipped,
            queried: jobs.len(),
        };
    }
    let Some(client) = client() else {
        return LookupResult {
            hits: HashMap::new(),
            status: LookupStatus::Failed,
            queried: jobs.len(),
        };
    };
    let mut hashes = Vec::with_capacity(jobs.len());
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for job in jobs {
        let gh = glossary_hash(&job.source, job.context.as_deref());
        hashes.push(gh.clone());
        if !seen.insert(gh.clone()) {
            continue;
        }
        let mut item = json!({
            "gh": gh,
            "ctx": job.context,
        });
        if let Some(scope) = &job.scope {
            item["pk"] = json!(scope.pack_key.clone());
            item["pn"] = json!(scope.pack_name.clone());
        }
        items.push(item);
    }
    let mut found = HashMap::new();
    let mut any_ok = false;
    let mut any_fail = false;
    for chunk in items.chunks(MAX_ITEMS) {
        let response = match client
            .post(format!("{}/glossary/lookup", base()))
            .json(&json!({ "items": chunk }))
            .send()
        {
            Ok(response) if response.status().is_success() => {
                any_ok = true;
                response
            }
            _ => {
                any_fail = true;
                continue;
            }
        };
        let value: Value = match response.json() {
            Ok(value) => value,
            Err(_) => {
                any_fail = true;
                continue;
            }
        };
        if let Some(hits) = value.get("hits").and_then(Value::as_object) {
            for (hash, translated) in hits {
                if let Some(text) = translated.as_str() {
                    if !text.trim().is_empty() {
                        found.insert(hash.clone(), text.to_string());
                    }
                }
            }
        }
    }
    let mut result = HashMap::new();
    for (index, hash) in hashes.iter().enumerate() {
        if let Some(text) = found.get(hash) {
            result.insert(index, text.clone());
        }
    }
    let status = if !any_ok && any_fail {
        LookupStatus::Failed
    } else if result.is_empty() {
        LookupStatus::Empty
    } else {
        LookupStatus::Hits
    };
    LookupResult {
        hits: result,
        status,
        queried: jobs.len(),
    }
}

pub fn contribute(entries: &[SharedGlossaryEntry]) -> ContributeResult {
    if entries.is_empty() {
        return ContributeResult::default();
    }
    let Some(client) = client() else {
        return ContributeResult {
            failed: true,
            ..ContributeResult::default()
        };
    };
    let items: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            let source = entry.source.trim();
            let translated = entry.translated.trim();
            entry.scope.is_known()
                && !source.is_empty()
                && source.len() <= MAX_SOURCE_LEN
                && !translated.is_empty()
                && translated.len() <= MAX_ZH_LEN
                && source != translated
                && placeholder::is_compatible(source, translated)
                && is_usable_zh(source, translated)
        })
        .map(|entry| {
            json!({
                "gh": glossary_hash(&entry.source, entry.context.as_deref()),
                "ctx": entry.context,
                "zh": entry.translated.trim(),
                "pk": entry.scope.pack_key,
                "pn": entry.scope.pack_name,
            })
        })
        .collect();
    if items.is_empty() {
        return ContributeResult::default();
    }
    let attempted = items.len();
    let mut accepted = 0usize;
    let mut conflicts = 0usize;
    let mut any_ok = false;
    let mut any_fail = false;
    for chunk in items.chunks(MAX_ITEMS) {
        match client
            .post(format!("{}/glossary/contribute", base()))
            .json(&json!({ "items": chunk }))
            .send()
        {
            Ok(response) if response.status().is_success() => {
                any_ok = true;
                if let Ok(value) = response.json::<Value>() {
                    accepted += value
                        .get("accepted")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize;
                    conflicts += value
                        .get("conflicts")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize;
                }
            }
            _ => any_fail = true,
        }
    }
    ContributeResult {
        attempted,
        accepted,
        conflicts,
        failed: any_fail && !any_ok,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glossary_hash_normalizes_spaces_and_context() {
        assert_eq!(
            glossary_hash("  Diamond   Sword ", Some("物品名")),
            glossary_hash("Diamond Sword", Some("物品名"))
        );
        assert_ne!(
            glossary_hash("Diamond Sword", Some("物品名")),
            glossary_hash("Diamond Sword", Some("提示說明"))
        );
    }

    #[test]
    fn empty_inputs_are_noops() {
        assert!(lookup(&[]).is_empty());
        assert_eq!(lookup_detailed(&[]).status, LookupStatus::Skipped);
        assert_eq!(contribute(&[]).attempted, 0);
    }
}
