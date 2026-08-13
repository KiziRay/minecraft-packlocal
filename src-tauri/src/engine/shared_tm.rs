//! 共享翻譯記憶（社群，隱藏、預設開啟）。
//!
//! 讓「別人翻過的、你也用得到」——但**以 `(模組, lang key, 原文)` 為單位**，不是「同一個
//! 整合包」。命中條件是三者的雜湊全中：
//! - namespace（模組 id）→ 同一個模組
//! - lang key → 同一個上下文槽（物品名／提示／死亡訊息…）
//! - 原文 → 模組沒改過這條字串（防過期）
//! 因此重用跨整合包也安全：任何含同模組同版本的包都能受惠。
//!
//! 設計原則：
//! - **完全隱藏、無勾選、預設開**；沒有網路或服務未就緒時**靜默略過**，絕不擋翻譯、不報錯。
//! - 只送**字串**（原文＋譯文＋模組 id＋key 的雜湊），**不含任何個人資料、路徑、身分**；匿名。
//! - 收回來的每條仍會過佔位符守衛才採用（呼叫端負責）。
//!
//! 儲存在開發者的 Cloudflare Worker + R2（依模組 id 分片）。端點 URL 非機密。

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};

use super::hashutil::sha256_hex;
use super::secrets::MANAGED_BASE_URL;
use super::translation_scope::TranslationScope;

/// 單次請求最多帶幾條（保護 Worker／回應大小）。
const MAX_ITEMS: usize = 3000;
/// 譯文太長不共享（整頁書本之類，重用率低又佔空間）。
const MAX_ZH_LEN: usize = 400;

#[derive(Clone, Debug)]
pub struct SharedTmJob {
    pub namespace: String,
    pub key: String,
    pub source: String,
    pub context: Option<String>,
    pub scope: Option<TranslationScope>,
}

#[derive(Clone, Debug)]
pub struct SharedTmEntry {
    pub namespace: String,
    pub key: String,
    pub source: String,
    pub translated: String,
    pub context: Option<String>,
    pub scope: Option<TranslationScope>,
}

pub fn normalize_source(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `(ns, key, src)` 的穩定鍵。取前 24 hex（96 bits，碰撞機率可忽略）。
fn short_hash(parts: &[&str]) -> String {
    let mut buf = String::new();
    for part in parts {
        buf.push_str(part);
        buf.push('\0');
    }
    sha256_hex(buf.as_bytes())[..24].to_string()
}

pub fn keyhash(ns: &str, key: &str, src: &str) -> String {
    let normalized = normalize_source(src);
    short_hash(&[ns.trim(), key.trim(), normalized.as_str()])
}

pub fn semantic_hash(key: &str, src: &str, context: Option<&str>) -> String {
    let normalized = normalize_source(src);
    short_hash(&[key.trim(), normalized.as_str(), context.unwrap_or("").trim()])
}

fn client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        // 連不上要快速放棄，不能為了共享庫拖住整個翻譯
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .ok()
}

fn base() -> String {
    MANAGED_BASE_URL.trim_end_matches('/').to_string()
}

