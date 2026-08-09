//! 翻譯記憶（Translation Memory）。
//!
//! 整合包之間重疊極大：同一批常見模組（JEI、Create、Mekanism…）在每個包裡都要再翻一次。
//! 把「英文 → 已驗證譯文」存在本機，下次直接命中：
//! - 省 API 費用與等待時間（第二個整合包常有 5–7 成命中）
//! - 同一句話在不同包翻出同樣結果，不會這包叫「熔爐」下包叫「鎔爐」
//!
//! 存放：`%APPDATA%\modpack-i18n-tool\tm.json`
//!
//! 設計取捨：鍵只用英文原文，不含語境。同一句英文在 Minecraft 模組圈幾乎都是同一個意思，
//! 換取的是簡單與高命中率。語境資訊仍會送進 AI prompt，只是不進記憶庫的鍵。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use super::placeholder;

/// 上限；超過就不再收新條目（避免無限長大拖慢啟動）。
const MAX_ENTRIES: usize = 300_000;
/// 過長的字串（整頁書本內容）不進記憶庫，重用機率低又佔空間。
const MAX_SOURCE_LEN: usize = 400;

#[derive(Debug, Default, Clone)]
pub struct Tm {
    entries: HashMap<String, String>,
    /// 本次新增的條目數
    added: usize,
    /// 本次命中的條目數
    hits: usize,
    full: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TmStats {
    pub hits: usize,
    pub added: usize,
    pub total: usize,
    pub full: bool,
}

pub fn tm_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("modpack-i18n-tool")
        .join("tm.json")
}

impl Tm {
    /// 從磁碟載入；檔案不存在或損壞都回傳空記憶庫（絕不讓翻譯流程失敗）。
    pub fn load() -> Self {
        let path = tm_path();
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<TmFile>(&t).ok())
            .map(|f| f.entries)
            .unwrap_or_default();
        Tm {
            entries,
            added: 0,
            hits: 0,
            full: false,
        }
    }

    /// 查詢並計入命中統計。
    pub fn get(&mut self, source: &str) -> Option<String> {
        let hit = self.entries.get(source.trim())?.clone();
        // 記憶庫是舊資料：仍要確認佔位符對得上目前這條原文
        if !placeholder::is_compatible(source, &hit) {
            return None;
        }
        self.hits += 1;
        Some(hit)
    }

    /// 寫入一條已驗證的譯文。
    pub fn insert(&mut self, source: &str, translated: &str) {
        let s = source.trim();
        let t = translated.trim();
        if s.is_empty() || t.is_empty() || s == t {
            return;
        }
        if s.len() > MAX_SOURCE_LEN {
            return;
        }
        if self.entries.contains_key(s) {
            return;
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.full = true;
            return;
        }
        self.entries.insert(s.to_string(), t.to_string());
        self.added += 1;
    }

    pub fn stats(&self) -> TmStats {
        TmStats {
            hits: self.hits,
            added: self.added,
            total: self.entries.len(),
            full: self.full,
        }
    }

    /// 存回磁碟。失敗只回報，不中斷翻譯（記憶庫是最佳化，不是真相來源）。
    pub fn save(&self) -> Result<(), String> {
        if self.added == 0 {
            return Ok(());
        }
        let path = tm_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let file = TmFile {
            version: 1,
            entries: self.entries.clone(),
        };
        let body = serde_json::to_string(&file).map_err(|e| e.to_string())?;
        // 先寫暫存再換名：中途斷電不會留下半個壞掉的記憶庫
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, body).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    pub fn note(&self) -> String {
        let s = self.stats();
        if s.hits == 0 && s.added == 0 {
            return "翻譯記憶：本次未使用".into();
        }
        let mut n = format!(
            "翻譯記憶：命中 {} 條（省下這些 AI 呼叫）、新增 {} 條、庫存 {} 條",
            s.hits, s.added, s.total
        );
        if s.full {
            n.push_str("；已達上限，新條目未收錄");
        }
        n
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct TmFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Tm {
        Tm::default()
    }

    #[test]
    fn insert_then_get_hits() {
        let mut tm = blank();
        tm.insert("Diamond Sword", "鑽石劍");
        assert_eq!(tm.get("Diamond Sword").as_deref(), Some("鑽石劍"));
        assert_eq!(tm.stats().hits, 1);
        assert_eq!(tm.stats().added, 1);
    }

    #[test]
    fn lookup_is_whitespace_tolerant() {
        let mut tm = blank();
        tm.insert("  Diamond Sword  ", "鑽石劍");
        assert_eq!(tm.get("Diamond Sword").as_deref(), Some("鑽石劍"));
    }

    #[test]
    fn miss_returns_none_and_does_not_count() {
        let mut tm = blank();
        assert!(tm.get("Nothing Here").is_none());
        assert_eq!(tm.stats().hits, 0);
    }

    #[test]
    fn rejects_stored_entry_whose_placeholders_no_longer_match() {
        let mut tm = blank();
        // 舊記憶沒有佔位符，但現在這條原文有 → 不可重用，否則遊戲會格式錯誤
        tm.insert("Deals %s damage", "造成傷害");
        assert!(tm.get("Deals %s damage").is_none());
    }

    #[test]
    fn does_not_store_untranslated_or_empty() {
        let mut tm = blank();
        tm.insert("Same", "Same");
        tm.insert("", "空");
        tm.insert("Key", "   ");
        assert_eq!(tm.stats().added, 0);
    }

    #[test]
    fn does_not_store_overly_long_sources() {
        let mut tm = blank();
        let long = "a".repeat(MAX_SOURCE_LEN + 1);
        tm.insert(&long, "很長");
        assert_eq!(tm.stats().added, 0);
    }

    #[test]
    fn first_write_wins_for_duplicate_keys() {
        let mut tm = blank();
        tm.insert("Sword", "劍");
        tm.insert("Sword", "刀");
        assert_eq!(tm.get("Sword").as_deref(), Some("劍"));
        assert_eq!(tm.stats().added, 1);
    }

    #[test]
    fn note_mentions_savings_when_used() {
        let mut tm = blank();
        tm.insert("Sword", "劍");
        let _ = tm.get("Sword");
        assert!(tm.note().contains("命中 1"));
    }
}
