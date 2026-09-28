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
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::placeholder;
use super::mech_tokens::is_poisoned_mech_translation;
use super::translation_quality::is_usable_zh;

/// 上限；超過就不再收新條目（避免無限長大拖慢啟動）。
const MAX_ENTRIES: usize = 300_000;
/// 過長的字串（整頁書本內容）不進記憶庫，重用機率低又佔空間。
const MAX_SOURCE_LEN: usize = 1200;

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
    super::paths::resolve_file(Path::new("tm.json"))
}

impl Tm {
    /// 從磁碟載入；檔案不存在或損壞都回傳空記憶庫（絕不讓翻譯流程失敗）。
    pub fn load() -> Self {
        Self::load_from(&tm_path())
    }

    /// 從指定檔案載入。舊版（沒有 `version`、沒有上下文鍵）的記憶檔照樣可讀。
    pub fn load_from(path: &Path) -> Self {
        let entries = fs::read_to_string(path)
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
        // B2：過不了 output guard（太長、圖示字、色碼…）的譯文不進記憶
        if !placeholder::is_compatible(s, t) || !is_usable_zh(s, t) || !super::output_guard::passes(s, t) {
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
            || !super::output_guard::passes(s, t)
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
        self.save_to(&tm_path())
    }

    /// 存到指定檔案（`save` 用預設位置；測試用暫存位置）。
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if self.added == 0 {
            return Ok(());
        }
        static SAVE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        // B4：別的執行緒寫到一半當掉（鎖中毒）時照樣寫，不讓這一輪的譯文存不進去
        let _lock = SAVE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let path = path.to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // 多個補充來源可能同時完成；把磁碟上剛寫入的條目合併回來，避免後完成的執行緒覆蓋前一批。
        let mut entries = self.entries.clone();
        if let Ok(existing) = fs::read_to_string(&path) {
            if let Ok(file) = serde_json::from_str::<TmFile>(&existing) {
                for (key, value) in file.entries {
                    entries.entry(key).or_insert(value);
                }
            }
        }
        let file = TmFile {
            version: 1,
            entries,
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

/// B4：逐批落盤的門檻——累積這麼多新條目、或距上次落盤超過這麼久，就先寫一次磁碟。
/// 每次寫都是整份重寫（含合併），所以不每批都寫（待使用者實測）。
const FLUSH_EVERY_ADDED: usize = 50;
const FLUSH_EVERY: std::time::Duration = std::time::Duration::from_secs(20);

/// 提前 return／panic 時仍盡力把本輪新增寫回磁碟；B4 起翻譯中也逐批落盤（`flush_if_due`）。
pub struct TmSaveGuard {
    tm: Tm,
    /// 上次落盤時的新增數
    flushed_added: usize,
    last_flush: std::time::Instant,
    /// 測試用的存放位置（`None`＝預設位置）
    path: Option<PathBuf>,
}

impl TmSaveGuard {
    pub fn new(tm: Tm) -> Self {
        Self {
            tm,
            flushed_added: 0,
            last_flush: std::time::Instant::now(),
            path: None,
        }
    }

    #[cfg(test)]
    pub fn with_path(tm: Tm, path: PathBuf) -> Self {
        Self {
            tm,
            flushed_added: 0,
            last_flush: std::time::Instant::now(),
            path: Some(path),
        }
    }

    fn save_now(&self) -> Result<(), String> {
        match &self.path {
            Some(path) => self.tm.save_to(path),
            None => self.tm.save(),
        }
    }

    /// 有新條目就寫磁碟。回傳這次有沒有寫。
    pub fn flush(&mut self) -> Result<bool, String> {
        if self.tm.added == self.flushed_added {
            return Ok(false);
        }
        self.save_now()?;
        self.flushed_added = self.tm.added;
        self.last_flush = std::time::Instant::now();
        Ok(true)
    }

    /// 逐批落盤：新條目夠多或時間夠久才寫（程式中途被關掉，已翻好的也留得住）。
    pub fn flush_if_due(&mut self) -> Result<bool, String> {
        let fresh = self.tm.added.saturating_sub(self.flushed_added);
        if fresh == 0 || (fresh < FLUSH_EVERY_ADDED && self.last_flush.elapsed() < FLUSH_EVERY) {
            return Ok(false);
        }
        self.flush()
    }
}

impl std::ops::Deref for TmSaveGuard {
    type Target = Tm;
    fn deref(&self) -> &Tm {
        &self.tm
    }
}

impl std::ops::DerefMut for TmSaveGuard {
    fn deref_mut(&mut self) -> &mut Tm {
        &mut self.tm
    }
}

impl Drop for TmSaveGuard {
    fn drop(&mut self) {
        let _ = self.save_now();
    }
}

fn storage_key(source: &str, context: Option<&str>) -> String {
    // Align with shared_tm::normalize_source (whitespace fold + NFC).
    let normalized = super::shared_tm::normalize_source(source);
    match context.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => format!("{normalized}\u{0}{value}"),
        None => normalized,
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

    fn scratch_tm(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mcpl-b4-tm-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("tm.json")
    }

    #[test]
    fn b4_translations_are_flushed_to_disk_batch_by_batch() {
        // 翻到一半被工作管理員結束：已經翻好的要已經在磁碟上，重開後接續補完才用得到
        let path = scratch_tm("flush");
        let mut guard = TmSaveGuard::with_path(Tm::default(), path.clone());
        for i in 0..FLUSH_EVERY_ADDED {
            guard.insert(&format!("Zq Flush Item {i}"), &format!("測試條目{}", "甲".repeat(i % 3 + 1)));
        }
        assert!(guard.flush_if_due().unwrap(), "累積夠多新條目就要落盤");
        let on_disk = Tm::load_from(&path);
        assert!(on_disk.entries.contains_key(&storage_key("Zq Flush Item 0", None)));
        // 才一條新的、時間也還沒到：先不寫（避免每批都整份重寫）
        guard.insert("Zq Flush Late", "晚到的條目");
        assert!(!guard.flush_if_due().unwrap());
        // 強制落盤
        assert!(guard.flush().unwrap());
        assert!(Tm::load_from(&path).entries.contains_key(&storage_key("Zq Flush Late", None)));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn b4_guard_still_saves_on_drop() {
        let path = scratch_tm("drop");
        {
            let mut guard = TmSaveGuard::with_path(Tm::default(), path.clone());
            guard.insert("Zq Drop Item", "丟棄前保存");
        }
        assert!(Tm::load_from(&path).entries.contains_key(&storage_key("Zq Drop Item", None)));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

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
    fn lookup_collapses_internal_whitespace() {
        let mut tm = blank();
        tm.insert("Diamond  Sword", "鑽石劍");
        assert_eq!(tm.get("Diamond Sword").as_deref(), Some("鑽石劍"));
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