/// 批次查詢：回傳 job 索引 → 譯文（只含命中）。任何失敗都回空表（靜默略過）。
pub fn lookup(jobs: &[SharedTmJob]) -> HashMap<usize, String> {
    let mut out = HashMap::new();
    if jobs.is_empty() {
        return out;
    }
    let Some(client) = client() else {
        return out;
    };

    // job → kh；同時去重送出的 (ns, kh)
    let mut kh_by_job: Vec<String> = Vec::with_capacity(jobs.len());
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut items: Vec<Value> = Vec::new();
    for job in jobs {
        let kh = keyhash(&job.namespace, &job.key, &job.source);
        let sk = semantic_hash(&job.key, &job.source, job.context.as_deref());
        if seen.insert(format!("{}:{kh}", job.namespace), ()).is_none() {
            let mut item = json!({
                "ns": job.namespace,
                "kh": kh,
                "sk": sk,
                "ctx": job.context,
            });
            if let Some(scope) = &job.scope {
                item["pk"] = json!(scope.pack_key.clone());
                item["pn"] = json!(scope.pack_name.clone());
            }
            items.push(item);
        }
        kh_by_job.push(kh);
    }

    // kh → zh（跨分批合併）
    let mut found: HashMap<String, String> = HashMap::new();
    for chunk in items.chunks(MAX_ITEMS) {
        let body = json!({ "items": chunk });
        let resp = match client.post(format!("{}/tm/lookup", base())).json(&body).send() {
            Ok(r) if r.status().is_success() => r,
            _ => continue, // 靜默略過這一批
        };
        let v: Value = match resp.json() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(obj) = v.get("hits").and_then(|h| h.as_object()) {
            for (kh, zh) in obj {
                if let Some(s) = zh.as_str() {
                    if !s.trim().is_empty() {
                        found.insert(kh.clone(), s.to_string());
                    }
                }
            }
        }
    }

    for (i, kh) in kh_by_job.iter().enumerate() {
        if let Some(zh) = found.get(kh) {
            out.insert(i, zh.clone());
        }
    }
    out
}

/// 批次貢獻（fire-and-forget）：`(ns, key, src, zh)`。失敗不影響翻譯。
pub fn contribute(entries: &[SharedTmEntry]) {
    if entries.is_empty() {
        return;
    }
    let Some(client) = client() else {
        return;
    };
    let items: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            let zh = entry.translated.trim();
            !zh.is_empty()
                && zh.len() <= MAX_ZH_LEN
                && !entry.source.trim().is_empty()
                && zh != entry.source.trim()
        })
        .map(|entry| {
            let mut item = json!({
                "ns": entry.namespace.clone(),
                "kh": keyhash(&entry.namespace, &entry.key, &entry.source),
                "sk": semantic_hash(&entry.key, &entry.source, entry.context.as_deref()),
                "ctx": entry.context.clone(),
                "zh": entry.translated.trim(),
            });
            if let Some(scope) = &entry.scope {
                item["pk"] = json!(scope.pack_key.clone());
                item["pn"] = json!(scope.pack_name.clone());
            }
            item
        })
        .collect();
    if items.is_empty() {
        return;
    }
    for chunk in items.chunks(MAX_ITEMS) {
        let body = json!({ "items": chunk });
        let _ = client
            .post(format!("{}/tm/contribute", base()))
            .json(&body)
            .send();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyhash_is_stable_and_scoped() {
        let a = keyhash("create", "item.create.wrench", "Wrench");
        assert_eq!(a, keyhash("create", "item.create.wrench", "Wrench"));
        // 不同模組 → 不同鍵（同字不互串）
        assert_ne!(a, keyhash("mekanism", "item.create.wrench", "Wrench"));
        // 不同 key（上下文）→ 不同鍵
        assert_ne!(a, keyhash("create", "create.tooltip.wrench", "Wrench"));
        // 原文改了（模組更新）→ 不同鍵（不重用過期譯文）
        assert_ne!(a, keyhash("create", "item.create.wrench", "Spanner"));
        assert_eq!(a.len(), 24);
    }

    #[test]
    fn hashes_normalize_layout_but_keep_context_separate() {
        assert_eq!(
            keyhash("create", "item.create.wrench", "Wrench\n"),
            keyhash("create", "item.create.wrench", "  Wrench  ")
        );
        assert_ne!(
            semantic_hash("item.create.wrench", "Wrench", Some("物品名")),
            semantic_hash("item.create.wrench", "Wrench", Some("提示說明"))
        );
    }

    #[test]
    fn empty_inputs_are_noops() {
        assert!(lookup(&[]).is_empty());
        contribute(&[]); // 不 panic
    }
}
