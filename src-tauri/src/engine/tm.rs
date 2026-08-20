//! 翻譯記憶（Translation Memory）。
//!
//! 整合包之間重疊極大：同一批常見模組（JEI、Create、Mekanism…）在每個包裡都要再翻一次。
//! 把「英文 → 已驗證譯文」存在本機，下次直接命中：
//! - 省 API 費用與等待時間（第二個整合包常有 5–7 成命中）
//! - 同一句話在不同包翻出同樣結果，不會這包叫「熔爐」下包叫「鎔爐」
//!
//! 存放：`%APPDATA%\modpack-i18n-tool\tm.json`
//!
//! 設計取捨：沒有上下文提示的短字串沿用英文原文鍵；有物品名／按鈕／描述等上下文提示時，
//! 會把上下文一起放進鍵，避免同一句英文在不同位置誤套用。舊版無上下文條目仍可讀取。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use super::placeholder;
use super::mech_tokens::is_poisoned_mech_translation;
use super::translation_quality::is_usable_zh;

/// 上限；超過就不再收新條目（避免無限長大拖慢啟動）。
const MAX_ENTRIES: usize = 300_000;
/// 過長的字串（整頁書本內容）不進記憶庫，重用機率低又佔空間。
const MAX_SOURCE_LEN: usize = 400;

#[derive(Debug, Default, Clone)]
pub struct Tm {
    entries: HashMap<String, String>,
    /// 本次新增的條目數
    added: usize,
    rejected: usize,
    /// 本次命中的條目數
    hits: usize,
    full: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TmStats {
    pub hits: usize,
    pub added: usize,
    pub rejected: usize,
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
            rejected: 0,
            hits: 0,
            full: false,
        }
    }

    /// 查詢並計入命中統計。
    pub fn get(&mut self, source: &str) -> Option<String> {
        self.get_with_context(source, None)
    }

    /// 依原文與可選上下文查詢；有上下文時不會誤用無關語境的同句翻譯。
    pub fn get_with_context(&mut self, source: &str, context: Option<&str>) -> Option<String> {
        let key = storage_key(source, context);
        let hit = self.entries.get(&key)?.clone();
        // 記憶庫是舊資料：仍要確認佔位符對得上目前這條原文
        if !placeholder::is_compatible(source, &hit) {
            self.rejected += 1;
            return None;
        }
        if !is_usable_zh(source, &hit) {
            self.rejected += 1;
            return None;
        }
        if is_poisoned_mech_translation(source, &hit) {
            self.rejected += 1;
            return None;
        }
        self.hits += 1;
        Some(hit)
    }

    /// 寫入一條已驗證的譯文。
    pub fn insert(&mut self, source: &str, translated: &str) {
        self.insert_with_context(source, translated, None);
    }

    pub fn insert_with_context(
        &mut self,
        source: &str,
        translated: &str,
        context: Option<&str>,
    ) {
        let s = source.trim();
        let t = translated.trim();
        if s.is_empty() || t.is_empty() || s == t {
            return;
        }
        if s.len() > MAX_SOURCE_LEN {
            return;
        }
        if !placeholder::is_compatible(s, t) || !is_usable_zh(s, t) {
            self.rejected += 1;
            return;
        }
        if is_poisoned_mech_translation(s, t) {
            self.rejected += 1;
            return;
        }
        let key = storage_key(s, context);
        if self.entries.contains_key(&key) {
            return;
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.full = true;
            return;
        }
        self.entries.insert(key, t.to_string());
        self.added += 1;
    }

    /// Force 模式使用：同一原文以新的安全譯文取代舊記憶。
    pub fn upsert(&mut self, source: &str, translated: &str) {
        self.upsert_with_context(source, translated, None);
    }

    pub fn upsert_with_context(
        &mut self,
        source: &str,
        translated: &str,
        context: Option<&str>,
    ) {
        let s = source.trim();
        let t = translated.trim();
        if s.is_empty()
            || t.is_empty()
            || s == t
            || s.len() > MAX_SOURCE_LEN
            || !placeholder::is_compatible(s, t)
            || !is_usable_zh(s, t)
            || is_poisoned_mech_translation(s, t)
        {
            self.rejected += 1;
            return;
        }
        let key = storage_key(s, context);
        if self.entries.get(&key).is_some_and(|old| old == t) {
            return;
        }
        if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key) {
            self.full = true;
            return;
        }
        self.entries.insert(key, t.to_string());
        self.added += 1;
    }

    pub fn stats(&self) -> TmStats {
        TmStats {
            hits: self.hits,
            added: self.added,
            rejected: self.rejected,
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
        if s.hits == 0 && s.added == 0 && s.rejected == 0 {
            return "翻譯記憶：本次未使用".into();
        }
        let mut n = format!(
            "翻譯記憶：命中 {} 條（省下這些 AI 呼叫）、新增 {} 條、庫存 {} 條",
            s.hits, s.added, s.total
        );
        if s.full {
            n.push_str("；已達上限，新條目未收錄");
        }
        if s.rejected > 0 {
            n.push_str(&format!("；{} 條格式不安全未寫入", s.rejected));
        }
        n
    }
}

fn storage_key(source: &str, context: Option<&str>) -> String {
    match context.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => format!("{}\u{0}{}", source.trim(), value),
        None => source.trim().to_string(),
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
        assert_eq!(tm.stats().rejected, 1);
    }

    #[test]
    fn rejects_unusable_zh_on_insert_and_lookup() {
        let mut tm = blank();
        tm.insert("Blue Journal", "Blue Journal");
        assert_eq!(tm.stats().added, 0);

        tm.entries
            .insert("Blue Journal".into(), "Blue Journal".into());
        assert!(tm.get("Blue Journal").is_none());
        assert_eq!(tm.stats().rejected, 1);
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
    fn force_upsert_replaces_a_safe_old_entry() {
        let mut tm = blank();
        tm.insert("Sword", "舊譯");
        tm.upsert("Sword", "新譯");
        assert_eq!(tm.get("Sword").as_deref(), Some("新譯"));
    }

    #[test]
    fn scoped_entries_do_not_cross_contexts() {
        let mut tm = blank();
        tm.insert_with_context("Open", "開啟", Some("按鈕"));
        tm.insert_with_context("Open", "開啟中", Some("狀態"));
        assert_eq!(
            tm.get_with_context("Open", Some("按鈕")).as_deref(),
            Some("開啟")
        );
        assert_eq!(
            tm.get_with_context("Open", Some("狀態")).as_deref(),
            Some("開啟中")
        );
        assert!(tm.get_with_context("Open", Some("物品名")).is_none());
    }

    #[test]
    fn note_mentions_savings_when_used() {
        let mut tm = blank();
        tm.insert("Sword", "劍");
        let _ = tm.get("Sword");
        assert!(tm.note().contains("命中 1"));
    }
}
